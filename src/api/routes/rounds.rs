//! QC round endpoints: starting a new round, and seeding its form.

use crate::api::error::ApiError;
use crate::api::state::AppState;
use crate::api::types::{
    RepairRoundApiRequest, RepairRoundResponse, RoundSeedResponse, StartRoundApiRequest,
    StartRoundResponse,
};
use crate::{
    GitProvider, IssueThread, RepairRoundRequest, RoundState, StartRoundRequest,
    get_issue_comments, prior_round_comment_body, repair_round, seed_checklist, start_round,
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use octocrab::models::issues::Issue;

/// Fetch an issue and fold its comment thread into an [`IssueThread`].
async fn issue_thread<G: GitProvider + 'static>(
    state: &AppState<G>,
    number: u64,
) -> Result<(Issue, IssueThread), ApiError> {
    let issue = state.git_info().get_issue(number).await?;
    let comments = get_issue_comments(&issue, state.disk_cache(), state.git_info()).await?;
    let thread =
        IssueThread::from_issue_comments(&issue, &comments, state.git_info(), state.disk_cache())?;
    Ok((issue, thread))
}

/// POST /api/issues/{number}/rounds
///
/// Thin wrapper over [`start_round`]. Only a failure that wrote nothing is an
/// error response; a round that was recorded but whose follow-up steps failed is
/// a 201 whose per-step outcomes say so.
pub async fn start_new_round<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<StartRoundApiRequest>,
) -> Result<(StatusCode, Json<StartRoundResponse>), ApiError> {
    let (issue, thread) = issue_thread(&state, number).await?;

    let round_request = StartRoundRequest {
        issue,
        checklist_content: request.checklist_content,
        checklist_name: request.checklist_name,
        note: request.note,
        notification: request.notification.into(),
    };

    let result = start_round(&round_request, &thread, state.git_info()).await?;

    Ok((StatusCode::CREATED, Json(StartRoundResponse::from(&result))))
}

/// POST /api/issues/{number}/rounds/repair
///
/// Thin wrapper over [`repair_round`]: re-runs only the follow-up steps the issue's
/// currently open round is actually missing. A repair that could not be attempted
/// at all (no round open, or the open round being Initial QC) is a 409; a repair
/// whose steps failed is a 200 whose per-step outcomes say so, exactly as starting
/// a round reports its steps.
pub async fn repair_open_round<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<RepairRoundApiRequest>,
) -> Result<(StatusCode, Json<RepairRoundResponse>), ApiError> {
    let (issue, thread) = issue_thread(&state, number).await?;

    let repair_request = RepairRoundRequest {
        issue,
        notification: request.notification.into(),
    };

    let result = repair_round(&repair_request, &thread, state.git_info()).await?;

    Ok((StatusCode::OK, Json(RepairRoundResponse::from(&result))))
}

/// GET /api/issues/{number}/rounds/seed
///
/// Side-effect free: everything a start-new-round form needs, including whether
/// the action is currently legal.
pub async fn get_round_seed<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<RoundSeedResponse>, ApiError> {
    let issue = state.git_info().get_issue(number).await?;
    let comments = get_issue_comments(&issue, state.disk_cache(), state.git_info()).await?;
    let thread =
        IssueThread::from_issue_comments(&issue, &comments, state.git_info(), state.disk_cache())?;

    let last = thread.rounds.last().ok_or_else(|| {
        ApiError::Internal(
            "No QC rounds could be derived for this issue (its commit history may be unreachable)"
                .to_string(),
        )
    })?;

    // A still-open round is the one thing that makes the action illegal; report it
    // rather than failing, since this endpoint is what the UI asks *before* acting.
    let (previous_approval, blocked_reason) = match &last.state {
        RoundState::Closed { commit, .. } => (Some(commit.to_string()), None),
        RoundState::Open => (
            None,
            Some(format!(
                "{} is still open. Approve it first — a `# QC New Round` comment posted now would \
                 extend that round instead of opening a new one.",
                last.name()
            )),
        ),
    };

    // The anchor is HEAD of the issue's branch, exactly as `start_round` reads it.
    let anchor = state
        .git_info()
        .branch_tip(&Some(thread.branch.clone()))
        .inspect_err(|error| {
            log::debug!(
                "could not resolve HEAD of branch '{}' for issue #{number}: {error}",
                thread.branch
            );
        })
        .ok()
        .map(|anchor| anchor.to_string());

    let seeded = seed_checklist(
        prior_round_comment_body(&thread, &comments),
        issue.body.as_deref(),
    );

    let next_round = last.index.saturating_add(1);
    let blocked_reason = blocked_reason.or_else(|| {
        anchor.is_none().then(|| {
            format!(
                "Could not resolve HEAD of branch '{}' — fetch or check out the branch locally.",
                thread.branch
            )
        })
    });

    Ok(Json(RoundSeedResponse {
        next_round,
        next_round_name: format!("Round {next_round}"),
        checklist_content: seeded.as_ref().map(|seed| seed.content.clone()),
        checklist_name: seeded.and_then(|seed| seed.name),
        anchor,
        previous_approval,
        can_start: blocked_reason.is_none(),
        blocked_reason,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::GitComment;
    use crate::api::tests::helpers::MockGitInfo;
    use crate::api::types::RepairRoundApiRequest;
    use crate::api::types::{
        NotificationModeRequest, RoundStateEnum, StepOutcomeResponse, StepStatusEnum,
    };
    use crate::test_utils::create_test_issue;

    const COMMIT: &str = "456def789abc012345678901234567890123cdef";
    const HEAD: &str = "abc1234567890abcdef1234567890abcdef12340";

    fn issue_body() -> String {
        format!(
            "Quality check issue for src/test.rs\n\n## Metadata\ninitial qc commit: {COMMIT}\ngit branch: main\nauthor: The Octocat <octocat@example.com>\n\n# Code Review Checklist\n- [x] Reviewed the logic\n"
        )
    }

    fn approval_comment() -> GitComment {
        GitComment {
            body: format!("# QC Approval\n\n## Metadata\napproved qc commit: {COMMIT}\n"),
            author_login: "reviewer".to_string(),
            created_at: chrono::Utc::now(),
            id: Some(7),
            html_url: Some("https://github.com/o/r/issues/1#issuecomment-7".to_string()),
            html: None,
        }
    }

    fn state(mock: MockGitInfo) -> AppState<MockGitInfo> {
        AppState::new(mock, Configuration::default(), None, None)
    }

    fn request() -> StartRoundApiRequest {
        StartRoundApiRequest {
            checklist_content: "- [ ] Reviewed the logic".to_string(),
            checklist_name: Some("Code Review Checklist".to_string()),
            note: None,
            notification: NotificationModeRequest::None,
        }
    }

    /// A round that was recorded but whose step 2 failed is a success response with
    /// the failure reported per step — never a 500.
    #[tokio::test]
    async fn a_failed_step_still_yields_created_with_the_failure_in_the_body() {
        let issue = create_test_issue("o", "r", 1, "src/test.rs", &issue_body(), Some(1), "closed");
        let mock = MockGitInfo::builder()
            .with_issue(1, issue)
            .with_comments(1, vec![approval_comment()])
            .with_branch_tip(Some(HEAD.to_string()))
            .with_open_issue_failure()
            .build();

        let (status, Json(body)) = start_new_round(State(state(mock)), Path(1), Json(request()))
            .await
            .expect("a step-2 failure is not an error response");

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body.round, 2);
        assert_eq!(body.anchor, HEAD);
        assert_eq!(body.reopened.status, StepStatusEnum::Failed);
        assert!(
            body.reopened.error.is_some(),
            "failure must carry a message"
        );
        assert_eq!(body.body_marker.status, StepStatusEnum::Done);
        assert_eq!(body.notification.status, StepStatusEnum::Skipped);
        assert!(body.needs_repair);
    }

    /// Every `StepOutcome` variant is distinguishable to a client.
    #[test]
    fn serialized_step_outcomes_distinguish_done_skipped_and_failed() {
        let json = |outcome: crate::StepOutcome| {
            serde_json::to_value(StepOutcomeResponse::from(&outcome)).unwrap()
        };
        assert_eq!(
            json(crate::StepOutcome::Done),
            serde_json::json!({"status": "done"})
        );
        assert_eq!(
            json(crate::StepOutcome::Skipped),
            serde_json::json!({"status": "skipped"})
        );
        assert_eq!(
            json(crate::StepOutcome::Failed("boom".to_string())),
            serde_json::json!({"status": "failed", "error": "boom"})
        );
    }

    /// A legacy issue with no round comments still exposes exactly one round.
    #[tokio::test]
    async fn a_legacy_issue_exposes_one_round_named_initial_qc() {
        use crate::api::routes::issues::{IssueStatusQuery, batch_get_issue_status};
        use axum::extract::Query;

        let issue = create_test_issue("o", "r", 1, "src/test.rs", &issue_body(), Some(1), "open");
        let mock = MockGitInfo::builder().with_issue(1, issue).build();

        let (status, Json(body)) = batch_get_issue_status(
            State(state(mock)),
            Query(IssueStatusQuery {
                issues: "1".to_string(),
            }),
        )
        .await
        .expect("status must be derivable");

        assert_eq!(status, StatusCode::OK);
        let response = &body.results[0];
        assert_eq!(response.rounds.len(), 1);
        let round = &response.rounds[0];
        assert_eq!(round.index, 1);
        assert_eq!(round.name, "Initial QC");
        assert_eq!(round.state, RoundStateEnum::Open);
        assert_eq!(round.opened_at, COMMIT);
        assert_eq!(round.previous_approval, None);
        assert_eq!(round.closing_commit, None);
        assert_eq!(round.event_count, 0);
        assert_eq!(round.retraction_count, 0);
        assert_eq!(round.extension_count, 0);
        assert_eq!(
            round.checklist_source.kind,
            crate::api::types::ChecklistSourceKind::IssueBody
        );
        assert_eq!(
            round.checklist_name.as_deref(),
            Some("Code Review Checklist")
        );
        // The one round is open, and with nothing notified yet the comparison base
        // is the initial commit.
        assert_eq!(response.open_round_index, Some(1));
        assert_eq!(response.next_notification_from, COMMIT);
    }

    // ── Repairing an open round ──────────────────────────────────────────────

    /// The body a round-2 issue has once its `## QC Round` marker is correct.
    fn round_two_body(round: u32) -> String {
        format!(
            "Quality check issue for src/test.rs\n\n## Metadata\ninitial qc commit: {COMMIT}\ngit branch: main\nauthor: The Octocat <octocat@example.com>\n\n## QC Round\n* current round: {round}\n* round comment: {ROUND_COMMENT_URL}\n\n# Code Review Checklist\n- [ ] Reviewed the logic\n"
        )
    }

    const ROUND_COMMENT_URL: &str = "https://github.com/o/r/issues/1#issuecomment-456";

    fn new_round_comment() -> GitComment {
        GitComment {
            body: format!(
                "# QC New Round\n\n## Metadata\n* round: 2\n* round commit: {HEAD}\n* previous approved commit: {COMMIT}\n* checklist: Code Review Checklist\n"
            ),
            author_login: "author".to_string(),
            created_at: chrono::Utc::now(),
            id: Some(456),
            html_url: Some(ROUND_COMMENT_URL.to_string()),
            html: None,
        }
    }

    fn notification_comment() -> GitComment {
        GitComment {
            body: format!("# QC Notification\n\n## Metadata\ncurrent qc commit: {HEAD}\n"),
            author_login: "author".to_string(),
            created_at: chrono::Utc::now(),
            id: Some(457),
            html_url: None,
            html: None,
        }
    }

    /// A repair step that failed is still a 200, exactly as a start-round step is
    /// still a 201: the action is best-effort and every step is retryable.
    #[tokio::test]
    async fn a_failed_repair_step_still_yields_ok_with_the_failure_in_the_body() {
        let issue = create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            &round_two_body(2),
            Some(1),
            "closed",
        );
        let mock = MockGitInfo::builder()
            .with_issue(1, issue)
            .with_comments(
                1,
                vec![
                    approval_comment(),
                    new_round_comment(),
                    notification_comment(),
                ],
            )
            .with_branch_tip(Some(HEAD.to_string()))
            .with_open_issue_failure()
            .build();

        let (status, Json(body)) = repair_open_round(
            State(state(mock)),
            Path(1),
            Json(RepairRoundApiRequest {
                notification: NotificationModeRequest::None,
            }),
        )
        .await
        .expect("a failed step is not an error response");

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.round, 2);
        assert_eq!(body.round_name, "Round 2");
        assert_eq!(body.round_comment_url.as_deref(), Some(ROUND_COMMENT_URL));
        assert_eq!(body.reopened.status, StepStatusEnum::Failed);
        assert!(
            body.reopened.error.is_some(),
            "failure must carry a message"
        );
        // The marker is already correct and the round was notified: nothing else ran.
        assert_eq!(body.body_marker.status, StepStatusEnum::Skipped);
        assert_eq!(body.notification.status, StepStatusEnum::Skipped);
        assert!(!body.repaired);
        assert!(body.needs_repair);
    }

    /// Nothing to repair is a 409 conflict, mapped exactly as `RoundStillOpen` is.
    #[tokio::test]
    async fn nothing_to_repair_is_a_conflict_and_writes_nothing() {
        let issue = create_test_issue("o", "r", 1, "src/test.rs", &issue_body(), Some(1), "closed");
        let mock = MockGitInfo::builder()
            .with_issue(1, issue)
            .with_comments(1, vec![approval_comment()])
            .with_branch_tip(Some(HEAD.to_string()))
            .build();

        let error = repair_open_round(
            State(state(mock.clone())),
            Path(1),
            Json(RepairRoundApiRequest {
                notification: NotificationModeRequest::Full,
            }),
        )
        .await
        .expect_err("a closed round has nothing to repair");

        assert!(
            matches!(error, ApiError::Conflict(ref message) if message.contains("Nothing to repair")),
            "unexpected error: {error:?}"
        );
        assert!(
            mock.write_calls().is_empty(),
            "a rejected repair must not write"
        );
    }

    /// The status surface is where a client detects a broken round, so the flags
    /// must be there without asking for a repair.
    #[tokio::test]
    async fn the_status_response_reports_which_follow_up_steps_are_incomplete() {
        use crate::api::routes::issues::{IssueStatusQuery, batch_get_issue_status};
        use axum::extract::Query;

        // Closed issue, open round 2, marker still naming round 1, no notification.
        let issue = create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            &round_two_body(1),
            Some(1),
            "closed",
        );
        let mock = MockGitInfo::builder()
            .with_issue(1, issue)
            .with_comments(1, vec![approval_comment(), new_round_comment()])
            .with_branch_tip(Some(HEAD.to_string()))
            .build();

        let (_, Json(body)) = batch_get_issue_status(
            State(state(mock)),
            Query(IssueStatusQuery {
                issues: "1".to_string(),
            }),
        )
        .await
        .expect("status must be derivable");

        let repair = body.results[0]
            .round_repair
            .as_ref()
            .expect("an open round 2 carries repair flags");
        assert_eq!(repair.round, 2);
        assert_eq!(repair.round_name, "Round 2");
        assert!(repair.reopen);
        assert!(repair.body_marker);
        assert!(repair.notification_missing);
        assert!(repair.needs_repair);
    }

    /// A round with nothing wrong still reports flags, but never asks for a repair —
    /// in particular a round that was deliberately opened without notifying.
    #[tokio::test]
    async fn a_healthy_open_round_never_asks_for_a_repair() {
        use crate::api::routes::issues::{IssueStatusQuery, batch_get_issue_status};
        use axum::extract::Query;

        let issue = create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            &round_two_body(2),
            Some(1),
            "open",
        );
        let mock = MockGitInfo::builder()
            .with_issue(1, issue)
            .with_comments(1, vec![approval_comment(), new_round_comment()])
            .with_branch_tip(Some(HEAD.to_string()))
            .build();

        let (_, Json(body)) = batch_get_issue_status(
            State(state(mock)),
            Query(IssueStatusQuery {
                issues: "1".to_string(),
            }),
        )
        .await
        .expect("status must be derivable");

        let repair = body.results[0]
            .round_repair
            .as_ref()
            .expect("an open round 2 carries repair flags");
        assert!(!repair.reopen);
        assert!(!repair.body_marker);
        // Not notified, and that is deliberately not a defect.
        assert!(repair.notification_missing);
        assert!(!repair.needs_repair);
    }

    /// A legacy issue whose only round is Initial QC has no follow-up steps at all,
    /// so it must never grow a repair affordance — however it is closed.
    #[tokio::test]
    async fn an_initial_qc_only_issue_reports_no_repair_status() {
        use crate::api::routes::issues::{IssueStatusQuery, batch_get_issue_status};
        use axum::extract::Query;

        let issue = create_test_issue("o", "r", 1, "src/test.rs", &issue_body(), Some(1), "closed");
        let mock = MockGitInfo::builder().with_issue(1, issue).build();

        let (_, Json(body)) = batch_get_issue_status(
            State(state(mock)),
            Query(IssueStatusQuery {
                issues: "1".to_string(),
            }),
        )
        .await
        .expect("status must be derivable");

        assert_eq!(body.results[0].open_round_index, Some(1));
        assert!(body.results[0].round_repair.is_none());
    }

    /// The seed endpoint must not write, and must report the base it would use.
    #[tokio::test]
    async fn the_seed_endpoint_is_side_effect_free() {
        let issue = create_test_issue("o", "r", 1, "src/test.rs", &issue_body(), Some(1), "closed");
        let mock = MockGitInfo::builder()
            .with_issue(1, issue)
            .with_comments(1, vec![approval_comment()])
            .with_branch_tip(Some(HEAD.to_string()))
            .build();

        let Json(seed) = get_round_seed(State(state(mock.clone())), Path(1))
            .await
            .expect("seed must succeed for a closed round");

        assert!(seed.can_start);
        assert_eq!(seed.next_round, 2);
        assert_eq!(seed.anchor.as_deref(), Some(HEAD));
        assert_eq!(seed.previous_approval.as_deref(), Some(COMMIT));
        // Boxes reset, `# <name>` heading dropped and returned as the name.
        assert_eq!(
            seed.checklist_content.as_deref(),
            Some("- [ ] Reviewed the logic")
        );
        assert_eq!(
            seed.checklist_name.as_deref(),
            Some("Code Review Checklist")
        );
        assert!(mock.write_calls().is_empty(), "the seed must not write");
    }
}
