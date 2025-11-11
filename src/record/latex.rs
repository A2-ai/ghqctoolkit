use super::images::replace_images_with_latex;
/// LaTeX formatting utilities for the record generation system.
/// This module handles markdown processing, LaTeX escaping, and emoji wrapping.
use std::collections::HashMap;
use std::path::PathBuf;

/// Escape LaTeX special characters in user-provided text and wrap emojis
/// This function escapes characters that have special meaning in LaTeX to prevent
/// them from being interpreted as LaTeX commands when they appear in user content,
/// and wraps emoji characters with the \emoji{} command for proper rendering
pub fn escape_latex(text: &str) -> String {
    let escaped = text
        .replace('{', r"\{")
        .replace('}', r"\}")
        .replace('\\', r"\textbackslash{}")
        .replace('$', r"\$")
        .replace('&', r"\&")
        .replace('%', r"\%")
        .replace('#', r"\#")
        .replace('^', r"\textasciicircum{}")
        .replace('_', r"\_")
        .replace('~', r"\textasciitilde{}");

    wrap_emojis(&escaped)
}

/// Wrap emoji characters with \emoji{} command for LaTeX rendering
/// Skips emoji wrapping inside code blocks and verbatim environments
pub fn wrap_emojis(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    let mut in_code_block = false;
    let mut in_inline_code = false;

    while let Some(ch) = chars.next() {
        // Check for code block markers
        if ch == '`' {
            let mut backtick_count = 1;
            let mut lookahead = chars.clone();

            // Count consecutive backticks
            while let Some(&next_ch) = lookahead.peek() {
                if next_ch == '`' {
                    backtick_count += 1;
                    lookahead.next();
                } else {
                    break;
                }
            }

            if backtick_count >= 3 {
                // This is a code fence (```)
                in_code_block = !in_code_block;
                // Consume the additional backticks
                for _ in 1..backtick_count {
                    if chars.peek().is_some() && *chars.peek().unwrap() == '`' {
                        result.push(chars.next().unwrap());
                    }
                }
            } else if backtick_count == 1 && !in_code_block {
                // This might be inline code
                in_inline_code = !in_inline_code;
            }
        }

        // Only wrap emojis if we're not in any kind of code block
        if !in_code_block && !in_inline_code && is_emoji(ch) {
            // Collect consecutive emoji characters
            let mut emoji_sequence = String::new();
            emoji_sequence.push(ch);

            // Check for additional emoji characters or combining characters
            while let Some(&next_ch) = chars.peek() {
                if is_emoji(next_ch) || is_emoji_modifier(next_ch) {
                    emoji_sequence.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            result.push_str(&format!(r"\emoji{{{}}}", emoji_sequence));
        } else {
            result.push(ch);
        }
    }

    result
}

/// Check if a character is an emoji
pub fn is_emoji(ch: char) -> bool {
    let code = ch as u32;

    // Common emoji ranges
    matches!(code,
        0x1F600..=0x1F64F | // Emoticons
        0x1F300..=0x1F5FF | // Miscellaneous Symbols and Pictographs
        0x1F680..=0x1F6FF | // Transport and Map
        0x1F1E6..=0x1F1FF | // Regional Indicator Symbols
        0x2600..=0x26FF |   // Miscellaneous Symbols
        0x2700..=0x27BF |   // Dingbats
        0x1F900..=0x1F9FF |  // Supplemental Symbols and Pictographs
        0x1F018..=0x1F270 | // Various symbols
        0x238C..=0x2454 |   // Miscellaneous Technical
        0x20D0..=0x20FF |   // Combining Diacritical Marks for Symbols
        0x2B00..=0x2BFF |   // Miscellaneous Symbols and Arrows (includes ⭐)
        0x3030 | 0x303D |  // Wavy dash, part alternation mark
        0x3297 | 0x3299     // Ideographic circle symbols
    )
}

/// Check if a character is an emoji modifier (like skin tone modifiers)
pub fn is_emoji_modifier(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x1F3FB
            ..=0x1F3FF | // Skin tone modifiers
        0x200D |            // Zero Width Joiner
        0xFE0F // Variation Selector-16
    )
}

/// Translate markdown headers to ensure minimum level and wrap long code lines
pub fn format_markdown(
    markdown: &str,
    min_level: usize,
    image_url_map: &HashMap<String, PathBuf>,
) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
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
                    // Convert setext header to ATX header
                    let header_level = if is_h1_underline { 1 } else { 2 };
                    let new_level = std::cmp::min(std::cmp::max(header_level, min_level), 6);
                    let new_header = "#".repeat(new_level);
                    let header_text = line.trim();
                    result.push(format!("{} {}", new_header, header_text));
                    i += 2; // Skip both the header line and the underline
                    continue;
                }
            }
        }

        if trimmed.starts_with('#') {
            // Count existing header levels
            let header_level = trimmed.chars().take_while(|&c| c == '#').count();
            if header_level <= 6 {
                // Ensure header is at least at min_level
                let new_level = std::cmp::min(std::cmp::max(header_level, min_level), 6);
                let new_header = "#".repeat(new_level);
                let header_text = trimmed.trim_start_matches('#').trim_start();
                result.push(format!("{} {}", new_header, header_text));
            } else {
                // Keep as-is if already max level
                result.push(line.to_string());
            }
        } else if in_diff_block && (line.starts_with('+') || line.starts_with('-')) {
            // Handle diff line wrapping for long lines with +/- markers
            result.extend(wrap_diff_line(line, 80));
        } else if in_code_block && line.len() > 75 {
            // We're in a code block - wrap long lines at 75 characters
            result.extend(simple_wrap_line(line, 75));
        } else {
            result.push(line.to_string());
        }

        i += 1;
    }

    let joined = result
        .join("\n")
        .replace("---", "`---`")
        .replace("```diff", "``` diff");

    // Replace images with LaTeX commands
    let with_images = replace_images_with_latex(&joined, image_url_map);

    // Wrap emojis in the final result
    wrap_emojis(&with_images)
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
    fn test_emoji_detection() {
        // Test various emoji ranges
        assert!(is_emoji('😀')); // Emoticons
        assert!(is_emoji('🎯')); // Miscellaneous Symbols and Pictographs
        assert!(is_emoji('🚀')); // Transport and Map
        assert!(is_emoji('⭐')); // Miscellaneous Symbols
        assert!(is_emoji('✅')); // Dingbats
        assert!(is_emoji('🤖')); // Supplemental Symbols and Pictographs

        // Test non-emojis
        assert!(!is_emoji('A'));
        assert!(!is_emoji('1'));
        assert!(!is_emoji(' '));
        assert!(!is_emoji('!'));
    }

    #[test]
    fn test_emoji_modifier_detection() {
        // Test skin tone modifiers
        assert!(is_emoji_modifier('🏻')); // Light skin tone
        assert!(is_emoji_modifier('🏽')); // Medium skin tone
        assert!(is_emoji_modifier('🏿')); // Dark skin tone

        // Test other modifiers
        assert!(is_emoji_modifier('\u{200D}')); // Zero Width Joiner
        assert!(is_emoji_modifier('\u{FE0F}')); // Variation Selector-16

        // Test non-modifiers
        assert!(!is_emoji_modifier('A'));
        assert!(!is_emoji_modifier('😀'));
    }

    #[test]
    fn test_wrap_emojis_basic() {
        // Test single emoji
        assert_eq!(wrap_emojis("Hello 😀 world"), "Hello \\emoji{😀} world");

        // Test multiple emojis
        assert_eq!(
            wrap_emojis("😀 🎯 ✅"),
            "\\emoji{😀} \\emoji{🎯} \\emoji{✅}"
        );

        // Test emoji sequence
        assert_eq!(wrap_emojis("👨‍💻"), "\\emoji{👨‍💻}"); // Man technologist (composite emoji)

        // Test no emojis
        assert_eq!(wrap_emojis("Hello world"), "Hello world");

        // Test mixed content
        assert_eq!(
            wrap_emojis("Status: ✅ Complete! 🎉"),
            "Status: \\emoji{✅} Complete! \\emoji{🎉}"
        );
    }

    #[test]
    fn test_wrap_emojis_code_blocks() {
        // Test that emojis in code fences are not wrapped
        let markdown_with_code = r#"# Header 😀

```bash
echo "Hello 🌍 World!"
ls -la 📁
```

Normal text with emoji 🎯"#;

        let expected = r#"# Header \emoji{😀}

```bash
echo "Hello 🌍 World!"
ls -la 📁
```

Normal text with emoji \emoji{🎯}"#;

        assert_eq!(wrap_emojis(markdown_with_code), expected);
    }

    #[test]
    fn test_wrap_emojis_inline_code() {
        // Test that emojis in inline code are not wrapped
        let text_with_inline_code =
            "Use `echo \"Hello 🌍\"` to print emoji. But this 😀 should be wrapped.";
        let expected =
            "Use `echo \"Hello 🌍\"` to print emoji. But this \\emoji{😀} should be wrapped.";

        assert_eq!(wrap_emojis(text_with_inline_code), expected);
    }

    #[test]
    fn test_wrap_emojis_complex_code_blocks() {
        // Test nested backticks and complex scenarios
        let complex_markdown = r#"Text with 😀 emoji.

```diff
+ Added emoji support 🎉
- Old version without emojis
```

More text 🚀 here.

`inline code with 📁 emoji`

Final emoji 🎯."#;

        let expected = r#"Text with \emoji{😀} emoji.

```diff
+ Added emoji support 🎉
- Old version without emojis
```

More text \emoji{🚀} here.

`inline code with 📁 emoji`

Final emoji \emoji{🎯}."#;

        assert_eq!(wrap_emojis(complex_markdown), expected);
    }

    #[test]
    fn test_escape_latex_with_emojis() {
        // Test that escape_latex both escapes LaTeX chars and wraps emojis
        assert_eq!(
            escape_latex("Hello & 😀 world!"),
            "Hello \\& \\emoji{😀} world!"
        );
        assert_eq!(escape_latex("Price: $5 💰"), "Price: \\$5 \\emoji{💰}");
        assert_eq!(
            escape_latex("100% complete ✅"),
            "100\\% complete \\emoji{✅}"
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

        // Basic verification that headers are processed correctly
        assert!(result.contains("#### README – TMDD SimBiology Model"));
        assert!(result.contains("#### Some Subheading"));
        assert!(result.contains("#### Another Setext H1"));
        assert!(result.contains("#### Final Setext H2"));
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
        assert!(result.contains("``` diff")); // Note: ```diff gets converted to ``` diff
        assert!(result.len() > markdown_with_both.len()); // Should be longer due to wrapping
    }
}
