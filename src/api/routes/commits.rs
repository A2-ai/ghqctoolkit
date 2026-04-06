//! Branch commit listing endpoint.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::GitProvider;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::{find_commits, find_or_cache_file_changes};

const DEFAULT_PAGE_SIZE: usize = 10;
const MAX_PAGE_SIZE: usize = 50;

#[derive(Deserialize)]
pub struct CommitsQuery {
    pub file: Option<String>,
    /// 0-indexed page number (default 0)
    #[serde(default)]
    pub page: usize,
    /// Commits per page (default 10, max 50)
    pub page_size: Option<usize>,
    /// Commit hash prefix; if set, returns the page containing that commit
    /// instead of using the `page` parameter.
    pub locate: Option<String>,
}

#[derive(Serialize)]
pub struct BranchCommit {
    pub hash: String,
    pub message: String,
    pub file_changed: bool,
}

#[derive(Serialize)]
pub struct PagedCommitsResponse {
    pub commits: Vec<BranchCommit>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

/// GET /api/commits?file=optional&page=0&page_size=10&locate=hash_prefix
///
/// Returns a page of commits on the current branch (HEAD), newest first.
/// If `file` is provided, `file_changed` is set for commits that touched it.
/// If `locate` is provided, returns the page containing that commit hash.
pub async fn get_commits<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(params): Query<CommitsQuery>,
) -> Result<Json<PagedCommitsResponse>, ApiError> {
    let git_info = state.git_info().clone();
    let file = params.file.clone();
    let disk_cache = state.disk_cache();
    let page_size = params
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    let all_commits = find_commits(&git_info, &None, None, disk_cache)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let total = all_commits.len();

    // Determine which page to return
    let page = if let Some(locate_hash) = &params.locate {
        all_commits
            .iter()
            .position(|c| c.commit.to_string().starts_with(locate_hash.as_str()))
            .map(|idx| idx / page_size)
            .unwrap_or(params.page)
    } else {
        params.page
    };

    let start = page * page_size;
    let end = (start + page_size).min(total);

    // If a file filter is requested, resolve which commits touch it (one git call, cached).
    let touching: Option<std::collections::HashSet<String>> = if let Some(ref f) = file {
        let hashes: Vec<String> = all_commits.iter().map(|c| c.commit.to_string()).collect();
        Some(
            find_or_cache_file_changes(&hashes, &git_info, None, &PathBuf::from(f), disk_cache)
                .map_err(ApiError::from)?,
        )
    } else {
        None
    };

    let commits: Vec<BranchCommit> = if start < total {
        all_commits[start..end]
            .iter()
            .map(|c| {
                let hash_str = c.commit.to_string();
                let file_changed = touching.as_ref().map_or(false, |s| s.contains(&hash_str));
                BranchCommit {
                    hash: hash_str,
                    message: c.message.trim().to_string(),
                    file_changed,
                }
            })
            .collect()
    } else {
        vec![]
    };

    Ok(Json(PagedCommitsResponse {
        commits,
        total,
        page,
        page_size,
    }))
}
