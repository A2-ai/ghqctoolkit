use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;
use serde::{Deserialize, Serialize};

use crate::comment_system::CommentBody;
use crate::git::{GitFileOps, GitHelpers};

pub struct QCApprove {
    pub file: PathBuf,
    pub commit: ObjectId,
    pub issue: Issue,
    pub note: Option<String>,
}

impl CommentBody for QCApprove {
    fn generate_body(&self, git_info: &(impl GitHelpers + GitFileOps)) -> String {
        let short_sha = &self.commit.to_string()[..7];
        let metadata = vec![
            "## Metadata".to_string(),
            format!("approved qc commit: {}", self.commit),
            format!(
                "[file contents at approved qc commit]({})",
                git_info.file_content_url(short_sha, &self.file)
            ),
        ];

        let mut body = vec!["# QC Approved".to_string()];

        if let Some(note) = &self.note {
            body.push(note.clone());
        }

        body.push(metadata.join("\n* "));
        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QCUnapprove {
    pub issue: Issue,
    pub reason: String,
}

impl CommentBody for QCUnapprove {
    fn generate_body(&self, _git_info: &(impl GitHelpers + GitFileOps)) -> String {
        // Enhanced QCUnapprove now uses GitHelpers for consistency
        let metadata = vec![
            "## Metadata".to_string(),
            format!("issue: #{}", self.issue.number),
            format!("unapproval reason: {}", self.reason),
        ];

        let mut body = vec!["# QC Un-Approval".to_string()];
        body.push(self.reason.clone());
        body.push(metadata.join("\n* "));
        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comment_system::CommentBody;
    use crate::git::{GitAuthor, GitCommit, GitFileOps, GitFileOpsError, GitHelpers};
    use std::path::Path;

    // Mock implementation for testing
    struct MockGitHelpers;

    impl GitHelpers for MockGitHelpers {
        fn file_content_url(&self, commit_sha: &str, file: &Path) -> String {
            format!(
                "https://github.com/owner/repo/blob/{}/{}",
                commit_sha,
                file.display()
            )
        }

        fn commit_comparison_url(
            &self,
            _current_commit: &gix::ObjectId,
            _previous_commit: &gix::ObjectId,
        ) -> String {
            "https://github.com/owner/repo/compare/abc123..def456".to_string()
        }

        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/owner/repo/issues/{issue_number}")
        }
    }

    impl GitFileOps for MockGitHelpers {
        fn commits(&self, _branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &Path,
            _commit: &gix::ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    fn load_issue(name: &str) -> Issue {
        let json_str =
            std::fs::read_to_string(format!("src/tests/github_api/issues/{}.json", name)).unwrap();
        serde_json::from_str(&json_str).unwrap()
    }

    #[test]
    fn test_qc_approve_body_with_note() {
        let commit = gix::ObjectId::from_hex(b"1234567890abcdef1234567890abcdef12345678").unwrap();
        let issue = load_issue("main_file_issue");

        let approve = QCApprove {
            file: PathBuf::from("src/main.rs"),
            commit,
            issue,
            note: Some("Everything looks good!".to_string()),
        };

        let git_helpers = MockGitHelpers;
        let body = approve.generate_body(&git_helpers);

        insta::assert_snapshot!(body);
    }

    #[test]
    fn test_qc_approve_body_without_note() {
        let commit = gix::ObjectId::from_hex(b"abcdef1234567890abcdef1234567890abcdef12").unwrap();
        let issue = load_issue("config_file_issue");

        let approve = QCApprove {
            file: PathBuf::from("src/lib.rs"),
            commit,
            issue,
            note: None,
        };

        let git_helpers = MockGitHelpers;
        let body = approve.generate_body(&git_helpers);

        insta::assert_snapshot!(body);
    }

    #[test]
    fn test_qc_unapprove_body() {
        let issue = load_issue("test_file_issue");

        let unapprove = QCUnapprove {
            issue,
            reason: "Found critical security vulnerability that needs to be addressed.".to_string(),
        };

        let git_helpers = MockGitHelpers;
        let body = unapprove.generate_body(&git_helpers);

        insta::assert_snapshot!(body);
    }
}
