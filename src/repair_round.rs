//! Repairing an open QC round: re-running the follow-up steps that
//! [`crate::start_round`] left incomplete.
//!
//! [`crate::start_round`] posts the round comment first — that is what
//! creates the round — and then performs three recoverable steps: reopen the
//! issue, refresh the `## QC Round` body marker, and optionally notify. Those
//! three are reported per step rather than aborting the action, on the promise
//! that each one is independently retryable.
//!
//! This module is that retry. It cannot be the new-round action re-run, because
//! the round is now *open*: `start_round` refuses to run against an open round,
//! and a second round comment would **extend** the round (see
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
//! Only the notification reads the round's placement — it names the round's own
//! anchor and the approval before it — so an unplaceable round costs that one step
//! and nothing else: it is skipped with its reason while the reopen and the body
//! marker, which need only the issue number and the round's index and comment URL,
//! still run. Refusing the whole repair instead would break this module's promise
//! for the one input whose round is most obviously broken.
//!
//! Every step is the same function `start_round` calls, so the two paths cannot
//! drift apart, and every step is idempotent — a repair that is run twice writes
//! the same body and reopens an already-open issue with a no-op PATCH.

use std::fmt;

use octocrab::models::{IssueState, issues::Issue};

use crate::git::{GitCommitOps, GitHubWriter};
use crate::issue::IssueThread;
use crate::new_round::{RoundMarker, parse_round_marker};
use crate::round::{Placement, Round, RoundEvent, RoundOpen, Segment, UnplaceableReason};
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
    /// Why the round's commits could not be located, when they could not be.
    ///
    /// A notification names the round's own anchor and the approval before it, so an
    /// unplaceable round — whose anchor may be the null-OID placeholder and which owns
    /// no commits — has nothing to notify *about*. That step is therefore skipped with
    /// this as its reason. Neither of the other two steps reads placement, so neither is
    /// affected, and [`RepairPlan::needs_repair`] is unchanged: a grayed round is
    /// degraded, not an error (**D4**).
    pub unplaceable: Option<UnplaceableReason>,
}

impl RepairPlan {
    /// Whether something is actually *wrong* and worth offering a repair for.
    ///
    /// Deliberately excludes [`RepairPlan::notification_missing`]: a round opened
    /// with [`NotificationMode::None`] is in exactly the state its author chose.
    pub fn needs_repair(&self) -> bool {
        self.reopen || self.body_marker
    }

    /// Whether a requested notification can actually be posted: the round must be
    /// missing one, and be placed — there is no commit to notify against otherwise.
    pub fn can_notify(&self) -> bool {
        self.notification_missing && self.unplaceable.is_none()
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
        unplaceable: match round.placement {
            Placement::Placed => None,
            Placement::Unplaceable(reason) => Some(reason),
        },
    }
}

/// The marker the open round's body *should* carry.
///
/// `None` when the round's identity URL is unknown — `RoundOpen::NewRound`'s
/// `comment_url` is optional because comments loaded from the disk cache carry no
/// identity, and for Initial QC, which no round comment opened. In
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

/// Why a repair posted no notification.
///
/// Lives on the **outcome** rather than on [`RepairPlan`] because only the outcome knows
/// what was asked for: the plan is derived from the issue and the round alone, and cannot
/// tell "could not" from "was never wanted". Ordering the causes needs both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSkip {
    /// The caller asked for no notification — [`NotificationMode::None`], which is the
    /// default on every surface. Not a defect and not a degradation.
    NotRequested,
    /// The round already carries a `# QC Notification`, so there was nothing to post.
    AlreadyNotified,
    /// The round is missing one and the caller asked for it, but the round could not be
    /// placed: there is no commit to notify against.
    Unplaceable(UnplaceableReason),
}

/// Everything the caller decides about a repair — whether to notify, and what the
/// notification should say.
///
/// The round, its anchor and its comparison base are all read from the derived
/// round, so a repair cannot invent a round state the model does not have.
#[derive(Debug, Clone)]
pub struct RepairRoundRequest {
    pub issue: Issue,
    /// Post a `# QC Notification` when the open round has none.
    /// [`NotificationMode::None`] (the default for every caller) posts nothing.
    pub notification: NotificationMode,
    /// Context for the reviewer, for the notification this repair may post.
    ///
    /// A notification-only note lives nowhere but the notification comment, so when
    /// that post is the step that failed the text is gone: the caller passes it
    /// again. `None` falls back to the round's own note, which is what a repair had
    /// to use before the two were separable.
    pub notification_note: Option<String>,
}

/// Outcome of [`repair_round`], in the same per-step shape as
/// [`crate::StartRoundResult`] so the API and CLI can render both uniformly.
#[derive(Debug, Clone)]
pub struct RepairRoundResult {
    /// Index of the open round that was repaired.
    pub round: u32,
    /// Human-readable name of that round, e.g. `"Round 2"`.
    pub round_name: String,
    /// URL of the round's round comment; `None` when the comment came
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
    /// Why no notification was posted, or `None` when the post was attempted.
    pub notification_skip: Option<NotificationSkip>,
}

impl RepairRoundResult {
    /// Why a step was skipped for a reason other than there being nothing to do, so the
    /// CLI and the API can say *"notification skipped: its branch is unavailable
    /// locally"* rather than leaving a bare `skipped`.
    ///
    /// Only a degradation earns a reason. "Not requested" and "already notified" are
    /// ordinary outcomes the caller either chose or already has, and both surfaces render
    /// them from their own wording; reporting them here would turn the default no-op
    /// repair into something that looks like a failure.
    ///
    /// Precedence is set where [`Self::notification_skip`] is computed: already-notified,
    /// then not-requested, then unplaceable. Placement is consulted **last** because it is
    /// the only cause the caller did not choose — blaming an unfetched branch for a
    /// notification nobody asked for is the misattribution this ordering exists to stop.
    pub fn notification_skip_reason(&self) -> Option<&'static str> {
        match self.notification_skip {
            Some(NotificationSkip::Unplaceable(reason)) => Some(reason.describe()),
            _ => None,
        }
    }

    /// Why the body marker was left alone, when the reason is not simply that it already
    /// matched. A round whose comment URL is unknown has no marker anyone can derive, so
    /// the step cannot be attempted — the same shape as a notification skipped for
    /// placement, and equally worth reporting rather than a bare `skipped`.
    pub fn body_marker_skip_reason(&self) -> Option<&'static str> {
        self.round_comment_url
            .is_none()
            .then_some("round comment URL unknown, marker left as it is")
    }

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
            self.body_marker_skip_reason()
                .unwrap_or("marker already matches this round"),
        )?;
        step(
            f,
            "QC notification posted",
            &self.notification,
            match self.notification_skip {
                Some(NotificationSkip::Unplaceable(reason)) => reason.describe(),
                Some(NotificationSkip::NotRequested) | None => "not requested",
                Some(NotificationSkip::AlreadyNotified) => "this round was already notified",
            },
        )?;

        // A notification skipped because the round could not be placed is not "nothing
        // needed repairing": something was wanted and could not be attempted. A skip the
        // caller chose, or one there was nothing to do for, *is* — and since every surface
        // defaults to `NotificationMode::None`, that is the ordinary case this reassurance
        // exists for.
        if !self.plan.needs_repair()
            && self.notification == StepOutcome::Skipped
            && matches!(
                self.notification_skip,
                Some(NotificationSkip::NotRequested | NotificationSkip::AlreadyNotified)
            )
        {
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
        "Nothing to repair: the open round is {round_name}, which was opened when the issue was created rather than by a round comment, so it has no follow-up steps."
    )]
    InitialRound { round: u32, round_name: String },
    #[error(
        "Cannot repair: no QC rounds could be derived for this issue (its commit history may be unreachable)"
    )]
    NoRounds,
}

/// The open round to repair, with **its own** segment position: the round the issue is
/// *currently in*, i.e. the active segment when that is an open round.
///
/// Only the active segment counts, which is what every other consumer of an open round
/// reads (`RoundRepairStatus::derive`, `round_basis`). Scanning for the newest open round
/// instead would name a round in the past: a round whose approval commit cannot be
/// resolved is reopened by the fold *and* grayed, so `[R1(open, unplaceable), Gap,
/// R2(closed), Gap]` is a legal shape whose *active* segment is a trailing gap — the
/// issue reads approved, and there is nothing to repair. More than one round may be open
/// at once for that reason, so "the open round" is only ever the active one.
///
/// The position is returned from the same lookup as the round so the two can never name
/// different segments — `previous_approval_of` reads a **round's** position, and a gap's
/// would silently answer `None`. That coupling is what pins the two together.
///
/// This *is* `segments.len() - 1` plus a filter, and makes no claim beyond it: were the
/// fold ever to emit an open round that is not the last segment, this would return `None`
/// rather than that round's position. Correcting an earlier note here, that is not a shape
/// the coupling has been shown to survive — only one it cannot currently encounter.
fn open_round(thread: &IssueThread) -> Option<(usize, &Round)> {
    let (position, segment) = thread.segments.iter().enumerate().next_back()?;
    let round = Segment::as_round(segment).filter(|round| round.is_open())?;
    Some((position, round))
}

/// Re-run the follow-up steps the open round is missing.
///
/// `thread` must be the thread of `request.issue`; the round it is currently in (see
/// [`open_round`]) is the round being repaired. Errors — writing nothing — only when
/// there is nothing to repair at all: no rounds, no round open, or the open round being
/// Initial QC, which no new-round action opened.
///
/// Never aborts part-way: each step's failure is captured and the rest still run, and an
/// unplaceable round costs only its notification (see [`RepairPlan::unplaceable`]).
pub async fn repair_round<T>(
    request: &RepairRoundRequest,
    thread: &IssueThread,
    git_info: &T,
) -> Result<RepairRoundResult, RepairRoundError>
where
    T: GitHubWriter + GitCommitOps + Sync,
{
    // The round being repaired, and the position it sits at — one lookup, so the round
    // operated on and the position the previous approval is read from are the same
    // segment by construction rather than by invariant.
    let (position, round) =
        open_round(thread).ok_or_else(|| match thread.rounds().next_back() {
            Some(newest) => RepairRoundError::NoOpenRound {
                round: newest.index,
                round_name: newest.name(),
            },
            None => RepairRoundError::NoRounds,
        })?;
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
    // round has none: the anchor is the round's own, so the comment is the one the
    // failed start would have posted, not one describing HEAD now. An unplaceable round
    // has no anchor to name — `opened_at` may be the null-OID placeholder and
    // `previous_approval_of` reads commits the model could not locate — so this one step
    // is skipped, with its reason (`plan.unplaceable`), rather than the repair refusing
    // the two steps that do not care where the round sits.
    let notification = if plan.can_notify() {
        notification_step(
            &thread.file,
            &request.issue,
            round.opened_at,
            // The same fallback the start path applies: on a branch that does not
            // contain the previous approval, diffing against it describes cross-branch
            // differences rather than this round's. A lookup failure leaves the
            // approval in place — a repair must not be blocked by an ancestry query.
            thread
                .previous_approval_of(position)
                .copied()
                .map(
                    |approval| match git_info.merge_base(&approval, &round.opened_at) {
                        Ok(Some(base)) => base,
                        Ok(None) => approval,
                        Err(error) => {
                            log::warn!("merge-base lookup failed during repair: {error}");
                            approval
                        }
                    },
                ),
            request.notification_note.clone().or_else(|| note.clone()),
            request.notification,
            git_info,
        )
        .await
    } else {
        StepOutcome::Skipped
    };

    // Precedence: already-notified, then not-requested, then unplaceable. The mode is only
    // known here, which is why the cause rides on the outcome rather than on the plan —
    // and the order matters because every surface defaults to `NotificationMode::None`, so
    // ranking placement first would blame an unfetched branch for a post nobody requested.
    let notification_skip = if !plan.notification_missing {
        Some(NotificationSkip::AlreadyNotified)
    } else if request.notification == NotificationMode::None {
        Some(NotificationSkip::NotRequested)
    } else {
        plan.unplaceable.map(NotificationSkip::Unplaceable)
    };

    Ok(RepairRoundResult {
        round: round.index,
        round_name: round.name(),
        round_comment_url: comment_url.clone(),
        plan,
        reopened,
        body_marker,
        notification,
        notification_skip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comment::QCComment;
    use crate::comment_system::CommentBody;
    use crate::git::{GitHubApiError, MockGitHubWriter};
    use crate::new_round::{QCNewRound, ROUND_MARKER_HEADING, upsert_round_marker};
    use crate::round::{ChecklistSource, Gap, GapContinuity, Placement, RoundState, Segment};
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
        /// What `merge_base` reports. `None` is the default: the first argument, i.e.
        /// the approval is an ancestor of the round's anchor — no divergence.
        merge_base: Option<Option<gix::ObjectId>>,
    }

    impl MockGit {
        fn new() -> Self {
            Self {
                writer: MockGitHubWriter::new(),
                merge_base: None,
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

    /// Only `merge_base` matters here: a repair never walks history. The default says
    /// the approval is an ancestor of the round's anchor — the no-divergence case —
    /// which is what every existing repair test assumes.
    impl GitCommitOps for MockGit {
        fn merge_base(
            &self,
            a: &gix::ObjectId,
            _b: &gix::ObjectId,
        ) -> Result<Option<gix::ObjectId>, crate::git::GitFileOpsError> {
            Ok(self.merge_base.unwrap_or(Some(*a)))
        }
        fn commits(
            &self,
            _branch: &Option<String>,
            _stop_at: Option<gix::ObjectId>,
        ) -> Result<Vec<crate::git::GitCommit>, crate::git::GitFileOpsError> {
            unimplemented!("a repair never walks history")
        }
        fn branch_tip(
            &self,
            _branch: &Option<String>,
        ) -> Result<gix::ObjectId, crate::git::GitFileOpsError> {
            unimplemented!("a repair uses the round's own anchor, not HEAD")
        }
        fn file_touching_commits(
            &self,
            _branch: Option<String>,
            _file: &std::path::Path,
        ) -> Result<std::collections::HashSet<String>, crate::git::GitFileOpsError> {
            unimplemented!()
        }
        fn get_branches_containing_commit(
            &self,
            _commit: &gix::ObjectId,
        ) -> Result<Vec<String>, crate::git::GitFileOpsError> {
            unimplemented!()
        }
        fn find_merged_into_branch(
            &self,
            _target_commit: &gix::ObjectId,
        ) -> Result<Option<String>, crate::git::GitFileOpsError> {
            unimplemented!()
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
            branch: "main".to_string(),
            opened: RoundOpen::NewRound {
                branch: Some("main".to_string()),
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
            commits: vec![commit(C)],
            placement: Placement::Placed,
        }
    }

    fn initial_round(state: RoundState) -> Round {
        Round {
            index: 1,
            opened_at: oid(A),
            branch: "main".to_string(),
            opened: RoundOpen::IssueCreated,
            checklist: ChecklistSource::IssueBody,
            checklist_name: None,
            state,
            events: Vec::new(),
            retractions: Vec::new(),
            extensions: Vec::new(),
            commits: vec![commit(B), commit(A)],
            placement: Placement::Placed,
        }
    }

    fn commit(sha: &str) -> crate::issue::IssueCommit {
        crate::issue::IssueCommit {
            hash: oid(sha),
            message: format!("commit {sha}"),
            file_changed: true,
        }
    }

    /// An empty gap: nothing here depends on what drifted between two rounds.
    fn gap() -> Gap {
        Gap {
            branch: "main".to_string(),
            commits: Vec::new(),
            continuity: GapContinuity::Linear,
            placement: Placement::Placed,
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

    /// `rounds` as a thread: the gaps between them, plus a trailing one when the last
    /// round is closed, since a closed round is never the active segment.
    fn thread(rounds: Vec<Round>) -> IssueThread {
        let mut segments: Vec<Segment> = Vec::new();
        for round in rounds {
            if !segments.is_empty() {
                segments.push(Segment::Gap(gap()));
            }
            segments.push(Segment::Round(round));
        }
        // A closed round is never the last segment.
        if segments
            .last()
            .and_then(Segment::as_round)
            .is_some_and(|round| !round.is_open())
        {
            segments.push(Segment::Gap(gap()));
        }
        thread_with_segments(segments)
    }

    /// A thread of exactly these segments, for the shapes the alternating builder above
    /// cannot express — an open round that is not the last segment.
    fn thread_with_segments(segments: Vec<Segment>) -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            open: true,
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies: Vec::new(),
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
            // Unset: these tests exercise the fallback to the round's own note.
            notification_note: None,
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

    /// A round the model could not place owns no commits and its anchor may be a
    /// null-OID placeholder, so there is no commit for a *notification* to name — and
    /// nothing else about it is in doubt. The notification is therefore skipped with its
    /// reason while the reopen and the body marker, which need only the issue number and
    /// the round's index and comment URL, still run. Refusing outright would leave the
    /// repair the status endpoint offers (`needs_repair` is `reopen || body_marker`)
    /// returning a 409 for the ordinary "branch not fetched locally" state.
    #[tokio::test]
    async fn an_unplaceable_open_round_skips_only_the_notification() {
        let mut git = MockGit::new();
        git.writer
            .expect_open_issue()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        // Requested, and the round has none — yet nothing is posted: there is no commit
        // to notify against.
        git.writer.expect_post_comment::<QCComment>().times(0);

        let mut round = round_two(Some(URL), Vec::new());
        round.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        round.commits.clear(); // an unplaceable segment owns nothing
        // The anchor the fold leaves behind when it cannot resolve one. Naming it in a
        // comment is exactly what the skip prevents.
        round.opened_at = ObjectId::null(gix::hash::Kind::Sha1);
        let thread = thread(vec![closed_initial_round(), round]);

        // A closed issue with no marker and no notification: every step is incomplete.
        let result = repair_round(
            &request(issue("closed", ""), NotificationMode::Full),
            &thread,
            &git,
        )
        .await
        .expect("a grayed round is still repairable");

        assert_eq!(result.round, 2);
        assert_eq!(result.reopened, StepOutcome::Done);
        assert_eq!(result.body_marker, StepOutcome::Done);
        assert_eq!(result.notification, StepOutcome::Skipped);
        assert_eq!(
            result.notification_skip_reason(),
            Some("its branch is unavailable locally")
        );
        assert_eq!(
            result.plan.unplaceable,
            Some(UnplaceableReason::BranchUnavailable)
        );
        assert!(!result.plan.can_notify());
        assert!(result.repaired());
        assert!(!result.needs_repair(), "a skip is not a failure");

        let display = result.to_string();
        assert!(
            display.contains("⏭️ QC notification posted (its branch is unavailable locally)"),
            "skip reason not reported: {display}"
        );
        assert!(
            !display.contains("Nothing needed repairing."),
            "a step that could not be attempted is not nothing: {display}"
        );
    }

    /// An already-notified unplaceable round reports the honest reason, not the
    /// placement: the notification is not missing, so placement never came into it.
    #[test]
    fn an_already_notified_round_reports_no_skip_reason() {
        let mut round = round_two(Some(URL), vec![notification_event(C)]);
        round.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let plan = plan_repair(&issue("open", &body_with_marker(2, URL)), &round);

        assert_eq!(
            plan.unplaceable,
            Some(UnplaceableReason::BranchUnavailable),
            "the plan still records the placement"
        );
        assert!(!plan.can_notify());
        let result = RepairRoundResult {
            round: 2,
            round_name: "Round 2".to_string(),
            round_comment_url: Some(URL.to_string()),
            plan,
            reopened: StepOutcome::Skipped,
            body_marker: StepOutcome::Skipped,
            notification: StepOutcome::Skipped,
            notification_skip: Some(NotificationSkip::AlreadyNotified),
        };
        assert_eq!(result.notification_skip_reason(), None);
        assert!(
            result
                .to_string()
                .contains("this round was already notified")
        );
    }

    /// A grayed round two segments in the past is not the round being QC'd. The fold
    /// reopens a round whose approval commit it cannot resolve *and* grays it, so
    /// `[R1(open, unplaceable), Gap, R2(closed), Gap]` is legal — and its active segment
    /// is a trailing gap, i.e. the issue reads approved and there is nothing to repair.
    /// Scanning newest-first for *any* open round would name Initial QC and offer a
    /// repair the status endpoint (which reads the active segment) does not report.
    #[tokio::test]
    async fn a_historic_grayed_open_round_does_not_hijack_the_open_round() {
        let git = MockGit::new().expect_no_writes();

        let mut reopened_initial = initial_round(RoundState::Open);
        reopened_initial.placement = Placement::Unplaceable(UnplaceableReason::AnchorUnreachable);
        reopened_initial.commits.clear();
        let mut round_two = round_two(Some(URL), vec![notification_event(C)]);
        round_two.state = RoundState::Closed {
            commit: oid(C),
            by: "reviewer".to_string(),
            at: chrono::Utc::now(),
            comment_index: 3,
            comment_id: None,
            comment_url: None,
        };
        let thread = thread_with_segments(vec![
            Segment::Round(reopened_initial),
            Segment::Gap(gap()),
            Segment::Round(round_two),
            Segment::Gap(gap()),
        ]);

        assert!(
            open_round(&thread).is_none(),
            "a gap is active, not a round"
        );

        let error = repair_round(
            &request(issue("open", ""), NotificationMode::Full),
            &thread,
            &git,
        )
        .await
        .expect_err("the active segment is a gap: nothing is open");

        assert!(
            matches!(error, RepairRoundError::NoOpenRound { round: 2, .. }),
            "the newest round, not the grayed one behind it: {error:?}"
        );
    }

    /// This pins [`open_round`]'s position/round coupling: the round repaired and the
    /// position the previous approval is read from come from one lookup, so they cannot
    /// name different segments. `previous_approval_of` reads a **round's** position two
    /// back, so handing it anything but this round's own — `0`, or a gap's — silently
    /// answers `None` and the notification would compare against nothing.
    ///
    /// What this actually pins is that the position is **not** hard-coded to `0` and not
    /// re-derived from the round's index: round 2 sits at position 2, and reading
    /// `previous_approval_of` at that position is what finds round 1's approval.
    ///
    /// It does **not** pin robustness against an open round with segments *after* it.
    /// [`open_round`] reads the last segment and filters it, so it *is* `len - 1`; a
    /// non-final open round would make it return `None` rather than a different position,
    /// and no assertion here would fail. The fold does not emit that shape — a trailing
    /// gap is appended only after a closed round — and an earlier revision of this comment
    /// claimed the coupling was verified to survive it, which is untestable as written.
    #[tokio::test]
    async fn an_open_round_is_repaired_at_its_own_position() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            // Round 1's approval — read from round 2's own position, not position 0's.
            .withf(|comment: &QCComment| {
                comment.current_commit == oid(C) && comment.previous_commit == Some(oid(B))
            })
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));

        // [R1(closed at B), gap, R2(open)] — the round being QC'd is the active segment,
        // and it is at position 2, not 0.
        let thread = thread_with_segments(vec![
            Segment::Round(closed_initial_round()),
            Segment::Gap(gap()),
            Segment::Round(round_two(Some(URL), Vec::new())),
        ]);

        // The lookup the repair uses names round 2 *and* its own position, and that
        // position is the one carrying round 1's approval.
        let (position, round) = open_round(&thread).expect("round 2 is open");
        assert_eq!((position, round.index), (2, 2));
        assert_eq!(
            thread.segments[position]
                .as_round()
                .map(|found| found.index),
            Some(round.index),
            "the position addresses a different segment than the round operated on"
        );
        assert_eq!(thread.previous_approval_of(position), Some(&oid(B)));

        let result = repair_round(
            &request(
                issue("open", &body_with_marker(2, URL)),
                NotificationMode::MetadataOnly,
            ),
            &thread,
            &git,
        )
        .await
        .expect("the active open round is repairable");

        assert_eq!(result.round, 2);
        assert_eq!(result.notification, StepOutcome::Done);
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

    /// An unplaceable round the caller asked *nothing* of is not a degradation to report.
    ///
    /// Every surface defaults to [`NotificationMode::None`] (`--notification none`,
    /// `body: {}`), so ranking placement above "not requested" made the ordinary
    /// branch-not-fetched-locally case blame the user's branch for a post they never
    /// requested — and suppressed *"Nothing needed repairing."*, which is the whole point
    /// of running the command on a round that turns out to be fine.
    #[tokio::test]
    async fn an_unrequested_notification_is_not_blamed_on_placement() {
        let mut git = MockGit::new().expect_no_writes();
        git.writer.expect_post_comment::<QCComment>().times(0);

        // Marker correct and issue open: genuinely nothing to repair.
        let issue = issue("open", &body_with_marker(2, URL));
        let mut round = round_two(Some(URL), Vec::new());
        round.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        round.commits.clear();
        round.opened_at = ObjectId::null(gix::hash::Kind::Sha1);
        let thread = thread(vec![closed_initial_round(), round]);

        let result = repair_round(&request(issue, NotificationMode::None), &thread, &git)
            .await
            .expect("nothing to do is not an error");

        // The plan still records the placement — it is a fact about the round.
        assert_eq!(
            result.plan.unplaceable,
            Some(UnplaceableReason::BranchUnavailable)
        );
        // ... but the *cause of the skip* is the caller's own choice.
        assert_eq!(
            result.notification_skip,
            Some(NotificationSkip::NotRequested)
        );
        assert_eq!(
            result.notification_skip_reason(),
            None,
            "a skip the caller chose is not a degradation to explain"
        );

        let display = result.to_string();
        assert!(
            display.contains("⏭️ QC notification posted (not requested)"),
            "the skip must be attributed to the request: {display}"
        );
        assert!(
            !display.contains("its branch is unavailable locally"),
            "placement must not be blamed for an unrequested post: {display}"
        );
        assert!(
            display.contains("Nothing needed repairing."),
            "nothing was wrong and nothing was asked for: {display}"
        );
    }

    /// Precedence's top rank: an already-notified round reports that, not the caller's
    /// mode and not its placement.
    #[tokio::test]
    async fn an_already_notified_round_outranks_both_other_skip_causes() {
        let git = MockGit::new().expect_no_writes();

        let issue = issue("open", &body_with_marker(2, URL));
        let mut round = round_two(Some(URL), vec![notification_event(C)]);
        round.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let thread = thread(vec![closed_initial_round(), round]);

        let result = repair_round(&request(issue, NotificationMode::None), &thread, &git)
            .await
            .expect("nothing to do is not an error");

        assert_eq!(
            result.notification_skip,
            Some(NotificationSkip::AlreadyNotified)
        );
        assert_eq!(result.notification_skip_reason(), None);
        let display = result.to_string();
        assert!(display.contains("this round was already notified"));
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

    /// The repair path applies the same merge-base fallback the start path does.
    ///
    /// Without this the two disagree: a start on a divergent branch diffs against the
    /// common ancestor, while the repair that finishes it diffs against an approval that
    /// is not on this branch — so retrying a failed notification would post a different,
    /// misleading diff.
    #[tokio::test]
    async fn a_repair_on_a_divergent_branch_compares_against_the_merge_base() {
        let mut git = MockGit::new();
        git.merge_base = Some(Some(oid(A)));
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            // The merge-base, not round 2's recorded previous approval (B).
            .withf(|comment: &QCComment| comment.previous_commit == Some(oid(A)))
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));

        let issue = issue("open", &body_with_marker(2, URL));
        let thread = thread(vec![closed_initial_round(), round_two(Some(URL), vec![])]);

        let result = repair_round(
            &request(issue, NotificationMode::MetadataOnly),
            &thread,
            &git,
        )
        .await
        .expect("the notification is posted");

        assert_eq!(result.notification, StepOutcome::Done);
    }

    /// A notification-only message survives nowhere but the notification comment, so
    /// when that post is the step that failed the text is gone and the caller must
    /// supply it again. Given one, the repair prefers it over the round's own note.
    #[tokio::test]
    async fn a_supplied_notification_note_overrides_the_rounds_own() {
        let mut git = MockGit::new();
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .withf(|comment: &QCComment| {
                comment.note.as_deref() == Some("Reposting: covariate block still open.")
            })
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));

        let issue = issue("open", &body_with_marker(2, URL));
        let thread = thread(vec![closed_initial_round(), round_two(Some(URL), vec![])]);

        let mut repair = request(issue, NotificationMode::MetadataOnly);
        repair.notification_note = Some("Reposting: covariate block still open.".to_string());

        let result = repair_round(&repair, &thread, &git)
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
