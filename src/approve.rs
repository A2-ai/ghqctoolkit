use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::git::GitHelpers;

pub struct QCApprove {
    pub(crate) file: PathBuf,
    pub(crate) commit: ObjectId,
    pub(crate) issue: Issue,
    pub(crate) note: Option<String>,
}

impl QCApprove {
    pub fn body(&self, git_info: &impl GitHelpers) -> String {
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
}

pub struct QCUnapprove {
    pub(crate) issue: Issue,
    pub(crate) reason: String,
}

impl QCUnapprove {
    pub fn body(&self) -> String {
        vec!["# QC Un-Approval", &self.reason].join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitHelpers;
    use std::path::Path;

    // Mock implementation for testing
    struct MockGitHelpers;

    impl GitHelpers for MockGitHelpers {
        fn file_content_url(&self, commit_sha: &str, file: &Path) -> String {
            format!("https://github.com/owner/repo/blob/{}/{}", commit_sha, file.display())
        }

        fn commit_comparison_url(
            &self,
            _current_commit: &gix::ObjectId,
            _previous_commit: &gix::ObjectId,
        ) -> String {
            "https://github.com/owner/repo/compare/abc123..def456".to_string()
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
        let body = approve.body(&git_helpers);
        
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
        let body = approve.body(&git_helpers);
        
        insta::assert_snapshot!(body);
    }

    #[test]
    fn test_qc_unapprove_body() {
        let issue = load_issue("test_file_issue");
        
        let unapprove = QCUnapprove {
            issue,
            reason: "Found critical security vulnerability that needs to be addressed.".to_string(),
        };

        let body = unapprove.body();
        
        insta::assert_snapshot!(body);
    }
}
