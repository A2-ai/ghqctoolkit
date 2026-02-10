//! Issue endpoints.

use crate::GitHubReader;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    BlockingQCError, BlockingQCItem, BlockingQCItemWithStatus, BlockingQCStatus, ChecklistSummary,
    CommitStatusEnum, CreateIssueRequest, CreateIssueResponse, GitStatus, GitStatusEnum, Issue,
    IssueCommit, IssueStatusResponse, QCStatus, QCStatusEnum,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
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

    // TODO: Implement batch status retrieval with caching
    // This will involve:
    // 1. Fetching issues from GitHub to get updated_at
    // 2. Checking cache for valid entries
    // 3. Computing status for cache misses
    // 4. Resolving blocking QC statuses
    todo!("Implement batch_get_issue_status")
}

/// GET /api/issues/{number}
pub async fn get_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
) -> Result<Json<Issue>, ApiError> {
    let issue = state.git_info.get_issue(number).await.map(Issue::from)?;

    Ok(Json(issue))
}
