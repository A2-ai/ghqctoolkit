//! File tree browsing endpoints.

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{
        HeaderMap, HeaderValue,
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
    },
    response::Response,
};
use serde::Deserialize;
use std::path::Path;

use crate::GitProvider;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{FileCollaboratorsResponse, FileTreeResponse, TreeEntry, TreeEntryKind};
use crate::create::{collaborator_override_for_policy, resolve_issue_people};
use crate::git::GitFileOpsError;

#[derive(Deserialize)]
pub struct FileTreeQuery {
    #[serde(default)]
    path: String,
}

fn sanitize_repo_relative_path(raw_path: &str) -> Result<String, ApiError> {
    let path = raw_path.trim_matches('/').to_string();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        if segment == ".." || segment == "." {
            return Err(ApiError::BadRequest(format!(
                "Invalid path segment: '{}'",
                segment
            )));
        }
    }
    Ok(path)
}

fn preview_content_type(file_path: &Path) -> &'static str {
    match file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    }
}

fn inline_content_disposition(file_path: &Path) -> String {
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("preview")
        .replace('\\', "_")
        .replace('"', "_");
    format!("inline; filename=\"{file_name}\"")
}

/// GET /api/files/tree?path=
pub async fn list_tree<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<FileTreeResponse>, ApiError> {
    // Sanitize: trim leading/trailing slashes, reject `.` and `..` segments
    let path = sanitize_repo_relative_path(&query.path)?;

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

/// GET /api/files/collaborators?path=
pub async fn get_file_collaborators<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Json<FileCollaboratorsResponse>, ApiError> {
    let path = sanitize_repo_relative_path(&query.path)?;

    let configured_author = state.git_info().configured_author();
    let current_user = state.git_info().get_current_user().await.ok().flatten();
    let include_collaborators = state.configuration.read().await.include_collaborators();
    let fallback_author = configured_author
        .as_ref()
        .map(crate::create::format_git_author)
        .or_else(|| current_user.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let git_info = state.git_info().clone();
    let path_for_task = path.clone();

    let (author, collaborators) =
        tokio::task::spawn_blocking(move || git_info.authors(std::path::Path::new(&path_for_task)))
            .await
            .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))?
            .map(|authors| {
                resolve_issue_people(
                    configured_author.as_ref(),
                    current_user.as_deref(),
                    &authors,
                    collaborator_override_for_policy(include_collaborators, None),
                )
            })
            .unwrap_or_else(|_| (fallback_author, Vec::new()));

    Ok(Json(FileCollaboratorsResponse {
        path,
        author: Some(author),
        collaborators,
    }))
}

/// GET /api/files/content?path=<repo-relative path>
///
/// Reads the file from the local filesystem and returns its content as plain text.
pub async fn get_file_content<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<String, ApiError> {
    let path = sanitize_repo_relative_path(&query.path)?;
    let repo_path = state.git_info().path().to_path_buf();
    let file_path = repo_path.join(&path);

    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|_| ApiError::NotFound(format!("File not found: {}", path)))?;

    String::from_utf8(bytes).map_err(|_| {
        ApiError::BadRequest(format!(
            "File '{}' is not valid UTF-8 text and cannot be shown in the text preview",
            path
        ))
    })
}

/// GET /api/files/raw?path=<repo-relative path>
///
/// Reads the file from the local filesystem and returns its raw bytes with an inline content type.
pub async fn get_file_raw<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<FileTreeQuery>,
) -> Result<Response, ApiError> {
    let path = sanitize_repo_relative_path(&query.path)?;
    let repo_path = state.git_info().path().to_path_buf();
    let file_path = repo_path.join(&path);

    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|_| ApiError::NotFound(format!("File not found: {}", path)))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(preview_content_type(&file_path)),
    );
    headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&inline_content_disposition(&file_path))
            .map_err(|e| ApiError::Internal(format!("Invalid content disposition header: {e}")))?,
    );

    let mut response = Response::new(Body::from(bytes));
    *response.headers_mut() = headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{inline_content_disposition, preview_content_type, sanitize_repo_relative_path};
    use std::path::Path;

    #[test]
    fn sanitize_repo_relative_path_rejects_dot_segments() {
        assert!(sanitize_repo_relative_path("../secret.txt").is_err());
        assert!(sanitize_repo_relative_path("./secret.txt").is_err());
        assert!(sanitize_repo_relative_path("safe/path.txt").is_ok());
    }

    #[test]
    fn preview_content_type_recognizes_pdf_and_word_files() {
        assert_eq!(
            preview_content_type(Path::new("report.pdf")),
            "application/pdf"
        );
        assert_eq!(
            preview_content_type(Path::new("report.docx")),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(
            preview_content_type(Path::new("report.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn inline_content_disposition_sanitizes_filename() {
        assert_eq!(
            inline_content_disposition(Path::new("unsafe\\name\".pdf")),
            "inline; filename=\"unsafe_name_.pdf\""
        );
    }
}
