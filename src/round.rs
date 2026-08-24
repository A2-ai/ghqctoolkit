//! The `# QC Round N` comment (D3) — the only way rounds 2..n are declared.
//!
//! Never by editing the issue body, never by labels: the comment log is the audit
//! surface and it is append-only.

use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::comment_system::CommentBody;
use crate::configuration::Checklist;
use crate::git::{GitFileOps, GitHelpers};

/// A round declaration comment. It carries metadata + the round's checklist and
/// **nothing else** (D4): never a diff, so `body_splitter` can never split a
/// checklist across comment parts. The "notify the difference" half is a separate,
/// ordinary `# QC Notification` comment (D5).
#[derive(Debug, Clone)]
pub struct QCRound {
    pub file: PathBuf,
    pub issue: Issue,
    /// 1-based round index; a round comment only ever declares 2..n.
    pub round: u32,
    /// This round's branch (D7) — read-only from the checkout (D23).
    pub branch: String,
    /// This round's start commit (D13: `CommitStatus::Initial` per round).
    pub start_commit: ObjectId,
    pub checklist: Checklist,
    /// The leading heading, kept so `title()` can hand out a `&str` that matches the
    /// `# {title}` line `body_splitter` looks for.
    title: String,
}

impl QCRound {
    pub fn new(
        file: PathBuf,
        issue: Issue,
        round: u32,
        branch: String,
        start_commit: ObjectId,
        checklist: Checklist,
    ) -> Self {
        Self {
            file,
            issue,
            round,
            branch,
            start_commit,
            checklist,
            title: format!("QC Round {round}"),
        }
    }
}

impl CommentBody for QCRound {
    /// The **exact same metadata keys as the issue body** (D20) — `initial qc commit:`
    /// means the same thing in both places and is parsed by the same
    /// `parse_commit_from_pattern` — plus one addition, `round: {N}`.
    ///
    /// No `author:` / `collaborators:`: those are issue-level (D20/R14). Metadata
    /// always precedes the checklist, so the metadata occurrence of
    /// `initial qc commit: ` is the one `find` hits.
    fn generate_body(&self, git_info: &(impl GitHelpers + GitFileOps)) -> String {
        let commit = self.start_commit.to_string();
        let short_commit = &commit[..commit.len().min(7)];
        let metadata = vec![
            "## Metadata".to_string(),
            format!("round: {}", self.round),
            format!("initial qc commit: {commit}"),
            format!("git branch: {}", self.branch),
            format!(
                "[file contents at initial qc commit]({})",
                git_info.file_content_url(short_commit, &self.file)
            ),
        ];

        // Detection keys off this H1, never off a metadata key (D20); the checklist
        // section is then the **second** H1 to the end of the comment (F4/R5).
        let body = vec![
            format!("# {}", self.title),
            metadata.join("\n* "),
            self.checklist.to_string(),
        ];

        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }

    fn title(&self) -> &str {
        &self.title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitAuthor, GitFileOpsError};
    use std::path::Path;

    struct MockGitHelpers;

    impl GitHelpers for MockGitHelpers {
        fn file_content_url(&self, git_ref: &str, file: &Path) -> String {
            format!(
                "https://github.com/owner/repo/blob/{git_ref}/{}",
                file.display()
            )
        }

        fn commit_comparison_url(&self, _current: &ObjectId, _previous: &ObjectId) -> String {
            "https://github.com/owner/repo/compare/abc123..def456".to_string()
        }

        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/owner/repo/issues/{issue_number}")
        }
    }

    impl GitFileOps for MockGitHelpers {
        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    fn round_comment() -> QCRound {
        let issue = crate::test_utils::create_test_issue(
            "owner",
            "repo",
            7,
            "src/main.rs",
            "body",
            Some(1),
            "closed",
        );
        QCRound::new(
            PathBuf::from("src/main.rs"),
            issue,
            2,
            "feature/round-two".to_string(),
            ObjectId::from_hex(b"1234567890abcdef1234567890abcdef12345678").unwrap(),
            Checklist {
                name: "Second Pass".to_string(),
                content: "- [ ] check one\n- [ ] check two\n".to_string(),
            },
        )
    }

    #[test]
    fn test_round_comment_body_carries_the_d20_key_set() {
        let body = round_comment().generate_body(&MockGitHelpers);

        assert!(body.starts_with("# QC Round 2\n"));
        assert!(body.contains("\n* round: 2\n"));
        assert!(body.contains("\n* initial qc commit: 1234567890abcdef1234567890abcdef12345678\n"));
        assert!(body.contains("\n* git branch: feature/round-two\n"));
        assert!(body.contains(
            "\n* [file contents at initial qc commit](https://github.com/owner/repo/blob/1234567/src/main.rs)"
        ));

        // D20: the round comment carries no issue-level people keys.
        assert!(!body.contains("author:"));
        assert!(!body.contains("collaborators:"));
    }

    #[test]
    fn test_round_comment_body_carries_the_checklist_and_no_diff() {
        let body = round_comment().generate_body(&MockGitHelpers);

        // The checklist is the second H1, running to the end of the comment (F4).
        assert!(body.contains("\n# Second Pass\n\n- [ ] check one\n- [ ] check two\n"));

        // D4: never a diff, so `body_splitter` can never split a checklist across
        // comment parts.
        assert!(!body.contains("## File Difference"));
        assert!(!body.contains("```diff"));
        assert!(!body.contains("previous commit:"));
    }

    /// The fold's round detection and metadata parsing must read back exactly what
    /// this type writes (D20/F3/F4).
    #[test]
    fn test_round_comment_body_round_trips_through_the_fold_parsers() {
        let body = round_comment().generate_body(&MockGitHelpers);

        assert_eq!(
            crate::parse_branch_from_body(&body).as_deref(),
            Some("feature/round-two")
        );
        assert!(body.lines().any(|line| line.starts_with("# QC Round")));

        // The metadata occurrence wins over the `[file contents ...]` link text,
        // which is why the key ordering in `generate_body` matters.
        let after_key = body
            .find("initial qc commit: ")
            .map(|start| &body[start + "initial qc commit: ".len()..])
            .and_then(|rest| rest.lines().next())
            .map(|line| line.split_whitespace().next().unwrap_or_default());
        assert_eq!(after_key, Some("1234567890abcdef1234567890abcdef12345678"));
    }
}
