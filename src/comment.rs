use std::path::PathBuf;

use diff::{Result as DiffResult, lines};
use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::git::{GitHelpers, LocalGitError, LocalGitInfo};

pub struct QCComment {
    pub(crate) file: PathBuf,
    pub(crate) issue: Issue,
    pub(crate) current_commit: ObjectId,
    pub(crate) previous_commit: Option<ObjectId>,
    pub(crate) note: Option<String>,
    pub(crate) no_diff: bool,
}

impl QCComment {
    pub(crate) fn body(
        &self,
        git_info: &(impl GitHelpers + LocalGitInfo),
    ) -> Result<String, CommentError> {
        let mut metadata = vec![
            "## Metadata".to_string(),
            format!("current commit: {}", self.current_commit),
        ];
        if let Some(p_c) = self.previous_commit {
            metadata.push(format!("previous commit: {p_c}"));
            metadata.push(format!(
                "[commit comparison]({})",
                git_info.commit_comparison_url(&self.current_commit, &p_c)
            ));
        }

        let assignees = self
            .issue
            .assignees
            .iter()
            .map(|a| format!("@{}", a.login))
            .collect::<Vec<_>>()
            .join(", ");

        let mut body = vec!["# QC Notification".to_string()];
        if !assignees.is_empty() {
            body.push(assignees);
        }

        if let Some(note) = &self.note {
            body.push(note.clone());
        }

        body.push(metadata.join("\n* "));

        if !self.no_diff {
            if let Some(previous_commit) = self.previous_commit {
                let current_content =
                    git_info.file_content_at_commit(&self.file, &self.current_commit)?;
                let previous_content =
                    git_info.file_content_at_commit(&self.file, &previous_commit)?;

                let difference = format!(
                    "## File Difference\n{}",
                    diff(&previous_content, &current_content)
                );

                body.push(difference);
            } else {
                log::debug!("Previous Commit not specified. Cannot generate diff...");
            }
        }

        Ok(body.join("\n\n"))
    }
}

/// Generate a markdown-formatted diff between two strings showing only changed hunks with context
fn diff(old_content: &str, new_content: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    // Check if files are identical
    if old_lines == new_lines {
        return "\nNo difference between file versions.\n".to_string();
    }

    let changeset = lines(old_content, new_content);

    // Group changes into hunks with context
    let hunks = create_hunks(&changeset, 3); // 3 lines of context

    if hunks.is_empty() {
        return "\nNo difference between file versions.\n".to_string();
    }

    let mut result = Vec::new();
    result.push("```diff".to_string());

    for hunk in hunks {
        result.push(format_hunk(&hunk));
    }

    result.push("```".to_string());
    result.join("\n")
}

#[derive(Debug, Clone)]
struct DiffHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
enum DiffLine {
    Context(String, usize, usize), // content, old_line_num, new_line_num
    Addition(String, usize),       // content, new_line_num
    Deletion(String, usize),       // content, old_line_num
}

fn create_hunks(changeset: &[DiffResult<&str>], context_lines: usize) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_hunk_lines = Vec::new();
    let mut old_line = 1;
    let mut new_line = 1;
    let mut last_change_idx = None;

    for (idx, change) in changeset.iter().enumerate() {
        let is_change = matches!(change, DiffResult::Left(_) | DiffResult::Right(_));

        if is_change {
            // If this is a change, include context before it if we haven't started a hunk
            if current_hunk_lines.is_empty() {
                let context_start = idx.saturating_sub(context_lines);
                for i in context_start..idx {
                    if let DiffResult::Both(line, _) = &changeset[i] {
                        let ctx_old = old_line - (idx - i);
                        let ctx_new = new_line - (idx - i);
                        current_hunk_lines.push(DiffLine::Context(
                            line.to_string(),
                            ctx_old,
                            ctx_new,
                        ));
                    }
                }
            }
            last_change_idx = Some(idx);
        }

        // Add the current line to the hunk
        match change {
            DiffResult::Left(line) => {
                current_hunk_lines.push(DiffLine::Deletion(line.to_string(), old_line));
                old_line += 1;
            }
            DiffResult::Right(line) => {
                current_hunk_lines.push(DiffLine::Addition(line.to_string(), new_line));
                new_line += 1;
            }
            DiffResult::Both(line, _) => {
                if !current_hunk_lines.is_empty() {
                    current_hunk_lines.push(DiffLine::Context(
                        line.to_string(),
                        old_line,
                        new_line,
                    ));
                }
                old_line += 1;
                new_line += 1;
            }
        }

        // Check if we should end the current hunk
        if let Some(last_change) = last_change_idx {
            let distance_from_last_change = idx - last_change;
            if distance_from_last_change >= context_lines * 2 && !current_hunk_lines.is_empty() {
                // Trim to exactly context_lines after the last change
                let mut lines_to_keep = current_hunk_lines.len();
                let mut context_after_change = 0;

                // Count backwards from the end to find where to cut off
                for (i, line) in current_hunk_lines.iter().enumerate().rev() {
                    if matches!(line, DiffLine::Context(_, _, _)) {
                        context_after_change += 1;
                        if context_after_change > context_lines {
                            lines_to_keep = i + 1;
                            break;
                        }
                    } else {
                        // Hit a change line, reset counter
                        context_after_change = 0;
                    }
                }

                current_hunk_lines.truncate(lines_to_keep);

                if let Some(hunk) = create_hunk_from_lines(current_hunk_lines.clone()) {
                    hunks.push(hunk);
                }
                current_hunk_lines.clear();
                last_change_idx = None;
            }
        }
    }

    // Handle remaining hunk
    if !current_hunk_lines.is_empty() {
        // Trim final hunk to exactly context_lines after the last change
        let mut lines_to_keep = current_hunk_lines.len();
        let mut context_after_change = 0;

        // Count backwards from the end to find where to cut off
        for (i, line) in current_hunk_lines.iter().enumerate().rev() {
            if matches!(line, DiffLine::Context(_, _, _)) {
                context_after_change += 1;
                if context_after_change > context_lines {
                    lines_to_keep = i + 1;
                    break;
                }
            } else {
                // Hit a change line, reset counter
                context_after_change = 0;
            }
        }

        current_hunk_lines.truncate(lines_to_keep);

        if let Some(hunk) = create_hunk_from_lines(current_hunk_lines) {
            hunks.push(hunk);
        }
    }

    hunks
}

fn create_hunk_from_lines(lines: Vec<DiffLine>) -> Option<DiffHunk> {
    if lines.is_empty() {
        return None;
    }

    let mut old_start = usize::MAX;
    let mut new_start = usize::MAX;
    let mut old_count = 0;
    let mut new_count = 0;

    for line in &lines {
        match line {
            DiffLine::Context(_, old_num, new_num) => {
                old_start = old_start.min(*old_num);
                new_start = new_start.min(*new_num);
                old_count += 1;
                new_count += 1;
            }
            DiffLine::Addition(_, new_num) => {
                new_start = new_start.min(*new_num);
                new_count += 1;
            }
            DiffLine::Deletion(_, old_num) => {
                old_start = old_start.min(*old_num);
                old_count += 1;
            }
        }
    }

    Some(DiffHunk {
        old_start,
        old_count,
        new_start,
        new_count,
        lines,
    })
}

fn format_hunk(hunk: &DiffHunk) -> String {
    let mut result = Vec::new();

    // Add hunk header
    result.push(format!(
        "@@ previous script: lines {}-{} @@",
        hunk.old_start,
        hunk.old_start + hunk.old_count - 1
    ));
    result.push(format!(
        "@@  current script: lines {}-{} @@",
        hunk.new_start,
        hunk.new_start + hunk.new_count - 1
    ));

    // Add hunk content with line numbers
    for line in &hunk.lines {
        match line {
            DiffLine::Context(content, _, new_num) => {
                result.push(format!("  {} {}", new_num, content));
            }
            DiffLine::Addition(content, new_num) => {
                result.push(format!("+ {} {}", new_num, content));
            }
            DiffLine::Deletion(content, old_num) => {
                result.push(format!("- {} {}", old_num, content));
            }
        }
    }

    result.join("\n")
}

#[derive(Debug, thiserror::Error)]
pub enum CommentError {
    #[error("Git command failed: {0}")]
    LocalGitError(#[from] LocalGitError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::ObjectId;
    use octocrab::models::issues::Issue;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[derive(Debug, Deserialize)]
    struct TestConfig {
        name: String,
        #[allow(dead_code)]
        description: String,
        issue_file: String,
        file_path: String,
        current_commit: String,
        previous_commit: Option<String>,
        note: Option<String>,
        no_diff: bool,
        previous_content: Option<ContentSection>,
        current_content: Option<ContentSection>,
    }

    #[derive(Debug, Deserialize)]
    struct ContentSection {
        content: String,
    }

    struct MockGitInfo {
        file_contents: HashMap<(PathBuf, String), String>,
    }

    impl MockGitInfo {
        fn new() -> Self {
            Self {
                file_contents: HashMap::new(),
            }
        }

        fn set_file_content(&mut self, file: PathBuf, commit: String, content: String) {
            self.file_contents.insert((file, commit), content);
        }
    }

    impl GitHelpers for MockGitInfo {
        fn file_content_url(&self, _commit: &str, _file: &std::path::Path) -> String {
            "https://github.com/owner/repo/blob/commit/file".to_string()
        }

        fn commit_comparison_url(
            &self,
            _current_commit: &gix::ObjectId,
            _previous_commit: &gix::ObjectId,
        ) -> String {
            "https://github.com/owner/repo/compare/prev..current".to_string()
        }
    }

    impl LocalGitInfo for MockGitInfo {
        fn commit(&self) -> Result<String, LocalGitError> {
            Ok("test_commit".to_string())
        }

        fn branch(&self) -> Result<String, LocalGitError> {
            Ok("test-branch".to_string())
        }

        fn file_commits(
            &self,
            _file: &std::path::Path,
        ) -> Result<Vec<(gix::ObjectId, String)>, LocalGitError> {
            Ok(Vec::new())
        }

        fn authors(
            &self,
            _file: &std::path::Path,
        ) -> Result<Vec<crate::git::local::GitAuthor>, LocalGitError> {
            Ok(Vec::new())
        }

        fn file_content_at_commit(
            &self,
            file: &std::path::Path,
            commit: &gix::ObjectId,
        ) -> Result<String, LocalGitError> {
            let key = (file.to_path_buf(), commit.to_string());
            self.file_contents
                .get(&key)
                .cloned()
                .ok_or_else(|| LocalGitError::FileNotFoundAtCommit(file.to_path_buf()))
        }

        fn status(&self) -> Result<crate::git::local::GitStatus, LocalGitError> {
            Ok(crate::git::local::GitStatus::Clean)
        }

        fn file_status(&self, _file: &std::path::Path) -> Result<crate::git::local::GitStatus, LocalGitError> {
            Ok(crate::git::local::GitStatus::Clean)
        }
    }

    fn load_test_config(test_file: &str) -> TestConfig {
        let path = format!("src/tests/comments/{}", test_file);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read test config file: {}", path));

        toml::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse test config file {}: {}", path, e))
    }

    fn load_issue(issue_file: &str) -> Issue {
        let path = format!("src/tests/github_api/issues/{}", issue_file);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    fn create_comment_from_config(config: &TestConfig) -> (QCComment, MockGitInfo) {
        let issue = load_issue(&config.issue_file);

        let current_commit = ObjectId::from_str(&config.current_commit)
            .unwrap_or_else(|_| panic!("Invalid current commit: {}", config.current_commit));

        let previous_commit = config.previous_commit.as_ref().map(|c| {
            ObjectId::from_str(c).unwrap_or_else(|_| panic!("Invalid previous commit: {}", c))
        });

        let comment = QCComment {
            file: PathBuf::from(&config.file_path),
            issue,
            current_commit,
            previous_commit,
            note: config.note.clone(),
            no_diff: config.no_diff,
        };

        let mut git_info = MockGitInfo::new();

        // Set up file content for current commit
        if let Some(current_content) = &config.current_content {
            git_info.set_file_content(
                PathBuf::from(&config.file_path),
                config.current_commit.clone(),
                current_content.content.clone(),
            );
        }

        // Set up file content for previous commit if it exists
        if let (Some(previous_commit), Some(previous_content)) =
            (&config.previous_commit, &config.previous_content)
        {
            git_info.set_file_content(
                PathBuf::from(&config.file_path),
                previous_commit.clone(),
                previous_content.content.clone(),
            );
        }

        (comment, git_info)
    }

    fn run_comment_test(test_file: &str) {
        let config = load_test_config(test_file);
        let (comment, git_info) = create_comment_from_config(&config);

        let result = comment.body(&git_info).unwrap_or_else(|e| {
            panic!(
                "Failed to generate comment body for test {}: {}",
                config.name, e
            )
        });

        // Use insta with a test-specific name
        let test_name = format!("comment_body_{}", config.name);
        insta::assert_snapshot!(test_name, result);
    }

    #[test]
    fn test_all_comment_scenarios() {
        // Get all .toml files in the test comments directory
        let test_dir = std::path::Path::new("src/tests/comments");

        if !test_dir.exists() {
            panic!("Test comments directory does not exist: {:?}", test_dir);
        }

        let mut test_files = std::fs::read_dir(test_dir)
            .unwrap_or_else(|e| panic!("Failed to read test comments directory: {}", e))
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension()? == "toml" {
                    path.file_name()?.to_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // Sort for consistent test ordering
        test_files.sort();

        if test_files.is_empty() {
            panic!("No test files found in {}", test_dir.display());
        }

        println!(
            "Running comment tests for {} files: {:?}",
            test_files.len(),
            test_files
        );

        for test_file in test_files {
            println!("Running test: {}", test_file);
            run_comment_test(&test_file);
        }
    }

    // Individual test functions for easier debugging
    #[test]
    fn test_single_hunk_change() {
        run_comment_test("single_hunk_change.toml");
    }

    #[test]
    fn test_multiple_hunks() {
        run_comment_test("multiple_hunks.toml");
    }

    #[test]
    fn test_no_diff_flag() {
        run_comment_test("no_diff_flag.toml");
    }

    #[test]
    fn test_no_previous_commit() {
        run_comment_test("no_previous_commit.toml");
    }

    #[test]
    fn test_separated_hunks() {
        run_comment_test("separated_hunks.toml");
    }
}
