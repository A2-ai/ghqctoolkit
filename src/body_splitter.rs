//! Utilities for splitting large GitHub comment/issue bodies into multiple parts.
//!
//! GitHub enforces a 65,536-character limit on issue and comment bodies.
//! When content (typically file diffs) exceeds this limit, we split it across
//! multiple sequential parts, repeating the metadata header on each part and
//! injecting a `(i/N)` label into the leading heading.

/// Conservative threshold that leaves headroom for part labels.
const SAFE_LIMIT: usize = 65200;

const DIFF_MARKER: &str = "\n\n## File Difference\n";
const CODE_FENCE_OPEN: &str = "```diff\n";
const CODE_FENCE_CLOSE: &str = "\n```";
const CODE_FENCE_OVERHEAD: usize = CODE_FENCE_OPEN.len() + CODE_FENCE_CLOSE.len();

const DETAILS_PREFIX: &str = "<details>\n<summary>View diff</summary>\n\n";
const DETAILS_SUFFIX: &str = "\n\n</details>";

/// Maximum characters a part label like ` (99/99)` can add to the heading.
const MAX_LABEL_LEN: usize = 20;

// ─── Public API ──────────────────────────────────────────────────────────────

/// Split a comment body into parts, each ≤ [`SAFE_LIMIT`] characters.
///
/// Returns `vec![body]` unchanged when no split is needed.
/// When N > 1 parts are produced, injects `(i/N)` into the leading `# Title`
/// heading of each part so recipients can follow the sequence.
///
/// Splitting strategy:
/// - Splits at the `## File Difference` section boundary; the header (everything
///   before it) repeats verbatim on every part.
/// - Within the diff code block, splits at natural chunk boundaries:
///   - **Text diffs**: at `@@ previous script:` hunk headers.
///   - **Excel diffs**: at `@@ Sheet:` page headers (preamble lines stay in part 1).
///   - **Unknown**: treats entire diff as one chunk, falls back to line splitting.
/// - If a single chunk exceeds the per-part budget, falls back to line splitting.
/// - If the comment header alone exceeds [`SAFE_LIMIT`], logs a warning and
///   returns `vec![body]` unchanged rather than truncating.
pub fn split_comment_body(title: &str, body: String) -> Vec<String> {
    if body.len() <= SAFE_LIMIT {
        return vec![body];
    }

    let Some(diff_pos) = body.find(DIFF_MARKER) else {
        return generic_line_split(title, body);
    };

    let header = &body[..diff_pos];
    let diff_section = &body[diff_pos + DIFF_MARKER.len()..];

    if header.len() + MAX_LABEL_LEN > SAFE_LIMIT {
        log::warn!(
            "Comment header alone ({} chars) exceeds GitHub limit for '{}'; posting as-is",
            header.len(),
            title
        );
        return vec![body];
    }

    // Detect optional <details> wrapper
    let has_details = diff_section.starts_with(DETAILS_PREFIX);
    let code_block: &str = if has_details {
        diff_section
            .strip_prefix(DETAILS_PREFIX)
            .and_then(|s| s.strip_suffix(DETAILS_SUFFIX))
            .unwrap_or(diff_section)
    } else {
        diff_section
    };

    let details_overhead = if has_details {
        DETAILS_PREFIX.len() + DETAILS_SUFFIX.len()
    } else {
        0
    };

    // Available characters for the inner diff content (between code fences) per part
    let per_part_budget = SAFE_LIMIT.saturating_sub(
        header.len() + MAX_LABEL_LEN + DIFF_MARKER.len() + details_overhead + CODE_FENCE_OVERHEAD,
    );

    // Strip code fences to get the raw inner diff lines
    let inner = code_block
        .strip_prefix(CODE_FENCE_OPEN)
        .and_then(|s| s.strip_suffix(CODE_FENCE_CLOSE))
        .unwrap_or(code_block);

    let chunks = extract_diff_chunks(inner);
    let part_inners = group_into_parts(chunks, per_part_budget);

    if part_inners.len() == 1 {
        // All content fit — shouldn't normally reach here, but be safe
        return vec![body];
    }

    let n = part_inners.len();

    part_inners
        .into_iter()
        .enumerate()
        .map(|(i, inner_content)| {
            let fenced = format!("{}{}{}", CODE_FENCE_OPEN, inner_content, CODE_FENCE_CLOSE);
            let diff_body = if has_details {
                format!("{}{}{}", DETAILS_PREFIX, fenced, DETAILS_SUFFIX)
            } else {
                fenced
            };
            let labeled_header = inject_part_label(header, title, i + 1, n);
            format!("{}{}{}", labeled_header, DIFF_MARKER, diff_body)
        })
        .collect()
}

/// Split an issue body into parts, each ≤ [`SAFE_LIMIT`] characters.
///
/// Returns `vec![body]` unchanged when no split is needed.
/// The first part is returned as-is for use as the issue body.
/// Subsequent parts are formatted as continuation comments prefixed with
/// `# QC Issue (N/M)` and the repeated metadata section.
pub fn split_issue_body(body: String) -> Vec<String> {
    if body.len() <= SAFE_LIMIT {
        return vec![body];
    }

    // The metadata section is the first double-newline-delimited block
    let sections: Vec<&str> = body.split("\n\n").collect();
    let metadata_section = sections.first().copied().unwrap_or("");

    if metadata_section.len() + MAX_LABEL_LEN > SAFE_LIMIT {
        log::warn!(
            "Issue metadata section alone ({} chars) exceeds GitHub limit; posting as-is",
            metadata_section.len()
        );
        return vec![body];
    }

    // Budget for continuation part content (header + label will be prepended)
    // "# QC Issue (NN/NN)\n\n{metadata}\n\n" ≈ metadata + 30 chars overhead
    let continuation_budget =
        SAFE_LIMIT.saturating_sub(metadata_section.len() + MAX_LABEL_LEN + "\n\n".len() * 2);

    let mut parts: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut current_len: usize = 0;
    let mut is_first_part = true;

    for &section in &sections {
        if section.is_empty() {
            // Skip empty sections produced by adjacent double newlines
            continue;
        }
        let sep = if current.is_empty() { 0 } else { 2 }; // "\n\n"
        let budget = if is_first_part {
            SAFE_LIMIT
        } else {
            continuation_budget
        };

        if current_len + sep + section.len() <= budget {
            current_len += sep + section.len();
            current.push(section);
        } else {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
                is_first_part = false;
            }
            current.push(section);
            current_len = section.len();
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    if parts.len() == 1 {
        return vec![body];
    }

    let n = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(i, secs)| {
            if i == 0 {
                secs.join("\n\n")
            } else {
                format!(
                    "# QC Issue ({}/{})\n\n{}\n\n{}",
                    i + 1,
                    n,
                    metadata_section,
                    secs.join("\n\n")
                )
            }
        })
        .collect()
}

// ─── Diff chunk extraction ────────────────────────────────────────────────────

/// Parse the raw inner diff content into natural chunks for splitting.
///
/// - Text diffs (containing `@@ previous script:`) → one chunk per hunk.
/// - Excel diffs (containing `@@ Sheet:`) → one chunk per sheet; preamble
///   lines (sheet add/remove notices) are prepended to the first chunk.
/// - Unknown format → single chunk.
fn extract_diff_chunks(inner: &str) -> Vec<String> {
    if inner.contains("@@ previous script:") {
        extract_text_hunks(inner)
    } else if inner.contains("@@ Sheet:") {
        extract_excel_sheets(inner)
    } else {
        vec![inner.to_string()]
    }
}

fn extract_text_hunks(inner: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in inner.lines() {
        if line.starts_with("@@ previous script:") && !current.is_empty() {
            chunks.push(current.join("\n"));
            current.clear();
        }
        current.push(line);
    }

    if !current.is_empty() {
        chunks.push(current.join("\n"));
    }

    if chunks.is_empty() {
        vec![inner.to_string()]
    } else {
        chunks
    }
}

fn extract_excel_sheets(inner: &str) -> Vec<String> {
    let mut preamble: Vec<&str> = Vec::new();
    let mut chunks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_sheet = false;

    for line in inner.lines() {
        if line.starts_with("@@ Sheet:") {
            if in_sheet && !current.is_empty() {
                chunks.push(current.join("\n"));
                current.clear();
            }
            in_sheet = true;
            current.push(line);
        } else if in_sheet {
            current.push(line);
        } else {
            preamble.push(line);
        }
    }

    if !current.is_empty() {
        chunks.push(current.join("\n"));
    }

    if chunks.is_empty() {
        return vec![inner.to_string()];
    }

    // Prepend preamble lines to the first sheet chunk
    if !preamble.is_empty() {
        let preamble_str = preamble.join("\n");
        if let Some(first) = chunks.first_mut() {
            *first = format!("{}\n{}", preamble_str, first);
        }
    }

    chunks
}

// ─── Chunk grouping ───────────────────────────────────────────────────────────

/// Greedily pack chunks into parts so each part's inner content ≤ `budget` chars.
///
/// Chunks are always kept whole. If a single chunk exceeds `budget`, it is
/// further split at newline boundaries (never mid-line).
fn group_into_parts(chunks: Vec<String>, budget: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();

    for chunk in chunks {
        let sep_len = if current.is_empty() { 0 } else { 1 }; // '\n' between chunks

        if current.len() + sep_len + chunk.len() <= budget {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&chunk);
        } else if chunk.len() > budget {
            // Chunk is too large on its own — flush current, then split by lines
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            let line_parts = split_by_lines(&chunk, budget);
            let count = line_parts.len();
            for (i, lp) in line_parts.into_iter().enumerate() {
                if i < count - 1 {
                    parts.push(lp);
                } else {
                    current = lp;
                }
            }
        } else {
            // Chunk fits alone — flush current and start new part
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            current = chunk;
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    if parts.is_empty() {
        parts.push(String::new());
    }

    parts
}

/// Split a string into parts ≤ `limit` chars by breaking at newline boundaries.
fn split_by_lines(content: &str, limit: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        let sep_len = if current.is_empty() { 0 } else { 1 };
        if current.len() + sep_len + line.len() > limit && !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    if parts.is_empty() {
        parts.push(content.to_string());
    }

    parts
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Replace the leading `# {title}` heading with `# {title} (part/total)`.
fn inject_part_label(header: &str, title: &str, part: usize, total: usize) -> String {
    let original = format!("# {}", title);
    let labeled = format!("# {} ({}/{})", title, part, total);

    if let Some(rest) = header.strip_prefix(&original) {
        format!("{}{}", labeled, rest)
    } else {
        // Heading not found at the start — prepend label as a fallback
        format!("_{}/{}_\n\n{}", part, total, header)
    }
}

/// Fallback splitter for bodies without a `## File Difference` section.
/// Splits at newline boundaries with the leading `# Title` heading repeated.
fn generic_line_split(title: &str, body: String) -> Vec<String> {
    let parts = split_by_lines(&body, SAFE_LIMIT);
    if parts.len() == 1 {
        return vec![body];
    }
    let n = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(i, part)| inject_part_label(&part, title, i + 1, n))
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hunk(old_start: usize, new_start: usize, lines: &[&str]) -> String {
        let mut s = format!(
            "@@ previous script: lines {}-{} @@\n@@  current script: lines {}-{} @@",
            old_start,
            old_start + lines.len(),
            new_start,
            new_start + lines.len()
        );
        for line in lines {
            s.push('\n');
            s.push_str(line);
        }
        s
    }

    fn make_text_diff(hunks: &[String]) -> String {
        let mut parts = vec!["```diff".to_string()];
        parts.extend(hunks.iter().cloned());
        parts.push("```".to_string());
        parts.join("\n")
    }

    fn make_comment_body(title: &str, diff: &str) -> String {
        format!(
            "# {}\n\n## Metadata\n* commit: abc123\n\n## File Difference\n{}",
            title, diff
        )
    }

    // ── No split needed ──────────────────────────────────────────────────────

    #[test]
    fn no_split_when_under_limit() {
        let body = "# QC Notification\n\n## Metadata\n* commit: abc".to_string();
        let result = split_comment_body("QC Notification", body.clone());
        assert_eq!(result, vec![body]);
    }

    #[test]
    fn issue_body_no_split_when_under_limit() {
        let body = "## Metadata\n* commit: abc\n\n# Checklist\n- [ ] item".to_string();
        let result = split_issue_body(body.clone());
        assert_eq!(result, vec![body]);
    }

    // ── Part labels ──────────────────────────────────────────────────────────

    #[test]
    fn part_labels_injected_when_split() {
        // Build a body that will exceed SAFE_LIMIT by having many large hunks
        let big_line = "  ".to_string() + &"x".repeat(1000);
        let hunk_lines: Vec<&str> = std::iter::repeat(big_line.as_str()).take(20).collect();
        let hunks: Vec<String> = (0..80)
            .map(|i| make_hunk(i * 25, i * 25, &hunk_lines))
            .collect();
        let diff = make_text_diff(&hunks);
        let body = make_comment_body("QC Notification", &diff);

        let parts = split_comment_body("QC Notification", body);
        assert!(parts.len() > 1, "Expected multiple parts");

        let n = parts.len();
        for (i, part) in parts.iter().enumerate() {
            let expected_label = format!("# QC Notification ({}/{})", i + 1, n);
            assert!(
                part.starts_with(&expected_label),
                "Part {} should start with label '{}', got: {}",
                i + 1,
                expected_label,
                &part[..expected_label.len().min(60)]
            );
        }
    }

    // ── No hunk splits across parts ──────────────────────────────────────────

    #[test]
    fn text_diff_no_hunk_split_across_parts() {
        let big_line = "  ".to_string() + &"x".repeat(1000);
        let hunk_lines: Vec<&str> = std::iter::repeat(big_line.as_str()).take(20).collect();
        let hunks: Vec<String> = (0..80)
            .map(|i| make_hunk(i * 25, i * 25, &hunk_lines))
            .collect();
        let diff = make_text_diff(&hunks);
        let body = make_comment_body("QC Notification", &diff);

        let parts = split_comment_body("QC Notification", body);
        assert!(parts.len() > 1);

        // Each part's diff section must only contain whole hunks
        // (i.e. every `@@ previous script:` line appears exactly once across all parts)
        let all_hunk_headers: Vec<String> = hunks
            .iter()
            .map(|h| h.lines().next().unwrap().to_string())
            .collect();

        let mut seen = 0usize;
        for part in &parts {
            for header in &all_hunk_headers {
                if part.contains(header.as_str()) {
                    seen += 1;
                }
            }
        }
        assert_eq!(
            seen,
            all_hunk_headers.len(),
            "Every hunk should appear exactly once"
        );
    }

    // ── Excel sheet splitting ─────────────────────────────────────────────────

    #[test]
    fn excel_diff_splits_at_sheet_boundaries() {
        // Each sheet is ~33 000 chars; two together exceed SAFE_LIMIT
        let sheet_content = "x".repeat(33000);
        let inner = format!(
            "- Sheet removed: OldSheet\n@@ Sheet: Sheet1 @@\n{}\n@@ Sheet: Sheet2 @@\n{}",
            sheet_content, sheet_content
        );
        let diff = format!("{}{}{}", "```diff\n", inner, "\n```");
        let body = make_comment_body("QC Notification", &diff);

        let parts = split_comment_body("QC Notification", body);
        assert!(
            parts.len() > 1,
            "Expected Excel diff to be split across parts"
        );

        // Preamble should only be in the first part
        let first_part_diff = parts[0].split("## File Difference\n").nth(1).unwrap_or("");
        assert!(first_part_diff.contains("Sheet removed: OldSheet"));

        // Each sheet marker should appear in exactly one part
        let sheet1_count = parts
            .iter()
            .filter(|p| p.contains("@@ Sheet: Sheet1 @@"))
            .count();
        let sheet2_count = parts
            .iter()
            .filter(|p| p.contains("@@ Sheet: Sheet2 @@"))
            .count();
        assert_eq!(sheet1_count, 1);
        assert_eq!(sheet2_count, 1);
    }

    // ── Oversized single chunk → line fallback ────────────────────────────────

    #[test]
    fn oversized_single_chunk_splits_at_newlines() {
        // One giant hunk that exceeds per-part budget on its own
        let big_line = "+ ".to_string() + &"y".repeat(500);
        let many_lines: Vec<&str> = std::iter::repeat(big_line.as_str()).take(200).collect();
        let hunk = make_hunk(1, 1, &many_lines);
        let diff = make_text_diff(&[hunk]);
        let body = make_comment_body("QC Notification", &diff);

        let parts = split_comment_body("QC Notification", body);
        // Should split without panicking, and each part should be ≤ GITHUB_LIMIT
        for part in &parts {
            assert!(
                part.len() <= GITHUB_LIMIT,
                "Part exceeded GitHub limit: {} chars",
                part.len()
            );
        }
    }

    // ── Details wrapper preserved ─────────────────────────────────────────────

    #[test]
    fn details_wrapper_preserved_on_each_part() {
        let big_line = "  ".to_string() + &"z".repeat(1000);
        let hunk_lines: Vec<&str> = std::iter::repeat(big_line.as_str()).take(20).collect();
        let hunks: Vec<String> = (0..80)
            .map(|i| make_hunk(i * 25, i * 25, &hunk_lines))
            .collect();
        let inner_diff = make_text_diff(&hunks);
        let wrapped_diff = format!("{}{}{}", DETAILS_PREFIX, inner_diff, DETAILS_SUFFIX);
        let body = make_comment_body("Previous QC", &wrapped_diff);

        let parts = split_comment_body("Previous QC", body);
        assert!(parts.len() > 1);
        for part in &parts {
            let diff_section = part.split("## File Difference\n").nth(1).unwrap_or("");
            assert!(
                diff_section.starts_with("<details>"),
                "Each part should preserve the <details> wrapper"
            );
            assert!(
                diff_section.ends_with("</details>"),
                "Each part should close the </details> wrapper"
            );
        }
    }

    // ── Header-over-limit safety valve ───────────────────────────────────────

    #[test]
    fn header_over_limit_returns_original() {
        // Craft a header that's larger than SAFE_LIMIT
        let giant_header = format!(
            "# QC Notification\n\n## Metadata\n* note: {}",
            "x".repeat(SAFE_LIMIT)
        );
        let diff = "```diff\n+ changed\n```".to_string();
        let body = format!("{}\n\n## File Difference\n{}", giant_header, diff);

        let parts = split_comment_body("QC Notification", body.clone());
        assert_eq!(
            parts.len(),
            1,
            "Should return original body when header alone exceeds limit"
        );
        assert_eq!(parts[0], body);
    }

    // ── Metadata repeats on all issue body continuations ─────────────────────

    #[test]
    fn issue_body_metadata_repeats_on_continuations() {
        let metadata =
            "## Metadata\n* initial qc commit: abc123\n* git branch: main\n* author: wes";
        let checklist = format!("# Big Checklist\n{}", "- [ ] item\n".repeat(5000));
        let body = format!("{}\n\n{}", metadata, checklist);

        let parts = split_issue_body(body);
        if parts.len() > 1 {
            for part in &parts[1..] {
                assert!(
                    part.contains("initial qc commit: abc123"),
                    "Continuation should repeat metadata"
                );
                assert!(
                    part.starts_with("# QC Issue ("),
                    "Continuation should start with part label"
                );
            }
        }
    }
}
