//! File tree browsing endpoints.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;

use crate::GitProvider;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{FileTreeResponse, TreeEntry, TreeEntryKind};
use crate::git::GitFileOpsError;

#[derive(Deserialize)]
pub struct FileTreeQuery {
    #[serde(default)]
    path: String,
}

/// GET /api/files/tree?path=
pub async fn list_tree<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<FileTreeResponse>, ApiError> {
    // Sanitize: trim leading/trailing slashes, reject `.` and `..` segments
    let path = query.path.trim_matches('/').to_string();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        if segment == ".." || segment == "." {
            return Err(ApiError::BadRequest(format!(
                "Invalid path segment: '{}'",
                segment
            )));
        }
    }

    let git_info = state.git_info().clone();
    let path_for_task = path.clone();

    let entries = tokio::task::spawn_blocking(move || git_info.list_tree_entries(&path_for_task))
        .await
        .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))?
        .map_err(|e| match e {
            GitFileOpsError::DirectoryNotFound(p) => {
                ApiError::NotFound(format!("Directory not found: {}", p))
            }
            GitFileOpsError::NotADirectory(p) => {
                ApiError::BadRequest(format!("Not a directory: {}", p))
            }
            other => ApiError::Internal(other.to_string()),
        })?;

    let response = FileTreeResponse {
        path,
        entries: entries
            .into_iter()
            .map(|(name, is_dir)| TreeEntry {
                name,
                kind: if is_dir {
                    TreeEntryKind::Directory
                } else {
                    TreeEntryKind::File
                },
            })
            .collect(),
    };

    Ok(Json(response))
}
