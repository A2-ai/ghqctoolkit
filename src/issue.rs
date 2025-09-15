use std::{path::PathBuf, str::FromStr};

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::{
    GitHubApi,
    cache::{CachedComments, DiskCache},
    git::{
        api::GitHubApiError,
        local::{LocalGitError, LocalGitInfo},
    },
};

pub struct IssueThread {
    file: PathBuf,
    branch: String,
    pub(crate) initial_commit: ObjectId,
    pub(crate) notification_commits: Vec<ObjectId>,
    pub(crate) approved_commit: Option<ObjectId>,
}

impl IssueThread {
    pub async fn from_issue(
        issue: &Issue,
        cache: Option<&DiskCache>,
        git_info: &(impl GitHubApi + LocalGitInfo),
    ) -> Result<Self, IssueError> {
        let file = PathBuf::from(&issue.title);

        // 1. Parse the branch from the issue body first
        let branch = issue
            .body
            .as_ref()
            .and_then(|body| parse_branch_from_body(body))
            .ok_or_else(|| IssueError::BranchNotFound)?;

        // 2. Get all commits for this file from the specific branch
        let file_commits = git_info
            .file_commits(&file, &Some(branch.clone()))
            .map_err(|e| IssueError::LocalGitError(e))?;
        let commit_ids: Vec<ObjectId> = file_commits.iter().map(|(id, _)| *id).collect();

        let issue_is_open = matches!(issue.state, octocrab::models::IssueState::Open);

        // 3. Parse the initial commit ObjectId from the issue body
        let initial_commit = issue
            .body
            .as_ref()
            .and_then(|body| parse_commit_from_pattern(body, "initial qc commit: ", &commit_ids))
            .ok_or_else(|| IssueError::InitialCommitNotFound)?;

        // 4. Get comment bodies directly from the API
        let comment_bodies = get_cached_issue_comments(issue, cache, git_info).await?;

        // 5. Parse notification and approval commits from comment bodies
        let (notification_commits, approved_commit) =
            parse_commits_from_comments(&comment_bodies, &commit_ids, issue_is_open);

        Ok(Self {
            file,
            branch,
            initial_commit,
            notification_commits,
            approved_commit,
        })
    }

    pub fn latest_commit(&self) -> &ObjectId {
        if let Some(a_c) = &self.approved_commit {
            return a_c;
        }

        if let Some(last_notif) = self.notification_commits.last() {
            return last_notif;
        }

        &self.initial_commit
    }

    pub async fn commits(
        &self,
        git_info: &impl LocalGitInfo,
    ) -> Result<Vec<(ObjectId, String)>, IssueError> {
        let commits = git_info.file_commits(&self.file, &Some(self.branch.clone()))?;
        Ok(commits)
    }
}

/// Parse notification and approval commits from comment bodies
/// Returns (notification_commits, approved_commit)
/// Approval is invalidated if issue is open or if an unapproval occurs after approval
fn parse_commits_from_comments(
    comment_bodies: &[String],
    commit_ids: &[ObjectId],
    issue_is_open: bool,
) -> (Vec<ObjectId>, Option<ObjectId>) {
    let mut notification_commits = Vec::new();
    let mut approved_commit = None;
    let mut approval_comment_index = None;

    // Parse all comments in order
    for (index, body) in comment_bodies.iter().enumerate() {
        // Check for notification commit: "current commit: {hash}"
        if let Some(commit) = parse_commit_from_pattern(body, "current commit: ", commit_ids) {
            notification_commits.push(commit);
        }

        // Check for approval commit: "approved qc commit: {hash}"
        if let Some(commit) = parse_commit_from_pattern(body, "approved qc commit: ", commit_ids) {
            if issue_is_open {
                // If issue is open, treat approval as notification
                notification_commits.push(commit);
            } else {
                approved_commit = Some(commit);
                approval_comment_index = Some(index);
            }
        }

        // Check for unapproval: "# QC Un-Approval"
        if body.contains("# QC Un-Approval") {
            // If this unapproval comes after an approval, invalidate the approval
            if let Some(approval_index) = approval_comment_index {
                if index > approval_index {
                    // Move the approved commit to notifications and clear approval
                    if let Some(commit) = approved_commit {
                        notification_commits.push(commit);
                    }
                    approved_commit = None;
                    approval_comment_index = None;
                }
            }
        }
    }

    (notification_commits, approved_commit)
}

/// Parse a commit from a body using the given pattern
/// Supports both full and short SHAs with minimum 7 character length
fn parse_commit_from_pattern(
    body: &str,
    pattern: &str,
    commit_ids: &[ObjectId],
) -> Option<ObjectId> {
    let start = body.find(pattern)?;
    let commit_start = start + pattern.len();

    let remaining = &body[commit_start..];
    let commit_hash = remaining.lines().next()?.split_whitespace().next()?;

    // Try to parse as full ObjectId first
    if let Ok(full_oid) = ObjectId::from_str(commit_hash) {
        return Some(full_oid);
    }

    // If that fails, try to match as short SHA against file commits
    // Require at least 7 characters for short SHA to avoid ambiguity
    if commit_hash.len() >= 7 {
        for commit_id in commit_ids {
            let full_hash = commit_id.to_string();
            if full_hash.starts_with(commit_hash) {
                return Some(*commit_id);
            }
        }
    }

    None
}

/// Parse branch name from issue body
/// Looks for pattern: "git branch: <branch-name>"
fn parse_branch_from_body(body: &str) -> Option<String> {
    let pattern = "git branch: ";
    let start = body.find(pattern)?;
    let branch_start = start + pattern.len();

    let remaining = &body[branch_start..];
    let branch_name = remaining.lines().next()?.trim();

    if branch_name.is_empty() {
        return None;
    }

    Some(branch_name.to_string())
}

/// Get issue comments with caching based on issue update timestamp
pub async fn get_cached_issue_comments(
    issue: &Issue,
    cache: Option<&DiskCache>,
    git_info: &impl GitHubApi,
) -> Result<Vec<String>, GitHubApiError> {
    // Create cache key from issue number
    let cache_key = format!("issue_{}", issue.number);

    // Try to get cached comments first
    let cached_comments: Option<CachedComments> = if let Some(cache) = cache {
        cache.read::<CachedComments>(&["issues", "comments"], &cache_key)
    } else {
        None
    };

    // Check if cached comments are still valid by comparing timestamps
    if let Some(cached) = cached_comments {
        if cached.issue_updated_at >= issue.updated_at {
            log::debug!(
                "Using cached comments for issue #{} (cache timestamp: {}, issue timestamp: {})",
                issue.number,
                cached.issue_updated_at,
                issue.updated_at
            );
            return Ok(cached.comments);
        } else {
            log::debug!(
                "Cached comments for issue #{} are stale (cache: {}, issue: {})",
                issue.number,
                cached.issue_updated_at,
                issue.updated_at
            );
        }
    }

    // Fetch fresh comments from API
    log::debug!("Fetching fresh comments for issue #{}", issue.number);
    let comments = git_info.get_issue_comments(issue).await?;

    // Cache the comments with the current issue timestamp (permanently)
    if let Some(cache) = cache {
        let cached_comments = CachedComments {
            comments: comments.clone(),
            issue_updated_at: issue.updated_at,
        };

        if let Err(e) = cache.write(&["issues", "comments"], &cache_key, &cached_comments, false) {
            log::warn!(
                "Failed to cache comments for issue #{}: {}",
                issue.number,
                e
            );
        }
    }

    Ok(comments)
}

#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    #[error(transparent)]
    GitHubApiError(#[from] GitHubApiError),
    #[error(transparent)]
    LocalGitError(#[from] LocalGitError),
    #[error("Initial commit not found in issue body")]
    InitialCommitNotFound,
    #[error("Branch not found in issue body")]
    BranchNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitHelpers;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[derive(Clone)]
    struct MockGitInfo {
        file_commits: Vec<(ObjectId, String)>,
        comment_bodies: Vec<String>,
    }

    impl MockGitInfo {
        fn new() -> Self {
            Self {
                file_commits: Vec::new(),
                comment_bodies: Vec::new(),
            }
        }

        fn with_file_commits(mut self, commits: Vec<(ObjectId, String)>) -> Self {
            self.file_commits = commits;
            self
        }

        fn with_comment_bodies(mut self, bodies: Vec<String>) -> Self {
            self.comment_bodies = bodies;
            self
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
        fn commit(&self) -> Result<String, crate::git::local::LocalGitError> {
            Ok("test_commit".to_string())
        }

        fn branch(&self) -> Result<String, crate::git::local::LocalGitError> {
            Ok("test-branch".to_string())
        }

        fn file_commits(
            &self,
            _file: &std::path::Path,
            _branch: &Option<String>,
        ) -> Result<Vec<(gix::ObjectId, String)>, crate::git::local::LocalGitError> {
            Ok(self.file_commits.clone())
        }

        fn authors(
            &self,
            _file: &std::path::Path,
        ) -> Result<Vec<crate::git::local::GitAuthor>, crate::git::local::LocalGitError> {
            Ok(Vec::new())
        }

        fn file_content_at_commit(
            &self,
            _file: &std::path::Path,
            _commit: &gix::ObjectId,
        ) -> Result<String, crate::git::local::LocalGitError> {
            Ok(String::new())
        }

        fn status(&self) -> Result<crate::git::local::GitStatus, crate::git::local::LocalGitError> {
            Ok(crate::git::local::GitStatus::Clean)
        }

        fn file_status(
            &self,
            _file: &std::path::Path,
            _branch: &Option<String>,
        ) -> Result<crate::git::local::GitStatus, crate::git::local::LocalGitError> {
            Ok(crate::git::local::GitStatus::Clean)
        }

        fn owner(&self) -> &str {
            "test-owner"
        }

        fn repo(&self) -> &str {
            "test-repo"
        }
    }

    impl GitHubApi for MockGitInfo {
        async fn get_milestones(
            &self,
        ) -> Result<Vec<octocrab::models::Milestone>, crate::git::api::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_milestone_issues(
            &self,
            _milestone: &octocrab::models::Milestone,
        ) -> Result<Vec<Issue>, crate::git::api::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn create_milestone(
            &self,
            _milestone_name: &str,
        ) -> Result<octocrab::models::Milestone, crate::git::api::GitHubApiError> {
            unimplemented!()
        }

        async fn post_issue(
            &self,
            _issue: &crate::QCIssue,
        ) -> Result<String, crate::git::api::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1".to_string())
        }

        async fn post_comment(
            &self,
            _comment: &crate::QCComment,
        ) -> Result<String, crate::git::api::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1#issuecomment-1".to_string())
        }

        async fn post_approval(
            &self,
            _approval: &crate::QCApprove,
        ) -> Result<String, crate::git::api::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1#issuecomment-1".to_string())
        }

        async fn post_unapproval(
            &self,
            _unapproval: &crate::QCUnapprove,
        ) -> Result<String, crate::git::api::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1#issuecomment-1".to_string())
        }

        async fn get_assignees(&self) -> Result<Vec<String>, crate::git::api::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_user_details(
            &self,
            _username: &str,
        ) -> Result<crate::git::api::RepoUser, crate::git::api::GitHubApiError> {
            Ok(crate::git::api::RepoUser {
                login: _username.to_string(),
                name: None,
            })
        }

        async fn get_labels(&self) -> Result<Vec<String>, crate::git::api::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn create_label(
            &self,
            _name: &str,
            _color: &str,
        ) -> Result<(), crate::git::api::GitHubApiError> {
            Ok(())
        }

        async fn get_issue_comments(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<String>, crate::git::api::GitHubApiError> {
            Ok(self.comment_bodies.clone())
        }
    }

    fn load_issue(file_name: &str) -> Issue {
        let path = format!("src/tests/issue_threads/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    fn load_comments(file_name: &str) -> Vec<serde_json::Value> {
        let path = format!("src/tests/issue_threads/comments/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read comments file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse comments file {}: {}", path, e))
    }

    fn create_test_commits() -> Vec<(ObjectId, String)> {
        vec![
            (
                ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                "Initial commit".to_string(),
            ),
            (
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                "Second commit".to_string(),
            ),
            (
                ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                "Third commit".to_string(),
            ),
            (
                ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap(),
                "Fourth commit".to_string(),
            ),
            (
                ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap(),
                "Fifth commit".to_string(),
            ),
            (
                ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap(),
                "Sixth commit".to_string(),
            ),
        ]
    }

    #[tokio::test]
    async fn test_from_issue_open_with_notifications() {
        let issue = load_issue("open_issue_with_notifications.json");
        let comments = load_comments("open_issue_notifications.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let git_info = MockGitInfo::new()
            .with_file_commits(create_test_commits())
            .with_comment_bodies(comment_bodies);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        // Verify initial commit parsing
        assert_eq!(
            result.initial_commit,
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap()
        );

        // Verify notification commits (both full and short SHAs should be parsed)
        assert_eq!(result.notification_commits.len(), 2);
        assert_eq!(
            result.notification_commits[0],
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap()
        );
        assert_eq!(
            result.notification_commits[1],
            ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap() // 123abcd matches this commit
        );

        // Open issue should have no approved commit
        assert_eq!(result.approved_commit, None);
        assert_eq!(result.file, PathBuf::from("src/main.rs"));
        assert_eq!(result.branch, "feature/new-feature");
    }

    #[tokio::test]
    async fn test_from_issue_closed_with_approval() {
        let issue = load_issue("closed_approved_issue.json");
        let comments = load_comments("closed_approved_comments.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let git_info = MockGitInfo::new()
            .with_file_commits(create_test_commits())
            .with_comment_bodies(comment_bodies);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        // Verify initial commit
        assert_eq!(
            result.initial_commit,
            ObjectId::from_str("def456abc789012345678901234567890123abcd").unwrap()
        );

        // Should have one notification commit and one approved commit
        assert_eq!(result.notification_commits.len(), 1);
        assert_eq!(
            result.notification_commits[0],
            ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap()
        );

        // Closed issue with approval should have approved commit
        assert_eq!(
            result.approved_commit,
            Some(ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap())
        );

        assert_eq!(result.file, PathBuf::from("src/lib.rs"));
        assert_eq!(result.branch, "bugfix/memory-leak");
    }

    #[tokio::test]
    async fn test_from_issue_with_unapproval() {
        let issue = load_issue("unapproved_issue.json");
        let comments = load_comments("unapproved_comments.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let git_info = MockGitInfo::new()
            .with_file_commits(create_test_commits())
            .with_comment_bodies(comment_bodies);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        // Verify initial commit
        assert_eq!(
            result.initial_commit,
            ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap()
        );

        // Should have notification commits (issue is open so approval treated as notification)
        // The same commit appears twice: once from "current commit" and once from "approved qc commit"
        assert_eq!(result.notification_commits.len(), 2);
        assert_eq!(
            result.notification_commits[0],
            ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap()
        );
        assert_eq!(
            result.notification_commits[1],
            ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap()
        );

        // Should have no approved commit due to unapproval
        assert_eq!(result.approved_commit, None);
        assert_eq!(result.file, PathBuf::from("src/utils.rs"));
        assert_eq!(result.branch, "feature/utils-refactor");
    }

    #[test]
    fn test_parse_commit_from_pattern_full_sha() {
        let body = "approved qc commit: abc123def456789012345678901234567890abcd";
        let commit_ids = vec![
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
        ];

        let result = parse_commit_from_pattern(body, "approved qc commit: ", &commit_ids);
        assert_eq!(
            result,
            Some(ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap())
        );
    }

    #[test]
    fn test_parse_commit_from_pattern_short_sha() {
        let body = "current commit: abc123d";
        let commit_ids = vec![
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
        ];

        let result = parse_commit_from_pattern(body, "current commit: ", &commit_ids);
        assert_eq!(
            result,
            Some(ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap())
        );
    }

    #[test]
    fn test_parse_commit_from_pattern_minimum_length() {
        let body = "current commit: abc123";
        let commit_ids =
            vec![ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap()];

        // Should fail because SHA is too short (< 7 characters)
        let result = parse_commit_from_pattern(body, "current commit: ", &commit_ids);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_commit_from_pattern_no_match() {
        let body = "current commit: nonexistent123";
        let commit_ids =
            vec![ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap()];

        let result = parse_commit_from_pattern(body, "current commit: ", &commit_ids);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_commit_from_pattern_not_found() {
        let body = "some other content";
        let commit_ids =
            vec![ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap()];

        let result = parse_commit_from_pattern(body, "current commit: ", &commit_ids);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_commits_from_comments_open_issue() {
        let comment_bodies = vec![
            "current commit: abc123def456789012345678901234567890abcd".to_string(),
            "approved qc commit: def456789abc012345678901234567890123abcd".to_string(),
        ];
        let commit_ids = vec![
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
        ];

        let (notifications, approved) =
            parse_commits_from_comments(&comment_bodies, &commit_ids, true);

        // Open issue: both should be notifications
        assert_eq!(notifications.len(), 2);
        assert_eq!(approved, None);
    }

    #[test]
    fn test_parse_commits_from_comments_closed_issue() {
        let comment_bodies = vec![
            "current commit: abc123def456789012345678901234567890abcd".to_string(),
            "approved qc commit: def456789abc012345678901234567890123abcd".to_string(),
        ];
        let commit_ids = vec![
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
        ];

        let (notifications, approved) =
            parse_commits_from_comments(&comment_bodies, &commit_ids, false);

        // Closed issue: notification + approval
        assert_eq!(notifications.len(), 1);
        assert_eq!(
            approved,
            Some(ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap())
        );
    }

    #[test]
    fn test_parse_commits_from_comments_with_unapproval() {
        let comment_bodies = vec![
            "current commit: abc123def456789012345678901234567890abcd".to_string(),
            "approved qc commit: def456789abc012345678901234567890123abcd".to_string(),
            "# QC Un-Approval\nWithdrawing approval".to_string(),
        ];
        let commit_ids = vec![
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
        ];

        let (notifications, approved) =
            parse_commits_from_comments(&comment_bodies, &commit_ids, false);

        // Unapproval should invalidate approval and move it to notifications
        assert_eq!(notifications.len(), 2);
        assert_eq!(approved, None);
    }

    #[test]
    fn test_parse_branch_from_body_basic() {
        let body = "## Metadata\ninitial qc commit: abc123\ngit branch: feature/new-feature\nauthor: John Doe";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_with_extra_whitespace() {
        let body = "git branch:   main  \nother content";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_complex_branch_name() {
        let body = "git branch: feature/JIRA-123_fix-memory-leak\n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/JIRA-123_fix-memory-leak".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_not_found() {
        let body = "## Metadata\ninitial qc commit: abc123\nauthor: John Doe";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_empty_branch() {
        let body = "git branch: \n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_only_spaces() {
        let body = "git branch:    \n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }
}
