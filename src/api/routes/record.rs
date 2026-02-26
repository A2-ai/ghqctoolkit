//! Record PDF generation endpoints.

use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use axum::extract::Multipart;
use serde::Deserialize;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    ContextPosition, GitProvider, QCContext, UreqDownloader, create_staging_dir,
    fetch_milestone_issues, get_milestone_issue_information, record,
    api::{error::ApiError, state::AppState},
    api::types::{RecordPreviewResponse, RecordRequest, RecordUploadResponse},
    render,
    utils::StdEnvProvider,
};

fn generate_key() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = DefaultHasher::new();
    timestamp.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// POST /api/record/upload
///
/// Accepts `multipart/form-data` with a single PDF field `file`.
/// Saves bytes to a temp file and returns the path.
pub async fn upload_context_file<G: GitProvider + 'static>(
    State(_state): State<AppState<G>>,
    mut multipart: Multipart,
) -> Result<Json<RecordUploadResponse>, ApiError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut field_content_type: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            field_content_type = field.content_type().map(|ct| ct.to_string());
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("Failed to read file bytes: {e}")))?;
            file_bytes = Some(data.to_vec());
        }
    }

    let bytes = file_bytes.ok_or_else(|| ApiError::BadRequest("No 'file' field found".into()))?;

    // Verify content-type is application/pdf
    let ct = field_content_type.unwrap_or_default();
    if ct != "application/pdf" {
        return Err(ApiError::BadRequest(format!(
            "Expected application/pdf, got '{ct}'"
        )));
    }

    // Save to temp dir
    let upload_dir = std::env::temp_dir().join("ghqc-uploads");
    tokio::fs::create_dir_all(&upload_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create upload dir: {e}")))?;

    let key = generate_key();
    let temp_path = upload_dir.join(format!("{key}.pdf"));
    tokio::fs::write(&temp_path, &bytes)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write uploaded file: {e}")))?;

    Ok(Json(RecordUploadResponse {
        temp_path: temp_path.to_string_lossy().into_owned(),
    }))
}

/// Shared helper: run the full record pipeline, returning the output PDF path.
async fn run_record_pipeline<G: GitProvider + 'static>(
    state: &AppState<G>,
    request: &RecordRequest,
    output_path: PathBuf,
) -> Result<(), ApiError> {
    let git_info = state.git_info().clone();

    // Fetch all milestones and filter to the requested ones
    let all_milestones = git_info.get_milestones().await?;
    let selected_milestones: Vec<octocrab::models::Milestone> = all_milestones
        .into_iter()
        .filter(|m| request.milestone_numbers.contains(&(m.number as u64)))
        .collect();

    if selected_milestones.is_empty() {
        return Err(ApiError::BadRequest(
            "No matching milestones found".to_string(),
        ));
    }

    // Fetch issues for each selected milestone
    let milestone_issues = fetch_milestone_issues(&selected_milestones, &git_info)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Create staging directory (used for images, logo, template)
    let staging_dir = create_staging_dir().map_err(|e| ApiError::Internal(e.to_string()))?;

    // Download images and build detailed issue information
    let http_downloader = UreqDownloader::new();
    let issue_information = get_milestone_issue_information(
        &milestone_issues,
        state.disk_cache(),
        &git_info,
        &http_downloader,
        &staging_dir,
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Generate Typst markup
    let configuration = state.configuration.read().await;
    let env = StdEnvProvider;
    let record_str = record(
        &selected_milestones,
        &issue_information,
        &configuration,
        &git_info,
        &env,
        request.tables_only,
        &staging_dir,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Build context file list
    let qc_contexts: Vec<QCContext> = request
        .context_files
        .iter()
        .map(|f| {
            let pos = if f.position == "prepend" {
                ContextPosition::Prepend
            } else {
                ContextPosition::Append
            };
            QCContext::new(&f.server_path, pos)
        })
        .collect();

    // Render Typst to PDF (synchronous, potentially slow — runs in blocking task)
    let http_for_render = http_downloader.clone();
    let staging_for_render = staging_dir.clone();
    tokio::task::spawn_blocking(move || {
        render(
            &record_str,
            &output_path,
            &staging_for_render,
            &qc_contexts,
            None, // disk cache not needed for Typst packages in API context
            &http_for_render,
        )
    })
    .await
    .map_err(|e| ApiError::Internal(format!("Render task panicked: {e}")))?
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(())
}

/// POST /api/record/preview
///
/// Runs the record pipeline, writes a temp PDF, stores it by a UUID key, and returns the key.
pub async fn preview_record<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<RecordRequest>,
) -> Result<Json<RecordPreviewResponse>, ApiError> {
    let key = generate_key();
    let output_path = std::env::temp_dir().join(format!("ghqc-preview-{key}.pdf"));

    run_record_pipeline(&state, &request, output_path.clone()).await?;

    state.preview_store().await.insert(key.clone(), output_path);

    Ok(Json(RecordPreviewResponse { key }))
}

#[derive(Deserialize)]
pub struct PreviewKeyQuery {
    key: String,
}

/// GET /api/record/preview.pdf?key=…
///
/// Looks up the key in the preview store and serves the PDF file.
pub async fn serve_preview_pdf<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<PreviewKeyQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let path = {
        let store = state.preview_store().await;
        store.get(&query.key).cloned()
    };

    let path = path.ok_or_else(|| ApiError::NotFound(format!("Preview key not found: {}", query.key)))?;

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read preview PDF: {e}")))?;

    let response = (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/pdf"),
        )],
        Bytes::from(bytes),
    );

    Ok(response)
}

/// POST /api/record/generate
///
/// Runs the record pipeline and writes the PDF to `request.output_path`.
pub async fn generate_record<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<RecordRequest>,
) -> Result<StatusCode, ApiError> {
    if request.output_path.is_empty() {
        return Err(ApiError::BadRequest("output_path is required".to_string()));
    }

    let output_path = PathBuf::from(&request.output_path);

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create output directory: {e}")))?;
    }

    run_record_pipeline(&state, &request, output_path).await?;

    Ok(StatusCode::OK)
}
