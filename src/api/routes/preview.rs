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
    ApproveRequest, CreateIssueRequest, PreviewRoundRequest, PreviousQCDiffPreviewRequest,
    RelevantIssueClass, ReviewRequest, UnapproveRequest,
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

/// POST /api/preview/round
///
/// D47: renders the `# QC Round N` comment through `QCRound`'s real [`CommentBody`]
/// implementation — the same code path `POST /rounds` posts. There is exactly one
/// implementation of the round comment body, so the preview cannot drift from what gets
/// posted; a client-side re-implementation could not produce the
/// `[file contents at initial qc commit](url)` line at all, because that URL comes from
/// `GitHelpers::file_content_url` and the UI would have to guess the host.
pub async fn preview_round<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Json(request): Json<PreviewRoundRequest>,
) -> Result<Html<String>, ApiError> {
    let start_commit = ObjectId::from_str(&request.start_commit)
        .map_err(|e| ApiError::BadRequest(format!("Invalid commit format: {e}")))?;

    let number = request.issue_number;
    let issue = state.git_info().get_issue(number).await?;

    // The round index is derived exactly as `create_round` derives it, not taken from
    // the request: a preview that titles itself `# QC Round 3` when creation would post
    // `# QC Round 2` is the drift this endpoint exists to prevent.
    let comments = crate::get_issue_comments(&issue, state.disk_cache(), state.git_info()).await?;
    let thread =
        IssueThread::from_issue_comments(&issue, &comments, state.git_info(), state.disk_cache())?;
    let round_index = thread.next_round_index();

    let round = crate::QCRound::new(
        PathBuf::from(&issue.title),
        issue,
        round_index,
        request.branch,
        start_commit,
        Checklist {
            name: request.checklist.name,
            content: request.checklist.content,
        },
    );

    let markdown = round.generate_body(state.git_info());
    let html = markdown_to_html(&markdown);

    Ok(Html(html))
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

    // D55: refuse rather than guess. When the previous QC's latest round has no
    // resolvable commit there is nothing honest to diff against, and the branch to fetch
    // is the actionable half of the message.
    let prev_commit = thread
        .latest_commit()
        .map(|commit| commit.hash)
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "Previous QC #{} has no resolvable commit for its latest round; fetch '{}'",
                request.previous_issue_number,
                thread.branch()
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
    use crate::Configuration;
    use crate::api::tests::helpers::MockGitInfo;
    use crate::api::types::RoundChecklistRequest;

    /// The commit `MockGitInfo`'s walk returns — the only one a mocked thread can
    /// anchor on.
    const MOCK_COMMIT: &str = "456def789abc012345678901234567890123cdef";

    fn state_with(comments: Vec<&str>) -> AppState<MockGitInfo> {
        let issue = crate::test_utils::create_test_issue(
            "test-owner",
            "test-repo",
            1,
            "src/test.rs",
            &format!("## Metadata\n* initial qc commit: {MOCK_COMMIT}\n* git branch: main\n"),
            Some(1),
            "closed",
        );
        let mock = MockGitInfo::builder()
            .with_issue(issue.number, issue)
            .with_comments(1, comments)
            .with_commit(MOCK_COMMIT)
            .with_branch("main")
            .build();
        AppState::new(mock, Configuration::default(), None, None)
    }

    fn preview_request() -> PreviewRoundRequest {
        PreviewRoundRequest {
            issue_number: 1,
            start_commit: MOCK_COMMIT.to_string(),
            branch: "feature/round-two".to_string(),
            checklist: RoundChecklistRequest {
                name: "Second Pass".to_string(),
                content: "- [ ] item\n".to_string(),
            },
        }
    }

    /// D47: the preview renders through `QCRound`'s real `CommentBody`, so it carries
    /// the `[file contents at initial qc commit](url)` line — the line a client-side
    /// re-implementation cannot produce, because the URL comes from
    /// `GitHelpers::file_content_url`.
    #[tokio::test]
    async fn test_preview_round_renders_the_real_round_comment_body() {
        let state = state_with(vec![&format!("approved qc commit: {MOCK_COMMIT}")]);

        let Html(html) = preview_round(State(state), Json(preview_request()))
            .await
            .expect("the round comment previews");

        assert!(html.contains("QC Round 2"), "unexpected html: {html}");
        assert!(
            html.contains("https://github.com/test-owner/test-repo/blob/456def7/src/test.rs"),
            "the file-content URL is the whole reason this endpoint exists: {html}"
        );
        assert!(html.contains("git branch: feature/round-two"));
        assert!(html.contains("round: 2"));
        assert!(html.contains("Second Pass"));
    }

    /// The rendered markdown is **byte-identical** to what `QCRound` produces for the
    /// same inputs: one implementation of the round comment body (D47).
    #[tokio::test]
    async fn test_preview_round_cannot_drift_from_what_gets_posted() {
        let state = state_with(vec![&format!("approved qc commit: {MOCK_COMMIT}")]);

        let Html(html) = preview_round(State(state.clone()), Json(preview_request()))
            .await
            .expect("the round comment previews");

        let issue = crate::GitHubReader::get_issue(state.git_info(), 1)
            .await
            .unwrap();
        let posted = crate::QCRound::new(
            PathBuf::from(&issue.title),
            issue,
            2,
            "feature/round-two".to_string(),
            ObjectId::from_str(MOCK_COMMIT).unwrap(),
            Checklist {
                name: "Second Pass".to_string(),
                content: "- [ ] item\n".to_string(),
            },
        )
        .generate_body(state.git_info());

        assert_eq!(html, markdown_to_html(&posted));
    }

    /// The round index is derived server-side, exactly as `POST /rounds` derives it —
    /// it is not a request field a client could get wrong.
    #[tokio::test]
    async fn test_preview_round_derives_the_round_index() {
        let round_two = format!(
            "# QC Round 2\n\n## Metadata\n* round: 2\n* initial qc commit: {MOCK_COMMIT}\n* git branch: main\n\n# Second Pass\n\n- [ ] item\n"
        );
        let state = state_with(vec![
            &format!("approved qc commit: {MOCK_COMMIT}"),
            &round_two,
            &format!("approved qc commit: {MOCK_COMMIT}"),
        ]);

        let Html(html) = preview_round(State(state), Json(preview_request()))
            .await
            .expect("the round comment previews");

        assert!(html.contains("QC Round 3"), "unexpected html: {html}");
    }

    /// A preview never writes: no comment, no body edit, no re-open.
    #[tokio::test]
    async fn test_preview_round_posts_nothing() {
        let state = state_with(vec![&format!("approved qc commit: {MOCK_COMMIT}")]);

        let _html = preview_round(State(state.clone()), Json(preview_request()))
            .await
            .expect("the round comment previews");

        assert!(
            state.git_info().write_calls().is_empty(),
            "a preview must not write: {:?}",
            state.git_info().write_calls()
        );
    }
}
