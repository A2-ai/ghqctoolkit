//! Preview endpoints — generate HTML previews without posting to GitHub.

use axum::{
    Json,
    extract::{Path, State},
    response::Html,
};
use gix::ObjectId;
use std::{path::PathBuf, str::FromStr};

use crate::api::state::AppState;
use crate::api::types::{
    ApproveRequest, CreateIssueRequest, PreviousQCDiffPreviewRequest, RelevantIssueClass,
    ReviewRequest, UnapproveRequest,
};
use crate::configuration::Checklist;
use crate::create::{
    QCIssue, collaborator_override_for_policy, normalize_collaborator_entries, resolve_issue_people,
};
use crate::issue::IssueThread;
use crate::relevant_files::{PreviousQCDiffComment, RelevantFile, RelevantFileClass};
use crate::{CommentBody, api::error::ApiError};
use crate::{
    GitProvider, QCApprove, QCComment, QCReview, QCUnapprove, api::types::CreateCommentRequest,
};

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
    let configured_author = state.git_info().configured_author();
    let current_user = state.git_info().get_current_user().await.ok().flatten();
    let include_collaborators = state.configuration.read().await.include_collaborators();
    let collaborator_override = request
        .collaborators
        .as_ref()
        .map(|entries| normalize_collaborator_entries(entries))
        .transpose()
        .map_err(ApiError::BadRequest)?;

    let (commit, branch, authors) = tokio::task::spawn_blocking(move || {
        let commit = git_info.commit().unwrap_or_else(|_| "unknown".to_string());
        let branch = git_info.branch().unwrap_or_else(|_| "unknown".to_string());
        let authors = git_info.authors(&file_path).unwrap_or_default();
        (commit, branch, authors)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))?;

    let relevant_files = build_relevant_files(&request);
    let (author, collaborators) = resolve_issue_people(
        configured_author.as_ref(),
        current_user.as_deref(),
        &authors,
        collaborator_override_for_policy(include_collaborators, collaborator_override),
    );

    let qc_issue = QCIssue::new_without_git(
        &request.file,
        0,
        commit,
        branch,
        author,
        collaborators,
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
                include_diff: rf.include_diff,
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

/// POST /api/preview/{number}/review
///
/// Accepts a `ReviewRequest`, generates the review body markdown using `QCReview::generate_body()`,
/// converts it to HTML, and returns the HTML string. The diff compares the given commit
/// against the current working directory.
pub async fn preview_review<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<ReviewRequest>,
) -> Result<Html<String>, ApiError> {
    let commit = ObjectId::from_str(&request.commit)
        .map_err(|e| ApiError::BadRequest(format!("Invalid commit format: {e}")))?;

    let issue = state.git_info().get_issue(number).await?;

    let review = QCReview {
        file: PathBuf::from(&issue.title),
        issue,
        commit,
        note: request.note,
        no_diff: !request.include_diff,
        stash_after_review: request.auto_stash,
        working_dir: state.git_info().path().to_path_buf(),
    };

    let markdown = review.generate_body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
}

/// POST /api/preview/{number}/approve
///
/// Generates the approval comment body as HTML without posting to GitHub.
pub async fn preview_approve<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<ApproveRequest>,
) -> Result<Html<String>, ApiError> {
    let commit = ObjectId::from_str(&request.commit)
        .map_err(|e| ApiError::BadRequest(format!("Invalid commit format: {e}")))?;

    let issue = state.git_info().get_issue(number).await?;

    let approval = QCApprove {
        file: PathBuf::from(&issue.title),
        commit,
        issue,
        note: request.note,
    };

    let markdown = approval.generate_body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
}

/// POST /api/preview/{number}/unapprove
///
/// Generates the unapproval comment body as HTML without posting to GitHub.
pub async fn preview_unapprove<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<UnapproveRequest>,
) -> Result<Html<String>, ApiError> {
    let issue = state.git_info().get_issue(number).await?;

    let unapprove = QCUnapprove {
        issue,
        reason: request.reason,
    };

    let markdown = unapprove.generate_body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
}

fn markdown_to_html(markdown: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let parser = Parser::new_ext(markdown, Options::all());
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

/// POST /api/preview/{number}/comment
///
/// Accepts a `CreateCommentRequest`, generates the issue body markdown using `QCIssue::body()`,
/// converts it to HTML, and returns the HTML string.
pub async fn preview_comment<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<CreateCommentRequest>,
) -> Result<Html<String>, ApiError> {
    let issue = state.git_info().get_issue(number).await?;

    let current_commit = ObjectId::from_str(&request.current_commit)
        .map_err(|e| ApiError::BadRequest(format!("Invalid current commit format: {e}")))?;
    let previous_commit = request
        .previous_commit
        .as_deref()
        .map(ObjectId::from_str)
        .transpose()
        .map_err(|e| ApiError::BadRequest(format!("Invalid previous commit format: {e}")))?;

    let qc_comment = QCComment {
        file: PathBuf::from(&issue.title),
        issue,
        current_commit,
        previous_commit,
        note: request.note,
        no_diff: !request.include_diff,
    };

    let markdown = qc_comment.generate_body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
}

/// POST /api/preview/previous-qc-diff
///
/// Generates the Previous QC diff comment body as HTML without posting it.
pub async fn preview_previous_qc_diff<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<PreviousQCDiffPreviewRequest>,
) -> Result<Html<String>, ApiError> {
    let current_commit = ObjectId::from_str(&request.current_commit)
        .map_err(|e| ApiError::BadRequest(format!("Invalid current commit format: {e}")))?;

    let prev_issue = state
        .git_info()
        .get_issue(request.previous_issue_number)
        .await?;
    let thread = IssueThread::from_issue(&prev_issue, None, state.git_info())
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to load previous QC issue thread: {e}")))?;

    // The approval first, the active segment's newest commit only as a fallback — the
    // same order `create.rs` uses when it posts this comment for real, and the same
    // order `archive.rs` and `record` use. A Previous QC is almost always an approved,
    // closed issue; reading the active segment first would diff against un-reviewed
    // drift here while the created issue diffs against the approval, so the preview
    // would show a comment that is not the one posted. Both can be absent only when
    // nothing about that issue could be placed.
    let prev_commit = thread
        .last_approved_commit()
        .copied()
        .or_else(|| thread.latest_commit().map(|commit| commit.hash))
        .ok_or_else(|| {
            ApiError::Internal(format!(
                "Previous QC issue #{} has no commit to diff against",
                request.previous_issue_number
            ))
        })?;

    let diff_comment = PreviousQCDiffComment {
        issue: prev_issue,
        prev_file: PathBuf::from(&request.previous_file),
        current_file: PathBuf::from(&request.current_file),
        prev_commit,
        current_commit,
        prev_issue_number: request.previous_issue_number,
    };

    let markdown = diff_comment.generate_body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helpers::MockGitInfo;
    use crate::test_utils::create_test_issue;
    use crate::{Configuration, GitComment};
    use axum::extract::State;

    /// Initial QC's anchor, the commit it was approved at, and the drift since.
    const ANCHOR: &str = "1111111111111111111111111111111111111111";
    const APPROVAL: &str = "2222222222222222222222222222222222222222";
    const DRIFT: &str = "3333333333333333333333333333333333333333";

    fn walk() -> Vec<crate::GitCommit> {
        [DRIFT, APPROVAL, ANCHOR]
            .iter()
            .map(|hash| crate::GitCommit {
                commit: ObjectId::from_str(hash).expect("a 40 hex digit sha"),
                message: format!("commit {hash}"),
            })
            .collect()
    }

    /// A Previous QC that was approved and has drifted since: its trailing gap owns
    /// `DRIFT`, which is what makes the two candidate bases differ.
    fn approved_then_drifted() -> MockGitInfo {
        let body = format!(
            "Quality check issue for src/prev.rs\n\n## Metadata\ninitial qc commit: {ANCHOR}\ngit branch: main\nauthor: The Octocat <octocat@example.com>\n\n# Code Review Checklist\n- [x] Reviewed the logic\n"
        );
        let issue = create_test_issue("o", "r", 2, "src/prev.rs", &body, Some(1), "closed");
        MockGitInfo::builder()
            .with_issue(2, issue)
            .with_comments(
                2,
                vec![GitComment {
                    body: format!("# QC Approval\n\n## Metadata\napproved qc commit: {APPROVAL}\n"),
                    author_login: "reviewer".to_string(),
                    created_at: chrono::Utc::now(),
                    id: Some(7),
                    html_url: None,
                    html: None,
                }],
            )
            .with_commits(walk())
            .with_branch_tip(Some(DRIFT.to_string()))
            .build()
    }

    /// The diff a Previous QC comment shows is "what changed since that QC was
    /// approved", so the base is the approval — un-reviewed drift since is not a base
    /// anyone reviewed. `create.rs` builds the very same comment in that order, and a
    /// preview that disagreed with the artifact would be a preview of nothing.
    #[tokio::test]
    async fn the_previous_qc_diff_preview_bases_on_the_approval_not_the_drift_since() {
        let state = AppState::new(
            approved_then_drifted(),
            Configuration::default(),
            None,
            None,
        );

        let Html(html) = preview_previous_qc_diff(
            State(state),
            Json(PreviousQCDiffPreviewRequest {
                current_file: "src/current.rs".to_string(),
                previous_file: "src/prev.rs".to_string(),
                previous_issue_number: 2,
                current_commit: ANCHOR.to_string(),
            }),
        )
        .await
        .expect("an approved previous QC has a base to diff against");

        assert!(
            html.contains(APPROVAL),
            "the approval must be the diff base: {html}"
        );
        assert!(
            !html.contains(DRIFT),
            "un-reviewed drift is not a base the created issue would use: {html}"
        );
    }
}
