//! Issue endpoints.

use std::collections::HashMap;

use crate::api::cache::{CacheEntry, CacheKey};
use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues, format_error_list};
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

    let dirty = state.git_info().dirty()?;

    let mut fetched_issues = FetchedIssues::fetch_issues(
        &issue_numbers,
        state.git_info(),
        &state.status_cache.blocking_read(),
    )
    .await;

    if !fetched_issues.errors.is_empty() {
        return Err(ApiError::GitHubApi(format!(
            "Failed to fetch all issues:\n  -{}",
            format_error_list(&fetched_issues.errors)
        )));
    }

    fetched_issues
        .fetch_blocking_qcs(state.git_info(), &state.status_cache.blocking_read())
        .await;

    let created_threads = CreatedThreads::create_threads(&fetched_issues.issues, &state).await;

    fetched_issues
        .cached_entries
        .extend(created_threads.entries);
    fetched_issues.errors.extend(
        created_threads
            .thread_errors
            .into_iter()
            .map(|(n, e)| (n, e.to_string())),
    );

    let mut errors = HashMap::new();
    let mut responses = Vec::new();

    for issue_number in issue_numbers {
        if let Some(entry) = fetched_issues.cached_entries.get(&issue_number) {
            let mut response = IssueStatusResponse::from_cache_entry(entry.clone(), &dirty);
            response.blocking_qc_status =
                determine_blocking_qc_status(&entry.blocking_qc_numbers, &fetched_issues);
            responses.push(response);
        } else {
            let error = fetched_issues
                .errors
                .get(&issue_number)
                .cloned()
                .unwrap_or("Failed to determine issue status".to_string());
            errors.insert(issue_number, error);
        }
    }

    if !errors.is_empty() {
        return Err(ApiError::Internal(format!(
            "Failed to determine status for all issues:\n  -{}",
            format_error_list(&errors)
        )));
    }

    Ok(Json(responses))
}

fn determine_blocking_qc_status(
    blocking_numbers: &[u64],
    fetched_issues: &FetchedIssues,
) -> BlockingQCStatus {
    let mut blocking_status = BlockingQCStatus::default();
    blocking_status.total = blocking_numbers.len() as u32;
    for number in blocking_numbers {
        if let Some(entry) = fetched_issues.cached_entries.get(number) {
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
        } else if let Some(error) = fetched_issues.errors.get(number) {
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

    let blocking_issues = state.git_info().get_blocked_issues(number).await?;

    let mut blocked_statuses = Vec::new();
    let mut need_to_fetch = Vec::new();
    for issue in blocking_issues {
        let key = CacheKey::build(git_info, issue.updated_at.clone())?;
        if let Some(entry) = state.status_cache.blocking_read().get(issue.number, &key) {
            blocked_statuses.push(BlockedIssueStatus {
                issue: issue.into(),
                qc_status: entry.qc_status.clone(),
            });
        } else {
            need_to_fetch.push(issue);
        }
    }

    let created_threads = CreatedThreads::create_threads(&need_to_fetch, &state).await;
    if !created_threads.thread_errors.is_empty() {
        return Err(ApiError::Internal(format!(
            "Failed to determine status:\n  -{}",
            format_error_list(&created_threads.thread_errors)
        )));
    }
    let statuses = created_threads
        .entries
        .into_values()
        .map(|entry| BlockedIssueStatus {
            issue: entry.issue,
            qc_status: entry.qc_status,
        })
        .collect();

    Ok(Json(statuses))
}
