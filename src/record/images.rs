use regex::Regex;
use reqwest::header::HeaderMap;
use reqwest::redirect::Policy;
use reqwest::{self, Client, StatusCode, Url};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::LazyLock;

use super::RecordError;

#[cfg(test)]
use mockall::automock;

// Compile-time regex patterns
static MD_IMG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").expect("Invalid markdown image regex")
});

static HTML_IMG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<img[^>]+src=["']([^"']+)["'][^>]*/?>"#).expect("Invalid HTML image regex")
});

/// Trait for downloading images from URLs for PDF embedding
///
/// This trait provides a testable interface for downloading images
/// while keeping the implementation details separate from the business logic.
#[cfg_attr(test, automock)]
pub trait ImageDownloader {
    /// Download an image from the given URL and return the local file path
    ///
    /// # Arguments
    /// * `url` - The URL of the image to download
    ///
    /// # Returns
    /// * `Ok(PathBuf)` - Path to the downloaded image file
    /// * `Err(RecordError)` - If download failed
    fn download_image(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<PathBuf, DownloadError>> + Send;

    /// Clean up all downloaded images
    ///
    /// # Returns
    /// * `Ok(())` - If cleanup succeeded
    /// * `Err(RecordError)` - If cleanup failed
    fn cleanup_images(&self) -> Result<(), RecordError>;
}

/// Extract image URLs from markdown content
///
/// Supports both markdown image syntax and HTML img tags commonly used in GitHub issues:
/// - `![alt text](url)`
/// - `<img src="url" />`
/// - `<img width="390" height="436" alt="Image" src="url" />`
pub fn extract_image_urls(markdown: &str) -> Vec<String> {
    let mut urls_with_positions = Vec::new();

    // Extract markdown images: ![alt](url)
    for captures in MD_IMG_REGEX.captures_iter(markdown) {
        if let Some(url_match) = captures.get(2) {
            urls_with_positions.push((url_match.start(), url_match.as_str().to_string()));
        }
    }

    // Extract HTML img tags: <img ... src="url" ... />
    for captures in HTML_IMG_REGEX.captures_iter(markdown) {
        if let Some(url_match) = captures.get(1) {
            urls_with_positions.push((url_match.start(), url_match.as_str().to_string()));
        }
    }

    // Sort by position in the document to preserve order
    urls_with_positions.sort_by_key(|(pos, _)| *pos);

    // Extract just the URLs
    urls_with_positions
        .into_iter()
        .map(|(_, url)| url)
        .collect()
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

    // Replace HTML img tags: <img src="url" /> -> \includegraphics[...]{path}
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

/// HTTP implementation of the ImageDownloader trait
pub struct HttpImageDownloader {
    temp_dir: PathBuf,
    auth_token: Option<String>,
}

impl HttpImageDownloader {
    /// Create a new HttpImageDownloader
    ///
    /// Downloads will be stored in a temporary directory under the system temp dir
    ///
    /// # Arguments
    /// * `auth_token` - Optional GitHub authentication token for accessing user-attachments
    pub fn new(auth_token: Option<String>) -> Result<Self, RecordError> {
        let temp_dir = std::env::temp_dir().join("ghqc-images");
        std::fs::create_dir_all(&temp_dir)?;

        Ok(Self {
            temp_dir,
            auth_token,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("unexpected response {status} from {url}")]
    Http { status: StatusCode, url: Url },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

fn is_amz_redirect(headers: &HeaderMap) -> bool {
    let server = headers
        .get("server")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let amz_id = headers
        .get("x-amz-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if server.eq_ignore_ascii_case("AmazonS3") && !amz_id.is_empty() {
        log::debug!("url is an amz redirect");
        true
    } else {
        false
    }
}

fn is_ghe_redirect(headers: &HeaderMap) -> bool {
    let server = headers
        .get("server")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let gh_id = headers
        .get("x-github-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if server.eq_ignore_ascii_case("GitHub.com") && !gh_id.is_empty() {
        log::debug!("url is an ghe redirect");
        true
    } else {
        false
    }
}

impl ImageDownloader for HttpImageDownloader {
    fn download_image(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<PathBuf, DownloadError>> + Send {
        let url = url.to_string();

        async move {
            // 1) Build two clients: one that DOESN'T follow redirects (to observe/log), and one that does.
            let client_noredir = Client::builder()
                .user_agent("ghqctoolkit/1.0")
                .redirect(Policy::none())
                .build()?;

            let client = Client::builder().user_agent("ghqctoolkit/1.0").build()?;

            // Helper that sets Accept + optional bearer
            let mut first_req = client_noredir
                .get(&url)
                .header(reqwest::header::ACCEPT, "application/vnd.github.full+json");
            if let Some(token) = &self.auth_token {
                first_req = first_req.bearer_auth(token);
            }

            log::debug!("Downloading {url}...");

            // 2) First hop WITHOUT following redirects: we can see 3xx + Location
            let first_resp = first_req.send().await?;
            let status = first_resp.status();
            let first_url = first_resp.url().clone();
            let headers = first_resp.headers().clone();

            log::debug!("Request to {} returned status: {}", first_url, status);

            // Prefer the explicit redirect Location when present
            let redirect_target = if status.is_redirection() {
                if let Some(loc) = first_resp.headers().get(reqwest::header::LOCATION) {
                    let loc = loc.to_str().unwrap_or_default();
                    let final_url = first_url
                        .join(loc)
                        .unwrap_or_else(|_| Url::parse(loc).unwrap());
                    log::debug!("{url} redirected to {final_url}");
                    Some(final_url)
                } else {
                    None
                }
            } else {
                None
            };

            // If not a 3xx, treat like your httr2 override: OK if success OR GH/S3 signal headers
            if redirect_target.is_none() {
                let ok_like_httr2 =
                    status.is_success() || is_amz_redirect(&headers) || is_ghe_redirect(&headers);
                if !ok_like_httr2 {
                    return Err(DownloadError::Http {
                        status,
                        url: first_url.clone(),
                    });
                }
            }

            // 3) Determine the asset URL to actually fetch
            let asset_url = redirect_target.unwrap_or_else(|| first_url.clone());

            // 4) Second hop: actually download the image (follow redirects is fine here)
            let mut img_req = client.get(asset_url.clone());
            if let Some(token) = &self.auth_token {
                img_req = img_req.bearer_auth(token);
            }
            // You can keep this Accept or omit it for the final asset.
            img_req = img_req.header(reqwest::header::ACCEPT, "application/vnd.github.full+json");

            let bytes = img_req.send().await?.error_for_status()?.bytes().await?;

            let path = self.temp_dir.join("image.png");
            log::debug!("Writing {} bytes to {}", bytes.len(), path.display());
            tokio::fs::write(&path, &bytes).await?;
            Ok(path.to_path_buf())
        }
    }

    fn cleanup_images(&self) -> Result<(), RecordError> {
        if self.temp_dir.exists() {
            std::fs::remove_dir_all(&self.temp_dir)
                .map_err(|e| RecordError::ImageCleanupFailed(e.to_string()))?;
            log::debug!("Cleaned up image directory: {}", self.temp_dir.display());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_regex_compilation() {
        // Test that our LazyLock regexes compile correctly
        assert!(MD_IMG_REGEX.is_match("![test](url)"));
        assert!(HTML_IMG_REGEX.is_match(r#"<img src="url" />"#));
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

        let urls = extract_image_urls(markdown);
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

        let urls = extract_image_urls(markdown);
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

        let urls = extract_image_urls(markdown);
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

        let urls = extract_image_urls(markdown);
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
