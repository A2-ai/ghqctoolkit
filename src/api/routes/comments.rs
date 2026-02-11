//! Comment, approve, unapprove, and review endpoints.

use std::path::PathBuf;

use crate::api::cache::{UpdateAction, update_cache_after_comment, update_cache_after_unapproval};
use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues};
use crate::api::state::AppState;
use crate::api::types::{
    ApprovalResponse, ApproveQuery, ApproveRequest, BlockingQCError, BlockingQCItem,
    BlockingQCItemWithStatus, BlockingQCStatus, CommentResponse, CreateCommentRequest,
    QCStatusEnum, ReviewRequest, UnapprovalResponse, UnapproveRequest,
};
use crate::{
    GitHubReader, GitHubWriter, QCApprove, QCComment, QCReview, QCUnapprove, parse_blocking_qcs,
};
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
    update_cache_after_comment(
        &state,
        &comment.issue,
        &current_commit.to_string(),
        UpdateAction::Notification,
    );

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

    let blocking_status = get_blocking_qc_status_with_cache(&blocking_qcs, &state).await;

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
        let json = serde_json::to_string_pretty(&conflict).unwrap_or_default();
        return Err(ApiError::Conflict(json));
    }

    let commit = parse_str_as_commit(&request.commit)?;

    let approval = QCApprove {
        file: PathBuf::from(&issue.title),
        commit,
        issue: issue.clone(),
        note: request.note,
    };

    let approval_url = state.git_info().post_comment(&approval).await?;
    state.git_info().close_issue(issue.number).await?;

    update_cache_after_comment(
        &state,
        &approval.issue,
        &commit.to_string(),
        UpdateAction::Approve,
    );

    Ok(Json(ApprovalResponse {
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
    }))
}

/// POST /api/issues/{number}/unapprove
pub async fn unapprove_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    Json(request): Json<UnapproveRequest>,
) -> Result<Json<UnapprovalResponse>, ApiError> {
    let issue = state.git_info().get_issue(number).await?;
    let unapprove = QCUnapprove {
        issue,
        reason: request.reason,
    };

    let unapproval_url = state.git_info().post_comment(&unapprove).await?;

    update_cache_after_unapproval(&state, &unapprove.issue);

    Ok(Json(UnapprovalResponse { unapproval_url }))
}

/// POST /api/issues/{number}/review
pub async fn review_issue(
    State(state): State<AppState>,
    Path(number): Path<u64>,
    Json(request): Json<ReviewRequest>,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let commit = parse_str_as_commit(&request.commit)?;

    let issue = state.git_info().get_issue(number).await?;

    let review = QCReview {
        file: PathBuf::from(&issue.title),
        issue: issue,
        commit,
        note: request.note,
        no_diff: !request.include_diff,
        working_dir: state.git_info().repository_path.clone(),
    };

    let comment_url = state.git_info().post_comment(&review).await?;

    update_cache_after_comment(
        &state,
        &review.issue,
        &commit.to_string(),
        UpdateAction::Review,
    );

    Ok((
        StatusCode::from_u16(200).unwrap(),
        Json(CommentResponse { comment_url }),
    ))
}

fn parse_str_as_commit(commit: &str) -> Result<ObjectId, ApiError> {
    commit
        .parse()
        .map_err(|e: gix::hash::decode::Error| ApiError::BadRequest(e.to_string()))
}

pub(crate) async fn get_blocking_qc_status_with_cache(
    blocking_qcs: &[u64],
    state: &AppState,
) -> BlockingQCStatus {
    let mut status = BlockingQCStatus::default();
    if blocking_qcs.is_empty() {
        status.summary = "No blocking QCs".to_string();
        return status;
    }
    status.total = blocking_qcs.len() as u32;

    let git_info = state.git_info();

    let mut fetched_issues =
        FetchedIssues::fetch_issues(blocking_qcs, git_info, &state.status_cache.blocking_read())
            .await;

    let created_threads = CreatedThreads::create_threads(&fetched_issues.issues, state).await;
    fetched_issues
        .cached_entries
        .extend(created_threads.entries);
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

    for blocking_qc in blocking_qcs {
        if let Some(entry) = fetched_issues.cached_entries.get(blocking_qc) {
            match entry.qc_status.status {
                QCStatusEnum::Approved | QCStatusEnum::ChangesAfterApproval => {
                    status.approved_count += 1;
                    status.approved.push(BlockingQCItem {
                        issue_number: *blocking_qc,
                        file_name: entry.issue.title.clone(),
                    });
                }
                _ => {
                    status.not_approved.push(BlockingQCItemWithStatus {
                        issue_number: *blocking_qc,
                        file_name: entry.issue.title.clone(),
                        status: entry.qc_status.status_detail.clone(),
                    });
                }
            }
        }
    }

    status.summary = if status.approved_count == status.total {
        "All blocking QCs approved".to_string()
    } else {
        format!(
            "{}/{} blocking QCs are approved",
            status.approved_count, status.total
        )
    };

    status
}
