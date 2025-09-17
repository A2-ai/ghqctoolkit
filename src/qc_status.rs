use gix::ObjectId;
use octocrab::models::issues::Issue;
use regex::Regex;
use std::sync::LazyLock;

use crate::issue::{IssueError, IssueThread};

static CHECKLIST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*-\s*\[([xX\s])\]").expect("Failed to compile checklist regex")
});

#[derive(Debug, Clone)]
pub enum QCStatus {
    Approved,
    ChangesAfterApproval(ObjectId),
    ApprovalRequired,
    AwaitingApproval,
    InProgress,
    ChangesToComment(ObjectId),
}

impl QCStatus {
    pub fn determine_status(
        issue_thread: &IssueThread,
        file_commits: &[ObjectId],
    ) -> Result<Self, QCStatusError> {
        let status = if let Some(approved) = &issue_thread.approved_commit {
            file_commits
                .first()
                .and_then(|latest_commit| {
                    if latest_commit != approved {
                        Some(Self::ChangesAfterApproval(*latest_commit))
                    } else {
                        None
                    }
                })
                .unwrap_or(Self::Approved)
        } else {
            // if not approved and closed
            if !issue_thread.open {
                Self::ApprovalRequired
            } else {
                file_commits
                    .first()
                    .map(|latest_commit| {
                        if latest_commit == issue_thread.latest_commit() {
                            Self::AwaitingApproval
                        } else {
                            Self::ChangesToComment(*latest_commit)
                        }
                    })
                    .unwrap_or(Self::InProgress)
            }
        };

        Ok(status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistSummary {
    pub completed: usize,
    pub total: usize,
}

impl ChecklistSummary {
    pub fn new(completed: usize, total: usize) -> Self {
        Self { completed, total }
    }

    pub fn completion_percentage(&self) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            (self.completed as f64 / self.total as f64) * 100.0
        }
    }

    pub fn is_complete(&self) -> bool {
        self.completed == self.total && self.total > 0
    }

    pub fn sum<'a, I>(summaries: I) -> Self
    where
        I: IntoIterator<Item = &'a Self>,
    {
        let mut total_completed = 0;
        let mut total_items = 0;

        for summary in summaries {
            total_completed += summary.completed;
            total_items += summary.total;
        }

        Self::new(total_completed, total_items)
    }
}

impl std::fmt::Display for ChecklistSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} ({:.1}%)",
            self.completed,
            self.total,
            self.completion_percentage()
        )
    }
}

/// Analyze checklists within an issue's body
/// Returns a vector of (checklist_name, summary) tuples
pub fn analyze_issue_checklists(issue: &Issue) -> Vec<(String, ChecklistSummary)> {
    let body = match &issue.body {
        Some(body) => body,
        None => return vec![],
    };

    let mut checklists = Vec::new();

    // Split body into sections by headers (any level # to ######)
    let sections = split_body_into_sections(body);

    for (section_name, section_content) in sections {
        let summary = analyze_checklist_in_text(&section_content);

        // Only include sections that have checklist items
        if summary.total > 0 {
            checklists.push((section_name, summary));
        }
    }

    checklists
}

/// Split the issue body into sections based on markdown headers
/// Only processes content starting from the first level 1 header (ignoring Metadata section)
fn split_body_into_sections(body: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_section = String::new();
    let mut current_header: Option<String> = None;
    let mut found_first_level1_header = false;

    for line in body.lines() {
        if let Some(header_text) = extract_header_text(line) {
            let is_level1_header = line.trim_start().starts_with("# ") && !line.trim_start().starts_with("## ");

            // Only start processing after we find the first level 1 header
            if !found_first_level1_header && !is_level1_header {
                continue; // Skip non-level-1 headers before the first level 1 header
            }

            if !found_first_level1_header && is_level1_header {
                found_first_level1_header = true;
            }

            // Save the previous section if it has content and a header
            if found_first_level1_header {
                if let Some(ref header) = current_header {
                    if !current_section.trim().is_empty() {
                        sections.push((header.clone(), current_section.clone()));
                    }
                }
            }

            // Start new section
            current_header = Some(header_text);
            current_section.clear();
        } else if found_first_level1_header {
            // Only collect content after we've found the first level 1 header
            current_section.push_str(line);
            current_section.push('\n');
        }
        // Ignore everything before the first level 1 header (like Metadata section)
    }

    // Don't forget the last section
    if found_first_level1_header {
        if let Some(header) = current_header {
            if !current_section.trim().is_empty() {
                sections.push((header, current_section));
            }
        }
    }

    sections
}

/// Extract header text from a line if it's a markdown header (# to ######)
/// Returns None if the line is not a valid header
fn extract_header_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start();

    if !trimmed.starts_with('#') {
        return None;
    }

    // Count the number of # symbols at the start
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();

    // Must be 1-6 # symbols followed by a space
    if hash_count < 1 || hash_count > 6 || trimmed.chars().nth(hash_count) != Some(' ') {
        return None;
    }

    // Extract the text after the # symbols and space
    let header_text = trimmed
        .chars()
        .skip(hash_count + 1)
        .collect::<String>()
        .trim()
        .to_string();

    if header_text.is_empty() {
        None
    } else {
        Some(header_text)
    }
}

/// Analyze checklist items in a text block
/// Recognizes patterns like:
/// - [ ] Unchecked item
/// - [x] Checked item
/// - [X] Checked item
fn analyze_checklist_in_text(text: &str) -> ChecklistSummary {
    let mut total = 0;
    let mut completed = 0;

    for capture in CHECKLIST_REGEX.captures_iter(text) {
        total += 1;

        // Check if the item is marked as complete
        if let Some(checkbox) = capture.get(1) {
            let checkbox_content = checkbox.as_str().trim();
            if checkbox_content.eq_ignore_ascii_case("x") {
                completed += 1;
            }
        }
    }

    ChecklistSummary::new(completed, total)
}

#[derive(Debug, thiserror::Error)]
pub enum QCStatusError {
    #[error("Failed to determine commits for issue due to: {0}")]
    IssueError(#[from] IssueError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use octocrab::models::issues::Issue;

    #[test]
    fn test_analyze_complex_issue_checklist() {
        let issue_json = include_str!("tests/qc_status/complex_issue_checklist.json");
        let issue: Issue = serde_json::from_str(issue_json).unwrap();
        let result = analyze_issue_checklists(&issue);
        insta::assert_debug_snapshot!(result);
    }
}
