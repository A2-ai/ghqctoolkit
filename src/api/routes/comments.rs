//! Comment, approve, unapprove, and review endpoints.

use std::path::PathBuf;

use crate::api::cache::{UpdateAction, update_cache_after_comment, update_cache_after_unapproval};
use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues};
use crate::api::routes::issues::determine_blocking_qc_status;
use crate::api::state::AppState;
use crate::api::types::{
    ApprovalResponse, ApproveQuery, ApproveRequest, BlockingQCError, BlockingQCItemWithStatus,
    BlockingQCStatus, CommentResponse, CreateCommentRequest, ReviewRequest, UnapprovalResponse,
    UnapproveRequest,
};
use crate::{GitProvider, QCApprove, QCComment, QCReview, QCUnapprove, parse_blocking_qcs};
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
    update_cache_after_comment(
        &state,
        &comment.issue,
        &current_commit.to_string(),
        UpdateAction::Notification,
    )
    .await;

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

    // Update cache immediately after posting comment, before closing issue
    // This ensures cache reflects the comment even if close_issue fails
    update_cache_after_comment(
        &state,
        &approval.issue,
        &commit.to_string(),
        UpdateAction::Approve,
    )
    .await;

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
    update_cache_after_unapproval(&state, &unapprove.issue).await;

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
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    let commit = parse_str_as_commit(&request.commit)?;

    let issue = state.git_info().get_issue(number).await?;

    let review = QCReview {
        file: PathBuf::from(&issue.title),
        issue,
        commit,
        note: request.note,
        no_diff: !request.include_diff,
        working_dir: state.git_info().path().to_path_buf(),
    };

    let comment_url = state.git_info().post_comment(&review).await?;

    update_cache_after_comment(
        &state,
        &review.issue,
        &commit.to_string(),
        UpdateAction::Review,
    )
    .await;

    Ok((StatusCode::CREATED, Json(CommentResponse { comment_url })))
}

fn parse_str_as_commit(commit: &str) -> Result<ObjectId, ApiError> {
    commit
        .parse()
        .map_err(|e: gix::hash::decode::Error| ApiError::BadRequest(e.to_string()))
}

pub(crate) async fn get_blocking_qc_status_with_cache<G: GitProvider>(
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

    let mut fetched_issues = {
        let cache_read = state.status_cache.read().await;
        FetchedIssues::fetch_issues(blocking_qcs, git_info, &cache_read).await
    };

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

    determine_blocking_qc_status(&mut status, blocking_qcs, &fetched_issues);

    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::api::cache::{CacheEntry, CacheKey};
    use crate::api::state::AppState;
    use crate::api::tests::helpers::{MockGitInfo, load_test_issue};
    use crate::api::types::{ChecklistSummary, Issue, QCStatus, QCStatusEnum};

    #[tokio::test]
    async fn test_get_blocking_qc_status_empty() {
        let mock = MockGitInfo::builder().build();
        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        let status = get_blocking_qc_status_with_cache(&[], &state).await;

        assert_eq!(status.total, 0);
        assert_eq!(status.approved_count, 0);
        assert_eq!(status.summary, "No blocking QCs");
        assert!(status.approved.is_empty());
        assert!(status.not_approved.is_empty());
        assert!(status.errors.is_empty());
    }

    #[tokio::test]
    async fn test_get_blocking_qc_status_all_approved() {
        let test_issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue.clone())
            .with_branch("main")
            .with_commit("abc123")
            .build();

        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        // Pre-populate cache with approved status
        let key = CacheKey {
            issue_updated_at: test_issue.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: test_issue.title.clone(),
                state: "closed".to_string(),
                html_url: test_issue.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue.created_at,
                updated_at: test_issue.updated_at,
                closed_at: Some(test_issue.updated_at),
                created_by: "Author".to_string(),
                branch: Some("main".to_string()),
                checklist_name: Some("checklist".to_string()),
                relevant_files: Vec::new(),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::Approved,
                status_detail: "Approved".to_string(),
                approved_commit: Some("abc123".to_string()),
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 1,
                total: 1,
                percentage: 1.0,
            },
            blocking_qc_numbers: vec![],
        };
        state.status_cache.write().await.insert(1, key, entry);

        let status = get_blocking_qc_status_with_cache(&[1], &state).await;

        assert_eq!(status.total, 1);
        assert_eq!(status.approved_count, 1);
        assert_eq!(status.summary, "All blocking QCs approved");
        assert_eq!(status.approved.len(), 1);
        assert_eq!(status.approved[0].issue_number, 1);
        assert!(status.not_approved.is_empty());
        assert!(status.errors.is_empty());
    }

    #[tokio::test]
    async fn test_get_blocking_qc_status_changes_after_approval_counts_as_approved() {
        let test_issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue.clone())
            .with_branch("main")
            .with_commit("abc123")
            .build();

        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        // Pre-populate cache with ChangesAfterApproval status
        let key = CacheKey {
            issue_updated_at: test_issue.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: test_issue.title.clone(),
                state: "open".to_string(),
                html_url: test_issue.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue.created_at,
                updated_at: test_issue.updated_at,
                closed_at: None,
                created_by: "Author".to_string(),
                branch: Some("main".to_string()),
                checklist_name: Some("checklist".to_string()),
                relevant_files: Vec::new(),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::ChangesAfterApproval,
                status_detail: "Approved; subsequent file changes".to_string(),
                approved_commit: Some("abc123".to_string()),
                initial_commit: "abc123".to_string(),
                latest_commit: "def456".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 1,
                total: 1,
                percentage: 1.0,
            },
            blocking_qc_numbers: vec![],
        };
        state.status_cache.write().await.insert(1, key, entry);

        let status = get_blocking_qc_status_with_cache(&[1], &state).await;

        assert_eq!(status.total, 1);
        assert_eq!(status.approved_count, 1);
        assert_eq!(status.summary, "All blocking QCs approved");
        assert_eq!(status.approved.len(), 1);
    }

    #[tokio::test]
    async fn test_get_blocking_qc_status_not_approved() {
        let test_issue = load_test_issue("test_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue.clone())
            .with_branch("main")
            .with_commit("abc123")
            .build();

        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        // Pre-populate cache with InProgress status
        let key = CacheKey {
            issue_updated_at: test_issue.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry = CacheEntry {
            issue: Issue {
                number: 1,
                title: test_issue.title.clone(),
                state: "open".to_string(),
                html_url: test_issue.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue.created_at,
                updated_at: test_issue.updated_at,
                closed_at: None,
                created_by: "Author".to_string(),
                branch: Some("main".to_string()),
                checklist_name: Some("checklist".to_string()),
                relevant_files: Vec::new(),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::InProgress,
                status_detail: "In Progress".to_string(),
                approved_commit: None,
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 1,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };
        state.status_cache.write().await.insert(1, key, entry);

        let status = get_blocking_qc_status_with_cache(&[1], &state).await;

        assert_eq!(status.total, 1);
        assert_eq!(status.approved_count, 0);
        assert_eq!(status.summary, "0/1 blocking QCs are approved");
        assert!(status.approved.is_empty());
        assert_eq!(status.not_approved.len(), 1);
        assert_eq!(status.not_approved[0].issue_number, 1);
        assert_eq!(status.not_approved[0].status, "In Progress");
        assert!(status.errors.is_empty());
    }

    #[tokio::test]
    async fn test_get_blocking_qc_status_mixed() {
        let test_issue1 = load_test_issue("test_file_issue");
        let test_issue2 = load_test_issue("config_file_issue");
        let mock = MockGitInfo::builder()
            .with_issue(1, test_issue1.clone())
            .with_issue(2, test_issue2.clone())
            .with_branch("main")
            .with_commit("abc123")
            .build();

        let config = Configuration::default();
        let state = AppState::new(mock, config, None, None);

        // Pre-populate cache with mixed statuses
        let key1 = CacheKey {
            issue_updated_at: test_issue1.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry1 = CacheEntry {
            issue: Issue {
                number: 1,
                title: test_issue1.title.clone(),
                state: "closed".to_string(),
                html_url: test_issue1.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue1.created_at,
                updated_at: test_issue1.updated_at,
                closed_at: Some(test_issue1.updated_at),
                created_by: "Author".to_string(),
                branch: Some("main".to_string()),
                checklist_name: Some("checklist".to_string()),
                relevant_files: Vec::new(),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::Approved,
                status_detail: "Approved".to_string(),
                approved_commit: Some("abc123".to_string()),
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 1,
                total: 1,
                percentage: 1.0,
            },
            blocking_qc_numbers: vec![],
        };

        let key2 = CacheKey {
            issue_updated_at: test_issue2.updated_at,
            branch: "main".to_string(),
            head_commit: "abc123".to_string(),
        };
        let entry2 = CacheEntry {
            issue: Issue {
                number: 2,
                title: test_issue2.title.clone(),
                state: "open".to_string(),
                html_url: test_issue2.html_url.to_string(),
                assignees: vec![],
                labels: vec![],
                milestone: None,
                created_at: test_issue2.created_at,
                updated_at: test_issue2.updated_at,
                closed_at: None,
                created_by: "Author".to_string(),
                branch: Some("main".to_string()),
                checklist_name: Some("checklist".to_string()),
                relevant_files: Vec::new(),
            },
            qc_status: QCStatus {
                status: QCStatusEnum::AwaitingReview,
                status_detail: "Awaiting review".to_string(),
                approved_commit: None,
                initial_commit: "abc123".to_string(),
                latest_commit: "abc123".to_string(),
            },
            branch: "main".to_string(),
            commits: vec![],
            checklist_summary: ChecklistSummary {
                completed: 0,
                total: 1,
                percentage: 0.0,
            },
            blocking_qc_numbers: vec![],
        };

        {
            let mut cache = state.status_cache.write().await;
            cache.insert(1, key1, entry1);
            cache.insert(2, key2, entry2);
        }

        let status = get_blocking_qc_status_with_cache(&[1, 2], &state).await;

        assert_eq!(status.total, 2);
        assert_eq!(status.approved_count, 1);
        assert_eq!(status.summary, "1/2 blocking QCs are approved");
        assert_eq!(status.approved.len(), 1);
        assert_eq!(status.approved[0].issue_number, 1);
        assert_eq!(status.not_approved.len(), 1);
        assert_eq!(status.not_approved[0].issue_number, 2);
        assert!(status.errors.is_empty());
    }
}
