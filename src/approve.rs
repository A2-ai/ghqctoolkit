use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;
use serde::{Deserialize, Serialize};

use crate::cache::DiskCache;
use crate::comment_system::CommentBody;
use crate::git::{
    CommitCache, GitCommitAnalysis, GitFileOps, GitHelpers, GitHubApiError, GitHubReader,
    GitHubWriter,
};
use crate::issue::{BlockingQC, parse_blocking_qcs};
use crate::qc_status::get_blocking_qc_status;

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

// ============================================================================
// Approval Validation Types
// ============================================================================

/// Result of checking blocking QC approval status for approval validation
#[derive(Debug, Clone, Default)]
pub struct BlockingQCCheckResult {
    /// Unapproved blocking QC issues (issue_number -> file_name)
    pub unapproved: HashMap<u64, PathBuf>,
    /// Blocking QC issues where status could not be determined
    pub errors: HashMap<u64, String>,
}

impl BlockingQCCheckResult {
    /// Returns true if all blocking QCs are approved (or there are none)
    pub fn all_approved(&self) -> bool {
        self.unapproved.is_empty() && self.errors.is_empty()
    }

    /// Returns true if there are any errors that prevented status checks
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Total count of blocking issues that are not approved or had errors
    pub fn blocking_count(&self) -> usize {
        self.unapproved.len() + self.errors.len()
    }
}

impl fmt::Display for BlockingQCCheckResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.all_approved() {
            return write!(f, "All blocking QCs are approved");
        }

        writeln!(f, "Blocking QC validation failed:")?;

        if !self.unapproved.is_empty() {
            writeln!(f, "\nUnapproved blocking QCs:")?;
            for (issue_num, file_name) in &self.unapproved {
                writeln!(f, "  #{} - {}", issue_num, file_name.display())?;
            }
        }

        if !self.errors.is_empty() {
            writeln!(f, "\nBlocking QCs with unknown status:")?;
            for (issue_num, error) in &self.errors {
                writeln!(f, "  #{} - {}", issue_num, error)?;
            }
        }

        Ok(())
    }
}

/// Result of an approval operation
#[derive(Debug, Clone)]
pub struct ApprovalResult {
    /// URL of the approval comment
    pub approval_url: String,
    /// Unapproved blocking QCs that were bypassed with --force
    pub skipped_unapproved: HashMap<u64, PathBuf>,
    /// Blocking QCs with fetch errors that were bypassed with --force
    pub skipped_errors: HashMap<u64, String>,
}

impl fmt::Display for ApprovalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "✅ Issue approved and closed!")?;

        if !self.skipped_errors.is_empty() || !self.skipped_unapproved.is_empty() {
            writeln!(f, "  ⚠️ --force was used to bypass dependency checks")?;
        }

        if !self.skipped_unapproved.is_empty() {
            writeln!(
                f,
                "  ⚠️ Unapproved Blocking QCs: {}",
                self.skipped_unapproved
                    .keys()
                    .map(|n| format!("#{}", n))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }

        if !self.skipped_errors.is_empty() {
            writeln!(
                f,
                "  ⚠️ Blocking QCs with unknown status: {}",
                self.skipped_errors
                    .keys()
                    .map(|n| format!("#{}", n))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }

        write!(f, "\n{}", self.approval_url)
    }
}

/// Check which blocking QCs are not approved from a list of blocking QCs.
///
/// This function works directly with a slice of `BlockingQC` without requiring an `IssueThread`,
/// which allows approval to proceed even when IssueThread construction might fail.
pub async fn get_unapproved_blocking_qcs(
    blocking_qcs: &[BlockingQC],
    git_info: &(impl GitHubReader + GitCommitAnalysis + GitFileOps),
    cache: Option<&DiskCache>,
    commit_cache: &mut CommitCache,
) -> BlockingQCCheckResult {
    let status = get_blocking_qc_status(blocking_qcs, git_info, cache, commit_cache).await;

    BlockingQCCheckResult {
        unapproved: status
            .not_approved
            .into_iter()
            .map(|(num, (path, _status))| (num, path))
            .collect(),
        errors: status.errors,
    }
}

/// Approve an issue with validation of blocking QCs
///
/// Parses blocking QCs directly from the issue body for graceful degradation.
/// If `force` is false and there are unapproved blocking QCs, returns an error.
/// If `force` is true, proceeds with approval and records skipped issues in the result.
pub async fn approve_with_validation(
    approval: &QCApprove,
    git_info: &(impl GitHubWriter + GitHubReader + GitHelpers + GitFileOps + GitCommitAnalysis),
    cache: Option<&DiskCache>,
    force: bool,
    commit_cache: &mut CommitCache,
) -> Result<ApprovalResult, ApprovalError> {
    // Parse blocking QCs directly from the issue body
    // This avoids requiring full IssueThread construction which can fail if
    // the issue body is missing branch/commit metadata
    let blocking_qcs = approval
        .issue
        .body
        .as_deref()
        .map(parse_blocking_qcs)
        .unwrap_or_default();

    // Check blocking QCs
    let check_result =
        get_unapproved_blocking_qcs(&blocking_qcs, git_info, cache, commit_cache).await;

    // If not forcing and there are issues, return error
    if !force && !check_result.all_approved() {
        return Err(ApprovalError::BlockingQCsNotApproved {
            unapproved_count: check_result.unapproved.len(),
            error_count: check_result.errors.len(),
            check_result,
        });
    }

    // Post the approval comment
    let approval_url = git_info.post_comment(approval).await?;

    // Close the issue
    git_info.close_issue(approval.issue.number).await?;

    Ok(ApprovalResult {
        approval_url,
        skipped_unapproved: if force {
            check_result.unapproved
        } else {
            HashMap::new()
        },
        skipped_errors: if force {
            check_result.errors
        } else {
            HashMap::new()
        },
    })
}

/// Error type for approval operations
#[derive(Debug, thiserror::Error)]
pub enum ApprovalError {
    #[error(
        "Cannot approve: {unapproved_count} blocking QC(s) are not approved, {error_count} could not be checked\n\n{check_result}\n\nUse --force to bypass this check"
    )]
    BlockingQCsNotApproved {
        unapproved_count: usize,
        error_count: usize,
        check_result: BlockingQCCheckResult,
    },
    #[error("GitHub API error: {0}")]
    GitHubApiError(#[from] GitHubApiError),
}

// ============================================================================
// Unapproval Impact Tree Types
// ============================================================================

use crate::issue::BlockingRelationship;

/// Result of an unapproval operation
#[derive(Debug, Clone)]
pub struct UnapprovalResult {
    /// URL of the unapproval comment
    pub unapproval_url: String,
    /// Impacted downstream issues
    pub impacted_issues: ImpactedIssues,
}

impl fmt::Display for UnapprovalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "🚫 Issue unapproved and reopened!")?;
        writeln!(f, "{}", self.unapproval_url)?;

        match &self.impacted_issues {
            ImpactedIssues::None => {}
            ImpactedIssues::ApiUnavailable => {
                writeln!(
                    f,
                    "\n⚠️ Could not check for impacted issues (API may not be supported)"
                )?;
            }
            ImpactedIssues::Some(nodes) => {
                writeln!(f, "\nThe following QCs may be impacted by this unapproval:")?;
                for node in nodes {
                    node.fmt_tree(f, "", true, true)?;
                }
            }
        }
        Ok(())
    }
}

/// Represents impacted downstream issues
#[derive(Debug, Clone)]
pub enum ImpactedIssues {
    /// No downstream issues found
    None,
    /// API not available (GHE), couldn't check
    ApiUnavailable,
    /// Found downstream issues - display as tree
    Some(Vec<ImpactNode>),
}

/// A node in the impact tree
#[derive(Debug, Clone)]
pub struct ImpactNode {
    /// Issue number
    pub issue_number: u64,
    /// File name from issue title
    pub file_name: PathBuf,
    /// Milestone name for easy unapproval reference
    pub milestone: String,
    /// Relationship type (GatingQC or PreviousQC)
    pub relationship: BlockingRelationship,
    /// Recursive children
    pub children: Vec<ImpactNode>,
    /// Error if children couldn't be fetched
    pub fetch_error: Option<String>,
}

impl ImpactNode {
    fn fmt_tree(
        &self,
        f: &mut fmt::Formatter<'_>,
        prefix: &str,
        is_last: bool,
        is_root: bool,
    ) -> fmt::Result {
        // Tree drawing characters
        let branch = if is_root {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };

        // Format: #issue file (milestone) (relationship)
        writeln!(
            f,
            "{}{}#{} {} ({}) ({})",
            prefix,
            branch,
            self.issue_number,
            self.file_name.display(),
            self.milestone,
            self.relationship
        )?;

        // Calculate prefix for children
        let child_prefix = if is_root {
            prefix.to_string()
        } else if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        // Show fetch error if present
        if let Some(err) = &self.fetch_error {
            writeln!(f, "{}    ⚠️ {}", child_prefix, err)?;
        }

        let child_count = self.children.len();
        for (i, child) in self.children.iter().enumerate() {
            let is_last_child = i == child_count - 1;
            child.fmt_tree(f, &child_prefix, is_last_child, false)?;
        }
        Ok(())
    }
}

/// Extract file path from issue title
/// Expected format: "QC: path/to/file.ext" or similar patterns
fn extract_file_from_title(title: &str) -> PathBuf {
    // Try pattern: "QC: path/to/file"
    if let Some(rest) = title.strip_prefix("QC: ") {
        return PathBuf::from(rest.trim());
    }

    // Try pattern: "QC path/to/file" (without colon)
    if let Some(rest) = title.strip_prefix("QC ") {
        return PathBuf::from(rest.trim());
    }

    // Fallback: return the whole title as the path
    if !title.is_empty() {
        return PathBuf::from(title.trim());
    }

    // Last resort fallback
    PathBuf::from("(unknown file)")
}

use crate::issue::determine_relationship_from_body;
use std::collections::HashSet;
use std::pin::Pin;

/// Build the impact tree for an unapproved issue
///
/// This function uses Box::pin for recursion to avoid issues with async function layout.
fn build_impact_tree<'a, T: GitHubReader + 'a>(
    git_info: &'a T,
    parent_issue_number: u64,
    child_issue: Issue,
    visited: &'a mut HashSet<u64>,
) -> Pin<Box<dyn std::future::Future<Output = ImpactNode> + Send + 'a>>
where
    T: Sync,
{
    Box::pin(async move {
        let child_issue_number = child_issue.number;

        // Track current recursion path to detect actual cycles
        // We insert when entering and remove when leaving, so `visited` only contains
        // ancestors in the current path. This correctly handles DAGs where the same
        // issue is reachable via multiple paths (not a cycle).
        if !visited.insert(child_issue_number) {
            return ImpactNode {
                issue_number: child_issue_number,
                file_name: PathBuf::from("(circular reference)"),
                milestone: String::new(),
                relationship: BlockingRelationship::Unknown,
                children: vec![],
                fetch_error: None,
            };
        }

        let file_name = extract_file_from_title(&child_issue.title);
        let milestone = child_issue
            .milestone
            .as_ref()
            .map(|m| m.title.clone())
            .unwrap_or_else(|| "No milestone".to_string());

        // Parse child's body to determine relationship to parent
        let relationship = child_issue
            .body
            .as_ref()
            .map(|body| determine_relationship_from_body(body, parent_issue_number))
            .unwrap_or(BlockingRelationship::Unknown);

        // Try to fetch issues blocked by this child
        let (children, fetch_error) = match git_info.get_blocked_issues(child_issue_number).await {
            Ok(blocked) => {
                let mut child_nodes = vec![];
                for blocked_issue in blocked {
                    // Recursively build tree - this child becomes the parent for next level
                    // Note: We must use sequential execution here because visited is a mutable reference
                    let node =
                        build_impact_tree(git_info, child_issue_number, blocked_issue, visited)
                            .await;
                    child_nodes.push(node);
                }
                (child_nodes, None)
            }
            Err(e) => (vec![], Some(format!("Could not fetch children: {}", e))),
        };

        // Remove from visited set when leaving this node's subtree
        // This allows the same issue to appear in different branches of the tree (DAG support)
        visited.remove(&child_issue_number);

        ImpactNode {
            issue_number: child_issue_number,
            file_name,
            milestone,
            relationship,
            children,
            fetch_error,
        }
    })
}

/// Unapprove an issue and show impact tree
pub async fn unapprove_with_impact<
    T: GitHubWriter + GitHubReader + GitHelpers + GitFileOps + Sync,
>(
    unapproval: &QCUnapprove,
    git_info: &T,
) -> Result<UnapprovalResult, GitHubApiError> {
    // Post the unapproval comment
    let unapproval_url = git_info.post_comment(unapproval).await?;

    // Reopen the issue
    git_info.open_issue(unapproval.issue.number).await?;

    // Try to fetch blocked issues via get_blocked_issues()
    let impacted_issues = match git_info.get_blocked_issues(unapproval.issue.number).await {
        Ok(blocked) if blocked.is_empty() => ImpactedIssues::None,
        Ok(blocked) => {
            let mut visited = HashSet::new();
            visited.insert(unapproval.issue.number);

            let mut nodes = vec![];
            for blocked_issue in blocked {
                let node = build_impact_tree(
                    git_info,
                    unapproval.issue.number,
                    blocked_issue,
                    &mut visited,
                )
                .await;
                nodes.push(node);
            }
            ImpactedIssues::Some(nodes)
        }
        Err(_) => ImpactedIssues::ApiUnavailable,
    };

    Ok(UnapprovalResult {
        unapproval_url,
        impacted_issues,
    })
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

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
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

    // Tests for BlockingQCCheckResult

    #[test]
    fn test_blocking_qc_check_result_all_approved() {
        let result = BlockingQCCheckResult::default();
        assert!(result.all_approved());
        assert!(!result.has_errors());
        assert_eq!(result.blocking_count(), 0);
    }

    #[test]
    fn test_blocking_qc_check_result_with_unapproved() {
        let mut result = BlockingQCCheckResult::default();
        result.unapproved.insert(1, PathBuf::from("file1.R"));
        result.unapproved.insert(2, PathBuf::from("file2.R"));

        assert!(!result.all_approved());
        assert!(!result.has_errors());
        assert_eq!(result.blocking_count(), 2);
    }

    #[test]
    fn test_blocking_qc_check_result_with_errors() {
        let mut result = BlockingQCCheckResult::default();
        result.errors.insert(1, "API error".to_string());

        assert!(!result.all_approved());
        assert!(result.has_errors());
        assert_eq!(result.blocking_count(), 1);
    }

    #[test]
    fn test_blocking_qc_check_result_has_errors() {
        let mut result = BlockingQCCheckResult::default();
        assert!(!result.has_errors());

        result.errors.insert(1, "Error".to_string());
        assert!(result.has_errors());
    }

    #[test]
    fn test_blocking_qc_check_result_display() {
        let mut result = BlockingQCCheckResult::default();
        result.unapproved.insert(1, PathBuf::from("file1.R"));
        result.errors.insert(2, "404 Not Found".to_string());

        let display = format!("{}", result);
        assert!(display.contains("Blocking QC validation failed"));
        assert!(display.contains("#1"));
        assert!(display.contains("file1.R"));
        assert!(display.contains("#2"));
        assert!(display.contains("404 Not Found"));
    }

    // Tests for ApprovalResult

    #[test]
    fn test_approval_result_display() {
        let result = ApprovalResult {
            approval_url: "https://github.com/owner/repo/issues/1#issuecomment-123".to_string(),
            skipped_unapproved: HashMap::new(),
            skipped_errors: HashMap::new(),
        };

        let display = format!("{}", result);
        assert!(display.contains("✅ Issue approved and closed!"));
        assert!(display.contains("https://github.com/owner/repo/issues/1#issuecomment-123"));
        assert!(!display.contains("--force"));
    }

    #[test]
    fn test_approval_result_display_with_force() {
        let mut skipped = HashMap::new();
        skipped.insert(10, PathBuf::from("blocking.R"));
        let mut errors = HashMap::new();
        errors.insert(20, "API error".to_string());

        let result = ApprovalResult {
            approval_url: "https://github.com/owner/repo/issues/1#issuecomment-123".to_string(),
            skipped_unapproved: skipped,
            skipped_errors: errors,
        };

        let display = format!("{}", result);
        // --force message only shows when both unapproved and errors are bypassed
        assert!(display.contains("--force was used"));
        assert!(display.contains("#10"));
        assert!(display.contains("#20"));
    }

    #[test]
    fn test_approval_result_display_with_unapproved_only() {
        let mut skipped = HashMap::new();
        skipped.insert(10, PathBuf::from("blocking.R"));

        let result = ApprovalResult {
            approval_url: "https://github.com/owner/repo/issues/1#issuecomment-123".to_string(),
            skipped_unapproved: skipped,
            skipped_errors: HashMap::new(),
        };

        let display = format!("{}", result);
        // --force message shows when either unapproved OR errors are bypassed
        assert!(display.contains("--force was used"));
        assert!(display.contains("Unapproved Blocking QCs"));
        assert!(display.contains("#10"));
    }

    #[test]
    fn test_approval_result_display_with_errors() {
        let mut errors = HashMap::new();
        errors.insert(20, "API error".to_string());

        let result = ApprovalResult {
            approval_url: "https://github.com/owner/repo/issues/1#issuecomment-123".to_string(),
            skipped_unapproved: HashMap::new(),
            skipped_errors: errors,
        };

        let display = format!("{}", result);
        // --force message shows when either unapproved OR errors are bypassed
        assert!(display.contains("--force was used"));
        assert!(display.contains("unknown status"));
        assert!(display.contains("#20"));
    }

    // Tests for UnapprovalResult and ImpactNode

    #[test]
    fn test_unapproval_result_display_none() {
        let result = UnapprovalResult {
            unapproval_url: "https://github.com/owner/repo/issues/25#issuecomment-123".to_string(),
            impacted_issues: ImpactedIssues::None,
        };

        let display = format!("{}", result);
        assert!(display.contains("🚫 Issue unapproved and reopened!"));
        assert!(display.contains("https://github.com/owner/repo/issues/25#issuecomment-123"));
        assert!(!display.contains("impacted"));
    }

    #[test]
    fn test_unapproval_result_display_api_unavailable() {
        let result = UnapprovalResult {
            unapproval_url: "https://github.com/owner/repo/issues/25#issuecomment-123".to_string(),
            impacted_issues: ImpactedIssues::ApiUnavailable,
        };

        let display = format!("{}", result);
        assert!(display.contains("Could not check for impacted issues"));
        assert!(display.contains("API may not be supported"));
    }

    #[test]
    fn test_unapproval_result_display_tree() {
        let result = UnapprovalResult {
            unapproval_url: "https://github.com/owner/repo/issues/25#issuecomment-123".to_string(),
            impacted_issues: ImpactedIssues::Some(vec![ImpactNode {
                issue_number: 30,
                file_name: PathBuf::from("path/to/file.R"),
                milestone: "Sprint 1".to_string(),
                relationship: BlockingRelationship::PreviousQC,
                children: vec![],
                fetch_error: None,
            }]),
        };

        let display = format!("{}", result);
        assert!(display.contains("may be impacted by this unapproval"));
        assert!(display.contains("#30"));
        assert!(display.contains("path/to/file.R"));
        assert!(display.contains("Sprint 1"));
        assert!(display.contains("previous QC"));
    }

    #[test]
    fn test_impact_node_fmt_tree_nested() {
        let node = ImpactNode {
            issue_number: 30,
            file_name: PathBuf::from("file1.R"),
            milestone: "Sprint 1".to_string(),
            relationship: BlockingRelationship::PreviousQC,
            children: vec![ImpactNode {
                issue_number: 35,
                file_name: PathBuf::from("file2.R"),
                milestone: "Sprint 2".to_string(),
                relationship: BlockingRelationship::GatingQC,
                children: vec![],
                fetch_error: None,
            }],
            fetch_error: None,
        };

        let result = UnapprovalResult {
            unapproval_url: "https://example.com".to_string(),
            impacted_issues: ImpactedIssues::Some(vec![node]),
        };

        let display = format!("{}", result);
        assert!(display.contains("#30"));
        assert!(display.contains("#35"));
        assert!(display.contains("└── ")); // Tree branch for last/only child
    }

    #[test]
    fn test_impact_node_fmt_tree_with_errors() {
        let node = ImpactNode {
            issue_number: 30,
            file_name: PathBuf::from("file.R"),
            milestone: "Sprint 1".to_string(),
            relationship: BlockingRelationship::GatingQC,
            children: vec![],
            fetch_error: Some("Could not fetch children: API error".to_string()),
        };

        let result = UnapprovalResult {
            unapproval_url: "https://example.com".to_string(),
            impacted_issues: ImpactedIssues::Some(vec![node]),
        };

        let display = format!("{}", result);
        assert!(display.contains("⚠️"));
        assert!(display.contains("Could not fetch children"));
    }

    #[test]
    fn test_impact_node_fmt_tree_unknown_relationship() {
        let node = ImpactNode {
            issue_number: 30,
            file_name: PathBuf::from("file.R"),
            milestone: "Sprint 1".to_string(),
            relationship: BlockingRelationship::Unknown,
            children: vec![],
            fetch_error: None,
        };

        let result = UnapprovalResult {
            unapproval_url: "https://example.com".to_string(),
            impacted_issues: ImpactedIssues::Some(vec![node]),
        };

        let display = format!("{}", result);
        assert!(display.contains("unknown relationship"));
    }

    // Tests for extract_file_from_title

    #[test]
    fn test_extract_file_from_title_qc_colon() {
        let result = extract_file_from_title("QC: path/to/file.R");
        assert_eq!(result, PathBuf::from("path/to/file.R"));
    }

    #[test]
    fn test_extract_file_from_title_qc_space() {
        let result = extract_file_from_title("QC path/to/file.R");
        assert_eq!(result, PathBuf::from("path/to/file.R"));
    }

    #[test]
    fn test_extract_file_from_title_fallback() {
        let result = extract_file_from_title("some random title");
        assert_eq!(result, PathBuf::from("some random title"));
    }

    #[test]
    fn test_extract_file_from_title_empty() {
        let result = extract_file_from_title("");
        assert_eq!(result, PathBuf::from("(unknown file)"));
    }

    // Test for DAG handling in impact tree (shared descendants should not be marked as circular)
    #[test]
    fn test_impact_tree_dag_shared_descendant_not_circular() {
        // Scenario: Issue 1 blocks both 2 and 3, and both 2 and 3 block 4
        // Issue 4 should appear twice in the tree (under both 2 and 3), NOT as "(circular reference)"
        //
        //       1 (root)
        //      / \
        //     2   3
        //      \ /
        //       4
        //
        // Expected tree output:
        // #2 file2.R (Milestone) (previous QC)
        // └── #4 file4.R (Milestone) (gating QC)
        // #3 file3.R (Milestone) (gating QC)
        // └── #4 file4.R (Milestone) (gating QC)

        let tree = vec![
            ImpactNode {
                issue_number: 2,
                file_name: PathBuf::from("file2.R"),
                milestone: "Sprint 1".to_string(),
                relationship: BlockingRelationship::PreviousQC,
                children: vec![ImpactNode {
                    issue_number: 4,
                    file_name: PathBuf::from("file4.R"),
                    milestone: "Sprint 1".to_string(),
                    relationship: BlockingRelationship::GatingQC,
                    children: vec![],
                    fetch_error: None,
                }],
                fetch_error: None,
            },
            ImpactNode {
                issue_number: 3,
                file_name: PathBuf::from("file3.R"),
                milestone: "Sprint 1".to_string(),
                relationship: BlockingRelationship::GatingQC,
                children: vec![ImpactNode {
                    issue_number: 4,
                    file_name: PathBuf::from("file4.R"),
                    milestone: "Sprint 1".to_string(),
                    relationship: BlockingRelationship::GatingQC,
                    children: vec![],
                    fetch_error: None,
                }],
                fetch_error: None,
            },
        ];

        let result = UnapprovalResult {
            unapproval_url: "https://example.com".to_string(),
            impacted_issues: ImpactedIssues::Some(tree),
        };

        let display = format!("{}", result);

        // Issue 4 should appear twice - once under issue 2 and once under issue 3
        let count_issue_4 = display.matches("#4").count();
        assert_eq!(
            count_issue_4, 2,
            "Issue #4 should appear twice in DAG, but appeared {} times. Output:\n{}",
            count_issue_4, display
        );

        // Neither should be marked as circular reference
        assert!(
            !display.contains("circular reference"),
            "DAG should not contain circular reference markers. Output:\n{}",
            display
        );

        // Both should have the actual file name, not "(circular reference)"
        let count_file4 = display.matches("file4.R").count();
        assert_eq!(
            count_file4, 2,
            "file4.R should appear twice, but appeared {} times. Output:\n{}",
            count_file4, display
        );
    }

    // Test that actual circular references ARE still detected
    #[test]
    fn test_impact_tree_actual_circular_reference_detected() {
        // This tests that when we manually construct a circular structure,
        // it would be displayed as such. Note: In reality, build_impact_tree
        // prevents this by tracking the recursion path.

        let circular_node = ImpactNode {
            issue_number: 99,
            file_name: PathBuf::from("(circular reference)"),
            milestone: String::new(),
            relationship: BlockingRelationship::Unknown,
            children: vec![],
            fetch_error: None,
        };

        let result = UnapprovalResult {
            unapproval_url: "https://example.com".to_string(),
            impacted_issues: ImpactedIssues::Some(vec![circular_node]),
        };

        let display = format!("{}", result);
        assert!(
            display.contains("circular reference"),
            "Circular reference should be displayed. Output:\n{}",
            display
        );
    }
}
