//! Comment, approve, unapprove, and review endpoints.

use std::path::PathBuf;

use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues};
use crate::api::routes::issues::determine_blocking_qc_status;
use crate::api::state::AppState;
use crate::api::types::{
    ApprovalResponse, ApproveQuery, ApproveRequest, BlockingQCError, BlockingQCItemWithStatus,
    BlockingQCStatus, CommentResponse, CreateCommentRequest, ReviewRequest, ReviewResponse,
    UnapprovalResponse, UnapproveRequest,
};
use crate::{
    GitProvider, QCApprove, QCComment, QCReview, QCUnapprove, parse_blocking_qcs, stash_review_file,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use gix::ObjectId;

/// POST /api/issues/{number}/comment
pub async fn create_comment<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<CreateCommentRequest>,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let previous_commit = request
        .previous_commit
        .as_deref()
        .map(parse_str_as_commit)
        .transpose()?;
    let current_commit = parse_str_as_commit(&request.current_commit)?;

    let issue = state.git_info().get_issue(number).await?;

    let comment = QCComment {
        file: PathBuf::from(&issue.title),
        issue,
        current_commit,
        previous_commit,
        note: request.note,
        no_diff: !request.include_diff,
    };

    let comment_url = state.git_info().post_comment(&comment).await?;

    Ok((StatusCode::CREATED, Json(CommentResponse { comment_url })))
}

/// POST /api/issues/{number}/approve
pub async fn approve_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Query(query): Query<ApproveQuery>,
    Json(request): Json<ApproveRequest>,
) -> Result<(StatusCode, Json<ApprovalResponse>), ApiError> {
    let issue = state.git_info().get_issue(number).await?;
    let blocking_qcs = issue
        .body
        .as_deref()
        .map(|b| {
            parse_blocking_qcs(b)
                .iter()
                .map(|b| b.issue_number)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let blocking_status = get_blocking_qc_status(&blocking_qcs, &state).await;

    if blocking_status.approved_count != blocking_status.total && !query.force {
        #[derive(serde::Serialize)]
        struct BlockingQCConflict {
            not_approved: Vec<BlockingQCItemWithStatus>,
            errors: Vec<BlockingQCError>,
        }
        let conflict = BlockingQCConflict {
            not_approved: blocking_status.not_approved,
            errors: blocking_status.errors,
        };
        // Use ConflictDetails to avoid double JSON encoding
        let value = serde_json::to_value(conflict).unwrap_or_else(
            |_| serde_json::json!({"error": "Failed to serialize conflict details"}),
        );
        return Err(ApiError::ConflictDetails(value));
    }

    let commit = parse_str_as_commit(&request.commit)?;

    let approval = QCApprove {
        file: PathBuf::from(&issue.title),
        commit,
        issue: issue.clone(),
        note: request.note,
    };

    let approval_url = state.git_info().post_comment(&approval).await?;
    let closed = state.git_info().close_issue(issue.number).await.is_ok();

    Ok((
        StatusCode::CREATED,
        Json(ApprovalResponse {
            approval_url,
            skipped_unapproved: if query.force {
                blocking_status
                    .not_approved
                    .iter()
                    .map(|c| c.issue_number)
                    .collect()
            } else {
                Vec::new()
            },
            skipped_errors: if query.force {
                blocking_status.errors
            } else {
                Vec::new()
            },
            closed,
        }),
    ))
}

/// POST /api/issues/{number}/unapprove
pub async fn unapprove_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<UnapproveRequest>,
) -> Result<(StatusCode, Json<UnapprovalResponse>), ApiError> {
    let issue = state.git_info().get_issue(number).await?;
    let unapprove = QCUnapprove {
        issue,
        reason: request.reason,
    };

    let unapproval_url = state.git_info().post_comment(&unapprove).await?;

    let opened = state.git_info().open_issue(number).await.is_ok();

    Ok((
        StatusCode::CREATED,
        Json(UnapprovalResponse {
            unapproval_url,
            opened,
        }),
    ))
}

/// POST /api/issues/{number}/review
pub async fn review_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<ReviewRequest>,
) -> Result<(StatusCode, Json<ReviewResponse>), ApiError> {
    let commit = parse_str_as_commit(&request.commit)?;

    let issue = state.git_info().get_issue(number).await?;
    let review_file = PathBuf::from(&issue.title);

    let review = QCReview {
        file: review_file.clone(),
        issue,
        commit,
        note: request.note,
        no_diff: !request.include_diff,
        stash_after_review: request.auto_stash,
        working_dir: state.git_info().path().to_path_buf(),
    };

    let comment_url = state.git_info().post_comment(&review).await?;

    let stash = stash_review_file(state.git_info(), number, &review_file, request.auto_stash);

    Ok((
        StatusCode::CREATED,
        Json(ReviewResponse { comment_url, stash }),
    ))
}

fn parse_str_as_commit(commit: &str) -> Result<ObjectId, ApiError> {
    commit
        .parse()
        .map_err(|e: gix::hash::decode::Error| ApiError::BadRequest(e.to_string()))
}

pub(crate) async fn get_blocking_qc_status<G: GitProvider>(
    blocking_qcs: &[u64],
    state: &AppState<G>,
) -> BlockingQCStatus {
    let mut status = BlockingQCStatus::default();
    if blocking_qcs.is_empty() {
        status.summary = "No blocking QCs".to_string();
        return status;
    }
    status.total = blocking_qcs.len() as u32;

    let git_info = state.git_info();

    let mut fetched_issues = FetchedIssues::fetch_issues(blocking_qcs, git_info).await;

    let created_threads = CreatedThreads::create_threads(&fetched_issues.issues, state).await;
    fetched_issues.errors.extend(
        created_threads
            .thread_errors
            .into_iter()
            .map(|(n, e)| (n, e.to_string())),
    );

    status.errors.extend(
        fetched_issues
            .errors
            .iter()
            .map(|(num, err)| BlockingQCError {
                issue_number: *num,
                error: err.clone(),
            }),
    );

    determine_blocking_qc_status(
        &mut status,
        blocking_qcs,
        &created_threads.responses,
        &fetched_issues.errors,
    );

    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::ReviewStashStatus;
    use crate::api::state::AppState;
    use crate::api::tests::helpers::{MockGitInfo, load_test_issue};

    #[tokio::test]
    async fn test_get_blocking_qc_status_empty() {
        let mock = MockGitInfo::builder().build();
        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        let status = get_blocking_qc_status(&[], &state).await;

        assert_eq!(status.total, 0);
        assert_eq!(status.approved_count, 0);
        assert_eq!(status.summary, "No blocking QCs");
        assert!(status.approved.is_empty());
        assert!(status.not_approved.is_empty());
        assert!(status.errors.is_empty());
    }

    #[tokio::test]
    async fn test_review_issue_reports_stash_failure_nonfatally() {
        let issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(issue.number, issue.clone())
            .with_stash_error("stash failed")
            .build();
        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        let response = review_issue(
            State(state),
            Path(issue.number),
            Json(ReviewRequest {
                commit: "456def789abc012345678901234567890123cdef".to_string(),
                note: Some("test".to_string()),
                include_diff: true,
                auto_stash: true,
            }),
        )
        .await
        .expect("review should succeed");

        assert_eq!(response.0, StatusCode::CREATED);
        assert_eq!(response.1.stash.status, ReviewStashStatus::Failed);
        assert!(
            response
                .1
                .stash
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("stash failed")
        );
    }
}
