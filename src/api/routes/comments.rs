//! Comment, approve, unapprove, and review endpoints.

use std::path::PathBuf;

use crate::api::cache::{CacheEntry, CacheKey, UpdateAction};
use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    ApprovalResponse, ApproveQuery, ApproveRequest, BlockingQCError, BlockingQCItem,
    BlockingQCItemWithStatus, BlockingQCStatus, CommentResponse, CreateCommentRequest, QCStatus,
    QCStatusEnum, ReviewRequest, UnapprovalResponse, UnapproveRequest,
};
use crate::{
    BlockingQC, GitHubReader, GitHubWriter, GitRepository, IssueThread, QCApprove, QCComment,
    QCReview, QCUnapprove, parse_blocking_qcs,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use gix::ObjectId;
use gix::hashtable::hash_map::HashMap;

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
    if let (Ok(head_commit), Ok(branch)) = (state.git_info().commit(), state.git_info().branch()) {
        let key = CacheKey {
            issue_updated_at: comment.issue.updated_at.clone(),
            branch,
            head_commit,
        };

        state.status_cache.blocking_write().update(
            key,
            &comment.issue,
            &current_commit.to_string(),
            UpdateAction::Notification,
        );
    } else {
        log::error!("Failed to determine HEAD commit and/or branch. Removing cache entry");
        state
            .status_cache
            .blocking_write()
            .remove(comment.issue.number);
    }

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
        .map(parse_blocking_qcs)
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

    if let (Ok(head_commit), Ok(branch)) = (state.git_info().commit(), state.git_info().branch()) {
        let key = CacheKey {
            issue_updated_at: approval.issue.updated_at.clone(),
            branch,
            head_commit,
        };

        state.status_cache.blocking_write().update(
            key,
            &approval.issue,
            &commit.to_string(),
            UpdateAction::Approve,
        );
    } else {
        log::error!("Failed to determine HEAD commit and/or branch. Removing cache entry");
        state
            .status_cache
            .blocking_write()
            .remove(approval.issue.number);
    }

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

    if let (Ok(head_commit), Ok(branch)) = (state.git_info().commit(), state.git_info().branch()) {
        let key = CacheKey {
            issue_updated_at: unapprove.issue.updated_at.clone(),
            branch,
            head_commit,
        };

        state
            .status_cache
            .blocking_write()
            .unapproval(key, &unapprove.issue);
    } else {
        log::error!("Failed to determine HEAD commit and/or branch. Removing cache entry");
        state
            .status_cache
            .blocking_write()
            .remove(unapprove.issue.number);
    }

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

    if let (Ok(head_commit), Ok(branch)) = (state.git_info().commit(), state.git_info().branch()) {
        let key = CacheKey {
            issue_updated_at: review.issue.updated_at.clone(),
            branch,
            head_commit,
        };

        state.status_cache.blocking_write().update(
            key,
            &review.issue,
            &commit.to_string(),
            UpdateAction::Review,
        );
    } else {
        log::error!("Failed to determine HEAD commit and/or branch. Removing cache entry");
        state
            .status_cache
            .blocking_write()
            .remove(review.issue.number);
    }

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
    blocking_qcs: &[BlockingQC],
    state: &AppState,
) -> BlockingQCStatus {
    let mut status = BlockingQCStatus::default();
    if blocking_qcs.is_empty() {
        status.summary = "No blocking QCs".to_string();
        return status;
    }
    status.total = blocking_qcs.len() as u32;

    let git_info = state.git_info();

    let branch = git_info.branch().unwrap_or("unknown".to_string());
    let commit = git_info.commit().unwrap_or("unknown".to_string());
    let cache_key = |updated_at: DateTime<Utc>| -> CacheKey {
        CacheKey {
            issue_updated_at: updated_at,
            branch: branch.to_string(),
            head_commit: commit.to_string(),
        }
    };

    let issue_futures = blocking_qcs
        .iter()
        .map(|b| async move { git_info.get_issue(b.issue_number).await })
        .collect::<Vec<_>>();
    let issue_results = futures::future::join_all(issue_futures).await;

    let mut known_status = HashMap::new();
    let mut status_to_fetch = Vec::new();

    for (result, blocking_qc) in issue_results.into_iter().zip(blocking_qcs) {
        match result {
            Ok(issue) => {
                let key = cache_key(issue.updated_at);
                if let Some(entry) = state.status_cache.blocking_read().get(issue.number, &key) {
                    known_status.insert(blocking_qc, entry.qc_status.clone());
                } else {
                    status_to_fetch.push(blocking_qc);
                }
            }
            Err(e) => {
                status.errors.push(BlockingQCError {
                    issue_number: blocking_qc.issue_number,
                    error: e.to_string(),
                });
            }
        }
    }

    let issue_thread_futures = status_to_fetch
        .iter()
        .map(|b| async move {
            let issue = git_info.get_issue(b.issue_number).await?;
            IssueThread::from_issue(&issue, state.disk_cache(), git_info)
                .await
                .map(|thread| (thread, issue))
        })
        .collect::<Vec<_>>();
    let issue_thread_results = futures::future::join_all(issue_thread_futures).await;

    for (result, blocking_qc) in issue_thread_results.into_iter().zip(status_to_fetch) {
        match result {
            Ok((issue_thread, issue)) => {
                let status = QCStatus::from(&issue_thread);
                known_status.insert(blocking_qc, status.clone());

                let key = cache_key(issue.updated_at);
                let entry = CacheEntry::new(&issue, &issue_thread);
                state
                    .status_cache
                    .blocking_write()
                    .insert(issue.number, key, entry);
            }
            Err(e) => {
                status.errors.push(BlockingQCError {
                    issue_number: blocking_qc.issue_number,
                    error: e.to_string(),
                });
            }
        }
    }
    for (blocking_qc, blocking_status) in known_status {
        match blocking_status.status {
            QCStatusEnum::Approved | QCStatusEnum::ChangesAfterApproval => {
                status.approved_count += 1;
                status.approved.push(BlockingQCItem {
                    issue_number: blocking_qc.issue_number,
                    file_name: blocking_qc.file_name.to_string_lossy().to_string(),
                });
            }
            _ => {
                status.not_approved.push(BlockingQCItemWithStatus {
                    issue_number: blocking_qc.issue_number,
                    file_name: blocking_qc.file_name.to_string_lossy().to_string(),
                    status: blocking_status.status_detail,
                });
            }
        };
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
