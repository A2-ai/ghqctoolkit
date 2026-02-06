use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::comment_system::CommentBody;
use crate::diff_utils;
use crate::git::{GitFileOps, GitHelpers};

#[derive(Debug, Clone)]
pub struct QCComment {
    pub file: PathBuf,
    pub issue: Issue,
    pub current_commit: ObjectId,
    pub previous_commit: Option<ObjectId>,
    pub note: Option<String>,
    pub no_diff: bool,
}

impl CommentBody for QCComment {
    fn generate_body(&self, git_info: &(impl GitHelpers + GitFileOps)) -> String {
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
                if let Some(difference) =
                    self.file_diff(&previous_commit, &self.current_commit, git_info)
                {
                    body.push(format!("## File Difference\n{}", difference));
                } else {
                    log::warn!("Could not generate diff for file {:?}", self.file);
                }
            } else {
                log::debug!("Previous Commit not specified. Cannot generate diff...");
            }
        }

        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }
}

impl QCComment {
    /// Generate a diff between two commits for this comment's file
    fn file_diff(
        &self,
        from_commit: &ObjectId,
        to_commit: &ObjectId,
        git_info: &impl GitFileOps,
    ) -> Option<String> {
        let Ok(from_bytes) = git_info.file_bytes_at_commit(&self.file, from_commit) else {
            log::debug!("Could not read file at from commit ({from_commit})...");
            return None;
        };
        // Get bytes from both commits
        let to_bytes = git_info.file_bytes_at_commit(&self.file, to_commit).ok()?;

        // Use the shared diff utilities
        diff_utils::file_diff(from_bytes, to_bytes, &self.file)
    }
}

#[cfg(test)]
mod tests {
    use crate::GitFileOpsError;
    use crate::{GitAuthor, git::GitCommit};

    use super::*;
    use crate::comment_system::CommentBody;
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
        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/owner/repo/issues/{issue_number}")
        }
    }

    impl GitFileOps for MockGitInfo {
        fn commits(&self, _branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn authors(&self, _file: &std::path::Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            file: &std::path::Path,
            commit: &gix::ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            let key = (file.to_path_buf(), commit.to_string());
            Ok(self
                .file_contents
                .get(&key)
                .cloned()
                .ok_or_else(|| GitFileOpsError::FileNotFoundAtCommit(file.to_path_buf()))?
                .into_bytes())
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

        let result = comment.generate_body(&git_info);

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
