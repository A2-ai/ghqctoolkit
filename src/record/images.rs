use regex::Regex;
use scraper::{Html, Selector};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::PathBuf;
use std::sync::LazyLock;

// Markdown image regex
static MD_IMG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").expect("Invalid markdown image regex")
});

// HTML image regex - required for sorting of image urls in markdown text since `scraper` does not maintain exact content
static HTML_IMG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<img[^>]+src=["']([^"']+)["'][^>]*/?>"#).expect("Invalid HTML image regex")
});

// Scraper selectors for HTML parsing
static IMG_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("img").expect("Invalid img selector")
});

/// Trait for downloading images from URLs for PDF embedding
///
/// This trait provides a testable interface for downloading images
/// while keeping the implementation details separate from the business logic.
#[cfg_attr(test, automock)]
pub trait ImageDownloader {
    /// Download an IssueImage using its HTML URL to its specified path
    ///
    /// # Arguments
    /// * `issue_image` - The IssueImage containing HTML URL and target path
    ///
    /// # Returns
    /// * `Ok(())` - If download succeeded
    /// * `Err(DownloadError)` - If download failed
    fn download_issue_image(&self, issue_image: &IssueImage) -> Result<(), DownloadError>;
}

/// HTTP implementation of the ImageDownloader trait
pub struct HttpImageDownloader;

impl ImageDownloader for HttpImageDownloader {
    fn download_issue_image(&self, issue_image: &IssueImage) -> Result<(), DownloadError> {
        let url = &issue_image.html;
        let path = &issue_image.path;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        log::debug!("Downloading {} to {}...", url, path.display());

        // Download using ureq - since JWT URLs already contain auth, we can download directly
        let response = ureq::get(url)
            .set("User-Agent", "ghqctoolkit/1.0")
            .call()?;

        let mut bytes = Vec::new();
        response.into_reader().read_to_end(&mut bytes)?;

        log::debug!("Writing {} bytes to {}", bytes.len(), path.display());
        std::fs::write(path, &bytes)?;

        Ok(())
    }
}

#[cfg(test)]
use mockall::automock;

/// Represents an image found in an issue with its text URL, HTML URL, and download path
///
/// This struct ensures one-to-one mapping between markdown text images and HTML images
/// to handle GitHub's redirect system properly.
#[derive(Debug, Clone)]
pub struct IssueImage {
    /// The URL as it appears in the markdown text
    pub text: String,
    /// The JWT-secured URL from the HTML (for downloading)
    pub html: String,
    /// The local path where the image should be downloaded
    pub path: PathBuf,
}

/// Create IssueImage structs from markdown text and HTML content
///
/// Maps markdown image URLs to HTML JWT URLs by position and generates download paths.
/// This ensures one-to-one mapping between text images and HTML images.
pub fn create_issue_images(
    markdown: &str,
    html: Option<&str>,
    base_download_dir: &std::path::Path,
) -> Vec<IssueImage> {
    let text_urls = extract_image_urls_from_markdown(markdown);
    let jwt_urls = html.map(extract_image_urls_from_html).unwrap_or_default();

    text_urls
        .into_iter()
        .enumerate()
        .map(|(index, text_url)| {
            // Use JWT URL at same index if available, otherwise use text URL
            let html_url = jwt_urls.get(index).cloned().unwrap_or_else(|| {
                log::debug!(
                    "No JWT URL at index {} for: {}, using text URL",
                    index,
                    text_url
                );
                text_url.clone()
            });

            // Generate a unique filename based on hash of text URL
            let mut hasher = DefaultHasher::new();
            text_url.hash(&mut hasher);
            let hash = hasher.finish();
            let filename = format!("image_{:x}.png", hash);
            let path = base_download_dir.join(filename);

            log::debug!(
                "Created IssueImage: text={}, html={}, path={}",
                text_url,
                html_url,
                path.display()
            );

            IssueImage {
                text: text_url,
                html: html_url,
                path,
            }
        })
        .collect()
}

/// Extract image URLs from markdown content in order of appearance
///
/// Supports both markdown image syntax and HTML img tags commonly used in GitHub issues:
/// - `![alt text](url)`
/// - `<img src="url" />`
/// - `<img width="390" height="436" alt="Image" src="url" />`
///
/// Returns URLs in the order they appear in the text, including duplicates if they appear multiple times.
pub fn extract_image_urls_from_markdown(markdown: &str) -> Vec<String> {
    let mut urls_with_positions = Vec::new();

    // Extract markdown images: ![alt](url) - use regex since scraper doesn't parse markdown
    for captures in MD_IMG_REGEX.captures_iter(markdown) {
        if let Some(url_match) = captures.get(2) {
            urls_with_positions.push((url_match.start(), url_match.as_str().to_string()));
        }
    }

    // Extract HTML img tags using scraper and find their positions in the original text
    let document = Html::parse_fragment(markdown);
    for element in document.select(&IMG_SELECTOR) {
        if let Some(src) = element.value().attr("src") {
            // Find the position of this img tag in the original markdown
            let img_html = element.html();
            log::debug!("{img_html:#?}");
            if let Some(pos) = markdown.find(&img_html) {
                urls_with_positions.push((pos, src.to_string()));
            } else {
                // Fallback: try to find just the src attribute in the text
                let src_pattern = format!("src=\"{}\"", src);
                if let Some(pos) = markdown.find(&src_pattern) {
                    urls_with_positions.push((pos, src.to_string()));
                } else {
                    // Last fallback: add at end to preserve at least the URL
                    urls_with_positions.push((markdown.len(), src.to_string()));
                }
            }
        }
    }

    // Sort by position in the document to preserve order (including duplicates)
    urls_with_positions.sort_by_key(|(pos, _)| *pos);

    // Extract just the URLs, preserving duplicates and order
    urls_with_positions
        .into_iter()
        .map(|(_, url)| url)
        .collect()
}

/// Extract image URLs from HTML content in order of appearance
///
/// GitHub HTML contains image URLs that correspond to markdown images by position,
/// not by URL base since GitHub uses redirects (github.com/user-attachments -> private-user-images).
pub fn extract_image_urls_from_html(html: &str) -> Vec<String> {
    let mut image_urls = Vec::new();

    // Parse HTML and extract all image URLs from img src attributes in document order
    let document = Html::parse_document(html);
    for element in document.select(&IMG_SELECTOR) {
        if let Some(src) = element.value().attr("src") {
            image_urls.push(src.to_string());
        }
    }

    log::debug!("Extracted {} image URLs from HTML", image_urls.len());
    image_urls
}

/// Replace image URLs in markdown with LaTeX includegraphics commands
///
/// This function processes the markdown content and replaces image references
/// with LaTeX commands that point to the downloaded local files.
///
/// # Arguments
/// * `markdown` - The original markdown content
/// * `url_to_path_map` - Map from original URLs to local file paths
///
/// # Returns
/// * Updated markdown with LaTeX image commands
pub fn replace_images_with_latex(
    markdown: &str,
    url_to_path_map: &HashMap<String, PathBuf>,
) -> String {
    let mut result = markdown.to_string();

    // Replace markdown images: ![alt](url) -> \includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{path}
    result = MD_IMG_REGEX
        .replace_all(&result, |caps: &regex::Captures| {
            let url = caps.get(2).unwrap().as_str();
            if let Some(local_path) = url_to_path_map.get(url) {
                // Use absolute path and escape backslashes for LaTeX
                let latex_path = local_path.display().to_string().replace('\\', "/");
                format!(
                    r"\includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{{{}}}",
                    latex_path
                )
            } else {
                // If image wasn't downloaded, keep original or show placeholder
                format!("\\textbf{{[Image not available: {}]}}", url)
            }
        })
        .to_string();

    // Replace HTML img tags using regex: <img src="url" /> -> \includegraphics[...]{path}
    result = HTML_IMG_REGEX
        .replace_all(&result, |caps: &regex::Captures| {
            let url = caps.get(1).unwrap().as_str();
            if let Some(local_path) = url_to_path_map.get(url) {
                // Use absolute path and escape backslashes for LaTeX
                let latex_path = local_path.display().to_string().replace('\\', "/");
                format!(
                    r"\includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{{{}}}",
                    latex_path
                )
            } else {
                // If image wasn't downloaded, keep original or show placeholder
                format!("\\textbf{{[Image not available: {}]}}", url)
            }
        })
        .to_string();

    result
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("request failed: {0}")]
    Ureq(#[from] ureq::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// fn is_amz_redirect(headers: &HeaderMap) -> bool {
//     let server = headers
//         .get("server")
//         .and_then(|v| v.to_str().ok())
//         .unwrap_or_default();
//     let amz_id = headers
//         .get("x-amz-request-id")
//         .and_then(|v| v.to_str().ok())
//         .unwrap_or_default();
//     if server.eq_ignore_ascii_case("AmazonS3") && !amz_id.is_empty() {
//         log::debug!("url is an amz redirect");
//         true
//     } else {
//         false
//     }
// }

// fn is_ghe_redirect(headers: &HeaderMap) -> bool {
//     let server = headers
//         .get("server")
//         .and_then(|v| v.to_str().ok())
//         .unwrap_or_default();
//     let gh_id = headers
//         .get("x-github-request-id")
//         .and_then(|v| v.to_str().ok())
//         .unwrap_or_default();
//     if server.eq_ignore_ascii_case("GitHub.com") && !gh_id.is_empty() {
//         log::debug!("url is an ghe redirect");
//         true
//     } else {
//         false
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_regex_compilation() {
        // Test that our LazyLock regexes and selectors compile correctly
        assert!(MD_IMG_REGEX.is_match("![test](url)"));
        assert!(HTML_IMG_REGEX.is_match(r#"<img src="url" />"#));

        // Test that the IMG_SELECTOR compiles correctly
        let html = Html::parse_fragment(r#"<img src="test.jpg" />"#);
        let elements: Vec<_> = html.select(&IMG_SELECTOR).collect();
        assert_eq!(elements.len(), 1);
    }

    #[test]
    fn test_extract_image_urls_markdown() {
        let markdown = r#"
# Header
![Alt text](https://example.com/image1.png)
Some text here.
![Another image](https://example.com/image2.jpg)
More text.
"#;

        let urls = extract_image_urls_from_markdown(markdown);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://example.com/image1.png");
        assert_eq!(urls[1], "https://example.com/image2.jpg");
    }

    #[test]
    fn test_extract_image_urls_html() {
        let markdown = r#"
# Header
<img width="390" height="436" alt="Image" src="https://github.com/user-attachments/assets/6df1bc0a-d30d-4b21-b4ac-51c297e43741" />
Some text here.
<img src="https://example.com/another.png" alt="Another" />
More text.
"#;

        let urls = extract_image_urls_from_markdown(markdown);
        assert_eq!(urls.len(), 2);
        assert_eq!(
            urls[0],
            "https://github.com/user-attachments/assets/6df1bc0a-d30d-4b21-b4ac-51c297e43741"
        );
        assert_eq!(urls[1], "https://example.com/another.png");
    }

    #[test]
    fn test_extract_image_urls_mixed() {
        let markdown = r#"
![Markdown image](https://example.com/md.png)
<img src="https://example.com/html.jpg" alt="HTML image" />
![Another markdown](https://example.com/md2.gif)
"#;

        let urls = extract_image_urls_from_markdown(markdown);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], "https://example.com/md.png");
        assert_eq!(urls[1], "https://example.com/html.jpg");
        assert_eq!(urls[2], "https://example.com/md2.gif");
    }

    #[test]
    fn test_extract_image_urls_no_images() {
        let markdown = r#"
# Header
This is just text with no images.
Some more text here.
"#;

        let urls = extract_image_urls_from_markdown(markdown);
        assert_eq!(urls.len(), 0);
    }

    #[test]
    fn test_replace_images_with_latex_markdown() {
        let markdown =
            "Here is an image: ![Alt text](https://example.com/image.png) and some more text.";

        let mut url_map = HashMap::new();
        url_map.insert(
            "https://example.com/image.png".to_string(),
            PathBuf::from("/tmp/downloaded_image.png"),
        );

        let result = replace_images_with_latex(markdown, &url_map);
        assert!(result.contains(r"\includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{/tmp/downloaded_image.png}"));
        assert!(!result.contains("![Alt text](https://example.com/image.png)"));
    }

    #[test]
    fn test_replace_images_with_latex_html() {
        let markdown = r#"Here is an image: <img src="https://example.com/image.jpg" alt="Test" /> and more text."#;

        let mut url_map = HashMap::new();
        url_map.insert(
            "https://example.com/image.jpg".to_string(),
            PathBuf::from("/tmp/downloaded_image.jpg"),
        );

        let result = replace_images_with_latex(markdown, &url_map);
        assert!(result.contains(r"\includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{/tmp/downloaded_image.jpg}"));
        assert!(!result.contains(r#"<img src="https://example.com/image.jpg" alt="Test" />"#));
    }

    #[test]
    fn test_replace_images_with_latex_missing_image() {
        let markdown =
            "Here is an image: ![Alt text](https://example.com/missing.png) and some more text.";

        let url_map = HashMap::new(); // Empty map - no downloaded images

        let result = replace_images_with_latex(markdown, &url_map);
        assert!(
            result.contains(r"\textbf{[Image not available: https://example.com/missing.png]}")
        );
        assert!(!result.contains("![Alt text](https://example.com/missing.png)"));
    }

    #[test]
    fn test_replace_images_with_latex_mixed() {
        let markdown = r#"
![Markdown image](https://example.com/md.png)
<img src="https://example.com/html.jpg" alt="HTML image" />
![Missing image](https://example.com/missing.png)
"#;

        let mut url_map = HashMap::new();
        url_map.insert(
            "https://example.com/md.png".to_string(),
            PathBuf::from("/tmp/md.png"),
        );
        url_map.insert(
            "https://example.com/html.jpg".to_string(),
            PathBuf::from("/tmp/html.jpg"),
        );
        // Note: missing.png not in map

        let result = replace_images_with_latex(markdown, &url_map);

        // Should replace available images
        assert!(result.contains(
            r"\includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{/tmp/md.png}"
        ));
        assert!(result.contains(
            r"\includegraphics[width=\textwidth,height=\textheight,keepaspectratio]{/tmp/html.jpg}"
        ));

        // Should show placeholder for missing image
        assert!(
            result.contains(r"\textbf{[Image not available: https://example.com/missing.png]}")
        );

        // Should not contain original syntax
        assert!(!result.contains("![Markdown image](https://example.com/md.png)"));
        assert!(!result.contains(r#"<img src="https://example.com/html.jpg" alt="HTML image" />"#));
    }

    #[test]
    fn test_windows_path_handling() {
        let markdown = "Image: ![test](https://example.com/test.png)";

        let mut url_map = HashMap::new();
        url_map.insert(
            "https://example.com/test.png".to_string(),
            PathBuf::from(r"C:\temp\test.png"),
        );

        let result = replace_images_with_latex(markdown, &url_map);
        // Should convert backslashes to forward slashes for LaTeX
        assert!(result.contains("C:/temp/test.png"));
        assert!(!result.contains(r"C:\temp\test.png"));
    }
}
