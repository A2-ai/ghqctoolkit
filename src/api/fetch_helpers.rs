use std::collections::{HashMap, HashSet};

use octocrab::models::issues::Issue;

use crate::{
    GitHubReader, GitProvider, IssueError, IssueThread,
    api::{AppState, types::IssueStatusResponse},
    get_issue_comments, parse_blocking_qcs,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct FetchedIssues {
    pub(crate) issues: Vec<Issue>,
    pub(crate) errors: HashMap<u64, String>,
}

impl FetchedIssues {
    pub(crate) async fn fetch_issues(issue_numbers: &[u64], git_info: &impl GitHubReader) -> Self {
        let issue_futures = issue_numbers
            .iter()
            .map(|number| async move { git_info.get_issue(*number).await })
            .collect::<Vec<_>>();
        let issue_results = futures::future::join_all(issue_futures).await;
        let mut fetched = Self::default();

        for (result, issue_number) in issue_results.into_iter().zip(issue_numbers) {
            match result {
                Ok(issue) => fetched.issues.push(issue),
                Err(e) => {
                    fetched.errors.insert(*issue_number, e.to_string());
                }
            }
        }

        fetched
    }

    pub(crate) async fn fetch_blocking_qcs(&mut self, git_info: &impl GitHubReader) {
        let issue_numbers = self
            .issues
            .iter()
            .filter_map(|issue| issue.body.as_deref())
            .flat_map(parse_blocking_qcs)
            .map(|b| b.issue_number)
            .collect::<HashSet<_>>();
        let blocking_qcs = issue_numbers
            .into_iter()
            .filter(|num| !self.issues.iter().any(|i| i.number == *num))
            .collect::<Vec<_>>();

        let fetched_issues = FetchedIssues::fetch_issues(&blocking_qcs, git_info).await;
        self.issues.extend(fetched_issues.issues);
        self.errors.extend(fetched_issues.errors);
    }
}

#[derive(Debug, Default)]
pub(crate) struct CreatedThreads {
    pub responses: HashMap<u64, IssueStatusResponse>,
    pub blocking_qc_numbers: HashMap<u64, Vec<u64>>,
    pub thread_errors: HashMap<u64, IssueError>,
}

impl CreatedThreads {
    pub(crate) async fn create_threads<G: GitProvider>(
        issues: &[Issue],
        app_state: &AppState<G>,
    ) -> Self {
        let git_info = app_state.git_info();
        let disk_cache = app_state.disk_cache();

        // Step 1: Fetch all comments in parallel
        let comment_futures =
            issues
                .iter()
                .map(|issue| async move {
                    (issue, get_issue_comments(issue, disk_cache, git_info).await)
                })
                .collect::<Vec<_>>();
        let comment_results = futures::future::join_all(comment_futures).await;

        // Step 2: Build IssueThreads, sharing the disk cache for commit lookups.
        let mut thread_results: Vec<(&Issue, Result<IssueThread, IssueError>)> = Vec::new();
        for (issue, comments_result) in comment_results {
            let result = match comments_result {
                Ok(comments) => {
                    IssueThread::from_issue_comments(issue, &comments, git_info, disk_cache)
                }
                Err(e) => Err(IssueError::GitHubApiError(e)),
            };
            thread_results.push((issue, result));
        }

        let mut created = CreatedThreads::default();
        let dirty = git_info.dirty().unwrap_or_default();

        for (issue, result) in thread_results {
            match result {
                Ok(issue_thread) => {
                    created.blocking_qc_numbers.insert(
                        issue.number,
                        IssueStatusResponse::blocking_qc_numbers(issue),
                    );
                    created.responses.insert(
                        issue.number,
                        IssueStatusResponse::new(issue, &issue_thread, &dirty),
                    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helpers::MockGitInfo;

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

    #[tokio::test]
    async fn test_fetch_issues_all_errors() {
        let mock = MockGitInfo::builder().build();
        let fetched = FetchedIssues::fetch_issues(&[1, 2], &mock).await;

        // MockGitInfo returns NotFound by default, so all should be errors
        assert_eq!(fetched.issues.len(), 0);
        assert_eq!(fetched.errors.len(), 2);
    }

    #[tokio::test]
    async fn test_fetch_issues_with_issue_data() {
        use crate::api::tests::helpers::load_test_issue;

        let test_issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder().with_issue(1, test_issue).build();

        let fetched = FetchedIssues::fetch_issues(&[1], &mock).await;

        assert_eq!(fetched.issues.len(), 1);
        assert_eq!(fetched.issues[0].number, 1);
        assert_eq!(fetched.errors.len(), 0);
    }

    #[tokio::test]
    async fn test_fetch_issues_mixed() {
        use crate::api::tests::helpers::load_test_issue;

        let test_issue1 = load_test_issue("test_file_issue");
        let test_issue2 = load_test_issue("config_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue1)
            .with_issue(2, test_issue2)
            .build();

        let fetched = FetchedIssues::fetch_issues(&[1, 2, 3], &mock).await;

        assert_eq!(fetched.issues.len(), 2);
        assert_eq!(fetched.errors.len(), 1);
        assert!(fetched.errors.contains_key(&3));
    }
}
