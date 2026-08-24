//! Archive generation endpoint.

use axum::{Json, extract::State};
use gix::ObjectId;
use std::path::{Component, PathBuf};

use crate::{
    GitProvider,
    api::{
        error::ApiError,
        state::AppState,
        types::{
            ArchiveFileRequest, ArchiveGenerateRequest, ArchiveGenerateResponse,
            SkippedFileRequest, SkippedFileResponse,
        },
    },
    archive::{ArchiveFile, ArchiveMetadata, ArchiveQC, SkippedFile, archive},
    utils::{EnvProvider, StdEnvProvider},
};

/// POST /api/archive/generate
///
/// Accepts an `ArchiveGenerateRequest` JSON body, builds an archive at
/// `output_path` containing each file at the specified commit, and returns
/// the resolved absolute path of the written archive along with the skips the
/// manifest recorded (D62).
pub async fn generate_archive<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<ArchiveGenerateRequest>,
) -> Result<Json<ArchiveGenerateResponse>, ApiError> {
    if request.output_path.is_empty() {
        return Err(ApiError::BadRequest("output_path is required".to_string()));
    }

    let raw = PathBuf::from(&request.output_path);

    // Reject user-supplied paths that explicitly traverse upward
    for comp in raw.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(ApiError::BadRequest(
                "output_path must not contain '..' components".to_string(),
            ));
        }
    }

    // Canonicalize the repo root first so a relative -d ../project doesn't inject '..'
    let repo_root_for_join = state
        .git_info()
        .path()
        .canonicalize()
        .map_err(|e| ApiError::Internal(format!("Failed to canonicalize repo root: {e}")))?;
    let output_path = if raw.is_absolute() {
        raw
    } else {
        repo_root_for_join.join(&raw)
    };

    let repo_root = repo_root_for_join;

    // Create parent dir if needed, then canonicalize to validate containment
    let parent = output_path.parent().unwrap_or(&output_path);
    std::fs::create_dir_all(parent).map_err(|e| {
        ApiError::BadRequest(format!(
            "Failed to create output directory {}: {e}",
            parent.display()
        ))
    })?;
    let canonical_parent = parent.canonicalize().map_err(|e| {
        ApiError::Internal(format!(
            "Failed to canonicalize output directory {}: {e}",
            parent.display()
        ))
    })?;
    if !canonical_parent.starts_with(&repo_root) {
        return Err(ApiError::BadRequest(
            "output_path must resolve within the repository".to_string(),
        ));
    }
    let output_path = canonical_parent.join(output_path.file_name().unwrap_or_default());

    let (metadata, skipped) = build_metadata(
        request.files,
        request.skipped,
        request.flatten,
        &StdEnvProvider,
    )?;

    let git_info = state.git_info().clone();
    let output_path_clone = output_path.clone();
    tokio::task::spawn_blocking(move || archive(metadata, &git_info, &output_path_clone))
        .await
        .map_err(|e| ApiError::Internal(format!("Archive task panicked: {e}")))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ArchiveGenerateResponse {
        output_path: output_path.to_string_lossy().into_owned(),
        skipped: skipped_responses(skipped),
    }))
}

/// D62: the response reports what was skipped so a client can confirm its declaration
/// was recorded in the manifest.
fn skipped_responses(skipped: Vec<SkippedFile>) -> Vec<SkippedFileResponse> {
    skipped
        .into_iter()
        .map(|skip| SkippedFileResponse {
            repository_file: skip.repository_file,
            round: skip.round,
            branch: skip.branch,
            reason: skip.reason,
        })
        .collect()
}

/// Build the archive manifest from the request body.
///
/// D62: the client decides which files it cannot archive — this path has an explicit
/// `commit` and no issue number, so the server cannot resolve a round for it — and the
/// server's job is to record that decision **verbatim**. This is the only place the
/// declared skips enter the manifest; they are also returned so the response can report
/// what was recorded. A file that appears in `files` still carries a resolvable commit:
/// skipping is not falling back to a substitute commit, on any path.
fn build_metadata(
    files: Vec<ArchiveFileRequest>,
    skipped: Vec<SkippedFileRequest>,
    flatten: bool,
    env: &impl EnvProvider,
) -> Result<(ArchiveMetadata, Vec<SkippedFile>), ApiError> {
    let archive_files = build_archive_files(files, flatten)?;
    let skipped: Vec<SkippedFile> = skipped
        .into_iter()
        .map(|skip| SkippedFile {
            repository_file: skip.repository_file,
            round: skip.round,
            branch: skip.branch,
            reason: skip.reason,
        })
        .collect();

    // D64: a request whose every file was skipped would produce a tarball containing a
    // manifest and no files — an error report in archive form, not an archive. Refuse it,
    // naming the skips. This does NOT reintroduce D62's defect: it fires only when nothing
    // is archivable, so no partial success is ever blocked.
    if archive_files.is_empty() && !skipped.is_empty() {
        let detail = skipped
            .iter()
            .map(|skip| format!("{}: {}", skip.repository_file.display(), skip.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ApiError::BadRequest(format!(
            "nothing to archive — all {} selected file(s) were skipped: {detail}",
            skipped.len()
        )));
    }

    let metadata = ArchiveMetadata::new_with_skipped(archive_files, skipped.clone(), env)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok((metadata, skipped))
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
            // Strip all root/prefix components (e.g. leading / or ..)
            let stripped: PathBuf = file_req
                .repository_file
                .components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .collect();
            if stripped.as_os_str().is_empty() {
                return Err(ApiError::BadRequest(format!(
                    "File path resolves to empty after normalization: {}",
                    file_req.repository_file.display()
                )));
            }
            stripped
        };

        // A6/M10: a QC-attached file carries all four frozen facts or none of them.
        // Only the client holds the round it selected (U5), and there is no safe default
        // for it — inventing one would freeze a wrong claim into an audit snapshot
        // (D28.3) — so the four are required together.
        let qc = match (
            &file_req.milestone,
            file_req.approved,
            file_req.round,
            file_req.subsequent_file_changes,
        ) {
            (Some(milestone), Some(approved), Some(round), Some(subsequent_file_changes)) => {
                Some(ArchiveQC {
                    milestone: milestone.clone(),
                    approved,
                    round,
                    subsequent_file_changes,
                })
            }
            (None, None, None, None) => None,
            _ => {
                return Err(ApiError::BadRequest(format!(
                    "milestone, approved, round and subsequent_file_changes must all be provided or all omitted for file: {}",
                    file_req.repository_file.display()
                )));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::MockEnvProvider;

    const COMMIT: &str = "1234567890123456789012345678901234567890";

    fn mock_env() -> MockEnvProvider {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USER"))
            .returning(|_| Ok("test_user".to_string()));
        mock_env
    }

    fn qc_request() -> ArchiveFileRequest {
        ArchiveFileRequest {
            repository_file: PathBuf::from("src/analysis.R"),
            commit: COMMIT.to_string(),
            milestone: Some("v1.0".to_string()),
            approved: Some(true),
            round: Some(2),
            subsequent_file_changes: Some(true),
        }
    }

    #[test]
    fn test_build_archive_files_carries_round_and_subsequent_file_changes() {
        // A6: the request's `round` and `subsequent_file_changes` reach `ArchiveQC` (M10).
        let files = build_archive_files(vec![qc_request()], false).unwrap();

        let qc = files[0].qc.as_ref().unwrap();
        assert_eq!(qc.milestone, "v1.0");
        assert!(qc.approved);
        assert_eq!(qc.round, 2);
        assert!(qc.subsequent_file_changes);
    }

    #[test]
    fn test_build_archive_files_without_qc_metadata() {
        let request = ArchiveFileRequest {
            milestone: None,
            approved: None,
            round: None,
            subsequent_file_changes: None,
            ..qc_request()
        };

        let files = build_archive_files(vec![request], false).unwrap();
        assert!(files[0].qc.is_none());
    }

    #[test]
    fn test_build_archive_files_rejects_partial_qc_metadata() {
        // There is no safe default for `round`: inventing one would freeze a wrong claim
        // into a detached audit snapshot (D28.3).
        let request = ArchiveFileRequest {
            round: None,
            ..qc_request()
        };

        match build_archive_files(vec![request], false) {
            Err(ApiError::BadRequest(message)) => {
                assert!(message.contains("subsequent_file_changes"));
                assert!(message.contains("src/analysis.R"));
            }
            other => panic!("Expected BadRequest, got {other:?}"),
        }
    }

    /// D62: the batch endpoint records the client's declared skips **verbatim** in the
    /// manifest, so a partial archive declares itself partial whichever interface
    /// produced it. The server cannot re-derive them: this path has an explicit commit
    /// and no issue number.
    #[test]
    fn test_declared_skips_are_recorded_in_the_manifest_verbatim() {
        let request: ArchiveGenerateRequest = serde_json::from_value(serde_json::json!({
            "output_path": "archive.zip",
            "flatten": false,
            "files": [{
                "repository_file": "src/kept.R",
                "commit": "0000000000000000000000000000000000000001",
            }],
            "skipped": [{
                "repository_file": "src/other.R",
                "round": 3,
                "branch": "feature/x",
                "reason": "start commit could not be placed on 'feature/x'",
            }],
        }))
        .unwrap();

        let (metadata, skipped) =
            build_metadata(request.files, request.skipped, request.flatten, &mock_env()).unwrap();

        let json = serde_json::to_value(&metadata).unwrap();
        let recorded = json["skipped"].as_array().expect("the skip is recorded");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["repository_file"], "src/other.R");
        assert_eq!(recorded[0]["round"], 3);
        assert_eq!(recorded[0]["branch"], "feature/x");
        assert_eq!(
            recorded[0]["reason"],
            "start commit could not be placed on 'feature/x'"
        );

        // D62: a partial archive succeeds — the resolvable file is archived and the
        // skipped one is NOT smuggled into `files` under a substitute commit. Skipping is
        // not falling back. Asserted by naming both sides, so an implementation that
        // silently added the skipped file would fail rather than merely changing a count.
        let files = json["files"].as_array().unwrap();
        assert_eq!(files.len(), 1, "the resolvable file is still archived");
        assert_eq!(files[0]["repository_file"], "src/kept.R");
        assert!(
            !files.iter().any(|f| f["repository_file"] == "src/other.R"),
            "the skipped file must never appear in `files`: {files:?}"
        );

        // And the response reports what the manifest recorded.
        let reported = skipped_responses(skipped);
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].repository_file, PathBuf::from("src/other.R"));
        assert_eq!(reported[0].round, 3);
        assert_eq!(reported[0].branch, "feature/x");
        assert_eq!(
            reported[0].reason,
            "start commit could not be placed on 'feature/x'"
        );
    }

    /// D62: `#[serde(default)]` keeps existing clients working unchanged, and a complete
    /// archive's manifest keeps its existing key set byte-for-byte — `skipped` is absent,
    /// not `[]` (D61).
    /// D64: every file skipped ⇒ refuse, rather than writing a tarball that holds a
    /// manifest and no files. Narrow by design — a *partially* skipped archive still
    /// succeeds (D62), which is the case that must never be blocked.
    #[test]
    fn test_an_archive_with_every_file_skipped_is_refused() {
        let skipped = vec![SkippedFileRequest {
            repository_file: PathBuf::from("src/only.rs"),
            round: 2,
            branch: "feature/gone".to_string(),
            reason: "start commit could not be placed on 'feature/gone'".to_string(),
        }];

        let err = build_metadata(Vec::new(), skipped, false, &StdEnvProvider)
            .expect_err("an archive with no files and a skip must be refused");
        let message = err.to_string();
        assert!(
            message.contains("nothing to archive") && message.contains("src/only.rs"),
            "the refusal must name what was skipped, got: {message}"
        );
    }

    #[test]
    fn test_request_without_skipped_keeps_the_existing_manifest_shape() {
        let request: ArchiveGenerateRequest = serde_json::from_value(serde_json::json!({
            "output_path": "archive.zip",
            "flatten": false,
            "files": [{
                "repository_file": "src/analysis.R",
                "commit": COMMIT,
                "milestone": "v1.0",
                "approved": true,
                "round": 2,
                "subsequent_file_changes": false,
            }],
        }))
        .unwrap();
        assert!(request.skipped.is_empty());

        let (metadata, skipped) =
            build_metadata(request.files, request.skipped, request.flatten, &mock_env()).unwrap();
        assert!(skipped.is_empty());
        assert!(skipped_responses(skipped).is_empty());

        let json = serde_json::to_value(&metadata).unwrap();
        let mut keys = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(keys, vec!["created_at", "creator", "files"]);
    }
}
