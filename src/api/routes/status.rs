//! Supporting data endpoints.

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::Assignee;
use crate::{get_repo_users, GitProvider};
use axum::{Json, extract::State};

/// GET /api/assignees
pub async fn list_assignees<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
) -> Result<Json<Vec<Assignee>>, ApiError> {
    let users = get_repo_users(state.disk_cache(), state.git_info()).await?;

    let response: Vec<Assignee> = users
        .into_iter()
        .map(|u| Assignee {
            login: u.login,
            name: u.name,
        })
        .collect();

    Ok(Json(response))
}
