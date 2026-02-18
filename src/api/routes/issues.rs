//! Issue endpoints.

use crate::api::cache::CacheKey;
use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues, format_error_list};
use crate::api::state::AppState;
use crate::api::types::{
    BlockedIssueStatus, BlockingQCError, BlockingQCItem, BlockingQCItemWithStatus,
    BlockingQCStatus, CreateIssueRequest, CreateIssueResponse, Issue, IssueStatusResponse,
    QCStatusEnum,
};
use crate::create::QCIssueError;
use crate::{GitProvider, QCEntry, batch_post_qc_entries, get_repo_users};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct IssueStatusQuery {
    /// Comma-separated list of issue numbers
    pub issues: String,
}

/// POST /api/milestones/{number}/issues
pub async fn create_issues<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(milestone_number): Path<u64>,
    Json(requests): Json<Vec<CreateIssueRequest>>,
) -> Result<(StatusCode, Json<Vec<CreateIssueResponse>>), ApiError> {
    // Validate milestone exists
    let milestones = state.git_info().get_milestones().await?;
    if !milestones
        .iter()
        .any(|m| m.number == milestone_number as i64)
    {
        return Err(ApiError::NotFound(format!(
            "Milestone {} not found",
            milestone_number
        )));
    }

    // Get existing issues in milestone
    let milestone_issues = state
        .git_info()
        .get_issues(Some(milestone_number))
        .await
        .unwrap_or_else(|e| {
            log::warn!(
                "Failed to determine issues in milestone {milestone_number}: {e}. Defaulting to none"
            );
            Vec::new()
        });

    let entries = requests
        .into_iter()
        .map(CreateIssueRequest::into)
        .collect::<Vec<QCEntry>>();

    // Check for duplicate filenames within the request
    let mut seen_files = HashSet::new();
    let mut duplicate_files = Vec::new();
    for entry in &entries {
        if !seen_files.insert(&entry.title) {
            duplicate_files.push(entry.title.to_string_lossy().to_string());
        }
    }
    if !duplicate_files.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Duplicate files in request:\n  - {}",
            duplicate_files.join("\n  - ")
        )));
    }

    // Validate assignees exist in repository
    let repo_users = get_repo_users(state.disk_cache(), state.git_info())
        .await?
        .into_iter()
        .map(|r| r.login)
        .collect::<HashSet<_>>();

    let unknown_assignees = entries
        .iter()
        .flat_map(|e| e.assignees.iter())
        .filter(|a| !repo_users.contains(*a))
        .collect::<HashSet<_>>();
    let mut unknown_assignees = unknown_assignees.into_iter().cloned().collect::<Vec<_>>();
    unknown_assignees.sort();
    if !unknown_assignees.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Unknown assignees: {}",
            unknown_assignees.join(", ")
        )));
    }

    // Check if any files already have issues in this milestone
    let duplicate_issues = entries
        .iter()
        .filter(|e| {
            milestone_issues
                .iter()
                .any(|i| PathBuf::from(&i.title) == e.title)
        })
        .map(|e| e.title.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if !duplicate_issues.is_empty() {
        return Err(ApiError::Conflict(format!(
            "Issues already exist in milestone for files:\n  - {}",
            duplicate_issues.join("\n  - ")
        )));
    }

    let res = batch_post_qc_entries(&entries, state.git_info(), milestone_number)
        .await
        .map_err(|e| match e {
            QCIssueError::DependencyResolution { errors } => ApiError::BadRequest(format!(
                "Failed to resolve issue creation order:\n  -{}",
                errors
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("\n  -")
            )),
            QCIssueError::GitHubApiError(e) => ApiError::GitHubApi(e.to_string()),
            _ => ApiError::Internal(e.to_string()),
        })?;

    Ok((
        StatusCode::CREATED,
        Json(res.into_iter().map(CreateIssueResponse::from).collect()),
    ))
}

/// GET /api/issues/status?issues=1,2,3
pub async fn batch_get_issue_status<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<IssueStatusQuery>,
) -> Result<Json<Vec<IssueStatusResponse>>, ApiError> {
    // Parse comma-separated issue numbers, validating all inputs
    let parts: Vec<&str> = query.issues.split(',').map(|s| s.trim()).collect();
    let mut issue_numbers = Vec::new();
    let mut invalid_parts = Vec::new();

    for part in parts {
        match part.parse::<u64>() {
            Ok(num) => issue_numbers.push(num),
            Err(_) => invalid_parts.push(part),
        }
    }

    if !invalid_parts.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Invalid issue numbers: {}",
            invalid_parts.join(", ")
        )));
    }

    if issue_numbers.is_empty() {
        return Err(ApiError::BadRequest(
            "No issue numbers provided".to_string(),
        ));
    }

    let dirty = state.git_info().dirty()?;

    let mut fetched_issues = {
        let cache_read = state.status_cache.read().await;
        let mut fetched_issues =
            FetchedIssues::fetch_issues(&issue_numbers, state.git_info(), &cache_read).await;

        if !fetched_issues.errors.is_empty() {
            return Err(ApiError::GitHubApi(format!(
                "Failed to fetch all issues:\n  -{}",
                format_error_list(&fetched_issues.errors)
            )));
        }

        fetched_issues
            .fetch_blocking_qcs(state.git_info(), &cache_read)
            .await;

        fetched_issues
    }; // cache_read lock released here

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
            determine_blocking_qc_status(
                &mut response.blocking_qc_status,
                &entry.blocking_qc_numbers,
                &fetched_issues,
            );
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

pub(crate) fn determine_blocking_qc_status(
    blocking_status: &mut BlockingQCStatus,
    blocking_numbers: &[u64],
    fetched_issues: &FetchedIssues,
) {
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
}

/// GET /api/issues/{number}
pub async fn get_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<Issue>, ApiError> {
    let issue = state.git_info().get_issue(number).await.map(Issue::from)?;

    Ok(Json(issue))
}

/// GET /api/issues/{number}/blocked
pub async fn get_blocked_issues<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<Vec<BlockedIssueStatus>>, ApiError> {
    let git_info = state.git_info();

    let blocking_issues = state.git_info().get_blocked_issues(number).await?;

    let mut blocked_statuses = Vec::new();
    let mut need_to_fetch = Vec::new();

    // need to drop read lock when done
    {
        let cache_read = state.status_cache.read().await;
        for issue in blocking_issues {
            let key = CacheKey::build(git_info, issue.updated_at.clone())?;
            if let Some(entry) = cache_read.get(issue.number, &key) {
                blocked_statuses.push(BlockedIssueStatus {
                    issue: issue.into(),
                    qc_status: entry.qc_status.clone(),
                });
            } else {
                need_to_fetch.push(issue);
            }
        }
    }

    let created_threads = CreatedThreads::create_threads(&need_to_fetch, &state).await;
    if !created_threads.thread_errors.is_empty() {
        return Err(ApiError::Internal(format!(
            "Failed to determine status:\n  -{}",
            format_error_list(&created_threads.thread_errors)
        )));
    }

    // Merge cached statuses with newly fetched ones
    let mut statuses = blocked_statuses;
    statuses.extend(
        created_threads
            .entries
            .into_values()
            .map(|entry| BlockedIssueStatus {
                issue: entry.issue,
                qc_status: entry.qc_status,
            }),
    );

    Ok(Json(statuses))
}
