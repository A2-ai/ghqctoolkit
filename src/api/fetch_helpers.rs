use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use octocrab::models::issues::Issue;
use tokio::sync::RwLockReadGuard;

use crate::{
    GitHubReader, GitRepository, IssueError, IssueThread,
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
                    && self.issues.iter().find(|i| i.number != *num).is_some()
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
    pub(crate) async fn create_threads(issues: &[Issue], app_state: &AppState) -> Self {
        let git_info = app_state.git_info();
        let cache = app_state.disk_cache();
        let thread_futures = issues
            .iter()
            .map(|issue| async move { IssueThread::from_issue(issue, cache, git_info).await })
            .collect::<Vec<_>>();
        let thread_results = futures::future::join_all(thread_futures).await;
        let mut created = CreatedThreads::default();

        for (result, issue) in thread_results.into_iter().zip(issues) {
            match result {
                Ok(issue_thread) => {
                    let entry = CacheEntry::new(issue, &issue_thread);
                    let key = cache_key_or_default(git_info, issue.updated_at.clone());

                    created.entries.insert(issue.number, entry.clone());
                    app_state
                        .status_cache
                        .blocking_write()
                        .insert(issue.number, key, entry);
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
