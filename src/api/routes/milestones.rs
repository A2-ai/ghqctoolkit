//! Milestone endpoints.

use std::path::PathBuf;

use crate::GitProvider;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{CreateMilestoneRequest, DetectedRename, Issue, Milestone};
use crate::detect_renames;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

/// GET /api/milestones
pub async fn list_milestones<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
) -> Result<Json<Vec<Milestone>>, ApiError> {
    let milestones = state.git_info().get_milestones().await?;

    let response: Vec<Milestone> = milestones.into_iter().map(Milestone::from).collect();

    Ok(Json(response))
}

/// POST /api/milestones
pub async fn create_milestone<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<CreateMilestoneRequest>,
) -> Result<(StatusCode, Json<Milestone>), ApiError> {
    if request.name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "Empty Milestone name not allowed".to_string(),
        ));
    }
    let milestone = state
        .git_info()
        .create_milestone(&request.name, &request.description)
        .await
        .map(Milestone::from)?;

    Ok((StatusCode::CREATED, Json(milestone)))
}

/// GET /api/milestones/{number}/issues
pub async fn list_milestone_issues<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<Vec<Issue>>, ApiError> {
    let issues = state.git_info().get_issues(Some(number)).await?;

    let response: Vec<Issue> = issues.into_iter().map(Issue::from).collect();

    Ok(Json(response))
}

/// GET /api/milestones/{number}/renames
///
/// Detect which open-issue file paths in this milestone have been renamed in git.
/// Returns one entry per detected rename: {issue_number, old_path, new_path}.
pub async fn list_milestone_renames<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<Vec<DetectedRename>>, ApiError> {
    let issues = state.git_info().get_issues(Some(number)).await?;

    // Only check open issues — closed ones are done and don't need rename tracking.
    let open_issue_paths: Vec<(u64, PathBuf)> = issues
        .into_iter()
        .filter(|i| matches!(i.state, octocrab::models::IssueState::Open))
        .map(|i| (i.number as u64, PathBuf::from(&i.title)))
        .collect();

    if open_issue_paths.is_empty() {
        return Ok(Json(vec![]));
    }

    let repo_path = state.git_info().path().to_path_buf();
    let issue_paths: Vec<PathBuf> = open_issue_paths.iter().map(|(_, p)| p.clone()).collect();

    let raw_renames = tokio::task::spawn_blocking(move || {
        detect_renames(&repo_path, &issue_paths)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("Rename detection task failed: {}", e)))?;

    // Map old_path back to issue_number
    let response: Vec<DetectedRename> = raw_renames
        .into_iter()
        .filter_map(|(old_path, new_path)| {
            open_issue_paths
                .iter()
                .find(|(_, p)| *p == old_path)
                .map(|(issue_number, _)| DetectedRename {
                    issue_number: *issue_number,
                    old_path: old_path.to_string_lossy().to_string(),
                    new_path: new_path.to_string_lossy().to_string(),
                })
        })
        .collect();

    Ok(Json(response))
}
