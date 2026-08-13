//! Repairing an open QC round: re-running the follow-up steps that
//! [`crate::start_round`] left incomplete.
//!
//! [`crate::start_round`] posts the `# QC New Round` comment first — that is what
//! creates the round — and then performs three recoverable steps: reopen the
//! issue, refresh the `## QC Round` body marker, and optionally notify. Those
//! three are reported per step rather than aborting the action, on the promise
//! that each one is independently retryable.
//!
//! This module is that retry. It cannot be the new-round action re-run, because
//! the round is now *open*: `start_round` refuses to run against an open round,
//! and a second `# QC New Round` comment would **extend** the round (see
//! [`crate::round`]'s extension semantics) rather than repair anything. So the
//! repair works on the currently open round and touches nothing else.
//!
//! What needs repairing is **derived**, never taken from the caller:
//!
//! * **reopen** — needed when the GitHub issue is closed while a round is open.
//! * **body marker** — needed when [`parse_round_marker`] finds no marker, or one
//!   that disagrees with the open round's index or round-comment URL.
//! * **notification** — the open round having no `Notification` event is a *fact*,
//!   not a defect: [`NotificationMode::None`] is a legitimate deliberate choice,
//!   so a notification is posted only when the caller explicitly asks for one.
//!   This is why [`RepairPlan::needs_repair`] ignores it.
//!
//! Every step is the same function `start_round` calls, so the two paths cannot
//! drift apart, and every step is idempotent — a repair that is run twice writes
//! the same body and reopens an already-open issue with a no-op PATCH.

use std::fmt;

use octocrab::models::{IssueState, issues::Issue};

use crate::git::GitHubWriter;
use crate::issue::IssueThread;
use crate::new_round::{RoundMarker, parse_round_marker};
use crate::round::{Round, RoundEvent, RoundOpen};
use crate::start_round::{
    NotificationMode, StepOutcome, body_marker_step, notification_step, reopen_step,
};

/// Which of the open round's follow-up steps are incomplete.
///
/// Purely derived from the issue and the round — computing it writes nothing and
/// makes no API calls, which is what lets the API report it alongside an issue's
/// status so a client can *detect* a broken round without attempting a repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RepairPlan {
    /// The issue is closed while its round is open.
    pub reopen: bool,
    /// The `## QC Round` body marker is missing or disagrees with the open round.
    pub body_marker: bool,
    /// The open round carries no `# QC Notification`. Informational: this is not a
    /// defect, and is repaired only on explicit request.
    pub notification_missing: bool,
}

impl RepairPlan {
    /// Whether something is actually *wrong* and worth offering a repair for.
    ///
    /// Deliberately excludes [`RepairPlan::notification_missing`]: a round opened
    /// with [`NotificationMode::None`] is in exactly the state its author chose.
    pub fn needs_repair(&self) -> bool {
        self.reopen || self.body_marker
    }
}

/// Derive what is incomplete for `round` (which the caller has established is the
/// issue's open round) from `issue`'s current state and body.
pub fn plan_repair(issue: &Issue, round: &Round) -> RepairPlan {
    RepairPlan {
        reopen: !matches!(issue.state, IssueState::Open),
        body_marker: expected_marker(round)
            .is_some_and(|expected| marker_is_stale(issue.body.as_deref(), &expected)),
        notification_missing: !round
            .events
            .iter()
            .any(|event| matches!(event, RoundEvent::Notification { .. })),
    }
}

/// The marker the open round's body *should* carry.
///
/// `None` when the round's identity URL is unknown — `RoundOpen::NewRound`'s
/// `comment_url` is optional because comments loaded from the disk cache carry no
/// identity, and for Initial QC, which no `# QC New Round` comment opened. In
/// neither case can a correct marker be derived, so the body is left alone rather
/// than rewritten with an invented URL: the marker is only a cache, and a wrong
/// one is worse than a stale one.
fn expected_marker(round: &Round) -> Option<RoundMarker> {
    match &round.opened {
        RoundOpen::NewRound { comment_url, .. } => comment_url.as_ref().map(|url| RoundMarker {
            round: round.index,
            comment_url: url.clone(),
        }),
        RoundOpen::IssueCreated => None,
    }
}

/// Whether `body`'s marker disagrees with `expected` (a missing marker counts).
fn marker_is_stale(body: Option<&str>, expected: &RoundMarker) -> bool {
    match parse_round_marker(body.unwrap_or_default()) {
        Some(marker) => {
            marker.round != expected.round || marker.comment_url != expected.comment_url
        }
        None => true,
    }
}

/// Everything the caller decides about a repair — which is only whether to notify.
///
/// The round, its anchor, its note and its comparison base are all read from the
/// derived round, so a repair cannot invent a round state the model does not have.
#[derive(Debug, Clone)]
pub struct RepairRoundRequest {
    pub issue: Issue,
    /// Post a `# QC Notification` when the open round has none.
    /// [`NotificationMode::None`] (the default for every caller) posts nothing.
    pub notification: NotificationMode,
}

/// Outcome of [`repair_round`], in the same per-step shape as
/// [`crate::StartRoundResult`] so the API and CLI can render both uniformly.
#[derive(Debug, Clone)]
pub struct RepairRoundResult {
    /// Index of the open round that was repaired.
    pub round: u32,
    /// Human-readable name of that round, e.g. `"Round 2"`.
    pub round_name: String,
    /// URL of the round's `# QC New Round` comment; `None` when the comment came
    /// from the disk cache and therefore carries no identity.
    pub round_comment_url: Option<String>,
    /// What was found to be incomplete before anything was written.
    pub plan: RepairPlan,
    /// Reopening the issue.
    pub reopened: StepOutcome,
    /// Refreshing the `## QC Round` block in the issue body.
    pub body_marker: StepOutcome,
    /// The `# QC Notification` comment.
    pub notification: StepOutcome,
}

impl RepairRoundResult {
    /// Whether a step that was attempted failed, and so still wants a retry.
    pub fn needs_repair(&self) -> bool {
        self.reopened.failed() || self.body_marker.failed() || self.notification.failed()
    }

    /// Whether anything was actually written.
    pub fn repaired(&self) -> bool {
        [&self.reopened, &self.body_marker, &self.notification]
            .iter()
            .any(|outcome| **outcome == StepOutcome::Done)
    }
}

impl fmt::Display for RepairRoundResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "🔧 Repairing {}", self.round_name)?;

        let step = |f: &mut fmt::Formatter<'_>,
                    label: &str,
                    outcome: &StepOutcome,
                    skip_reason: &str| match outcome {
            StepOutcome::Done => writeln!(f, "  ✅ {label}"),
            StepOutcome::Skipped => writeln!(f, "  ⏭️ {label} ({skip_reason})"),
            StepOutcome::Failed(error) => writeln!(f, "  ⚠️ {label} failed: {error}"),
        };
        step(
            f,
            "Issue set back to open",
            &self.reopened,
            "issue is already open",
        )?;
        step(
            f,
            "Issue body round marker updated",
            &self.body_marker,
            if self.round_comment_url.is_none() {
                "round comment URL unknown, marker left as it is"
            } else {
                "marker already matches this round"
            },
        )?;
        step(
            f,
            "QC notification posted",
            &self.notification,
            if self.plan.notification_missing {
                "not requested"
            } else {
                "this round was already notified"
            },
        )?;

        if !self.plan.needs_repair() && self.notification == StepOutcome::Skipped {
            writeln!(f, "\nNothing needed repairing.")?;
        }
        if self.needs_repair() {
            writeln!(
                f,
                "\n⚠️ The step(s) above still failed. Every step is idempotent, so this command \
                 can be run again safely once the cause is fixed."
            )?;
        }
        Ok(())
    }
}

/// Why a repair could not be attempted. Returned only when nothing was written.
#[derive(Debug, thiserror::Error)]
pub enum RepairRoundError {
    #[error(
        "Nothing to repair: {round_name} is closed, so no round is open on this issue. A repair only ever completes the follow-up steps of a round that is currently open."
    )]
    NoOpenRound { round: u32, round_name: String },
    #[error(
        "Nothing to repair: the open round is {round_name}, which was opened when the issue was created rather than by a `# QC New Round` comment, so it has no follow-up steps."
    )]
    InitialRound { round: u32, round_name: String },
    #[error(
        "Cannot repair: no QC rounds could be derived for this issue (its commit history may be unreachable)"
    )]
    NoRounds,
}

/// Re-run the follow-up steps the open round is missing.
///
/// `thread` must be the thread of `request.issue`; its last round supplies the
/// round being repaired. Errors — writing nothing — when there is nothing to
/// repair: no rounds at all, the last round closed, or the open round being
/// Initial QC, which no new-round action opened.
///
/// Never aborts part-way: each step's failure is captured and the rest still run.
pub async fn repair_round<T>(
    request: &RepairRoundRequest,
    thread: &IssueThread,
    git_info: &T,
) -> Result<RepairRoundResult, RepairRoundError>
where
    T: GitHubWriter + Sync,
{
    let round = thread.rounds.last().ok_or(RepairRoundError::NoRounds)?;
    if !round.is_open() {
        return Err(RepairRoundError::NoOpenRound {
            round: round.index,
            round_name: round.name(),
        });
    }
    let RoundOpen::NewRound {
        comment_url, note, ..
    } = &round.opened
    else {
        return Err(RepairRoundError::InitialRound {
            round: round.index,
            round_name: round.name(),
        });
    };

    let plan = plan_repair(&request.issue, round);
    log::info!(
        "repairing round {} on issue #{}: {plan:?}",
        round.index,
        request.issue.number
    );

    let reopened = if plan.reopen {
        reopen_step(request.issue.number, git_info).await
    } else {
        StepOutcome::Skipped
    };

    let body_marker = match expected_marker(round).filter(|_| plan.body_marker) {
        Some(marker) => body_marker_step(&request.issue, &marker, git_info).await,
        None => StepOutcome::Skipped,
    };

    // A notification is only ever posted on an explicit request, and only when the
    // round has none: the anchor and the note are the round's own, so the comment
    // is the one the failed start would have posted, not one describing HEAD now.
    let notification = if plan.notification_missing {
        notification_step(
            &thread.file,
            &request.issue,
            round.opened_at,
            round.previous_approval,
            note.clone(),
            request.notification,
            git_info,
        )
        .await
    } else {
        StepOutcome::Skipped
    };

    Ok(RepairRoundResult {
        round: round.index,
        round_name: round.name(),
        round_comment_url: comment_url.clone(),
        plan,
        reopened,
        body_marker,
        notification,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comment::QCComment;
    use crate::comment_system::CommentBody;
    use crate::git::{GitHubApiError, MockGitHubWriter};
    use crate::new_round::{QCNewRound, ROUND_MARKER_HEADING, upsert_round_marker};
    use crate::round::{ChecklistSource, RoundState};
    use crate::test_utils::create_test_issue;
    use gix::ObjectId;
    use octocrab::models::Milestone;
    use std::future::Future;
    use std::path::PathBuf;
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";
    const URL: &str = "https://github.com/o/r/issues/1#issuecomment-2";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    /// A repair only ever writes, so the mock only needs the writer half.
    struct MockGit {
        writer: MockGitHubWriter,
    }

    impl MockGit {
        fn new() -> Self {
            Self {
                writer: MockGitHubWriter::new(),
            }
        }

        /// Nothing at all may be written.
        fn expect_no_writes(mut self) -> Self {
            self.writer.expect_open_issue().times(0);
            self.writer.expect_update_issue().times(0);
            self.writer.expect_post_comment::<QCComment>().times(0);
            self.writer.expect_post_comment::<QCNewRound>().times(0);
            self
        }
    }

    impl GitHubWriter for MockGit {
        fn create_milestone(
            &self,
            name: &str,
            description: &Option<String>,
        ) -> impl Future<Output = Result<Milestone, GitHubApiError>> + Send {
            self.writer.create_milestone(name, description)
        }
        fn post_issue(
            &self,
            issue: &crate::QCIssue,
        ) -> impl Future<Output = Result<Issue, GitHubApiError>> + Send {
            self.writer.post_issue(issue)
        }
        fn post_comment<T: CommentBody + Sync + 'static>(
            &self,
            comment: &T,
        ) -> impl Future<Output = Result<String, GitHubApiError>> + Send {
            self.writer.post_comment(comment)
        }
        fn close_issue(
            &self,
            issue_number: u64,
        ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
            self.writer.close_issue(issue_number)
        }
        fn open_issue(
            &self,
            issue_number: u64,
        ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
            self.writer.open_issue(issue_number)
        }
        fn create_label(
            &self,
            name: &str,
            color: &str,
        ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
            self.writer.create_label(name, color)
        }
        fn block_issue(
            &self,
            blocked: u64,
            blocking: u64,
        ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
            self.writer.block_issue(blocked, blocking)
        }
        fn update_issue(
            &self,
            issue_number: u64,
            new_title: Option<String>,
            new_body: Option<String>,
        ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
            self.writer.update_issue(issue_number, new_title, new_body)
        }
    }

    /// The body of an issue whose `## QC Round` marker is correct for round 2.
    fn body_with_marker(round: u32, comment_url: &str) -> String {
        upsert_round_marker(
            "Quality check issue for src/main.rs\n\n# Code Review Checklist\n- [ ] item\n",
            &RoundMarker {
                round,
                comment_url: comment_url.to_string(),
            },
        )
    }

    fn issue(state: &str, body: &str) -> Issue {
        create_test_issue("o", "r", 1, "src/main.rs", body, Some(1), state)
    }

    /// Round 1 closed at `B`, round 2 open at `C` — the state a start-round leaves.
    fn round_two(comment_url: Option<&str>, events: Vec<RoundEvent>) -> Round {
        Round {
            index: 2,
            opened_at: oid(C),
            previous_approval: Some(oid(B)),
            opened: RoundOpen::NewRound {
                comment_index: 1,
                comment_id: Some(2),
                comment_url: comment_url.map(str::to_string),
                author: "author".to_string(),
                at: chrono::Utc::now(),
                note: Some("Second pass.".to_string()),
            },
            checklist: ChecklistSource::Comment {
                comment_index: 1,
                comment_id: Some(2),
                comment_url: comment_url.map(str::to_string),
            },
            checklist_name: Some("Code Review Checklist".to_string()),
            state: RoundState::Open,
            events,
            retractions: Vec::new(),
            extensions: Vec::new(),
        }
    }

    fn initial_round(state: RoundState) -> Round {
        Round {
            index: 1,
            opened_at: oid(A),
            previous_approval: None,
            opened: RoundOpen::IssueCreated,
            checklist: ChecklistSource::IssueBody,
            checklist_name: None,
            state,
            events: Vec::new(),
            retractions: Vec::new(),
            extensions: Vec::new(),
        }
    }

    fn closed_initial_round() -> Round {
        initial_round(RoundState::Closed {
            commit: oid(B),
            by: "reviewer".to_string(),
            at: chrono::Utc::now(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        })
    }

    fn thread(rounds: Vec<Round>) -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            branch: "main".to_string(),
            open: true,
            commits: Vec::new(),
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            rounds,
            round_anomalies: Vec::new(),
        }
    }

    fn notification_event(commit: &str) -> RoundEvent {
        RoundEvent::Notification {
            commit: oid(commit),
            by: "author".to_string(),
            at: chrono::Utc::now(),
            comment_index: 2,
            comment_id: Some(3),
            comment_url: None,
        }
    }

    fn request(issue: Issue, notification: NotificationMode) -> RepairRoundRequest {
        RepairRoundRequest {
            issue,
            notification,
        }
    }

    fn api_error() -> GitHubApiError {
        GitHubApiError::NoApi
    }

    // ── Preconditions: nothing to repair means nothing written ───────────────

    #[tokio::test]
    async fn a_closed_last_round_is_rejected_without_writing_anything() {
        let git = MockGit::new().expect_no_writes();
        let thread = thread(vec![closed_initial_round()]);

        let error = repair_round(
            &request(issue("open", ""), NotificationMode::Full),
            &thread,
            &git,
        )
        .await
        .expect_err("a closed round has nothing to repair");

        assert!(
            matches!(error, RepairRoundError::NoOpenRound { round: 1, .. }),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("Nothing to repair"));
    }

    #[tokio::test]
    async fn an_open_initial_qc_round_is_rejected_without_writing_anything() {
        let git = MockGit::new().expect_no_writes();
        let thread = thread(vec![initial_round(RoundState::Open)]);

        // Closed issue *and* an open round, i.e. exactly the shape a broken round 2
        // has — but Initial QC was never opened by a new-round action, so there are
        // no follow-up steps of its to complete.
        let error = repair_round(
            &request(issue("closed", ""), NotificationMode::None),
            &thread,
            &git,
        )
        .await
        .expect_err("Initial QC has no follow-up steps");

        assert!(
            matches!(error, RepairRoundError::InitialRound { round: 1, .. }),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("Initial QC"));
    }

    #[tokio::test]
    async fn a_thread_without_rounds_is_rejected() {
        let git = MockGit::new().expect_no_writes();

        let error = repair_round(
            &request(issue("closed", ""), NotificationMode::None),
            &thread(Vec::new()),
            &git,
        )
        .await
        .expect_err("no rounds means nothing to repair");
        assert!(matches!(error, RepairRoundError::NoRounds));
    }

    // ── Only what is incomplete is re-run ────────────────────────────────────

    #[tokio::test]
    async fn only_the_reopen_is_re_run_when_only_the_reopen_is_missing() {
        let mut git = MockGit::new();
        git.writer
            .expect_open_issue()
            .times(1)
            .withf(|number: &u64| *number == 1)
            .returning(|_| Box::pin(async { Ok(()) }));
        // The marker is already correct and the round was notified: no other write.
        git.writer.expect_update_issue().times(0);
        git.writer.expect_post_comment::<QCComment>().times(0);

        let issue = issue("closed", &body_with_marker(2, URL));
        let thread = thread(vec![
            closed_initial_round(),
            round_two(Some(URL), vec![notification_event(C)]),
        ]);

        let result = repair_round(&request(issue, NotificationMode::Full), &thread, &git)
            .await
            .expect("an open round can be repaired");

        assert_eq!(result.round, 2);
        assert_eq!(result.round_name, "Round 2");
        assert_eq!(result.round_comment_url.as_deref(), Some(URL));
        assert_eq!(result.reopened, StepOutcome::Done);
        assert_eq!(result.body_marker, StepOutcome::Skipped);
        assert_eq!(result.notification, StepOutcome::Skipped);
        assert!(result.repaired());
        assert!(!result.needs_repair());

        let display = result.to_string();
        assert!(display.contains("Repairing Round 2"));
        assert!(display.contains("✅ Issue set back to open"));
        assert!(display.contains("marker already matches this round"));
        assert!(display.contains("this round was already notified"));
    }

    #[tokio::test]
    async fn only_the_body_marker_is_re_run_when_only_the_marker_is_stale() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer
            .expect_update_issue()
            .times(1)
            .withf(|number, title, body| {
                *number == 1
                    && title.is_none()
                    && body.as_deref().is_some_and(|body| {
                        body.contains("* current round: 2")
                            && body.contains(&format!("* round comment: {URL}"))
                    })
            })
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer.expect_post_comment::<QCComment>().times(0);

        // The issue is open, but its marker still names round 1.
        let issue = issue(
            "open",
            &body_with_marker(1, "https://github.com/o/r/issues/1#issuecomment-1"),
        );
        let thread = thread(vec![
            closed_initial_round(),
            round_two(Some(URL), vec![notification_event(C)]),
        ]);

        let result = repair_round(&request(issue, NotificationMode::None), &thread, &git)
            .await
            .expect("a stale marker is repairable");

        assert_eq!(result.reopened, StepOutcome::Skipped);
        assert_eq!(result.body_marker, StepOutcome::Done);
        assert_eq!(result.notification, StepOutcome::Skipped);
        assert!(result.to_string().contains("issue is already open"));
    }

    #[tokio::test]
    async fn a_missing_marker_counts_as_stale() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer
            .expect_update_issue()
            .times(1)
            .withf(|_, _, body| {
                body.as_deref()
                    .is_some_and(|body| body.contains(ROUND_MARKER_HEADING))
            })
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        let issue = issue("open", "Quality check issue for src/main.rs\n");
        let thread = thread(vec![
            closed_initial_round(),
            round_two(Some(URL), vec![notification_event(C)]),
        ]);

        let result = repair_round(&request(issue, NotificationMode::None), &thread, &git)
            .await
            .expect("a missing marker is repairable");
        assert_eq!(result.body_marker, StepOutcome::Done);
    }

    /// A cache-loaded round comment has no URL, so no correct marker can be
    /// derived: the body is left exactly as it is rather than rewritten wrongly.
    #[tokio::test]
    async fn an_unknown_round_comment_url_leaves_the_marker_alone() {
        let mut git = MockGit::new();
        git.writer.expect_update_issue().times(0);
        git.writer
            .expect_open_issue()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));

        // No marker at all — and still nothing is written to the body.
        let issue = issue("closed", "Quality check issue for src/main.rs\n");
        let thread = thread(vec![
            closed_initial_round(),
            round_two(None, vec![notification_event(C)]),
        ]);

        let result = repair_round(&request(issue, NotificationMode::None), &thread, &git)
            .await
            .expect("an unknown URL is not an error");

        assert_eq!(result.round_comment_url, None);
        assert!(!result.plan.body_marker);
        assert_eq!(result.body_marker, StepOutcome::Skipped);
        assert!(
            result
                .to_string()
                .contains("round comment URL unknown, marker left as it is")
        );
    }

    // ── The notification is never implicit ───────────────────────────────────

    #[tokio::test]
    async fn a_missing_notification_is_not_posted_unless_it_is_requested() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        // The round has no Notification event, and the caller did not ask for one.
        git.writer.expect_post_comment::<QCComment>().times(0);

        let issue = issue("open", &body_with_marker(2, URL));
        let thread = thread(vec![closed_initial_round(), round_two(Some(URL), vec![])]);

        let result = repair_round(&request(issue, NotificationMode::None), &thread, &git)
            .await
            .expect("nothing to do is not an error");

        assert!(result.plan.notification_missing);
        // ... yet nothing is *wrong*: choosing not to notify is a decision.
        assert!(!result.plan.needs_repair());
        assert_eq!(result.notification, StepOutcome::Skipped);
        assert!(!result.repaired());
        let display = result.to_string();
        assert!(display.contains("not requested"));
        assert!(display.contains("Nothing needed repairing."));
    }

    #[tokio::test]
    async fn a_requested_notification_reuses_the_rounds_own_anchor_and_note() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .withf(|comment: &QCComment| {
                // The round's anchor and previous approval, not HEAD-now, and the
                // round's own note rather than anything the caller supplied.
                comment.current_commit == oid(C)
                    && comment.previous_commit == Some(oid(B))
                    && comment.note.as_deref() == Some("Second pass.")
                    && comment.no_diff
            })
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));

        let issue = issue("open", &body_with_marker(2, URL));
        let thread = thread(vec![closed_initial_round(), round_two(Some(URL), vec![])]);

        let result = repair_round(
            &request(issue, NotificationMode::MetadataOnly),
            &thread,
            &git,
        )
        .await
        .expect("a requested notification is posted");

        assert_eq!(result.notification, StepOutcome::Done);
    }

    #[tokio::test]
    async fn an_already_notified_round_is_never_notified_again() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        // Requested, but the round already has a Notification: no double-post.
        git.writer.expect_post_comment::<QCComment>().times(0);

        let issue = issue("open", &body_with_marker(2, URL));
        let thread = thread(vec![
            closed_initial_round(),
            round_two(Some(URL), vec![notification_event(C)]),
        ]);

        let result = repair_round(&request(issue, NotificationMode::Full), &thread, &git)
            .await
            .expect("nothing to do is not an error");

        assert!(!result.plan.notification_missing);
        assert_eq!(result.notification, StepOutcome::Skipped);
    }

    // ── Partial failure ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_failing_step_is_reported_and_the_remaining_steps_still_run() {
        let mut git = MockGit::new();
        git.writer
            .expect_open_issue()
            .times(1)
            .returning(|_| Box::pin(async { Err(api_error()) }));
        git.writer
            .expect_update_issue()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));

        // Everything is incomplete: closed issue, no marker, no notification.
        let issue = issue("closed", "Quality check issue for src/main.rs\n");
        let thread = thread(vec![closed_initial_round(), round_two(Some(URL), vec![])]);

        let result = repair_round(&request(issue, NotificationMode::Full), &thread, &git)
            .await
            .expect("a failing step is not a hard error");

        assert!(result.reopened.failed());
        assert_eq!(result.body_marker, StepOutcome::Done);
        assert_eq!(result.notification, StepOutcome::Done);
        assert!(result.needs_repair());
        assert!(result.repaired());
        let display = result.to_string();
        assert!(display.contains("⚠️ Issue set back to open failed"));
        assert!(display.contains("can be run again safely"));
    }

    // ── The plan itself ─────────────────────────────────────────────────────

    #[test]
    fn the_plan_derives_each_flag_independently() {
        let complete = plan_repair(
            &issue("open", &body_with_marker(2, URL)),
            &round_two(Some(URL), vec![notification_event(C)]),
        );
        assert_eq!(complete, RepairPlan::default());
        assert!(!complete.needs_repair());

        let closed = plan_repair(
            &issue("closed", &body_with_marker(2, URL)),
            &round_two(Some(URL), vec![notification_event(C)]),
        );
        assert!(closed.reopen && !closed.body_marker && closed.needs_repair());

        // A marker whose round agrees but whose URL does not is still stale.
        let wrong_url = plan_repair(
            &issue(
                "open",
                &body_with_marker(2, "https://example.invalid/other"),
            ),
            &round_two(Some(URL), vec![notification_event(C)]),
        );
        assert!(wrong_url.body_marker && wrong_url.needs_repair());

        // A missing notification alone never asks for a repair.
        let unnotified = plan_repair(
            &issue("open", &body_with_marker(2, URL)),
            &round_two(Some(URL), vec![]),
        );
        assert!(unnotified.notification_missing && !unnotified.needs_repair());
    }
}
