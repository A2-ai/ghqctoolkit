use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::comment_system::CommentBody;
use crate::diff_utils;
use crate::git::{GitFileOps, GitHelpers};

#[derive(Debug, Clone)]
pub struct QCReview {
    pub file: PathBuf,
    pub issue: Issue,
    pub commit: ObjectId, // Commit to compare against (defaults to HEAD)
    pub note: Option<String>,
    pub no_diff: bool,
    pub working_dir: PathBuf, // Working directory path for reading local files
}

impl CommentBody for QCReview {
    fn generate_body(&self, git_info: &(impl GitHelpers + GitFileOps)) -> String {
        let metadata = vec![
            "## Metadata".to_string(),
            format!("comparing commit: {}", self.commit),
            format!(
                "[file at commit]({})",
                git_info.file_content_url(&self.commit.to_string()[..7], &self.file)
            ),
        ];

        let mut body = vec![
            "# QC Review".to_string(),
            format!("@{}", self.issue.user.login),
        ];

        if let Some(note) = &self.note {
            body.push(note.clone());
        }

        body.push(metadata.join("\n* "));

        if !self.no_diff {
            if let Some(difference) = self.file_diff_to_local(git_info) {
                body.push(format!("## File Difference\n{}", difference));
            } else {
                log::warn!("Could not generate diff for file {:?}", self.file);
            }
        }

        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }
}

impl QCReview {
    /// Generate a diff between a commit and the current working directory
    fn file_diff_to_local(&self, git_info: &impl GitFileOps) -> Option<String> {
        // Get file bytes from the commit
        let commit_bytes = match git_info.file_bytes_at_commit(&self.file, &self.commit) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!(
                    "Failed to read file {:?} at commit {}: {}",
                    self.file,
                    self.commit,
                    e
                );
                return None;
            }
        };

        // Get file bytes from working directory
        let working_file_path = self.working_dir.join(&self.file);
        let local_bytes = match std::fs::read(&working_file_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!(
                    "Failed to read file {:?} from working directory (tried path: {:?}): {}",
                    self.file,
                    working_file_path,
                    e
                );
                return None;
            }
        };

        // Use the shared diff utilities
        diff_utils::file_diff(commit_bytes, local_bytes, &self.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitAuthor, GitFileOpsError, git::GitCommit};
    use gix::ObjectId;
    use octocrab::models::issues::Issue;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::str::FromStr;

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
        fn file_content_url(&self, commit: &str, file: &std::path::Path) -> String {
            format!(
                "https://github.com/owner/repo/blob/{}/{}",
                commit,
                file.display()
            )
        }

        fn commit_comparison_url(
            &self,
            _current_commit: &gix::ObjectId,
            _previous_commit: &gix::ObjectId,
        ) -> String {
            "https://github.com/owner/repo/compare/prev..current".to_string()
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

    #[test]
    fn test_review_body_generation() {
        let mut git_info = MockGitInfo::new();
        let commit = ObjectId::from_str("1234567890abcdef1234567890abcdef12345678").unwrap();
        let file_path = PathBuf::from("src/example.rs");

        // Set up mock commit content
        git_info.set_file_content(
            file_path.clone(),
            commit.to_string(),
            "fn old_function() {\n    println!(\"old\");\n}".to_string(),
        );

        // Create a mock issue using existing test data
        let json_str =
            std::fs::read_to_string("src/tests/github_api/issues/test_file_issue.json").unwrap();
        let mut issue: Issue = serde_json::from_str(&json_str).unwrap();
        // Customize the issue for our test
        issue.number = 123;
        issue.title = "Test Issue".to_string();
        issue.user.login = "testuser".to_string();

        let review = QCReview {
            file: file_path,
            issue,
            commit,
            note: Some("Testing commit-to-local diff".to_string()),
            no_diff: true, // Skip diff for this test
            working_dir: PathBuf::from("/tmp/test-repo"), // Test working directory
        };

        let body = review.generate_body(&git_info);

        assert!(body.contains("# QC Review"));
        assert!(body.contains("Testing commit-to-local diff"));
        assert!(body.contains("comparing commit: 1234567890abcdef1234567890abcdef12345678"));
        assert!(body.contains(
            "[file at commit](https://github.com/owner/repo/blob/1234567/src/example.rs)"
        ));
    }
}
