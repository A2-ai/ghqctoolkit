//! Milestone endpoints.

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{CreateMilestoneRequest, Issue, Milestone};
use crate::{GitHubReader, GitHubWriter};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

/// GET /api/milestones
pub async fn list_milestones(
    State(state): State<AppState>,
) -> Result<Json<Vec<Milestone>>, ApiError> {
    let milestones = state.git_info.get_milestones().await?;

    let response: Vec<Milestone> = milestones.into_iter().map(Milestone::from).collect();

    Ok(Json(response))
}

/// POST /api/milestones
pub async fn create_milestone(
    State(state): State<AppState>,
    Json(request): Json<CreateMilestoneRequest>,
) -> Result<(StatusCode, Json<Milestone>), ApiError> {
    let milestone = state
        .git_info
        .create_milestone(&request.name, &request.description)
        .await
        .map(Milestone::from)?;

    Ok((StatusCode::CREATED, Json(milestone)))
}

/// GET /api/milestones/{number}/issues
pub async fn list_milestone_issues(
    State(state): State<AppState>,
    Path(number): Path<u64>,
) -> Result<Json<Vec<Issue>>, ApiError> {
    let issues = state.git_info.get_issues(Some(number)).await?;

    let response: Vec<Issue> = issues.into_iter().map(Issue::from).collect();

    Ok(Json(response))
}
