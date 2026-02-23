//! Preview endpoints — generate HTML previews without posting to GitHub.

use axum::{
    Json,
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;
use std::path::PathBuf;

use crate::GitProvider;
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{CreateIssueRequest, RelevantIssueClass};
use crate::configuration::Checklist;
use crate::create::QCIssue;
use crate::relevant_files::{RelevantFile, RelevantFileClass};

#[derive(Deserialize)]
pub struct FileContentQuery {
    path: String,
}

/// GET /api/files/content?path=<repo-relative path>
///
/// Reads the file from the local filesystem and returns its content as plain text.
pub async fn get_file_content<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<FileContentQuery>,
) -> Result<String, ApiError> {
    let path = query.path.trim_matches('/').to_string();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        if segment == ".." || segment == "." {
            return Err(ApiError::BadRequest(format!(
                "Invalid path segment: '{}'",
                segment
            )));
        }
    }

    let repo_path = state.git_info().path().to_path_buf();
    let file_path = repo_path.join(&path);

    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|_| ApiError::NotFound(format!("File not found: {}", path)))?;

    Ok(content)
}

/// POST /api/preview/issue
///
/// Accepts a `CreateIssueRequest`, generates the issue body markdown using `QCIssue::body()`,
/// converts it to HTML, and returns the HTML string.
pub async fn preview_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<CreateIssueRequest>,
) -> Result<Html<String>, ApiError> {
    let git_info = state.git_info().clone();
    let file_path = PathBuf::from(&request.file);

    let (commit, branch, authors) = tokio::task::spawn_blocking(move || {
        let commit = git_info.commit().unwrap_or_else(|_| "unknown".to_string());
        let branch = git_info.branch().unwrap_or_else(|_| "unknown".to_string());
        let authors = git_info.authors(&file_path).unwrap_or_default();
        (commit, branch, authors)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))?;

    let relevant_files = build_relevant_files(&request);

    let qc_issue = QCIssue::new_without_git(
        &request.file,
        0,
        commit,
        branch,
        authors,
        request.assignees.clone(),
        Checklist {
            name: request.checklist_name.clone(),
            content: request.checklist_content.clone(),
        },
        relevant_files,
    );

    let markdown = qc_issue.body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
}

/// Convert `CreateIssueRequest` relevant-file fields into `Vec<RelevantFile>`.
/// `New` batch references use issue_number 0 as a placeholder.
fn build_relevant_files(request: &CreateIssueRequest) -> Vec<RelevantFile> {
    let mut files = Vec::new();

    for rf in &request.gating_qc {
        let (issue_number, issue_id) = resolve_issue_class(&rf.issue_class);
        files.push(RelevantFile {
            file_name: rf.file_name.clone(),
            class: RelevantFileClass::GatingQC {
                issue_number,
                issue_id,
                description: rf.description.clone(),
            },
        });
    }

    for rf in &request.previous_qc {
        let (issue_number, issue_id) = resolve_issue_class(&rf.issue_class);
        files.push(RelevantFile {
            file_name: rf.file_name.clone(),
            class: RelevantFileClass::PreviousQC {
                issue_number,
                issue_id,
                description: rf.description.clone(),
            },
        });
    }

    for rf in &request.relevant_qc {
        let (issue_number, _) = resolve_issue_class(&rf.issue_class);
        files.push(RelevantFile {
            file_name: rf.file_name.clone(),
            class: RelevantFileClass::RelevantQC {
                issue_number,
                description: rf.description.clone(),
            },
        });
    }

    for rf in &request.relevant_files {
        files.push(RelevantFile {
            file_name: PathBuf::from(&rf.file_path),
            class: RelevantFileClass::File {
                justification: rf.justification.clone(),
            },
        });
    }

    files
}

fn resolve_issue_class(class: &RelevantIssueClass) -> (u64, Option<u64>) {
    match class {
        RelevantIssueClass::Exists {
            issue_number,
            issue_id,
        } => (*issue_number, *issue_id),
        RelevantIssueClass::New(_) => (0, None),
    }
}

fn markdown_to_html(markdown: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let parser = Parser::new_ext(markdown, Options::all());
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}
