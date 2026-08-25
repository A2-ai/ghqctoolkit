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

    // D8/§22.0.4: an `Approved` round owns `[start_n ..= approval_n]`, so an approval
    // outside the latest round's commits gives that round a negative span. The fold then
    // lands in D54's state — `latest_commit()` is `None`, `archive_commit` is null, and
    // status refuses with a "fetch the branch" remedy that is *wrong*, because the branch
    // is local and fine.
    //
    // The CLI has always enforced this (`QCApprove::from_args` resolves the commit against
    // `latest_round().commits`); this makes the API agree rather than inventing a rule.
    // Deliberately **not** overridable by `force`, which exists to bypass blocking-QC
    // policy: this is a model invariant, and no caller has standing to waive it.
    let comments = crate::get_issue_comments(&issue, state.disk_cache(), state.git_info()).await?;
    let thread = crate::issue::IssueThread::from_issue_comments(
        &issue,
        &comments,
        state.git_info(),
        state.disk_cache(),
    )?;
    let latest = thread.latest_round();
    if !latest.commits.iter().any(|owned| owned.hash == commit) {
        // D55: when the round could not be placed there is nothing to check against, and
        // the branch to fetch is the actionable half of the message.
        return Err(ApiError::Conflict(
            if let crate::issue::RoundPlacement::Unplaceable { branch } = &latest.placement {
                format!(
                    "Round {} of issue #{number} could not be placed on '{branch}', so no \
                     approval can be verified against it; fetch '{branch}' first",
                    latest.index
                )
            } else {
                format!(
                    "Commit {commit} is not one of round {}'s commits, and an approval \
                     belongs to the round it closes",
                    latest.index
                )
            },
        ));
    }

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
    fetched_issues.errors.extend(created_threads.thread_errors);

    let titles: std::collections::HashMap<u64, String> = fetched_issues
        .issues
        .iter()
        .map(|i| (i.number, i.title.clone()))
        .collect();

    determine_blocking_qc_status(
        &mut status,
        blocking_qcs,
        &created_threads.responses,
        &fetched_issues.errors,
        &titles,
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

    // ── D8/§22.0.4: the approval must lie inside the round that will own it ──────

    /// The commit `MockGitInfo`'s walk returns — the only one a mocked thread can anchor
    /// on, and therefore the only legal approval for that thread.
    const IN_ROUND_COMMIT: &str = "456def789abc012345678901234567890123cdef";
    /// A well-formed commit that is *not* in the round's walk.
    const OUT_OF_ROUND_COMMIT: &str = "1111111111111111111111111111111111111111";

    fn approve_state(branch: &str) -> AppState<MockGitInfo> {
        let issue = crate::test_utils::create_test_issue(
            "test-owner",
            "test-repo",
            1,
            "src/test.rs",
            &format!(
                "## Metadata\n* initial qc commit: {IN_ROUND_COMMIT}\n* git branch: {branch}\n"
            ),
            Some(1),
            "open",
        );
        let mock = MockGitInfo::builder()
            .with_issue(issue.number, issue)
            .with_comments(1, vec![])
            .with_commit(IN_ROUND_COMMIT)
            .with_branch("main")
            .build();
        AppState::new(mock, Configuration::default(), None, None)
    }

    async fn approve(
        state: AppState<MockGitInfo>,
        commit: &str,
        force: bool,
    ) -> Result<(StatusCode, Json<ApprovalResponse>), ApiError> {
        approve_issue(
            State(state),
            Path(1),
            Query(ApproveQuery { force }),
            Json(ApproveRequest {
                commit: commit.to_string(),
                note: None,
            }),
        )
        .await
    }

    /// The legitimate case still works — a guard that refused real approvals would be
    /// worse than the hole it closes.
    #[tokio::test]
    async fn test_approving_a_commit_the_round_owns_succeeds() {
        let state = approve_state("main");

        let (status, _) = approve(state, IN_ROUND_COMMIT, false)
            .await
            .expect("a commit inside the round is approvable");

        assert_eq!(status, StatusCode::CREATED);
    }

    /// D8: an `Approved` round owns `[start_n ..= approval_n]`, so this would give the
    /// round a negative span — and it is what the pre-§22 Approve tab could post.
    #[tokio::test]
    async fn test_approving_a_commit_outside_the_round_is_refused() {
        let state = approve_state("main");

        let error = approve(state.clone(), OUT_OF_ROUND_COMMIT, false)
            .await
            .expect_err("an approval outside the round it closes is refused");

        assert!(
            matches!(&error, ApiError::Conflict(message)
                if message.contains("not one of round 1's commits")),
            "unexpected error: {error:?}"
        );

        // The refusal is total: no approval comment, and the issue stays open. A posted
        // comment is an audit record, so a partial write here would be the lie itself.
        assert!(
            state.git_info().write_calls().is_empty(),
            "a refused approval must write nothing: {:?}",
            state.git_info().write_calls()
        );
    }

    /// `force` exists to bypass blocking-QC *policy*. This is a model invariant, so no
    /// caller has standing to waive it.
    #[tokio::test]
    async fn test_force_does_not_waive_the_round_span_rule() {
        let state = approve_state("main");

        let error = approve(state, OUT_OF_ROUND_COMMIT, true)
            .await
            .expect_err("force does not reach this rule");

        assert!(
            matches!(&error, ApiError::Conflict(message)
                if message.contains("not one of round 1's commits")),
            "unexpected error: {error:?}"
        );
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
