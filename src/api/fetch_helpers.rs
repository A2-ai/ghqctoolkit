use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use octocrab::models::issues::Issue;
use tokio::sync::RwLockReadGuard;

use crate::{
    GitHubReader, GitProvider, GitRepository, IssueError, IssueThread,
    api::{
        AppState,
        cache::{CacheEntry, CacheKey, StatusCache},
    },
    parse_blocking_qcs,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct FetchedIssues {
    pub(crate) issues: Vec<Issue>,
    pub(crate) cached_entries: HashMap<u64, CacheEntry>,
    pub(crate) errors: HashMap<u64, String>,
}

impl FetchedIssues {
    pub(crate) async fn fetch_issues(
        issue_numbers: &[u64],
        git_info: &(impl GitHubReader + GitRepository),
        cache: &RwLockReadGuard<'_, StatusCache>,
    ) -> Self {
        let issue_futures = issue_numbers
            .iter()
            .map(|number| async move { git_info.get_issue(*number).await })
            .collect::<Vec<_>>();
        let issue_results = futures::future::join_all(issue_futures).await;
        let mut fetched = Self::default();

        for (result, issue_number) in issue_results.into_iter().zip(issue_numbers) {
            match result {
                Ok(issue) => {
                    let key = cache_key_or_default(git_info, issue.updated_at.clone());
                    if let Some(entry) = cache.get(*issue_number, &key) {
                        fetched.cached_entries.insert(*issue_number, entry.clone());
                    } else {
                        fetched.issues.push(issue);
                    }
                }
                Err(e) => {
                    fetched.errors.insert(*issue_number, e.to_string());
                }
            }
        }

        fetched
    }

    pub(crate) async fn fetch_blocking_qcs(
        &mut self,
        git_info: &(impl GitHubReader + GitRepository),
        cache: &RwLockReadGuard<'_, StatusCache>,
    ) {
        let mut issue_numbers = self
            .issues
            .iter()
            .filter_map(|issue| issue.body.as_deref())
            .flat_map(parse_blocking_qcs)
            .map(|b| b.issue_number)
            .collect::<HashSet<_>>();
        issue_numbers.extend(
            self.cached_entries
                .values()
                .flat_map(|entry| entry.blocking_qc_numbers.clone())
                .collect::<HashSet<u64>>(),
        );
        let blocking_qcs = issue_numbers
            .into_iter()
            .filter(|num| {
                !self.cached_entries.contains_key(num)
                    && !self.issues.iter().any(|i| i.number == *num)
            })
            .collect::<Vec<_>>();

        let fetched_issues = FetchedIssues::fetch_issues(&blocking_qcs, git_info, cache).await;
        self.cached_entries.extend(fetched_issues.cached_entries);
        self.issues.extend(fetched_issues.issues);
        self.errors.extend(fetched_issues.errors);
    }
}

#[derive(Debug, Default)]
pub(crate) struct CreatedThreads {
    pub entries: HashMap<u64, CacheEntry>,
    pub thread_errors: HashMap<u64, IssueError>,
}

impl CreatedThreads {
    pub(crate) async fn create_threads<G: GitProvider>(issues: &[Issue], app_state: &AppState<G>) -> Self {
        let git_info = app_state.git_info();
        let cache = app_state.disk_cache();
        let thread_futures = issues
            .iter()
            .map(|issue| async move { IssueThread::from_issue(issue, cache, git_info).await })
            .collect::<Vec<_>>();
        let thread_results = futures::future::join_all(thread_futures).await;
        let mut created = CreatedThreads::default();
        let mut cache_write = app_state.status_cache.write().await;

        for (result, issue) in thread_results.into_iter().zip(issues) {
            match result {
                Ok(issue_thread) => {
                    let entry = CacheEntry::new(issue, &issue_thread);
                    let key = cache_key_or_default(git_info, issue.updated_at.clone());

                    created.entries.insert(issue.number, entry.clone());
                    cache_write.insert(issue.number, key, entry);
                }
                Err(e) => {
                    created.thread_errors.insert(issue.number, e);
                }
            }
        }

        created
    }
}

pub(crate) fn format_error_list(errors: &HashMap<u64, impl std::fmt::Display>) -> String {
    errors
        .iter()
        .map(|(num, err)| format!("#{}: {}", num, err))
        .collect::<Vec<_>>()
        .join("\n  -")
}

pub(crate) fn cache_key_or_default(
    git_info: &impl GitRepository,
    updated_at: DateTime<Utc>,
) -> CacheKey {
    CacheKey::build(git_info, updated_at.clone()).unwrap_or(CacheKey {
        issue_updated_at: updated_at,
        branch: "unknown".to_string(),
        head_commit: "unknown".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helpers::MockGitInfo;
    use crate::git::GitRepositoryError;

    #[test]
    fn test_format_error_list_empty() {
        let errors: HashMap<u64, String> = HashMap::new();
        let result = format_error_list(&errors);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_error_list_single() {
        let mut errors = HashMap::new();
        errors.insert(1, "Not found".to_string());
        let result = format_error_list(&errors);
        assert_eq!(result, "#1: Not found");
    }

    #[test]
    fn test_format_error_list_multiple() {
        let mut errors = HashMap::new();
        errors.insert(1, "Not found".to_string());
        errors.insert(2, "Access denied".to_string());
        let result = format_error_list(&errors);
        // Order is not guaranteed, but both should be present
        assert!(result.contains("#1: Not found"));
        assert!(result.contains("#2: Access denied"));
    }

    #[test]
    fn test_cache_key_or_default_success() {
        let mock = MockGitInfo::builder()
            .with_branch("main")
            .with_commit("abc123")
            .build();
        let updated_at = Utc::now();

        let key = cache_key_or_default(&mock, updated_at);

        assert_eq!(key.branch, "main");
        assert_eq!(key.head_commit, "abc123");
        assert_eq!(key.issue_updated_at, updated_at);
    }

    struct MockGitRepoError;

    impl GitRepository for MockGitRepoError {
        fn branch(&self) -> Result<String, GitRepositoryError> {
            Err(GitRepositoryError::DetachedHead)
        }

        fn commit(&self) -> Result<String, GitRepositoryError> {
            Err(GitRepositoryError::DetachedHead)
        }

        fn owner(&self) -> &str {
            "test"
        }

        fn repo(&self) -> &str {
            "test"
        }

        fn path(&self) -> &std::path::Path {
            std::path::Path::new("/test")
        }
    }

    #[test]
    fn test_cache_key_or_default_fallback() {
        let mock = MockGitRepoError;
        let updated_at = Utc::now();

        let key = cache_key_or_default(&mock, updated_at);

        assert_eq!(key.branch, "unknown");
        assert_eq!(key.head_commit, "unknown");
        assert_eq!(key.issue_updated_at, updated_at);
    }

    #[tokio::test]
    async fn test_fetch_issues_all_cached() {
        use crate::api::cache::StatusCache;

        let mock = MockGitInfo::builder().build();
        let cache = StatusCache::new();
        let cache_read = tokio::sync::RwLock::new(cache);
        let cache_guard = cache_read.read().await;

        let fetched = FetchedIssues::fetch_issues(&[1, 2], &mock, &cache_guard).await;

        // MockGitInfo returns NotFound by default, so all should be errors
        assert_eq!(fetched.issues.len(), 0);
        assert_eq!(fetched.cached_entries.len(), 0);
        assert_eq!(fetched.errors.len(), 2);
    }

    #[tokio::test]
    async fn test_fetch_issues_with_issue_data() {
        use crate::api::cache::StatusCache;
        use crate::api::tests::helpers::load_test_issue;

        let test_issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder().with_issue(1, test_issue).build();

        let cache = StatusCache::new();
        let cache_read = tokio::sync::RwLock::new(cache);
        let cache_guard = cache_read.read().await;

        let fetched = FetchedIssues::fetch_issues(&[1], &mock, &cache_guard).await;

        // Issue 1 should be fetched since not in cache
        assert_eq!(fetched.issues.len(), 1);
        assert_eq!(fetched.issues[0].number, 1);
        assert_eq!(fetched.cached_entries.len(), 0);
        assert_eq!(fetched.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_fetch_issues_with_cache_hit() {
        use crate::api::cache::{CacheEntry, StatusCache};
        use crate::api::tests::helpers::load_test_issue;
        use crate::api::types::{ChecklistSummary, Issue, QCStatus, QCStatusEnum};

        let test_issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder().with_issue(1, test_issue.clone()).build();

        let mut cache = StatusCache::new();
        let key = CacheKey {
            issue_updated_at: test_issue.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: test_issue.title.clone(),
                state: "open".to_string(),
                html_url: test_issue.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue.created_at,
                updated_at: test_issue.updated_at,
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
        cache.insert(1, key, entry);

        let cache_read = tokio::sync::RwLock::new(cache);
        let cache_guard = cache_read.read().await;

        let fetched = FetchedIssues::fetch_issues(&[1], &mock, &cache_guard).await;

        // Issue 1 should be in cache, not fetched
        assert_eq!(fetched.issues.len(), 0);
        assert_eq!(fetched.cached_entries.len(), 1);
        assert_eq!(fetched.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_fetch_issues_mixed() {
        use crate::api::cache::{CacheEntry, StatusCache};
        use crate::api::tests::helpers::load_test_issue;
        use crate::api::types::{ChecklistSummary, Issue, QCStatus, QCStatusEnum};

        let test_issue1 = load_test_issue("test_file_issue");
        let test_issue2 = load_test_issue("config_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue1.clone())
            .with_issue(2, test_issue2.clone())
            .build();

        let mut cache = StatusCache::new();
        let key = CacheKey {
            issue_updated_at: test_issue1.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: test_issue1.title.clone(),
                state: "open".to_string(),
                html_url: test_issue1.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue1.created_at,
                updated_at: test_issue1.updated_at,
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
        cache.insert(1, key, entry);

        let cache_read = tokio::sync::RwLock::new(cache);
        let cache_guard = cache_read.read().await;

        let fetched = FetchedIssues::fetch_issues(&[1, 2], &mock, &cache_guard).await;

        // Issue 1 in cache, issue 2 fetched
        assert_eq!(fetched.issues.len(), 1);
        assert_eq!(fetched.issues[0].number, 2);
        assert_eq!(fetched.cached_entries.len(), 1);
        assert_eq!(fetched.errors.len(), 0);
    }
}
