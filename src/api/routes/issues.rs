//! Issue endpoints.

use std::collections::{HashMap, HashSet};

use crate::api::cache::{CacheEntry, CacheKey};
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    BlockedIssueStatus, BlockingQCError, BlockingQCItem, BlockingQCItemWithStatus,
    BlockingQCStatus, CreateIssueRequest, CreateIssueResponse, Issue, IssueStatusResponse,
    QCStatusEnum,
};
use crate::{GitHubReader, GitRepository, GitStatusOps, IssueError, IssueThread};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct IssueStatusQuery {
    /// Comma-separated list of issue numbers
    pub issues: String,
}

/// POST /api/issues
pub async fn create_issue(
    State(state): State<AppState>,
    Json(request): Json<CreateIssueRequest>,
) -> Result<(StatusCode, Json<CreateIssueResponse>), ApiError> {
    // TODO: Implement issue creation logic
    // This will involve:
    // 1. Validating assignees
    // 2. Creating the issue with checklist
    // 3. Handling blocking QC creation
    todo!("Implement create_issue")
}

/// GET /api/issues/status?issues=1,2,3
pub async fn batch_get_issue_status(
    State(state): State<AppState>,
    Query(query): Query<IssueStatusQuery>,
) -> Result<Json<Vec<IssueStatusResponse>>, ApiError> {
    // Parse comma-separated issue numbers
    let issue_numbers: Vec<u64> = query
        .issues
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if issue_numbers.is_empty() {
        return Err(ApiError::BadRequest(
            "No valid issue numbers provided".to_string(),
        ));
    }

    let git_info = state.git_info();
    let disk_cache = state.disk_cache();
    let dirty = git_info.dirty()?;
    let commit = git_info.commit()?;
    let branch = git_info.branch()?;

    let cache_key = |updated_at: DateTime<Utc>| -> CacheKey {
        CacheKey {
            issue_updated_at: updated_at,
            branch: branch.to_string(),
            head_commit: commit.to_string(),
        }
    };

    let issue_futures = issue_numbers
        .iter()
        .map(|i| async move { git_info.get_issue(*i).await })
        .collect::<Vec<_>>();
    let issue_results = futures::future::join_all(issue_futures).await;

    let mut errors = Vec::new();
    // track the Blocking QC numbers that are not also issue numbers to be fetched
    let mut blocking_qc_numbers = HashSet::new();
    let mut entries = HashMap::new();
    let mut threads_to_fetch = Vec::new();

    for (res, issue_number) in issue_results.into_iter().zip(&issue_numbers) {
        match res {
            Ok(issue) => {
                let key = cache_key(issue.updated_at.clone());
                if let Some(entry) = state.status_cache.blocking_read().get(*issue_number, &key) {
                    blocking_qc_numbers.extend(
                        entry
                            .blocking_qc_numbers
                            .iter()
                            .filter(|n| !issue_numbers.contains(*n)),
                    );
                    entries.insert(*issue_number, entry.clone());
                } else {
                    threads_to_fetch.push(issue);
                }
            }
            Err(e) => errors.push(format!("# {}: {}", issue_number, e)),
        }
    }

    if !errors.is_empty() {
        return Err(ApiError::GitHubApi(format!(
            "Failed to fetch all issues:\n  -{}",
            errors.join("\n  -")
        )));
    }

    let blocking_qc_numbers = blocking_qc_numbers.into_iter().collect::<Vec<_>>();

    let blocking_issue_futures = blocking_qc_numbers
        .iter()
        .map(|i| async move { git_info.get_issue(*i).await })
        .collect::<Vec<_>>();
    let blocking_issue_results = futures::future::join_all(blocking_issue_futures).await;

    let mut blocking_qc_errors = HashMap::new();
    for (res, blocking_issue_number) in blocking_issue_results.into_iter().zip(blocking_qc_numbers)
    {
        match res {
            Ok(issue) => {
                let key = cache_key(issue.updated_at.clone());
                if let Some(entry) = state
                    .status_cache
                    .blocking_read()
                    .get(blocking_issue_number, &key)
                {
                    entries.insert(blocking_issue_number, entry.clone());
                } else {
                    threads_to_fetch.push(issue);
                }
            }
            Err(e) => {
                blocking_qc_errors.insert(
                    blocking_issue_number,
                    BlockingQCError {
                        issue_number: blocking_issue_number,
                        error: e.to_string(),
                    },
                );
            }
        }
    }

    let threads_futures = threads_to_fetch
        .iter()
        .map(|i| async move { IssueThread::from_issue(i, disk_cache, git_info).await })
        .collect::<Vec<_>>();
    let thread_results = futures::future::join_all(threads_futures).await;
    let mut thread_errors = HashMap::new();

    for (res, issue) in thread_results.into_iter().zip(threads_to_fetch) {
        match res {
            Ok(issue_thread) => {
                let entry = CacheEntry::new(&issue, &issue_thread);
                entries.insert(issue.number, entry.clone());
                let key = cache_key(issue.updated_at.clone());
                state
                    .status_cache
                    .blocking_write()
                    .insert(issue.number, key, entry);
            }
            Err(e) => {
                thread_errors.insert(issue.number, e);
            }
        }
    }

    let mut errors = Vec::new();
    let mut responses = Vec::new();
    for issue_number in issue_numbers {
        if let Some(entry) = entries.get(&issue_number) {
            let mut response = IssueStatusResponse::from_cache_entry(entry.clone(), &dirty);
            response.blocking_qc_status = determine_blocking_qc_status(
                &entry.blocking_qc_numbers,
                &entries,
                &blocking_qc_errors,
                &thread_errors,
            );
            responses.push(response);
        } else if let Some(error) = thread_errors.get(&issue_number) {
            errors.push(format!("#{}: {}", issue_number, error));
        } else {
            errors.push(format!(
                "#{}: Failed to determine issue status",
                issue_number
            ));
        }
    }

    if !errors.is_empty() {
        return Err(ApiError::Internal(format!(
            "Failed to determine status for all issues:\n  -{}",
            errors.join("\n  -")
        )));
    }

    Ok(Json(responses))
}

fn determine_blocking_qc_status(
    blocking_numbers: &[u64],
    entries: &HashMap<u64, CacheEntry>,
    blocking_qc_errors: &HashMap<u64, BlockingQCError>,
    thread_errors: &HashMap<u64, IssueError>,
) -> BlockingQCStatus {
    let mut blocking_status = BlockingQCStatus::default();
    blocking_status.total = blocking_numbers.len() as u32;
    for number in blocking_numbers {
        if let Some(entry) = entries.get(number) {
            match entry.qc_status.status {
                QCStatusEnum::Approved | QCStatusEnum::ChangesAfterApproval => {
                    blocking_status.approved_count += 1;
                    blocking_status.approved.push(BlockingQCItem {
                        issue_number: *number,
                        file_name: entry.issue.title.clone(),
                    });
                }
                _ => {
                    blocking_status.not_approved.push(BlockingQCItemWithStatus {
                        issue_number: *number,
                        file_name: entry.issue.title.clone(),
                        status: entry.qc_status.status_detail.clone(),
                    });
                }
            }
        } else if let Some(error) = blocking_qc_errors.get(number) {
            blocking_status.errors.push(error.clone());
        } else if let Some(error) = thread_errors.get(number) {
            blocking_status.errors.push(BlockingQCError {
                issue_number: *number,
                error: error.to_string(),
            });
        } else {
            blocking_status.errors.push(BlockingQCError {
                issue_number: *number,
                error: "Failed to determine status".to_string(),
            });
        }
    }

    blocking_status.summary = if blocking_status.approved_count == blocking_status.total {
        "All blocking QCs approved".to_string()
    } else {
        format!(
            "{}/{} blocking QCs are approved",
            blocking_status.approved_count, blocking_status.total
        )
    };

    blocking_status
}

/// GET /api/issues/{number}
pub async fn get_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
) -> Result<Json<Issue>, ApiError> {
    let issue = state
        .git_info()
        .get_issue(number)
        .await
        .map(Issue::from)?
        .into();

    Ok(Json(issue))
}

/// GET /api/issues/{number}/blocked
pub async fn get_blocked_issues(
    State(state): State<AppState>,
    Path(number): Path<u64>,
) -> Result<Json<Vec<BlockedIssueStatus>>, ApiError> {
    let git_info = state.git_info();
    let cache = state.disk_cache();

    let branch = git_info.branch()?;
    let commit = git_info.commit()?;
    let cache_key = |updated_at: DateTime<Utc>| -> CacheKey {
        CacheKey {
            issue_updated_at: updated_at,
            branch: branch.clone(),
            head_commit: commit.clone(),
        }
    };

    let blocking_issues = state.git_info().get_blocked_issues(number).await?;

    let mut blocked_statuses = Vec::new();
    let mut need_to_fetch = Vec::new();
    for issue in blocking_issues {
        let key = cache_key(issue.updated_at.clone());
        if let Some(entry) = state.status_cache.blocking_read().get(issue.number, &key) {
            blocked_statuses.push(BlockedIssueStatus {
                issue: issue.into(),
                qc_status: entry.qc_status.clone(),
            });
        } else {
            need_to_fetch.push(issue);
        }
    }

    let thread_futures = need_to_fetch
        .iter()
        .map(|issue| async move { IssueThread::from_issue(issue, cache, git_info).await })
        .collect::<Vec<_>>();
    let thread_results = futures::future::join_all(thread_futures).await;

    let mut errors = Vec::new();
    for (res, issue) in thread_results.iter().zip(need_to_fetch) {
        match res {
            Ok(issue_thread) => {
                let entry = CacheEntry::new(&issue, issue_thread);
                let issue_number = issue.number;
                let key = cache_key(issue.updated_at.clone());

                blocked_statuses.push(BlockedIssueStatus {
                    issue: issue.into(),
                    qc_status: entry.qc_status.clone(),
                });

                state
                    .status_cache
                    .blocking_write()
                    .insert(issue_number, key, entry);
            }
            Err(e) => errors.push(format!("#{}: {}", issue.number, e)),
        }
    }

    if !errors.is_empty() {
        return Err(ApiError::Internal(format!(
            "Failed to determine status:\n  -{}",
            errors.join("\n  -")
        )));
    }

    Ok(Json(blocked_statuses))
}
