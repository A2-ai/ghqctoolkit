//! Comment, approve, unapprove, and review endpoints.

use std::path::PathBuf;
use std::str::FromStr;

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    ApprovalResponse, ApproveQuery, ApproveRequest, CommentResponse, CreateCommentRequest,
    ImpactNode, ImpactedIssues, Issue, ReviewRequest, UnapprovalResponse, UnapproveRequest,
};
use crate::{GitHubReader, GitHubWriter, QCComment};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use gix::ObjectId;

/// POST /api/issues/{number}/comment
pub async fn create_comment(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    Json(request): Json<CreateCommentRequest>,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let parse_str_commit = |commit: String| -> Result<ObjectId, ApiError> {
        commit
            .parse()
            .map_err(|e: gix::hash::decode::Error| ApiError::BadRequest(e.to_string()))
    };

    let previous_commit = match request.previous_commit {
        Some(c) => Some(parse_str_commit(c)?),
        None => None,
    };
    let current_commit = parse_str_commit(request.current_commit)?;

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

    // TODO: Implement comment creation
    // This will involve:
    // 1. Fetching the issue
    // 2. Creating QCComment with commit range
    // 3. Posting comment via GitHubWriter
    // 4. Updating cache

    Ok((
        StatusCode::from_u16(200).unwrap(),
        Json(CommentResponse { comment_url }),
    ))
}

/// POST /api/issues/{number}/approve
pub async fn approve_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    Query(query): Query<ApproveQuery>,
    Json(request): Json<ApproveRequest>,
) -> Result<Json<ApprovalResponse>, ApiError> {
    // TODO: Implement approval
    // This will involve:
    // 1. Checking blocking QCs (unless force=true)
    // 2. Creating QCApprove
    // 3. Posting approval comment and closing issue
    // 4. Updating cache
    todo!("Implement approve_issue")
}

/// POST /api/issues/{number}/unapprove
pub async fn unapprove_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    Json(request): Json<UnapproveRequest>,
) -> Result<Json<UnapprovalResponse>, ApiError> {
    // TODO: Implement unapproval
    // This will involve:
    // 1. Creating QCUnapprove
    // 2. Posting unapproval comment and reopening issue
    // 3. Determining impacted issues
    // 4. Updating cache
    todo!("Implement unapprove_issue")
}

/// POST /api/issues/{number}/review
pub async fn review_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    Json(request): Json<ReviewRequest>,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    // TODO: Implement review
    // This will involve:
    // 1. Fetching the issue
    // 2. Creating QCReview with working directory diff
    // 3. Posting review comment via GitHubWriter
    // 4. Updating cache
    todo!("Implement review_issue")
}
