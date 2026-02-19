//! In-memory cache for issue status responses.

use crate::{
    GitRepository, IssueThread, analyze_issue_checklists,
    api::{
        ApiError, AppState,
        types::{ChecklistSummary, CommitStatusEnum, Issue, IssueCommit, QCStatus, QCStatusEnum},
    },
    parse_blocking_qcs,
};
use chrono::{DateTime, Utc};
use octocrab::models::issues::Issue as octoIssue;
use std::collections::HashMap;

/// Cache validation key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub issue_updated_at: DateTime<Utc>,
    pub branch: String,
    pub head_commit: String,
}

impl CacheKey {
    pub fn build(
        git_info: &impl GitRepository,
        issue_updated_at: DateTime<Utc>,
    ) -> Result<Self, ApiError> {
        match (git_info.branch(), git_info.commit()) {
            (Ok(branch), Ok(head_commit)) => Ok(Self {
                issue_updated_at,
                branch,
                head_commit,
            }),
            (Err(e), Ok(_)) => Err(ApiError::Internal(format!(
                "Failed to determine branch: {e}"
            ))),
            (Ok(_), Err(e)) => Err(ApiError::Internal(format!(
                "Failed to determine HEAD commit: {e}"
            ))),
            (Err(b), Err(c)) => Err(ApiError::Internal(format!(
                "Failed to determine HEAD commit: {c} and branch: {b}"
            ))),
        }
    }
}

/// Status cache entries
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub issue: Issue,
    pub qc_status: QCStatus,
    pub branch: String,
    pub commits: Vec<IssueCommit>,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_numbers: Vec<u64>,
}

impl CacheEntry {
    pub fn new(issue: &octoIssue, issue_thread: &IssueThread) -> Self {
        Self {
            issue: issue.clone().into(),
            qc_status: issue_thread.into(),
            branch: issue
                .body
                .as_deref()
                .and_then(crate::parse_branch_from_body)
                .unwrap_or("unknown".to_string()),
            commits: issue_thread.commits.iter().map(IssueCommit::from).collect(),
            checklist_summary: analyze_issue_checklists(issue.body.as_deref()).into(),
            blocking_qc_numbers: issue
                .body
                .as_deref()
                .map(|body| {
                    parse_blocking_qcs(body)
                        .iter()
                        .map(|b| b.issue_number)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// In-memory cache for issue status responses.
#[derive(Debug)]
pub struct StatusCache {
    entries: HashMap<u64, (CacheKey, CacheEntry)>,
}

impl StatusCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get a cache entry if it exists and the key matches.
    pub fn get(&self, issue_number: u64, key: &CacheKey) -> Option<&CacheEntry> {
        self.entries.get(&issue_number).and_then(
            |(cached_key, entry)| {
                if cached_key == key { Some(entry) } else { None }
            },
        )
    }

    /// Insert or update a cache entry.
    pub fn insert(&mut self, issue_number: u64, key: CacheKey, entry: CacheEntry) {
        self.entries.insert(issue_number, (key, entry));
    }

    pub fn remove(&mut self, issue_number: u64) {
        self.entries.remove(&issue_number);
    }

    pub fn update(
        &mut self,
        key: CacheKey,
        update_issue: &octoIssue,
        current_commit: &str,
        action: UpdateAction,
    ) {
        let mut update_issue = update_issue.clone();
        update_issue.updated_at = key.issue_updated_at;
        if let Some((cache_key, cache_entry)) = self.entries.get_mut(&update_issue.number) {
            *cache_key = key;
            cache_entry.issue = update_issue.clone().into();
            let is_latest_commit = action.update_commits(&mut cache_entry.commits, current_commit);
            action.update_status(&mut cache_entry.qc_status, is_latest_commit, current_commit);
            let checklist_summaries = analyze_issue_checklists(update_issue.body.as_deref());
            cache_entry.checklist_summary = checklist_summaries.into();
        }
    }

    pub fn unapproval(&mut self, key: CacheKey, update_issue: &octoIssue) {
        let mut update_issue = update_issue.clone();
        update_issue.updated_at = key.issue_updated_at;
        if let Some((cache_key, cache_entry)) = self.entries.get_mut(&update_issue.number) {
            *cache_key = key;

            cache_entry.issue = update_issue.clone().into();
            match cache_entry.qc_status.status {
                QCStatusEnum::Approved => {
                    cache_entry.qc_status.status = QCStatusEnum::ChangeRequested;
                    // Clear approved commit since approval is revoked
                    cache_entry.qc_status.approved_commit = None;
                }
                QCStatusEnum::ChangesAfterApproval => {
                    cache_entry.qc_status.status = QCStatusEnum::ChangesToComment;
                    // Clear approved commit since approval is revoked
                    cache_entry.qc_status.approved_commit = None;
                }
                _ => (),
            };
            // Convert Approved commit statuses to Notification
            for commit in cache_entry.commits.iter_mut() {
                if let Some(pos) = commit
                    .statuses
                    .iter()
                    .position(|s| *s == CommitStatusEnum::Approved)
                {
                    if !commit.statuses.contains(&CommitStatusEnum::Notification) {
                        commit.statuses[pos] = CommitStatusEnum::Notification;
                    } else {
                        commit.statuses.remove(pos);
                    }
                }
            }

            cache_entry.checklist_summary =
                analyze_issue_checklists(update_issue.body.as_deref()).into();
        }
    }
}

impl Default for StatusCache {
    fn default() -> Self {
        Self::new()
    }
}

pub enum UpdateAction {
    Notification,
    Review,
    Approve,
}

impl UpdateAction {
    /// updates commits and returns if it updated the latest commit
    fn update_commits(&self, commits: &mut Vec<IssueCommit>, current_commit: &str) -> bool {
        if let Some(commit) = commits.iter_mut().find(|c| c.hash == current_commit) {
            if !commit.statuses.contains(&self.commit_status()) {
                commit.statuses.push(self.commit_status());
            }
            commits
                .first()
                .map(|c| c.hash == current_commit)
                .unwrap_or(false)
        } else {
            commits.insert(
                0,
                IssueCommit {
                    hash: current_commit.to_string(),
                    message: "New commit".to_string(),
                    statuses: vec![self.commit_status()],
                    file_changed: true,
                },
            );
            true
        }
    }

    fn commit_status(&self) -> CommitStatusEnum {
        match self {
            Self::Notification => CommitStatusEnum::Notification,
            Self::Review => CommitStatusEnum::Reviewed,
            Self::Approve => CommitStatusEnum::Approved,
        }
    }

    fn update_status(
        &self,
        qc_status: &mut QCStatus,
        is_latest_commit: bool,
        current_commit: &str,
    ) {
        match self {
            UpdateAction::Notification => {
                if is_latest_commit {
                    if qc_status.status == QCStatusEnum::Approved {
                        qc_status.status = QCStatusEnum::ChangesAfterApproval;
                        qc_status.status_detail = "Approved; subsequent file changes".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    } else {
                        qc_status.status = QCStatusEnum::AwaitingReview;
                        qc_status.status_detail = "Awaiting review".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    }
                }
            }
            UpdateAction::Review => {
                if is_latest_commit {
                    if qc_status.status == QCStatusEnum::Approved {
                        qc_status.status = QCStatusEnum::ChangesAfterApproval;
                        qc_status.status_detail = "Approved; subsequent file changes".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    } else {
                        qc_status.status = QCStatusEnum::ChangeRequested;
                        qc_status.status_detail = "Change Requested".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    }
                }
            }
            UpdateAction::Approve => {
                // Always record the approved commit
                qc_status.approved_commit = Some(current_commit.to_string());

                if is_latest_commit {
                    qc_status.status = QCStatusEnum::Approved;
                    qc_status.status_detail = "Approved".to_string();
                    qc_status.latest_commit = current_commit.to_string();
                } else {
                    qc_status.status = QCStatusEnum::ChangesAfterApproval;
                    qc_status.status_detail = "Approved; subsequent file changes".to_string();
                }
            }
        }
    }
}

pub async fn update_cache_after_comment<G: crate::GitProvider>(
    state: &AppState<G>,
    issue: &octoIssue,
    commit: &str,
    action: UpdateAction,
) {
    match CacheKey::build(state.git_info(), issue.updated_at.clone()) {
        Ok(key) => {
            state
                .status_cache
                .write()
                .await
                .update(key, issue, commit, action);
        }
        Err(e) => {
            log::error!("{e}. Removing cache entry");
            state.status_cache.write().await.remove(issue.number);
        }
    }
}

pub async fn update_cache_after_unapproval<G: crate::GitProvider>(
    state: &AppState<G>,
    issue: &octoIssue,
) {
    match CacheKey::build(state.git_info(), issue.updated_at.clone()) {
        Ok(key) => {
            state.status_cache.write().await.unapproval(key, issue);
        }
        Err(e) => {
            log::error!("{e}. Removing cache entry");
            state.status_cache.write().await.remove(issue.number);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitRepositoryError;

    struct MockGitRepository {
        branch: Option<String>,
        commit: Option<String>,
        branch_error: bool,
        commit_error: bool,
    }

    impl GitRepository for MockGitRepository {
        fn branch(&self) -> Result<String, GitRepositoryError> {
            if self.branch_error {
                Err(GitRepositoryError::DetachedHead)
            } else {
                Ok(self.branch.clone().unwrap_or_else(|| "main".to_string()))
            }
        }

        fn commit(&self) -> Result<String, GitRepositoryError> {
            if self.commit_error {
                Err(GitRepositoryError::DetachedHead)
            } else {
                Ok(self.commit.clone().unwrap_or_else(|| "abc123".to_string()))
            }
        }

        fn owner(&self) -> &str {
            "test-owner"
        }

        fn repo(&self) -> &str {
            "test-repo"
        }

        fn path(&self) -> &std::path::Path {
            std::path::Path::new("/test")
        }

        fn fetch(&self) -> Result<bool, GitRepositoryError> {
            Ok(false) // Mock: no changes fetched
        }
    }

    #[test]
    fn test_cache_key_build_success() {
        let mock = MockGitRepository {
            branch: Some("main".to_string()),
            commit: Some("abc123".to_string()),
            branch_error: false,
            commit_error: false,
        };
        let updated_at = Utc::now();

        let key = CacheKey::build(&mock, updated_at).unwrap();

        assert_eq!(key.branch, "main");
        assert_eq!(key.head_commit, "abc123");
        assert_eq!(key.issue_updated_at, updated_at);
    }

    #[test]
    fn test_cache_key_build_branch_error() {
        let mock = MockGitRepository {
            branch: None,
            commit: Some("abc123".to_string()),
            branch_error: true,
            commit_error: false,
        };
        let updated_at = Utc::now();

        let result = CacheKey::build(&mock, updated_at);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("branch"));
    }

    #[test]
    fn test_cache_key_build_commit_error() {
        let mock = MockGitRepository {
            branch: Some("main".to_string()),
            commit: None,
            branch_error: false,
            commit_error: true,
        };
        let updated_at = Utc::now();

        let result = CacheKey::build(&mock, updated_at);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("HEAD commit"));
    }

    #[test]
    fn test_cache_key_build_both_errors() {
        let mock = MockGitRepository {
            branch: None,
            commit: None,
            branch_error: true,
            commit_error: true,
        };
        let updated_at = Utc::now();

        let result = CacheKey::build(&mock, updated_at);

        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("HEAD commit"));
        assert!(error.contains("branch"));
    }

    #[test]
    fn test_status_cache_get_hit() {
        let mut cache = StatusCache::new();
        let key = CacheKey {
            issue_updated_at: Utc::now(),
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: "test".to_string(),
                state: "open".to_string(),
                html_url: "https://github.com/test/test/issues/1".to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                closed_at: None,
            },
            qc_status: QCStatus {
                status: QCStatusEnum::InProgress,
                status_detail: "In Progress".to_string(),
                approved_commit: None,
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 0,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };

        cache.insert(1, key.clone(), entry.clone());
        let result = cache.get(1, &key);

        assert!(result.is_some());
        assert_eq!(result.unwrap().issue.number, 1);
    }

    #[test]
    fn test_status_cache_get_miss_wrong_key() {
        let mut cache = StatusCache::new();
        let key1 = CacheKey {
            issue_updated_at: Utc::now(),
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let key2 = CacheKey {
            issue_updated_at: Utc::now(),
            branch: "main".to_string(),
            head_commit: "def456".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: "test".to_string(),
                state: "open".to_string(),
                html_url: "https://github.com/test/test/issues/1".to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                closed_at: None,
            },
            qc_status: QCStatus {
                status: QCStatusEnum::InProgress,
                status_detail: "In Progress".to_string(),
                approved_commit: None,
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 0,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };

        cache.insert(1, key1, entry);
        let result = cache.get(1, &key2);

        assert!(result.is_none());
    }

    #[test]
    fn test_status_cache_get_miss_no_entry() {
        let cache = StatusCache::new();
        let key = CacheKey {
            issue_updated_at: Utc::now(),
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };

        let result = cache.get(1, &key);

        assert!(result.is_none());
    }

    #[test]
    fn test_status_cache_insert_and_remove() {
        let mut cache = StatusCache::new();
        let key = CacheKey {
            issue_updated_at: Utc::now(),
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: "test".to_string(),
                state: "open".to_string(),
                html_url: "https://github.com/test/test/issues/1".to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                closed_at: None,
            },
            qc_status: QCStatus {
                status: QCStatusEnum::InProgress,
                status_detail: "In Progress".to_string(),
                approved_commit: None,
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 0,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };

        cache.insert(1, key.clone(), entry);
        assert!(cache.get(1, &key).is_some());

        cache.remove(1);
        assert!(cache.get(1, &key).is_none());
    }

    #[test]
    fn test_update_action_update_commits_new_commit() {
        let mut commits = vec![IssueCommit {
            hash: "abc123".to_string(),
            message: "Old commit".to_string(),
            statuses: vec![CommitStatusEnum::Initial],
            file_changed: true,
        }];

        let is_latest = UpdateAction::Notification.update_commits(&mut commits, "def456");

        assert!(is_latest);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "def456");
        assert_eq!(commits[0].message, "New commit");
        assert!(
            commits[0]
                .statuses
                .contains(&CommitStatusEnum::Notification)
        );
    }

    #[test]
    fn test_update_action_update_commits_existing_commit() {
        let mut commits = vec![
            IssueCommit {
                hash: "def456".to_string(),
                message: "Latest commit".to_string(),
                statuses: vec![CommitStatusEnum::Initial],
                file_changed: true,
            },
            IssueCommit {
                hash: "abc123".to_string(),
                message: "Old commit".to_string(),
                statuses: vec![CommitStatusEnum::Initial],
                file_changed: true,
            },
        ];

        let is_latest = UpdateAction::Review.update_commits(&mut commits, "def456");

        assert!(is_latest);
        assert_eq!(commits.len(), 2);
        assert!(commits[0].statuses.contains(&CommitStatusEnum::Reviewed));
    }

    #[test]
    fn test_update_action_update_commits_not_latest() {
        let mut commits = vec![
            IssueCommit {
                hash: "def456".to_string(),
                message: "Latest commit".to_string(),
                statuses: vec![CommitStatusEnum::Initial],
                file_changed: true,
            },
            IssueCommit {
                hash: "abc123".to_string(),
                message: "Old commit".to_string(),
                statuses: vec![CommitStatusEnum::Initial],
                file_changed: true,
            },
        ];

        let is_latest = UpdateAction::Review.update_commits(&mut commits, "abc123");

        assert!(!is_latest);
        assert_eq!(commits.len(), 2);
        assert!(commits[1].statuses.contains(&CommitStatusEnum::Reviewed));
    }

    #[test]
    fn test_update_action_notification_on_approved() {
        let mut qc_status = QCStatus {
            status: QCStatusEnum::Approved,
            status_detail: "Approved".to_string(),
            approved_commit: Some("abc123".to_string()),
            initial_commit: "abc123".to_string(),
            latest_commit: "abc123".to_string(),
        };

        UpdateAction::Notification.update_status(&mut qc_status, true, "def456");

        assert_eq!(qc_status.status, QCStatusEnum::ChangesAfterApproval);
        assert_eq!(qc_status.latest_commit, "def456");
    }

    #[test]
    fn test_update_action_notification_on_in_progress() {
        let mut qc_status = QCStatus {
            status: QCStatusEnum::InProgress,
            status_detail: "In Progress".to_string(),
            approved_commit: None,
            initial_commit: "abc123".to_string(),
            latest_commit: "abc123".to_string(),
        };

        UpdateAction::Notification.update_status(&mut qc_status, true, "def456");

        assert_eq!(qc_status.status, QCStatusEnum::AwaitingReview);
        assert_eq!(qc_status.latest_commit, "def456");
    }

    #[test]
    fn test_update_action_approve_latest() {
        let mut qc_status = QCStatus {
            status: QCStatusEnum::AwaitingReview,
            status_detail: "Awaiting review".to_string(),
            approved_commit: None,
            initial_commit: "abc123".to_string(),
            latest_commit: "abc123".to_string(),
        };

        UpdateAction::Approve.update_status(&mut qc_status, true, "abc123");

        assert_eq!(qc_status.status, QCStatusEnum::Approved);
        assert_eq!(qc_status.latest_commit, "abc123");
    }

    #[test]
    fn test_update_action_approve_not_latest() {
        let mut qc_status = QCStatus {
            status: QCStatusEnum::AwaitingReview,
            status_detail: "Awaiting review".to_string(),
            approved_commit: None,
            initial_commit: "abc123".to_string(),
            latest_commit: "def456".to_string(),
        };

        UpdateAction::Approve.update_status(&mut qc_status, false, "abc123");

        assert_eq!(qc_status.status, QCStatusEnum::ChangesAfterApproval);
    }

    // Helper function to create a minimal octocrab Issue for testing
    fn create_test_octocrab_issue(
        number: u64,
        updated_at: DateTime<Utc>,
    ) -> octocrab::models::issues::Issue {
        // Use serde to deserialize from minimal JSON
        let json = serde_json::json!({
            "id": number,
            "node_id": format!("node{}", number),
            "number": number,
            "title": "test issue",
            "user": {
                "login": "test-user",
                "id": 1,
                "node_id": "user1",
                "avatar_url": "https://github.com/avatar.png",
                "gravatar_id": "gravatar123",
                "url": "https://api.github.com/users/test-user",
                "html_url": "https://github.com/test-user",
                "followers_url": "https://api.github.com/users/test-user/followers",
                "following_url": "https://api.github.com/users/test-user/following{/other_user}",
                "gists_url": "https://api.github.com/users/test-user/gists{/gist_id}",
                "starred_url": "https://api.github.com/users/test-user/starred{/owner}{/repo}",
                "subscriptions_url": "https://api.github.com/users/test-user/subscriptions",
                "organizations_url": "https://api.github.com/users/test-user/orgs",
                "repos_url": "https://api.github.com/users/test-user/repos",
                "events_url": "https://api.github.com/users/test-user/events{/privacy}",
                "received_events_url": "https://api.github.com/users/test-user/received_events",
                "type": "User",
                "site_admin": false
            },
            "labels": [],
            "state": "open",
            "locked": false,
            "assignees": [],
            "comments": 0,
            "created_at": updated_at,
            "updated_at": updated_at,
            "html_url": format!("https://github.com/test/test/issues/{}", number),
            "labels_url": format!("https://api.github.com/repos/test/test/issues/{}/labels{{/name}}", number),
            "comments_url": format!("https://api.github.com/repos/test/test/issues/{}/comments", number),
            "events_url": format!("https://api.github.com/repos/test/test/issues/{}/events", number),
            "repository_url": "https://api.github.com/repos/test/test",
            "url": format!("https://api.github.com/repos/test/test/issues/{}", number),
            "author_association": "OWNER"
        });
        serde_json::from_value(json).expect("Failed to create test issue")
    }

    #[test]
    fn test_status_cache_unapproval_from_approved() {
        let mut cache = StatusCache::new();
        let updated_at = Utc::now();
        let key = CacheKey {
            issue_updated_at: updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: "test".to_string(),
                state: "closed".to_string(),
                html_url: "https://github.com/test/test/issues/1".to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: updated_at,
                updated_at,
                closed_at: Some(updated_at),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::Approved,
                status_detail: "Approved".to_string(),
                approved_commit: Some("abc123".to_string()),
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![IssueCommit {
                hash: "abc123".to_string(),
                message: "Test commit".to_string(),
                statuses: vec![CommitStatusEnum::Approved],
                file_changed: true,
            }],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 0,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };

        cache.insert(1, key.clone(), entry);

        let issue = create_test_octocrab_issue(1, updated_at);
        cache.unapproval(key.clone(), &issue);

        let cached = cache.get(1, &key).unwrap();
        assert_eq!(cached.qc_status.status, QCStatusEnum::ChangeRequested);
        assert_eq!(
            cached.commits[0].statuses,
            vec![CommitStatusEnum::Notification]
        );
    }

    #[test]
    fn test_status_cache_unapproval_from_changes_after_approval() {
        let mut cache = StatusCache::new();
        let updated_at = Utc::now();
        let key = CacheKey {
            issue_updated_at: updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: "test".to_string(),
                state: "closed".to_string(),
                html_url: "https://github.com/test/test/issues/1".to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: updated_at,
                updated_at,
                closed_at: Some(updated_at),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::ChangesAfterApproval,
                status_detail: "Changes after approval".to_string(),
                approved_commit: Some("abc123".to_string()),
                initial_commit: "abc123".to_string(),
                latest_commit: "def456".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![
                IssueCommit {
                    hash: "def456".to_string(),
                    message: "New commit".to_string(),
                    statuses: vec![CommitStatusEnum::Notification],
                    file_changed: true,
                },
                IssueCommit {
                    hash: "abc123".to_string(),
                    message: "Test commit".to_string(),
                    statuses: vec![CommitStatusEnum::Approved],
                    file_changed: true,
                },
            ],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 0,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };

        cache.insert(1, key.clone(), entry);

        let issue = create_test_octocrab_issue(1, updated_at);
        cache.unapproval(key.clone(), &issue);

        let cached = cache.get(1, &key).unwrap();
        assert_eq!(cached.qc_status.status, QCStatusEnum::ChangesToComment);
        // Approved status should be converted to Notification
        assert!(
            cached.commits[1]
                .statuses
                .contains(&CommitStatusEnum::Notification)
        );
        assert!(
            !cached.commits[1]
                .statuses
                .contains(&CommitStatusEnum::Approved)
        );
    }
}
