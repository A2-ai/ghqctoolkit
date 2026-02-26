//! Archive generation endpoint.

use axum::{Json, extract::State};
use gix::ObjectId;
use std::path::PathBuf;

use crate::{
    GitProvider,
    api::{
        error::ApiError,
        state::AppState,
        types::{ArchiveFileRequest, ArchiveGenerateRequest, ArchiveGenerateResponse},
    },
    archive::{ArchiveFile, ArchiveMetadata, ArchiveQC, archive},
    utils::StdEnvProvider,
};

/// POST /api/archive/generate
///
/// Accepts an `ArchiveGenerateRequest` JSON body, builds an archive at
/// `output_path` containing each file at the specified commit, and returns
/// the resolved absolute path of the written archive.
pub async fn generate_archive<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<ArchiveGenerateRequest>,
) -> Result<Json<ArchiveGenerateResponse>, ApiError> {
    if request.output_path.is_empty() {
        return Err(ApiError::BadRequest("output_path is required".to_string()));
    }

    let raw = PathBuf::from(&request.output_path);
    let output_path = if raw.is_absolute() {
        raw
    } else {
        state.git_info().path().join(&raw)
    };

    let flatten = request.flatten;
    let archive_files = build_archive_files(request.files, flatten)?;

    let env = StdEnvProvider;
    let metadata = ArchiveMetadata::new(archive_files, &env)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let git_info = state.git_info().clone();
    let output_path_clone = output_path.clone();
    tokio::task::spawn_blocking(move || archive(metadata, &git_info, &output_path_clone))
        .await
        .map_err(|e| ApiError::Internal(format!("Archive task panicked: {e}")))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ArchiveGenerateResponse {
        output_path: output_path.to_string_lossy().into_owned(),
    }))
}

fn build_archive_files(
    files: Vec<ArchiveFileRequest>,
    flatten: bool,
) -> Result<Vec<ArchiveFile>, ApiError> {
    let mut archive_files = Vec::with_capacity(files.len());

    for file_req in files {
        let commit = ObjectId::from_hex(file_req.commit.as_bytes()).map_err(|e| {
            ApiError::BadRequest(format!("Invalid commit hash '{}': {}", file_req.commit, e))
        })?;

        let archive_file_path = if flatten {
            file_req
                .repository_file
                .file_name()
                .map(PathBuf::from)
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "File has no name: {}",
                        file_req.repository_file.display()
                    ))
                })?
        } else {
            file_req
                .repository_file
                .strip_prefix("/")
                .unwrap_or(&file_req.repository_file)
                .to_path_buf()
        };

        let qc = match (file_req.milestone, file_req.approved) {
            (Some(milestone), Some(approved)) => Some(ArchiveQC { milestone, approved }),
            _ => None,
        };

        archive_files.push(ArchiveFile {
            repository_file: file_req.repository_file,
            archive_file: archive_file_path,
            commit,
            qc,
        });
    }

    Ok(archive_files)
}
