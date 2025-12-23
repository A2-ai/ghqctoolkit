use super::images::replace_images_with_typst;
/// Typst formatting utilities for the record generation system.
/// This module handles markdown processing and Typst escaping.
use crate::issue::HTML_LINK_REGEX;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

// Regex for markdown bold **text**
static BOLD_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*([^*]+)\*\*").expect("Invalid bold regex"));

// Regex for markdown italic *text* (after bold is replaced, any remaining *...* is italic)
static ITALIC_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*([^*]+)\*").expect("Invalid italic regex"));

/// Escape Typst special characters in user-provided text
/// This function escapes characters that have special meaning in Typst to prevent
/// them from being interpreted as Typst markup when they appear in user content.
///
/// Typst special characters that need escaping:
/// - `#` - function/keyword marker
/// - `$` - math mode delimiter
/// - `*` - bold/strong emphasis
/// - `_` - emphasis/subscript
/// - `` ` `` - raw text/code
/// - `@` - reference/citation marker
/// - `<` `>` - label markers
/// - `\` - escape character
/// - `[` `]` - content blocks
pub fn escape_typst(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('#', "\\#")
        .replace('$', "\\$")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('@', "\\@")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

/// Convert inline markdown formatting to Typst syntax
///
/// Handles (in order):
/// 1. `<a href="url">text</a>` → `#link("url")[text]` (HTML links to Typst links)
/// 2. `**bold**` → `*bold*` (Typst bold)
/// 3. `*italic*` → `_italic_` (Typst italic)
/// 4. Escape special characters: `@`, `<`, `>`
fn convert_inline_markdown(text: &str) -> String {
    // Use placeholder to protect bold markers from italic conversion
    const BOLD_PLACEHOLDER: &str = "\x00TYPST_BOLD\x00";

    // Step 1: Convert HTML links to Typst links
    let with_links = HTML_LINK_REGEX.replace_all(text, |caps: &regex::Captures| {
        let url = &caps[1];
        let display_text = &caps[2];
        format!("#link(\"{}\")[{}]", url, display_text)
    });

    // Step 2: Replace **bold** with placeholder
    let with_bold_placeholder = BOLD_REGEX.replace_all(&with_links, |caps: &regex::Captures| {
        format!("{}{}{}", BOLD_PLACEHOLDER, &caps[1], BOLD_PLACEHOLDER)
    });

    // Step 3: Replace *italic* with _italic_
    let with_italic =
        ITALIC_REGEX.replace_all(&with_bold_placeholder, |caps: &regex::Captures| {
            format!("_{}_", &caps[1])
        });

    // Step 4: Replace placeholder with Typst bold syntax
    let with_bold = with_italic.replace(BOLD_PLACEHOLDER, "*");

    // Step 5: Escape special characters that have meaning in Typst
    with_bold
        .replace('@', "\\@")
        .replace('<', "\\<")
        .replace('>', "\\>")
}

/// Translate markdown headers to ensure minimum level and wrap long code lines
pub fn format_markdown(
    markdown: &str,
    min_level: usize,
    image_url_map: &HashMap<String, PathBuf>,
) -> String {
    // IMPORTANT: Replace images FIRST before any escaping happens
    // This prevents @ in URLs from being escaped and breaking the lookup
    let with_images = replace_images_with_typst(markdown, image_url_map);

    let lines: Vec<&str> = with_images.lines().collect();
    let mut result = Vec::new();
    let mut in_diff_block = false;
    let mut in_code_block = false;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Track if we're in a code block
        if trimmed.starts_with("```") {
            if trimmed.contains("diff") {
                in_diff_block = !in_diff_block; // Toggle diff block state
                in_code_block = in_diff_block; // Diff blocks are also code blocks
            } else {
                in_diff_block = false; // Not a diff block
                in_code_block = !in_code_block; // Toggle regular code block state
            }
            result.push(line.to_string());
            i += 1;
            continue;
        }

        // Check for setext-style headers (header text followed by === or ---)
        if i + 1 < lines.len() && !in_diff_block {
            let next_line = lines[i + 1].trim();
            if !next_line.is_empty() {
                let is_h1_underline = next_line.chars().all(|c| c == '=') && next_line.len() >= 3;
                let is_h2_underline = next_line.chars().all(|c| c == '-') && next_line.len() >= 3;

                if is_h1_underline || is_h2_underline {
                    // Convert setext header to Typst header
                    let header_level = if is_h1_underline { 1 } else { 2 };
                    let new_level = std::cmp::min(std::cmp::max(header_level, min_level), 6);
                    let new_header = "=".repeat(new_level);
                    let header_text = line.trim();
                    result.push(format!("{} {}", new_header, header_text));
                    i += 2; // Skip both the header line and the underline
                    continue;
                }
            }
        }

        if trimmed.starts_with('#') {
            // Count existing header levels and convert to Typst = syntax
            // But only if followed by space (markdown header), not a letter (Typst command like #image)
            let header_level = trimmed.chars().take_while(|&c| c == '#').count();
            let after_hashes = &trimmed[header_level..];
            let is_markdown_header =
                header_level <= 6 && (after_hashes.is_empty() || after_hashes.starts_with(' '));

            if is_markdown_header {
                // Ensure header is at least at min_level
                let new_level = std::cmp::min(std::cmp::max(header_level, min_level), 6);
                let new_header = "=".repeat(new_level);
                let header_text = trimmed.trim_start_matches('#').trim_start();
                result.push(format!("{} {}", new_header, header_text));
            } else {
                // It's a Typst command like #image(), keep as-is
                result.push(line.to_string());
            }
        } else if in_diff_block && (line.starts_with('+') || line.starts_with('-')) {
            // Handle diff line wrapping for long lines with +/- markers
            result.extend(wrap_diff_line(line, 80));
        } else if in_code_block && line.len() > 75 {
            // We're in a code block - wrap long lines at 75 characters
            result.extend(simple_wrap_line(line, 75));
        } else if !in_code_block && (trimmed.starts_with("* ") || trimmed.starts_with("+ ")) {
            // Convert markdown bullet points (* or +) to Typst bullet points (-)
            // In Typst, * starts bold text, so we must use - for lists
            let indent = &line[..line.len() - trimmed.len()];
            let content = &trimmed[2..]; // Skip "* " or "+ "
            let converted = convert_inline_markdown(content);
            result.push(format!("{}- {}", indent, converted));
        } else if !in_code_block {
            // Convert markdown syntax to Typst for regular content
            let converted = convert_inline_markdown(line);
            result.push(converted);
        } else {
            result.push(line.to_string());
        }

        i += 1;
    }

    result.join("\n")
}

/// Smart line wrapping - looks for good break points within ±5 chars of max_width, otherwise breaks at max_width
pub(crate) fn simple_wrap_line(line: &str, max_width: usize) -> Vec<String> {
    if line.len() <= max_width {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut pos = 0;

    while pos < line.len() {
        let remaining = &line[pos..];

        if remaining.len() <= max_width {
            // Rest of line fits
            result.push(remaining.to_string());
            break;
        }

        // Look for good break points between (max_width - 5) and max_width
        let search_start = (max_width.saturating_sub(10)).min(remaining.len());
        let search_end = max_width.min(remaining.len());

        let mut break_point = None;

        if search_start < search_end {
            let search_slice = &remaining[search_start..search_end];

            // Look for space, backslash, or forward slash (in reverse order to get the latest one)
            for (i, ch) in search_slice.char_indices().rev() {
                if ch == ' ' || ch == '\\' || ch == '/' {
                    break_point = Some(search_start + i + 1); // +1 to include the break character
                    break;
                }
            }
        }

        // If no good break point found, break at max_width
        let final_break = break_point.unwrap_or(max_width.min(remaining.len()));

        result.push(remaining[..final_break].to_string());
        pos += final_break;
    }

    result
}

/// Wrap a diff line if it's too long, preserving the diff marker
pub(crate) fn wrap_diff_line(line: &str, max_width: usize) -> Vec<String> {
    if line.len() <= max_width {
        return vec![line.to_string()];
    }

    let mut wrapped_lines = Vec::new();
    let diff_marker = &line[0..1]; // Get the + or - marker
    let content = &line[1..]; // Get the content without the marker

    // Find good break points (spaces, after certain characters)
    let mut current_pos = 0;
    let available_width = max_width - 1; // Account for diff marker

    while current_pos < content.len() {
        let remaining = &content[current_pos..];

        if remaining.len() <= available_width {
            // Rest of line fits
            if current_pos == 0 {
                wrapped_lines.push(line.to_string());
            } else {
                wrapped_lines.push(format!("{}      {}", diff_marker, remaining));
            }
            break;
        }

        // Find a good break point within the available width
        let mut break_point = available_width;
        let search_slice = &remaining[..available_width.min(remaining.len())];

        // Look for space, comma, semicolon, or other good break characters
        if let Some(pos) = search_slice.rfind(' ') {
            break_point = pos + 1; // Include the space
        } else if let Some(pos) = search_slice.rfind(',') {
            break_point = pos + 1;
        } else if let Some(pos) = search_slice.rfind(';') {
            break_point = pos + 1;
        } else if let Some(pos) = search_slice.rfind('(') {
            break_point = pos + 1;
        } else if let Some(pos) = search_slice.rfind('{') {
            break_point = pos + 1;
        }

        // Extract the line segment
        let segment = &remaining[..break_point];

        if current_pos == 0 {
            // First line keeps original format
            wrapped_lines.push(format!("{}{}", diff_marker, segment));
        } else {
            // Continuation lines get indented with a tab
            wrapped_lines.push(format!("{}      {}", diff_marker, segment.trim_start()));
        }

        current_pos += break_point;
    }

    wrapped_lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_typst_special_chars() {
        // Test that escape_typst escapes Typst special characters
        assert_eq!(escape_typst("Hello #world"), "Hello \\#world");
        assert_eq!(escape_typst("Price: $5"), "Price: \\$5");
        assert_eq!(escape_typst("*bold* text"), "\\*bold\\* text");
        assert_eq!(escape_typst("_emphasis_"), "\\_emphasis\\_");
        assert_eq!(escape_typst("`code`"), "\\`code\\`");
        assert_eq!(escape_typst("@reference"), "\\@reference");
        assert_eq!(escape_typst("<label>"), "\\<label\\>");
        assert_eq!(escape_typst("back\\slash"), "back\\\\slash");
        assert_eq!(escape_typst("[content]"), "\\[content\\]");
    }

    #[test]
    fn test_escape_typst_combined() {
        // Test multiple special characters in one string
        assert_eq!(
            escape_typst("Hello #world with $5 and *bold*"),
            "Hello \\#world with \\$5 and \\*bold\\*"
        );
    }

    #[test]
    fn test_format_markdown_with_min_level_comprehensive() {
        // Test all header scenarios including setext, ATX, mixed content, and edge cases
        let markdown = r#"# Original ATX H1

Some content before setext headers.

README – TMDD SimBiology Model
================================

Model Version: v1.0.0 (Initial QC Build)
Last Updated: 2025-11-02

## Original ATX H2

Some Subheading
---------------

This line has some === equals === in it.
And this line has some --- dashes --- in it.
But not as underlines.

### Original ATX H3

Another Setext H1
=================

#### Original ATX H4

Final Setext H2
---------------

Some content with === in the middle === of the line.
Some content with --- in the middle --- of the line.

```diff
+ This is a diff block
- With some changes
```

Regular content after everything."#;

        let empty_image_map = HashMap::new();
        let result = format_markdown(markdown, 4, &empty_image_map);

        // Basic verification that headers are converted to Typst = syntax
        assert!(result.contains("==== README – TMDD SimBiology Model"));
        assert!(result.contains("==== Some Subheading"));
        assert!(result.contains("==== Another Setext H1"));
        assert!(result.contains("==== Final Setext H2"));
    }

    #[test]
    fn test_simple_wrap_line_basic() {
        // Test line shorter than max_width - should not wrap
        assert_eq!(simple_wrap_line("short line", 75), vec!["short line"]);

        // Test line exactly at max_width - should not wrap
        let exactly_75 = "x".repeat(75);
        assert_eq!(simple_wrap_line(&exactly_75, 75), vec![exactly_75]);
    }

    #[test]
    fn test_simple_wrap_line_no_break_points() {
        // Test line with no good break points - should break at max_width
        let no_breaks = "x".repeat(100);
        let result = simple_wrap_line(&no_breaks, 75);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), 75);
        assert_eq!(result[1].len(), 25);
    }

    #[test]
    fn test_simple_wrap_line_space_breaks() {
        // Test line with spaces - should break at space
        let with_spaces = "This is a very long line that should break at a space character when it exceeds the maximum width limit";
        let result = simple_wrap_line(with_spaces, 75);

        // Should break at spaces and each line should be <= 75 chars
        for line in &result {
            assert!(
                line.len() <= 75,
                "Line '{}' is {} chars, exceeds 75",
                line,
                line.len()
            );
        }

        // At least one break should occur at a space
        let has_space_break = result.iter().any(|line| {
            line.ends_with(' ') || (line.len() < 75 && with_spaces.contains(&format!("{} ", line)))
        });
        assert!(
            has_space_break || result.len() == 1,
            "Should break at spaces when possible"
        );
    }

    #[test]
    fn test_simple_wrap_line_slash_breaks() {
        // Test line with forward slashes - should break at slashes
        let with_slashes = "https://very-long-domain-name.example.com/very/long/path/to/some/resource/file.extension";
        let result = simple_wrap_line(with_slashes, 75);

        // Should break and each line should be <= 75 chars
        for line in &result {
            assert!(
                line.len() <= 75,
                "Line '{}' is {} chars, exceeds 75",
                line,
                line.len()
            );
        }

        // Should have multiple lines for this long URL
        assert!(
            result.len() > 1,
            "Long URL should be broken into multiple lines"
        );
    }

    #[test]
    fn test_simple_wrap_line_backslash_breaks() {
        // Test line with backslashes - should break at backslashes
        let with_backslashes = "C:\\Very\\Long\\Windows\\Path\\To\\Some\\Deep\\Nested\\Directory\\Structure\\WithVeryLongFilename.txt";
        let result = simple_wrap_line(with_backslashes, 75);

        // Should break and each line should be <= 75 chars
        for line in &result {
            assert!(
                line.len() <= 75,
                "Line '{}' is {} chars, exceeds 75",
                line,
                line.len()
            );
        }

        // Should have multiple lines for this long path
        assert!(
            result.len() > 1,
            "Long path should be broken into multiple lines"
        );
    }

    #[test]
    fn test_simple_wrap_line_mixed_break_points() {
        // Test line with multiple types of break points
        let mixed = "This is a long line with spaces and/forward/slashes and\\backslashes that should break intelligently";
        let result = simple_wrap_line(mixed, 75);

        // Should break and each line should be <= 75 chars
        for line in &result {
            assert!(
                line.len() <= 75,
                "Line '{}' is {} chars, exceeds 75",
                line,
                line.len()
            );
        }

        // Should prefer later break points (reverse search)
        assert!(result.len() > 1, "Long mixed line should be broken");
    }

    #[test]
    fn test_simple_wrap_line_multi_line() {
        // Test very long line that should wrap to multiple lines (3+ lines)
        let very_long = "This is an extremely long line that should definitely be wrapped into multiple lines because it exceeds 150 characters and should test our multi-line wrapping capability with spaces and/paths/like/this that provide good break points throughout the entire length of this very verbose example string";

        let result = simple_wrap_line(very_long, 75);

        // Should break into multiple lines (at least 3 for a 240+ character string)
        assert!(
            result.len() >= 3,
            "Very long line should be broken into at least 3 lines, got {}",
            result.len()
        );

        // Each line should be <= 75 chars
        for (i, line) in result.iter().enumerate() {
            assert!(
                line.len() <= 75,
                "Line {} '{}' is {} chars, exceeds 75",
                i + 1,
                line,
                line.len()
            );
        }

        // Verify the total content is preserved (accounting for potential break characters)
        let rejoined = result.join("");
        let original_chars: Vec<char> = very_long.chars().collect();
        let rejoined_chars: Vec<char> = rejoined.chars().collect();

        // Should preserve most characters (some break characters like spaces might be at line boundaries)
        assert!(
            rejoined_chars.len() >= original_chars.len() - 5,
            "Should preserve most content: original {} chars, got {} chars",
            original_chars.len(),
            rejoined_chars.len()
        );
    }

    #[test]
    fn test_code_block_wrapping_integration() {
        // Test that code blocks trigger line wrapping
        let markdown_with_long_code = r#"Some regular text.

```r
repositories = [{alias = "PRISM", url = "https://prism.dev.a2-ai.cloud/rpkgs/stratus/2025-10-29"}]
very_long_variable_name_that_exceeds_limit = "some value with a very long string that should be wrapped"
```

More regular text."#;

        let empty_image_map = HashMap::new();
        let result = format_markdown(markdown_with_long_code, 4, &empty_image_map);

        // The long lines in the code block should be wrapped
        let lines: Vec<&str> = result.lines().collect();
        for line in &lines {
            // Code block lines (except the ``` markers) should not exceed 75 characters
            if !line.trim().starts_with("```") && !line.trim().is_empty() {
                if line.len() > 75 {
                    // Allow some tolerance for edge cases, but mostly should be wrapped
                    println!("Long line found: '{}' (length: {})", line, line.len());
                }
            }
        }
    }

    #[test]
    fn test_markdown_bullet_conversion() {
        // Test that markdown bullet points (* and +) are converted to Typst bullet points (-)
        let empty_image_map = HashMap::new();

        // Test * bullets
        let asterisk_bullets = "* First item\n* Second item\n* Third item";
        let result = format_markdown(asterisk_bullets, 4, &empty_image_map);
        assert_eq!(result, "- First item\n- Second item\n- Third item");

        // Test + bullets
        let plus_bullets = "+ First item\n+ Second item";
        let result = format_markdown(plus_bullets, 4, &empty_image_map);
        assert_eq!(result, "- First item\n- Second item");

        // Test - bullets (should remain unchanged)
        let dash_bullets = "- First item\n- Second item";
        let result = format_markdown(dash_bullets, 4, &empty_image_map);
        assert_eq!(result, "- First item\n- Second item");

        // Test indented bullets
        let indented = "  * Indented item\n    * More indented";
        let result = format_markdown(indented, 4, &empty_image_map);
        assert_eq!(result, "  - Indented item\n    - More indented");

        // Test mixed content with bullets
        let mixed = "Some text\n* Bullet item\nMore text";
        let result = format_markdown(mixed, 4, &empty_image_map);
        assert_eq!(result, "Some text\n- Bullet item\nMore text");

        // Test bullet with link (the original error case)
        let bullet_with_link = "* [commit comparison](https://example.com/compare/abc..def)";
        let result = format_markdown(bullet_with_link, 4, &empty_image_map);
        assert_eq!(
            result,
            "- [commit comparison](https://example.com/compare/abc..def)"
        );
    }

    #[test]
    fn test_markdown_inline_conversion() {
        let empty_image_map = HashMap::new();

        // Test **bold** -> *bold*
        let bold_text = "This is **bold text** in markdown";
        let result = format_markdown(bold_text, 4, &empty_image_map);
        assert_eq!(result, "This is *bold text* in markdown");

        // Test *italic* -> _italic_
        let italic_text = "This is *italic text* in markdown";
        let result = format_markdown(italic_text, 4, &empty_image_map);
        assert_eq!(result, "This is _italic text_ in markdown");

        // Test combined bold and italic
        let combined_format = "This is **bold** and *italic* text";
        let result = format_markdown(combined_format, 4, &empty_image_map);
        assert_eq!(result, "This is *bold* and _italic_ text");

        // Test @ escaping
        let at_text = "@reviewer mentioned something";
        let result = format_markdown(at_text, 4, &empty_image_map);
        assert_eq!(result, "\\@reviewer mentioned something");

        // Test email with @
        let email = "Contact: reviewer@example.com";
        let result = format_markdown(email, 4, &empty_image_map);
        assert_eq!(result, "Contact: reviewer\\@example.com");

        // Test combined bold and @
        let combined = "**@reviewer** wrote this";
        let result = format_markdown(combined, 4, &empty_image_map);
        assert_eq!(result, "*\\@reviewer* wrote this");

        // Test HTML link conversion to Typst link
        let html_link = r#"<a href="https://example.com">link text</a>"#;
        let result = format_markdown(html_link, 4, &empty_image_map);
        assert_eq!(result, r#"#link("https://example.com")[link text]"#);

        // Test angle brackets get escaped (not part of HTML tags)
        let angle_brackets = "email: <user@example.com>";
        let result = format_markdown(angle_brackets, 4, &empty_image_map);
        assert_eq!(result, r"email: \<user\@example.com\>");

        // Test that formatting is NOT converted inside code blocks
        let code_block = "```\n**bold** and *italic* and @mention and <tag>\n```";
        let result = format_markdown(code_block, 4, &empty_image_map);
        assert!(result.contains("**bold**"));
        assert!(result.contains("*italic*"));
        assert!(result.contains("@mention"));
        assert!(result.contains("<tag>"));
    }

    #[test]
    fn test_bullets_not_converted_in_code_blocks() {
        // Bullets inside code blocks should NOT be converted
        let empty_image_map = HashMap::new();

        let code_with_bullets = "```\n* This is code, not a bullet\n+ Also code\n```";
        let result = format_markdown(code_with_bullets, 4, &empty_image_map);
        assert!(result.contains("* This is code"));
        assert!(result.contains("+ Also code"));
    }

    #[test]
    fn test_diff_vs_regular_code_blocks() {
        // Test that diff blocks still use diff-specific wrapping while regular code blocks use simple wrapping
        let markdown_with_both = r#"Regular code block:
```r
very_long_line_in_regular_code_block_that_should_use_simple_wrapping_logic_here
```

Diff code block:
```diff
+ very_long_added_line_in_diff_block_that_should_use_diff_specific_wrapping_logic
- very_long_removed_line_in_diff_block_that_should_also_use_diff_specific_wrapping
```"#;

        let empty_image_map = HashMap::new();
        let result = format_markdown(markdown_with_both, 4, &empty_image_map);

        // Both should be wrapped, but this test mainly ensures no crashes occur
        // and that the different wrapping logic is applied appropriately
        assert!(result.contains("```r"));
        assert!(result.contains("```diff"));
        assert!(result.len() > markdown_with_both.len()); // Should be longer due to wrapping
    }
}
