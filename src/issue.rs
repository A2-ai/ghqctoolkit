use std::{path::PathBuf, str::FromStr};

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::{
    cache::{CachedComments, DiskCache},
    git::{
        GitCommitAnalysis, GitFileOps, GitFileOpsError, GitHubApiError, GitHubReader,
        get_file_commits_robust,
    },
};

pub struct IssueThread {
    file: PathBuf,
    branch: String,
    pub(crate) initial_commit: ObjectId,
    pub(crate) notification_commits: Vec<ObjectId>,
    pub(crate) approved_commit: Option<ObjectId>,
    pub(crate) open: bool,
}

impl IssueThread {
    pub async fn from_issue(
        issue: &Issue,
        cache: Option<&DiskCache>,
        git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
    ) -> Result<Self, IssueError> {
        let file = PathBuf::from(&issue.title);
        let issue_is_open = matches!(issue.state, octocrab::models::IssueState::Open);

        // 1. Parse the branch from the issue body first
        let branch = issue
            .body
            .as_ref()
            .and_then(|body| parse_branch_from_body(body))
            .ok_or_else(|| IssueError::BranchNotFound)?;

        // 2. Parse the commit string from the issue body
        let initial_commit_str = issue
            .body
            .as_ref()
            .and_then(|body| parse_commit_from_pattern(body, "initial qc commit: "))
            .ok_or_else(|| IssueError::InitialCommitNotFound)?;

        // 3. Get the comment bodies (cached based on issue update time)
        let comment_bodies = get_cached_issue_comments(issue, cache, git_info).await?;

        // 4. Parse notification and approval commit strings from comments
        let (notification_commit_strs, approved_commit_str) =
            parse_commits_from_comments(&comment_bodies);

        // 5. Try to parse all commit strings to ObjectIds
        let mut all_commit_strs = vec![initial_commit_str];
        all_commit_strs.extend(notification_commit_strs.iter().copied());
        if let Some(approved_str) = approved_commit_str {
            all_commit_strs.push(approved_str);
        }

        let mut parsed_commits = Vec::new();
        let mut unparsable_commits = Vec::new();

        for commit_str in &all_commit_strs {
            match ObjectId::from_str(commit_str) {
                Ok(object_id) => parsed_commits.push(object_id),
                Err(_) => unparsable_commits.push(commit_str),
            }
        }

        // 6. Only get file commits if we have unparsable commits (as fallback)
        let commit_ids = if !unparsable_commits.is_empty() {
            log::debug!(
                "Found {} unparsable commits, fetching file commits as fallback: {:?}",
                unparsable_commits.len(),
                unparsable_commits
            );
            if !parsed_commits.is_empty() {
                // Use any parsed commit as the reference commit for get_file_commits_robust
                let file_commits =
                    get_file_commits_robust(git_info, &file, &branch, &parsed_commits[0])?;
                file_commits.iter().map(|(id, _)| *id).collect()
            } else {
                // No parsed commits available, fall back to basic file_commits call
                let file_commits = git_info.file_commits(&file, &Some(branch.clone()))?;
                file_commits.iter().map(|(id, _)| *id).collect()
            }
        } else {
            // All commits parsed successfully, use them directly
            parsed_commits
        };

        // 7. Parse final ObjectIds using file commits as reference if needed
        let initial_commit = parse_commit_to_object_id(initial_commit_str, &commit_ids)?;

        let notification_commits: Result<Vec<_>, _> = notification_commit_strs
            .iter()
            .map(|s| parse_commit_to_object_id(s, &commit_ids))
            .collect();
        let notification_commits = notification_commits?;

        let approved_commit = if let Some(approved_str) = approved_commit_str {
            Some(parse_commit_to_object_id(approved_str, &commit_ids)?)
        } else {
            None
        };

        Ok(Self {
            file,
            branch,
            initial_commit,
            notification_commits,
            approved_commit,
            open: issue_is_open,
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
        git_info: &(impl GitFileOps + GitCommitAnalysis),
    ) -> Result<Vec<(ObjectId, String)>, IssueError> {
        get_file_commits_robust(git_info, &self.file, &self.branch, &self.initial_commit)
            .map_err(|e| e.into())
    }
}

/// Parse notification and approval commits from comment bodies
/// Returns (notification_commits, approved_commit)
/// Approval is only invalidated if an unapproval occurs after approval
fn parse_commits_from_comments<'a>(
    comment_bodies: &'a [String],
) -> (Vec<&'a str>, Option<&'a str>) {
    let mut notification_commits = Vec::new();
    let mut approved_commit = None;
    let mut approval_comment_index = None;

    // Parse all comments in order
    for (index, body) in comment_bodies.iter().enumerate() {
        // Check for notification commit: "current commit: {hash}"
        if let Some(commit) = parse_commit_from_pattern(body, "current commit: ") {
            notification_commits.push(commit);
        }

        // Check for approval commit: "approved qc commit: {hash}"
        if let Some(commit) = parse_commit_from_pattern(body, "approved qc commit: ") {
            approved_commit = Some(commit);
            approval_comment_index = Some(index);
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
fn parse_commit_from_pattern<'a>(body: &'a str, pattern: &str) -> Option<&'a str> {
    let start = body.find(pattern)?;
    let commit_start = start + pattern.len();

    let remaining = &body[commit_start..];
    remaining.lines().next()?.split_whitespace().next()
}

/// Try to parse a commit string to an ObjectId, with fallback lookup in commit list
fn parse_commit_to_object_id(
    commit_str: &str,
    available_commits: &[ObjectId],
) -> Result<ObjectId, IssueError> {
    // First try to parse as a full ObjectId
    if let Ok(object_id) = ObjectId::from_str(commit_str) {
        return Ok(object_id);
    }

    // If that fails, try to match against available commits (for short SHAs)
    if commit_str.len() >= 7 {
        for commit in available_commits {
            let commit_str_full = commit.to_string();
            if commit_str_full.starts_with(commit_str) {
                return Ok(*commit);
            }
        }
    }

    Err(IssueError::CommitNotParseable(commit_str.to_string()))
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
    git_info: &impl GitHubReader,
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
    GitFileOpsError(#[from] GitFileOpsError),
    #[error("Initial commit not found in issue body")]
    InitialCommitNotFound,
    #[error("Branch not found in issue body")]
    BranchNotFound,
    #[error("Commit string '{0}' could not be parsed to a valid ObjectId")]
    CommitNotParseable(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitAuthor;
    use crate::git::{GitCommitAnalysisError, GitHelpers, GitHubWriter};
    use std::path::PathBuf;
    use std::str::FromStr;

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
            (
                ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap(),
                "Seventh commit".to_string(),
            ),
        ]
    }

    #[tokio::test]
    async fn test_from_issue_open_with_notifications() {
        // Comment sequence:
        // 1. Initial commit: abc123def456789012345678901234567890abcd (from issue body)
        // 2. Notification: current commit: def456789abc012345678901234567890123abcd
        // 3. Notification: current commit: 123abcd (short SHA)
        // No approval commits in this test

        let issue = load_issue("open_issue_with_notifications.json");
        let comments = load_comments("open_issue_notifications.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(
                Some("feature/new-feature".to_string()),
                Ok(create_test_commits()),
            )
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
        // Comment sequence:
        // 1. Initial commit: def456abc789012345678901234567890123abcd (from issue body)
        // 2. Notification: current commit: 456def789abc012345678901234567890123cdef
        // 3. Approval: approved qc commit: 456def789abc012345678901234567890123cdef
        // No unapproval - approval remains valid

        let issue = load_issue("closed_approved_issue.json");
        let comments = load_comments("closed_approved_comments.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(
                Some("bugfix/memory-leak".to_string()),
                Ok(create_test_commits()),
            )
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
        // Comment sequence:
        // 1. Initial commit: 789abc12def345678901234567890123456789ef (from issue body)
        // 2. Notification: current commit: 890cdef123abc456789012345678901234567890
        // 3. Approval: approved qc commit: 890cdef123abc456789012345678901234567890
        // 4. Notification: current commit: abc1234 (short SHA)
        // 5. Unapproval: # QC Un-Approval (invalidates the approval from step 3)

        let issue = load_issue("unapproved_issue.json");
        let comments = load_comments("unapproved_comments.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let branch = "feature/utils-refactor";
        let test_commits = create_test_commits();

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(Some(branch.to_string()), Ok(test_commits.clone()))
            .with_comment_bodies(comment_bodies);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        // Verify initial commit
        assert_eq!(
            result.initial_commit,
            ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap()
        );

        // Should have notification commits: original notification, later notification, and invalidated approval
        // Three commits: "890cdef..." (current), "abc1234" (later current), "890cdef..." (invalidated approval)
        assert_eq!(result.notification_commits.len(), 3);
        assert_eq!(
            result.notification_commits[0],
            ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap()
        );
        assert_eq!(
            result.notification_commits[1],
            ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap()
        );
        assert_eq!(
            result.notification_commits[2],
            ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap()
        );

        // Should have no approved commit due to unapproval
        assert_eq!(result.approved_commit, None);
        assert_eq!(result.file, PathBuf::from("src/utils.rs"));
        assert_eq!(result.branch, "feature/utils-refactor");
    }

    #[tokio::test]
    async fn test_from_issue_open_with_approval_and_notification() {
        // Comment sequence:
        // 1. Initial commit: 111def456789012345678901234567890123abcd (from issue body)
        // 2. Notification: current commit: 222abc123456789012345678901234567890def
        // 3. Approval: approved qc commit: 222abc123456789012345678901234567890def
        // 4. Notification: current commit: 333cdef78 (short SHA)
        // Issue is open but approval remains valid (no unapproval)

        let issue = load_issue("open_issue_with_approval_and_notification.json");
        let comments = load_comments("open_issue_approval_and_notification.json");

        // Extract comment bodies from the test data
        let comment_bodies: Vec<String> = comments
            .into_iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect();

        let branch = "feature/test-branch";
        let test_commits = vec![
            (ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap(), "Initial".to_string()),
            (ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap(), "Second".to_string()),
            (ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap(), "Third".to_string()),
        ];

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(Some(branch.to_string()), Ok(test_commits.clone()))
            .with_comment_bodies(comment_bodies);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        // Verify initial commit
        assert_eq!(
            result.initial_commit,
            ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap()
        );

        // Should have 2 notification commits: one before approval, one after
        assert_eq!(result.notification_commits.len(), 2);
        assert_eq!(
            result.notification_commits[0],
            ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap()
        );
        assert_eq!(
            result.notification_commits[1],
            ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap() // Resolved from short SHA
        );

        // Should have approved commit (remains valid despite issue being open)
        assert_eq!(
            result.approved_commit,
            Some(ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap())
        );

        assert_eq!(result.file, PathBuf::from("src/test.rs"));
        assert_eq!(result.branch, "feature/test-branch");
        assert_eq!(result.open, true);
    }

    #[test]
    fn test_parse_commit_from_pattern_full_sha() {
        let body = "approved qc commit: abc123def456789012345678901234567890abcd";

        let result = parse_commit_from_pattern(body, "approved qc commit: ");
        assert_eq!(result, Some("abc123def456789012345678901234567890abcd"));
    }

    #[test]
    fn test_parse_commit_from_pattern_short_sha() {
        let body = "current commit: abc123d";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("abc123d"));
    }

    #[test]
    fn test_parse_commit_from_pattern_minimum_length() {
        let body = "current commit: abc123";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("abc123"));
    }

    #[test]
    fn test_parse_commit_from_pattern_no_match() {
        let body = "current commit: nonexistent123";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("nonexistent123"));
    }

    #[test]
    fn test_parse_commit_from_pattern_not_found() {
        let body = "some other content";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_commits_from_comments_with_approval() {
        let comment_bodies = vec![
            "current commit: abc123def456789012345678901234567890abcd".to_string(),
            "approved qc commit: def456789abc012345678901234567890123abcd".to_string(),
        ];

        let (notifications, approved) = parse_commits_from_comments(&comment_bodies);

        // Should have notification + approval (regardless of issue open/closed status)
        assert_eq!(notifications.len(), 1);
        assert_eq!(approved, Some("def456789abc012345678901234567890123abcd"));
    }

    #[test]
    fn test_parse_commits_from_comments_notifications_only() {
        let comment_bodies = vec![
            "current commit: abc123def456789012345678901234567890abcd".to_string(),
            "current commit: def456789abc012345678901234567890123abcd".to_string(),
        ];

        let (notifications, approved) = parse_commits_from_comments(&comment_bodies);

        // Only notifications, no approval
        assert_eq!(notifications.len(), 2);
        assert_eq!(approved, None);
    }

    #[test]
    fn test_parse_commits_from_comments_with_unapproval() {
        let comment_bodies = vec![
            "current commit: abc123def456789012345678901234567890abcd".to_string(),
            "approved qc commit: def456789abc012345678901234567890123abcd".to_string(),
            "# QC Un-Approval\nWithdrawing approval".to_string(),
        ];

        let (notifications, approved) = parse_commits_from_comments(&comment_bodies);

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

    // Enhanced MockGitInfo for testing robust branch handling
    struct RobustMockGitInfo {
        file_commits_responses: std::collections::HashMap<
            Option<String>,
            Result<Vec<(ObjectId, String)>, GitFileOpsError>,
        >,
        merge_commits: Vec<ObjectId>,
        commit_parents: std::collections::HashMap<ObjectId, Vec<ObjectId>>,
        ancestor_relationships: std::collections::HashMap<(ObjectId, ObjectId), bool>,
        branches_containing_commits: std::collections::HashMap<ObjectId, Vec<String>>,
        comment_bodies: Vec<String>,
    }

    impl RobustMockGitInfo {
        fn new() -> Self {
            Self {
                file_commits_responses: std::collections::HashMap::new(),
                merge_commits: Vec::new(),
                commit_parents: std::collections::HashMap::new(),
                ancestor_relationships: std::collections::HashMap::new(),
                branches_containing_commits: std::collections::HashMap::new(),
                comment_bodies: Vec::new(),
            }
        }

        fn with_file_commits_result(
            mut self,
            branch: Option<String>,
            result: Result<Vec<(ObjectId, String)>, GitFileOpsError>,
        ) -> Self {
            self.file_commits_responses.insert(branch, result);
            self
        }

        fn with_merge_commits(mut self, commits: Vec<ObjectId>) -> Self {
            self.merge_commits = commits;
            self
        }

        fn with_commit_parents(mut self, commit: ObjectId, parents: Vec<ObjectId>) -> Self {
            self.commit_parents.insert(commit, parents);
            self
        }

        fn with_ancestor_relationship(
            mut self,
            ancestor: ObjectId,
            descendant: ObjectId,
            is_ancestor: bool,
        ) -> Self {
            self.ancestor_relationships
                .insert((ancestor, descendant), is_ancestor);
            self
        }

        fn with_branches_containing_commit(
            mut self,
            commit: ObjectId,
            branches: Vec<String>,
        ) -> Self {
            self.branches_containing_commits.insert(commit, branches);
            self
        }

        fn with_comment_bodies(mut self, bodies: Vec<String>) -> Self {
            self.comment_bodies = bodies;
            self
        }
    }

    impl GitHelpers for RobustMockGitInfo {
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

    impl GitFileOps for RobustMockGitInfo {
        fn file_commits(
            &self,
            _file: &std::path::Path,
            branch: &Option<String>,
        ) -> Result<Vec<(gix::ObjectId, String)>, GitFileOpsError> {
            match self.file_commits_responses.get(branch) {
                Some(Ok(commits)) => Ok(commits.clone()),
                Some(Err(GitFileOpsError::BranchNotFound(branch_name))) => {
                    Err(GitFileOpsError::BranchNotFound(branch_name.clone()))
                }
                Some(Err(_e)) => Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))), // Fallback error for testing
                None => Ok(Vec::new()),
            }
        }

        fn authors(&self, _file: &std::path::Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_content_at_commit(
            &self,
            _file: &std::path::Path,
            _commit: &gix::ObjectId,
        ) -> Result<String, GitFileOpsError> {
            Ok(String::new())
        }
    }

    impl GitCommitAnalysis for RobustMockGitInfo {
        fn get_all_merge_commits(&self) -> Result<Vec<gix::ObjectId>, GitCommitAnalysisError> {
            Ok(self.merge_commits.clone())
        }

        fn get_commit_parents(
            &self,
            commit: &gix::ObjectId,
        ) -> Result<Vec<gix::ObjectId>, GitCommitAnalysisError> {
            Ok(self.commit_parents.get(commit).cloned().unwrap_or_default())
        }

        fn is_ancestor(
            &self,
            ancestor: &gix::ObjectId,
            descendant: &gix::ObjectId,
        ) -> Result<bool, GitCommitAnalysisError> {
            Ok(self
                .ancestor_relationships
                .get(&(*ancestor, *descendant))
                .copied()
                .unwrap_or(false))
        }

        fn get_branches_containing_commit(
            &self,
            commit: &gix::ObjectId,
        ) -> Result<Vec<String>, GitCommitAnalysisError> {
            Ok(self
                .branches_containing_commits
                .get(commit)
                .cloned()
                .unwrap_or_default())
        }
    }

    impl GitHubReader for RobustMockGitInfo {
        async fn get_milestones(
            &self,
        ) -> Result<Vec<octocrab::models::Milestone>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_milestone_issues(
            &self,
            _milestone: &octocrab::models::Milestone,
        ) -> Result<Vec<Issue>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_assignees(&self) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_user_details(
            &self,
            _username: &str,
        ) -> Result<crate::RepoUser, crate::git::GitHubApiError> {
            Ok(crate::RepoUser {
                login: _username.to_string(),
                name: None,
            })
        }

        async fn get_labels(&self) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issue_comments(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(self.comment_bodies.clone())
        }
    }

    impl GitHubWriter for RobustMockGitInfo {
        async fn create_milestone(
            &self,
            _milestone_name: &str,
        ) -> Result<octocrab::models::Milestone, crate::git::GitHubApiError> {
            unimplemented!()
        }

        async fn post_issue(
            &self,
            _issue: &crate::QCIssue,
        ) -> Result<String, crate::git::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1".to_string())
        }

        async fn post_comment(
            &self,
            _comment: &crate::QCComment,
        ) -> Result<String, crate::git::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1#issuecomment-1".to_string())
        }

        async fn post_approval(
            &self,
            _approval: &crate::QCApprove,
        ) -> Result<String, crate::git::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1#issuecomment-1".to_string())
        }

        async fn post_unapproval(
            &self,
            _unapproval: &crate::QCUnapprove,
        ) -> Result<String, crate::git::GitHubApiError> {
            Ok("https://github.com/owner/repo/issues/1#issuecomment-1".to_string())
        }

        async fn create_label(
            &self,
            _name: &str,
            _color: &str,
        ) -> Result<(), crate::git::GitHubApiError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_success_on_first_try() {
        let test_commits = create_test_commits();
        let file = PathBuf::from("src/main.rs");
        let branch = "feature-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(Some(branch.to_string()), Ok(test_commits.clone()));

        let result = get_file_commits_robust(&git_info, &file, branch, &initial_commit).unwrap();

        assert_eq!(result.len(), test_commits.len());
        assert_eq!(result, test_commits);
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_branch_not_found_uses_merge_detection() {
        let test_commits = create_test_commits();
        let file = PathBuf::from("src/main.rs");
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let merge_commit = ObjectId::from_str("1234567890abcdef123456789012345678901234").unwrap();
        let parent1 = ObjectId::from_str("2345678901234567890123456789012345678901").unwrap();
        let parent2 = ObjectId::from_str("3456789012345678901234567890123456789012").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // Merge detection finds the target branch
            .with_merge_commits(vec![merge_commit])
            .with_commit_parents(merge_commit, vec![parent1, parent2])
            .with_ancestor_relationship(initial_commit, parent2, true) // initial_commit is ancestor of parent2 (merged branch)
            .with_branches_containing_commit(merge_commit, vec!["main".to_string()])
            // Target branch has the commits
            .with_file_commits_result(Some("main".to_string()), Ok(test_commits.clone()));

        let result = get_file_commits_robust(&git_info, &file, branch, &initial_commit).unwrap();

        assert_eq!(result.len(), test_commits.len());
        assert_eq!(result, test_commits);
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_fallback_to_branches_containing_commit() {
        let test_commits = create_test_commits();
        let file = PathBuf::from("src/main.rs");
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // No merge commits found
            .with_merge_commits(vec![])
            // But initial commit is found in some branches
            .with_branches_containing_commit(
                initial_commit,
                vec!["main".to_string(), "develop".to_string()],
            )
            // First branch with file commits wins
            .with_file_commits_result(Some("main".to_string()), Ok(test_commits.clone()))
            .with_file_commits_result(Some("develop".to_string()), Ok(Vec::new()));

        let result = get_file_commits_robust(&git_info, &file, branch, &initial_commit).unwrap();

        assert_eq!(result.len(), test_commits.len());
        assert_eq!(result, test_commits);
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_final_fallback_to_all_commits() {
        let test_commits = create_test_commits();
        let file = PathBuf::from("src/main.rs");
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // No merge commits found
            .with_merge_commits(vec![])
            // No branches contain the commit
            .with_branches_containing_commit(initial_commit, vec![])
            // Fall back to all commits (no branch restriction)
            .with_file_commits_result(None, Ok(test_commits.clone()));

        let result = get_file_commits_robust(&git_info, &file, branch, &initial_commit).unwrap();

        assert_eq!(result.len(), test_commits.len());
        assert_eq!(result, test_commits);
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_no_initial_commit_in_issue_body() {
        let file = PathBuf::from("src/main.rs");
        let branch = "deleted-branch";
        let invalid_commit =
            ObjectId::from_str("0000000000000000000000000000000000000000").unwrap();

        let git_info = RobustMockGitInfo::new().with_file_commits_result(
            Some(branch.to_string()),
            Err(GitFileOpsError::BranchNotFound(branch.to_string())),
        );

        let result = get_file_commits_robust(&git_info, &file, branch, &invalid_commit);

        // Should succeed but return empty commits since all fallbacks return empty
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_git_error_propagated() {
        let file = PathBuf::from("src/main.rs");
        let branch = "test-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new().with_file_commits_result(
            Some(branch.to_string()),
            Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))),
        );

        let result = get_file_commits_robust(&git_info, &file, branch, &initial_commit);

        assert!(matches!(result, Err(GitFileOpsError::AuthorNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_file_commits_robust_multiple_merge_commits() {
        let test_commits = create_test_commits();
        let file = PathBuf::from("src/main.rs");
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let merge_commit1 = ObjectId::from_str("1111111111111111111111111111111111111111").unwrap();
        let merge_commit2 = ObjectId::from_str("2222222222222222222222222222222222222222").unwrap();
        let parent1_1 = ObjectId::from_str("3333333333333333333333333333333333333333").unwrap();
        let parent2_1 = ObjectId::from_str("4444444444444444444444444444444444444444").unwrap();
        let parent1_2 = ObjectId::from_str("5555555555555555555555555555555555555555").unwrap();
        let parent2_2 = ObjectId::from_str("6666666666666666666666666666666666666666").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // Multiple merge commits
            .with_merge_commits(vec![merge_commit1, merge_commit2])
            .with_commit_parents(merge_commit1, vec![parent1_1, parent2_1])
            .with_commit_parents(merge_commit2, vec![parent1_2, parent2_2])
            // First merge commit doesn't match
            .with_ancestor_relationship(initial_commit, parent2_1, false)
            // Second merge commit matches
            .with_ancestor_relationship(initial_commit, parent2_2, true)
            .with_branches_containing_commit(merge_commit2, vec!["develop".to_string()])
            // Target branch has the commits
            .with_file_commits_result(Some("develop".to_string()), Ok(test_commits.clone()));

        let result = get_file_commits_robust(&git_info, &file, branch, &initial_commit).unwrap();

        assert_eq!(result.len(), test_commits.len());
        assert_eq!(result, test_commits);
    }
}
