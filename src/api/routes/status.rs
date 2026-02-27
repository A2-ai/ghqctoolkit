//! Supporting data endpoints.

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{Assignee, RepoInfoResponse};
use crate::{GitProvider, get_repo_users};
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

pub async fn repo_info<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
) -> Result<Json<RepoInfoResponse>, ApiError> {
    let response = RepoInfoResponse::new(state.git_info()).await?;

    // Invalidate the commit cache for this branch if HEAD has moved.
    {
        let mut commit_cache = state.commit_cache.write().await;
        if let Some(commits) = commit_cache.get(&response.branch) {
            let cached_head = commits.first().map(|c| c.commit.to_string());
            if cached_head.as_deref() != Some(&response.local_commit) {
                log::debug!(
                    "Commit cache invalidated for branch '{}': cached HEAD {:?} != current HEAD {}",
                    response.branch,
                    cached_head,
                    response.local_commit
                );
                commit_cache.remove(&response.branch);
            }
        }
    }

    Ok(Json(response))
}
