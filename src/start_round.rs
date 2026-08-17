//! Starting a new QC round: the write half of [`crate::round`]'s model.
//!
//! [`crate::new_round`] renders the round comment; this module performs
//! the *action* — four separate GitHub calls, in this order:
//!
//! 1. post the round comment (this is what actually creates the round),
//! 2. reopen the issue,
//! 3. upsert the `## QC Round` cache block into the issue body,
//! 4. optionally post a `# QC Notification` for `previous approval → anchor`.
//!
//! Only step 1 is a hard requirement. The round exists the moment that comment
//! lands, and the fold derives everything else from the thread, so every prefix of
//! steps 2-4 leaves the issue in a state that folds to a correct round 2 — a
//! still-closed issue or a stale body marker is cosmetic, never structural. The
//! action therefore never aborts after step 1: it completes what it can and reports
//! per-step outcomes in [`StartRoundResult`], each of which is independently
//! retryable because each of those steps is idempotent (`open_issue` on an open
//! issue is a no-op PATCH, [`upsert_round_marker`] replaces its block in place, and
//! a repeated notification is just another event on the same round).
//!
//! The anchor is always HEAD of the issue's branch at open time, read from the
//! repository rather than accepted from the caller, so no caller can open a round
//! at a commit that is not actually the tip.
//!
//! Steps 2-4 live in [`reopen_step`], [`body_marker_step`] and
//! [`notification_step`], which [`crate::repair_round`] also calls: the start and
//! repair paths run the *same* code so they cannot drift apart.

use std::borrow::Cow;
use std::fmt;
use std::path::Path;

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::approve::{ImpactNode, ImpactedIssues, extract_file_from_title};
use crate::comment::QCComment;
use crate::git::{
    GitCommitOps, GitFileOpsError, GitHubApiError, GitHubReader, GitHubWriter, GitRepository,
};
use crate::issue::{BlockingRelationship, IssueThread, determine_relationship_from_body};
use crate::new_round::{QCNewRound, RoundMarker, upsert_round_marker};
use crate::round::{GapContinuity, RoundState};

/// Whether — and how loudly — reviewers are notified about the new round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationMode {
    /// Post a `# QC Notification` with the inline file diff.
    Full,
    /// Post a `# QC Notification` with metadata only, no inline diff.
    MetadataOnly,
    /// Post no notification comment at all.
    None,
}

/// Everything the caller decides about a new round. The anchor is deliberately
/// absent: it is always HEAD at open time (see the module docs).
#[derive(Debug, Clone)]
pub struct StartRoundRequest {
    pub issue: Issue,
    /// Author-edited checklist markdown, already reset to unchecked (see
    /// [`crate::reset_checklist`]).
    pub checklist_content: String,
    /// Name of the checklist template the content came from, for the audit record.
    pub checklist_name: Option<String>,
    /// Why the round is being opened. Recorded in the `# QC Round` comment, which
    /// is the round's durable audit record.
    pub note: Option<String>,
    /// Context for the reviewer, carried by the `# QC Notification` comment only.
    ///
    /// Separate from `note` on purpose: the reason a round exists and the message
    /// addressed to whoever must review it are different things, and only the
    /// former belongs in the permanent record.
    pub notification_note: Option<String>,
    pub notification: NotificationMode,
}

/// How one of the recoverable steps 2-4 turned out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Done,
    /// The step was not attempted: it was not requested, or (on the repair path)
    /// there was nothing to do because the step is already correct.
    Skipped,
    Failed(String),
}

impl StepOutcome {
    pub fn failed(&self) -> bool {
        matches!(self, StepOutcome::Failed(_))
    }

    pub(crate) fn from_result(result: Result<(), GitHubApiError>) -> Self {
        match result {
            Ok(()) => StepOutcome::Done,
            Err(error) => StepOutcome::Failed(error.to_string()),
        }
    }
}

/// Outcome of [`start_round`]. Step 1 succeeded — otherwise this value would not
/// exist — so the round is real; the remaining fields say what else landed.
#[derive(Debug, Clone)]
pub struct StartRoundResult {
    /// Derived index of the round that was opened (always >= 2).
    pub round: u32,
    /// URL of the round comment: the round's identity.
    pub round_comment_url: String,
    /// The anchor the round opened at (HEAD at open time).
    pub anchor: ObjectId,
    /// The branch the round was opened on, recorded in its round comment.
    pub branch: String,
    /// The approval this round builds on, as the fold derives it.
    pub previous_approval: ObjectId,
    /// The branch the round that granted `previous_approval` was reviewed on.
    pub previous_branch: String,
    /// What the notification diff compared against: the previous approval, unless it
    /// was unreachable from `branch`.
    pub comparison_base: ObjectId,
    /// How `previous_approval` relates to `anchor` — the same fact the gap this round
    /// closes will carry once the round comment lands.
    pub continuity: GapContinuity,
    /// Step 2: reopening the issue.
    pub reopened: StepOutcome,
    /// Step 3: refreshing the `## QC Round` block in the issue body.
    pub body_marker: StepOutcome,
    /// Step 4: the `# QC Notification` comment.
    pub notification: StepOutcome,
    /// Direct downstream issues, one layer deep. Display only: this action never
    /// comments on a downstream issue.
    pub impacted_issues: ImpactedIssues,
}

impl StartRoundResult {
    /// Whether any of steps 2-4 failed and therefore wants a retry.
    pub fn needs_repair(&self) -> bool {
        self.reopened.failed() || self.body_marker.failed() || self.notification.failed()
    }
}

impl fmt::Display for StartRoundResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "🔄 Round {} started!", self.round)?;
        writeln!(f, "{}", self.round_comment_url)?;
        writeln!(f, "  Branch: {}", self.branch)?;

        // Reported before the per-step outcomes: it changes what the diff in the
        // notification actually means, so it is not a footnote.
        // D5 says a branch is always declared, so an empty one means the previous
        // round's comment was malformed. Naming that beats rendering `(on '')`.
        let previous_branch = if self.previous_branch.is_empty() {
            Cow::Borrowed("an undeclared branch")
        } else {
            Cow::Owned(format!("'{}'", self.previous_branch))
        };
        match self.continuity {
            GapContinuity::Linear => {}
            GapContinuity::Diverged { merge_base } => writeln!(
                f,
                "  ⚠️ The previous approval {} (on {previous_branch}) is not an ancestor of '{}'. \
                 Compared against their common ancestor {merge_base} instead.",
                self.previous_approval, self.branch,
            )?,
            GapContinuity::Unrelated => writeln!(
                f,
                "  ⚠️ The previous approval (on {previous_branch}) shares no history with '{}'. \
                 The comparison is not meaningful; review the file directly.",
                self.branch,
            )?,
        }

        let step = |f: &mut fmt::Formatter<'_>, label: &str, outcome: &StepOutcome| match outcome {
            StepOutcome::Done => writeln!(f, "  ✅ {label}"),
            StepOutcome::Skipped => writeln!(f, "  ⏭️ {label} (skipped)"),
            StepOutcome::Failed(error) => writeln!(f, "  ⚠️ {label} failed: {error}"),
        };
        step(f, "Issue set back to open", &self.reopened)?;
        step(f, "Issue body round marker updated", &self.body_marker)?;
        step(f, "QC notification posted", &self.notification)?;

        if self.needs_repair() {
            writeln!(
                f,
                "\n⚠️ The round itself was recorded; the step(s) above can be retried safely with \
                 `ghqc issue repair-round`, which re-runs only what is still incomplete. Do not \
                 re-run the new-round action itself — it would extend the round rather than open \
                 another one."
            )?;
        }

        match &self.impacted_issues {
            ImpactedIssues::None => {}
            ImpactedIssues::ApiUnavailable => {
                writeln!(
                    f,
                    "\n⚠️ Could not check which QCs depend on this file (API may not be supported)"
                )?;
            }
            ImpactedIssues::Some(nodes) => {
                writeln!(
                    f,
                    "\nFor your information — these QCs depend on this file. The previous round's \
                     approval still stands, so they remain valid; nothing was written to them:"
                )?;
                for node in nodes {
                    node.fmt_tree_root(f)?;
                }
            }
        }
        Ok(())
    }
}

/// Why a new round could not be started. Returned only when nothing was written.
#[derive(Debug, thiserror::Error)]
pub enum StartRoundError {
    #[error(
        "Cannot start a new round: {round_name} is still open. Approve it first — a `# QC Round` comment posted now would extend that round instead of opening a new one."
    )]
    RoundStillOpen { round: u32, round_name: String },
    #[error(
        "Cannot start a new round: no QC rounds could be derived for this issue (its commit history may be unreachable)"
    )]
    NoRounds,
    #[error("Could not resolve HEAD of branch '{branch}': {source}")]
    AnchorUnresolved {
        branch: String,
        #[source]
        source: GitFileOpsError,
    },
    #[error("GitHub API error: {0}")]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Could not determine the current branch: {0}")]
    BranchUnresolved(String),
}

/// Where a new round would anchor, and what its diff compares against.
///
/// Derived, never accepted from a caller, and shared by [`start_round`] and the seed
/// endpoint so what the form previews is what the action performs.
#[derive(Debug, Clone)]
pub struct RoundBasis {
    /// The branch the round opens on: whatever is currently checked out.
    pub branch: String,
    /// HEAD of `branch`.
    pub anchor: ObjectId,
    /// The previous round's approval — the round's base as the fold derives it.
    pub previous_approval: ObjectId,
    /// The branch the round that granted `previous_approval` was reviewed on.
    pub previous_branch: String,
    /// What the diff actually compares against: `previous_approval` normally, or the
    /// merge-base when that approval is not an ancestor of `anchor`.
    pub comparison_base: ObjectId,
    /// How `previous_approval` relates to `anchor`: [`GapContinuity::Linear`] unless
    /// the approval is unreachable from it.
    pub continuity: GapContinuity,
}

/// Resolve the branch, anchor and comparison base for the next round on `thread`.
///
/// The branch is the one currently checked out, not the issue body's: a round is QC'd
/// where the work is now. The anchor is HEAD of it, read from the repository rather
/// than accepted from a caller, so a round can never claim a commit nobody was at.
pub fn round_basis<T>(thread: &IssueThread, git_info: &T) -> Result<RoundBasis, StartRoundError>
where
    T: GitCommitOps + GitRepository,
{
    let last = thread
        .rounds()
        .next_back()
        .ok_or(StartRoundError::NoRounds)?;
    let previous_approval = match &last.state {
        RoundState::Closed { commit, .. } => *commit,
        RoundState::Open => {
            return Err(StartRoundError::RoundStillOpen {
                round: last.index,
                round_name: last.name(),
            });
        }
    };

    let branch = git_info
        .branch()
        .map_err(|error| StartRoundError::BranchUnresolved(error.to_string()))?;
    let anchor = git_info
        .branch_tip(&Some(branch.clone()))
        .map_err(|source| StartRoundError::AnchorUnresolved {
            branch: branch.clone(),
            source,
        })?;

    // `merge_base(a, b) == Some(a)` *is* the ancestry test — see `GitCommitOps`.
    // A lookup failure is treated as "no divergence": the round must not be blocked or
    // silently re-based because an ancestry query failed.
    let base = git_info
        .merge_base(&previous_approval, &anchor)
        .unwrap_or_else(|error| {
            log::warn!("merge-base lookup failed, assuming no divergence: {error}");
            Some(previous_approval)
        });

    // The same fact the gap this round closes carries, so the write path and the fold
    // describe divergence with one type rather than two.
    let continuity = match base {
        Some(base) if base == previous_approval => GapContinuity::Linear,
        Some(merge_base) => GapContinuity::Diverged { merge_base },
        None => GapContinuity::Unrelated,
    };
    if continuity != GapContinuity::Linear {
        log::warn!(
            "{}'s approval {previous_approval} is not an ancestor of {branch}@{anchor}; comparing \
             against {continuity:?} instead",
            last.name(),
        );
    }

    Ok(RoundBasis {
        branch,
        anchor,
        previous_approval,
        // The branch of the round that granted the approval we may be diverging from.
        previous_branch: last.branch.clone(),
        // No common ancestor at all leaves nothing better to compare against; the
        // divergence is reported so the caller can say so.
        comparison_base: base.unwrap_or(previous_approval),
        continuity,
    })
}

/// Start a new QC round on `thread`'s issue.
///
/// `thread` supplies the derived state the round is built on: the last round's
/// index and closing commit, and the branch whose HEAD becomes the anchor. It must
/// be the thread of `request.issue`.
///
/// Errors only when nothing has been written: the last round is still open, no
/// round could be derived, HEAD could not be resolved, or step 1 itself failed.
/// Everything after step 1 is reported in [`StartRoundResult`].
pub async fn start_round<T>(
    request: &StartRoundRequest,
    thread: &IssueThread,
    git_info: &T,
) -> Result<StartRoundResult, StartRoundError>
where
    T: GitHubWriter + GitHubReader + GitCommitOps + GitRepository + Sync,
{
    let last = thread
        .rounds()
        .next_back()
        .ok_or(StartRoundError::NoRounds)?;
    // Derived exactly as the fold derives it, so the written `round:` agrees and
    // the comment opens a round rather than extending one.
    let round = last.index.saturating_add(1);

    // Branch, anchor and comparison base together, so the form's preview and this
    // action can never disagree about where the round lands.
    let basis = round_basis(thread, git_info)?;
    let RoundBasis {
        branch,
        anchor,
        previous_approval,
        previous_branch,
        comparison_base,
        continuity,
    } = basis;

    // ── Step 1: the round comment. The only hard failure. ───────────────────
    let new_round = QCNewRound {
        issue: request.issue.clone(),
        round,
        round_commit: anchor,
        previous_approved_commit: previous_approval,
        checklist_name: request.checklist_name.clone(),
        note: request.note.clone(),
        branch: branch.clone(),
        checklist_content: request.checklist_content.clone(),
    };
    let round_comment_url = git_info.post_comment(&new_round).await?;
    log::info!(
        "opened round {round} on issue #{} at {anchor}",
        request.issue.number
    );

    // ── Step 2: reopen the issue (idempotent PATCH). ─────────────────────────
    let reopened = reopen_step(request.issue.number, git_info).await;

    // ── Step 3: refresh the denormalised `## QC Round` block. ────────────────
    let body_marker = body_marker_step(
        &request.issue,
        &RoundMarker {
            round,
            comment_url: round_comment_url.clone(),
        },
        git_info,
    )
    .await;

    // ── Step 4: notify, unless the caller asked us not to. ───────────────────
    let notification = notification_step(
        &thread.file,
        &request.issue,
        anchor,
        // The comparison base, which is the approval itself unless it is unreachable
        // from this branch — diffing against a commit that is not an ancestor would
        // describe changes that are not this round's.
        Some(comparison_base),
        // The notification's own note, not the round's: this is the comment
        // reviewers are @-mentioned in, so it carries what the author wants to tell
        // them rather than the audit reason recorded on the round comment.
        request.notification_note.clone(),
        request.notification,
        git_info,
    )
    .await;

    let impacted_issues = direct_impact(request.issue.number, git_info).await;

    Ok(StartRoundResult {
        round,
        round_comment_url,
        anchor,
        branch,
        previous_approval,
        previous_branch,
        comparison_base,
        continuity,
        reopened,
        body_marker,
        notification,
        impacted_issues,
    })
}

// ── The recoverable steps, shared by the start and repair paths ──────────────

/// Reopen the issue. Idempotent: `open_issue` on an already-open issue is a no-op
/// PATCH, so this may be re-run freely.
pub(crate) async fn reopen_step<T: GitHubWriter + Sync>(
    issue_number: u64,
    git_info: &T,
) -> StepOutcome {
    let outcome = StepOutcome::from_result(git_info.open_issue(issue_number).await);
    if let StepOutcome::Failed(error) = &outcome {
        log::warn!("reopening issue #{issue_number} failed: {error}");
    }
    outcome
}

/// Refresh the denormalised `## QC Round` cache block in the issue body.
/// Idempotent: [`upsert_round_marker`] replaces the block in place, so writing the
/// same marker twice produces the same body.
pub(crate) async fn body_marker_step<T: GitHubWriter + Sync>(
    issue: &Issue,
    marker: &RoundMarker,
    git_info: &T,
) -> StepOutcome {
    let new_body = upsert_round_marker(issue.body.as_deref().unwrap_or_default(), marker);
    let outcome = StepOutcome::from_result(
        git_info
            .update_issue(issue.number, None, Some(new_body))
            .await,
    );
    if let StepOutcome::Failed(error) = &outcome {
        log::warn!("updating issue #{}'s body failed: {error}", issue.number);
    }
    outcome
}

/// Post the `# QC Notification` for `previous_commit → current_commit`.
///
/// [`NotificationMode::None`] is [`StepOutcome::Skipped`]: choosing not to notify
/// is a decision, never a defect, and this is the single place that honours it.
pub(crate) async fn notification_step<T: GitHubWriter + Sync>(
    file: &Path,
    issue: &Issue,
    current_commit: ObjectId,
    previous_commit: Option<ObjectId>,
    note: Option<String>,
    mode: NotificationMode,
    git_info: &T,
) -> StepOutcome {
    if mode == NotificationMode::None {
        return StepOutcome::Skipped;
    }
    let comment = QCComment {
        file: file.to_path_buf(),
        issue: issue.clone(),
        current_commit,
        previous_commit,
        note,
        no_diff: mode == NotificationMode::MetadataOnly,
    };
    let outcome = StepOutcome::from_result(git_info.post_comment(&comment).await.map(|_| ()));
    if let StepOutcome::Failed(error) = &outcome {
        log::warn!("notifying issue #{} failed: {error}", issue.number);
    }
    outcome
}

/// Direct downstream issues only — one layer, no recursion, and deliberately no
/// writes: a new round invalidates *this* issue's approval, and it is the reviewer,
/// not this action, who decides what that means downstream.
async fn direct_impact<T: GitHubReader + Sync>(issue_number: u64, git_info: &T) -> ImpactedIssues {
    match git_info.get_blocked_issues(issue_number).await {
        Ok(blocked) if blocked.is_empty() => ImpactedIssues::None,
        Ok(blocked) => ImpactedIssues::Some(
            blocked
                .into_iter()
                .map(|blocked_issue| ImpactNode {
                    issue_number: blocked_issue.number,
                    file_name: extract_file_from_title(&blocked_issue.title),
                    milestone: blocked_issue
                        .milestone
                        .as_ref()
                        .map(|milestone| milestone.title.clone())
                        .unwrap_or_else(|| "No milestone".to_string()),
                    relationship: blocked_issue
                        .body
                        .as_ref()
                        .map(|body| determine_relationship_from_body(body, issue_number))
                        .unwrap_or(BlockingRelationship::Unknown),
                    children: Vec::new(),
                    fetch_error: None,
                })
                .collect(),
        ),
        Err(error) => {
            log::debug!("could not check issues blocked by #{issue_number}: {error}");
            ImpactedIssues::ApiUnavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comment_system::CommentBody;
    use crate::git::{
        GitAuthor, GitComment, GitCommit, GitFileOps, GitFileOpsError, GitHelpers,
        MockGitHubReader, MockGitHubWriter, RepoUser,
    };
    use crate::issue::IssueCommit;
    use crate::round::{
        BranchWalks, ChecklistSource, Gap, Placement, Round, RoundOpen, Segment,
        fold_rounds_from_comments, resolve_segments, round_branches,
    };
    use octocrab::models::Milestone;
    use std::collections::{HashMap, HashSet};
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    /// One `git_info` implementing every trait `start_round` needs: the GitHub
    /// halves are mockall mocks (so calls and their order are asserted), while the
    /// repository half is a fixed HEAD.
    struct MockGit {
        writer: MockGitHubWriter,
        reader: MockGitHubReader,
        tip: ObjectId,
        branch: String,
        /// What `merge_base` reports. `None` means the default: the first argument,
        /// i.e. the previous approval *is* an ancestor of the anchor — no divergence.
        merge_base: Option<Option<ObjectId>>,
    }

    impl MockGit {
        fn new() -> Self {
            Self {
                writer: MockGitHubWriter::new(),
                reader: MockGitHubReader::new(),
                tip: oid(C),
                branch: "main".to_string(),
                merge_base: None,
            }
        }

        /// The previous approval is not an ancestor of the anchor; the histories meet
        /// at `base` (or nowhere, for `None`).
        fn diverged(mut self, base: Option<ObjectId>) -> Self {
            self.merge_base = Some(base);
            self
        }

        fn on_branch(mut self, branch: &str) -> Self {
            self.branch = branch.to_string();
            self
        }

        /// No downstream issues: the impact lookup is display-only noise here.
        fn without_downstream(mut self) -> Self {
            self.reader
                .expect_get_blocked_issues()
                .returning(|_| Box::pin(async { Ok(Vec::new()) }));
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

    impl GitHubReader for MockGit {
        fn get_milestones(
            &self,
        ) -> impl Future<Output = Result<Vec<Milestone>, GitHubApiError>> + Send {
            self.reader.get_milestones()
        }
        fn get_issues(
            &self,
            milestone: Option<u64>,
        ) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send {
            self.reader.get_issues(milestone)
        }
        fn get_issue(
            &self,
            issue_number: u64,
        ) -> impl Future<Output = Result<Issue, GitHubApiError>> + Send {
            self.reader.get_issue(issue_number)
        }
        fn get_assignees(
            &self,
        ) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
            self.reader.get_assignees()
        }
        fn get_user_details(
            &self,
            username: &str,
        ) -> impl Future<Output = Result<RepoUser, GitHubApiError>> + Send {
            self.reader.get_user_details(username)
        }
        fn get_labels(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
            self.reader.get_labels()
        }
        fn get_issue_comments(
            &self,
            issue: &Issue,
        ) -> impl Future<Output = Result<Vec<GitComment>, GitHubApiError>> + Send {
            self.reader.get_issue_comments(issue)
        }
        fn get_issue_events(
            &self,
            issue: &Issue,
        ) -> impl Future<Output = Result<Vec<serde_json::Value>, GitHubApiError>> + Send {
            self.reader.get_issue_events(issue)
        }
        fn get_blocked_issues(
            &self,
            issue_number: u64,
        ) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send {
            self.reader.get_blocked_issues(issue_number)
        }
        fn get_current_user(
            &self,
        ) -> impl Future<Output = Result<Option<String>, GitHubApiError>> + Send {
            self.reader.get_current_user()
        }
    }

    impl GitRepository for MockGit {
        fn branch(&self) -> Result<String, crate::git::GitRepositoryError> {
            Ok(self.branch.clone())
        }
        fn commit(&self) -> Result<String, crate::git::GitRepositoryError> {
            Ok(self.tip.to_string())
        }
        fn owner(&self) -> &str {
            "o"
        }
        fn repo(&self) -> &str {
            "r"
        }
        fn remote_name(&self) -> &str {
            "origin"
        }
        fn path(&self) -> &Path {
            Path::new(".")
        }
        fn fetch(&self) -> Result<bool, crate::git::GitRepositoryError> {
            Ok(false)
        }
        fn stash_file(
            &self,
            _file: &Path,
            _message: &str,
        ) -> Result<crate::git::FileStashOutcome, crate::git::GitRepositoryError> {
            unimplemented!("start_round never stashes")
        }
        fn configured_author(&self) -> Option<crate::git::GitAuthor> {
            None
        }
    }

    impl GitCommitOps for MockGit {
        fn merge_base(
            &self,
            a: &ObjectId,
            _b: &ObjectId,
        ) -> Result<Option<ObjectId>, GitFileOpsError> {
            Ok(self.merge_base.unwrap_or(Some(*a)))
        }
        fn commits(
            &self,
            _branch: &Option<String>,
            _stop_at: Option<ObjectId>,
        ) -> Result<Vec<GitCommit>, GitFileOpsError> {
            unimplemented!("start_round never walks history")
        }
        fn branch_tip(&self, _branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
            Ok(self.tip)
        }
        fn file_touching_commits(
            &self,
            _branch: Option<String>,
            _file: &Path,
        ) -> Result<HashSet<String>, GitFileOpsError> {
            unimplemented!()
        }
        fn get_branches_containing_commit(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<String>, GitFileOpsError> {
            unimplemented!()
        }
        fn find_merged_into_branch(
            &self,
            _target_commit: &ObjectId,
        ) -> Result<Option<String>, GitFileOpsError> {
            unimplemented!()
        }
    }

    /// Renders comment bodies in tests, exactly as the `new_round` tests do.
    struct MockGitHelpers;

    impl GitHelpers for MockGitHelpers {
        fn file_content_url(&self, commit_sha: &str, file: &Path) -> String {
            format!(
                "https://github.com/o/r/blob/{commit_sha}/{}",
                file.display()
            )
        }
        fn commit_comparison_url(&self, _current: &ObjectId, _previous: &ObjectId) -> String {
            "https://github.com/o/r/compare/a..b".to_string()
        }
        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/o/r/issues/{issue_number}")
        }
    }

    impl GitFileOps for MockGitHelpers {
        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }
        fn file_bytes_at_commit(
            &self,
            _file: &Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }
        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    fn load_issue() -> Issue {
        let json =
            std::fs::read_to_string("src/tests/github_api/issues/main_file_issue.json").unwrap();
        serde_json::from_str(&json).unwrap()
    }

    fn commit(sha: &str) -> IssueCommit {
        IssueCommit {
            hash: oid(sha),
            message: format!("commit {sha}"),
            file_changed: true,
        }
    }

    /// `shas` oldest-first, as a segment's newest-first commit list.
    fn issue_commits(shas: &[&str]) -> Vec<IssueCommit> {
        shas.iter().rev().map(|sha| commit(sha)).collect()
    }

    fn initial_qc(state: RoundState, commits: Vec<IssueCommit>) -> Round {
        Round {
            index: 1,
            opened_at: oid(A),
            branch: "main".to_string(),
            opened: RoundOpen::IssueCreated,
            checklist: ChecklistSource::IssueBody,
            checklist_name: Some("Code Review Checklist".to_string()),
            state,
            events: Vec::new(),
            retractions: Vec::new(),
            extensions: Vec::new(),
            commits,
            placement: Placement::Placed,
        }
    }

    /// A closed round on a named branch, for threads with more than one.
    fn closed_round(
        index: u32,
        opened_at: &str,
        branch: &str,
        closed_at: &str,
        commits: Vec<IssueCommit>,
    ) -> Round {
        Round {
            index,
            opened_at: oid(opened_at),
            branch: branch.to_string(),
            state: RoundState::Closed {
                commit: oid(closed_at),
                by: "reviewer".to_string(),
                at: chrono::Utc::now(),
                comment_index: index as usize,
                comment_id: None,
                comment_url: None,
            },
            ..initial_qc(RoundState::Open, commits)
        }
    }

    /// A thread whose only round is Initial QC, closed at `B` — leaving `C` in the
    /// trailing gap — or left open, in which case it owns `C` itself.
    fn thread(open: bool) -> IssueThread {
        let segments = if open {
            vec![Segment::Round(initial_qc(
                RoundState::Open,
                issue_commits(&[A, B, C]),
            ))]
        } else {
            vec![
                Segment::Round(initial_qc(
                    RoundState::Closed {
                        commit: oid(B),
                        by: "reviewer".to_string(),
                        at: chrono::Utc::now(),
                        comment_index: 0,
                        comment_id: None,
                        comment_url: None,
                    },
                    issue_commits(&[A, B]),
                )),
                Segment::Gap(Gap {
                    branch: "main".to_string(),
                    commits: vec![commit(C)],
                    continuity: GapContinuity::Linear,
                    placement: Placement::Placed,
                }),
            ]
        };
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            open: !open,
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies: Vec::new(),
        }
    }

    fn request(notification: NotificationMode) -> StartRoundRequest {
        StartRoundRequest {
            issue: load_issue(),
            checklist_content: "- [ ] item one\n- [ ] item two".to_string(),
            checklist_name: Some("Code Review Checklist".to_string()),
            note: Some("Second pass after the refactor.".to_string()),
            notification_note: Some("Please re-check the covariate block.".to_string()),
            notification,
        }
    }

    const URL: &str = "https://github.com/o/r/issues/1#issuecomment-1";

    fn api_error() -> GitHubApiError {
        GitHubApiError::NoApi
    }

    // ── Preconditions ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn open_last_round_is_rejected_without_writing_anything() {
        let mut git = MockGit::new();
        // Nothing at all may be written.
        git.writer.expect_post_comment::<QCNewRound>().times(0);
        git.writer.expect_post_comment::<QCComment>().times(0);
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        git.reader.expect_get_blocked_issues().times(0);

        let error = start_round(&request(NotificationMode::Full), &thread(true), &git)
            .await
            .expect_err("an open round must be rejected");
        assert!(
            matches!(error, StartRoundError::RoundStillOpen { round: 1, .. }),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("still open"));
    }

    #[tokio::test]
    async fn a_thread_without_rounds_is_rejected() {
        let mut git = MockGit::new();
        git.writer.expect_post_comment::<QCNewRound>().times(0);
        let mut thread = thread(false);
        thread.segments.clear();

        let error = start_round(&request(NotificationMode::None), &thread, &git)
            .await
            .expect_err("no rounds means nothing to build on");
        assert!(matches!(error, StartRoundError::NoRounds));
    }

    // ── Happy paths ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn full_notification_runs_all_four_steps_in_order() {
        let mut sequence = mockall::Sequence::new();
        let mut git = MockGit::new().without_downstream();

        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .in_sequence(&mut sequence)
            .withf(|round: &QCNewRound| {
                round.round == 2
                    && round.round_commit == oid(C)
                    && round.previous_approved_commit == oid(B)
                    && round.checklist_name.as_deref() == Some("Code Review Checklist")
                    // The audit reason, not the message addressed to the reviewer.
                    && round.note.as_deref() == Some("Second pass after the refactor.")
            })
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .times(1)
            .in_sequence(&mut sequence)
            .withf(|_, title, body| {
                title.is_none()
                    && body.as_deref().is_some_and(|body| {
                        body.contains("## QC Round")
                            && body.contains("* current round: 2")
                            && body.contains(&format!("* round comment: {URL}"))
                            && body.contains("> Checklist below is from Initial QC")
                    })
            })
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .in_sequence(&mut sequence)
            .withf(|comment: &QCComment| {
                !comment.no_diff
                    && comment.current_commit == oid(C)
                    && comment.previous_commit == Some(oid(B))
                    // The reviewer-facing message, not the round's audit reason.
                    && comment.note.as_deref() == Some("Please re-check the covariate block.")
            })
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("all four steps succeed");

        assert_eq!(result.round, 2);
        assert_eq!(result.round_comment_url, URL);
        assert_eq!(result.anchor, oid(C));
        assert_eq!(result.reopened, StepOutcome::Done);
        assert_eq!(result.body_marker, StepOutcome::Done);
        assert_eq!(result.notification, StepOutcome::Done);
        assert!(!result.needs_repair());
        assert!(matches!(result.impacted_issues, ImpactedIssues::None));
    }

    // ── Branches ─────────────────────────────────────────────────────────────

    /// A round is QC'd where the work is *now*, which need not be where the issue was
    /// created: analysis moves between branches. The round comment records the branch
    /// it was actually reviewed on; the issue body is never rewritten.
    #[tokio::test]
    async fn the_round_records_the_checked_out_branch_not_the_issue_bodys() {
        let mut git = MockGit::new()
            .without_downstream()
            .on_branch("feature/reanalysis");
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .withf(|round: &QCNewRound| round.branch == "feature/reanalysis")
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        // The thread's own branch is `main` — deliberately different.
        let thread = thread(false);
        assert_eq!(thread.active_branch(), "main");

        let result = start_round(&request(NotificationMode::None), &thread, &git)
            .await
            .expect("the round opens on the checked-out branch");

        assert_eq!(result.branch, "feature/reanalysis");
        assert_eq!(
            result.continuity,
            GapContinuity::Linear,
            "an ancestor approval is not divergence"
        );
        assert_eq!(result.comparison_base, oid(B), "the approval itself");
    }

    /// The common case for cross-branch QC: the older round's branch was merged into
    /// this one, so its approval is an ancestor of the new tip and nothing changes.
    #[tokio::test]
    async fn a_merged_in_approval_is_not_a_divergence() {
        let mut git = MockGit::new()
            .without_downstream()
            .on_branch("feature/reanalysis");
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            // Compared against the approval, because it is reachable.
            .withf(|comment: &QCComment| comment.previous_commit == Some(oid(B)))
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("the round opens");
        assert_eq!(result.continuity, GapContinuity::Linear);
    }

    /// The divergent case: the previous branch was never merged in, so its approval is
    /// not an ancestor of this tip. Diffing against it would describe changes that are
    /// not this round's, so the comparison falls back to the common ancestor — and the
    /// round still opens, because a QC round is a human judgement, not a git property.
    #[tokio::test]
    async fn a_divergent_approval_is_compared_against_the_merge_base() {
        let mut git = MockGit::new()
            .without_downstream()
            .on_branch("feature/reanalysis")
            .diverged(Some(oid(A)));
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            // The merge-base, not the unreachable approval.
            .withf(|comment: &QCComment| comment.previous_commit == Some(oid(A)))
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("divergence does not block the round");

        assert_eq!(result.comparison_base, oid(A));
        assert_eq!(
            result.continuity,
            GapContinuity::Diverged { merge_base: oid(A) }
        );
        // The round comment still anchors at HEAD and records the round's real base.
        assert_eq!(result.anchor, oid(C));

        // The whole sentence, not just its shape. Every value in it has a near neighbour
        // that would read just as plausibly and be wrong: the *comparison base* is the
        // merge-base, not the approval, and the round's own branch is not the branch the
        // approval was granted on.
        assert_eq!(
            result.to_string().lines().nth(3).unwrap(),
            format!(
                "  ⚠️ The previous approval {} (on 'main') is not an ancestor of \
                 'feature/reanalysis'. Compared against their common ancestor {} instead.",
                oid(B),
                oid(A),
            )
        );
    }

    /// Wholly unrelated histories share no ancestor at all. There is nothing better to
    /// compare against, so the approval stands as the base and the report says the
    /// comparison is not meaningful rather than implying the diff is trustworthy.
    #[tokio::test]
    async fn unrelated_histories_report_that_no_comparison_is_meaningful() {
        let mut git = MockGit::new()
            .without_downstream()
            .on_branch("orphan")
            .diverged(None);
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        let result = start_round(&request(NotificationMode::None), &thread(false), &git)
            .await
            .expect("the round still opens");

        assert_eq!(result.comparison_base, oid(B), "nothing better to use");
        assert_eq!(result.continuity, GapContinuity::Unrelated);

        // `previous_branch` is `main` and the round's own branch is `orphan`, so the
        // sentence discriminates: rendering the round's branch here would read
        // "(on 'orphan') shares no history with 'orphan'".
        assert_eq!(
            result.to_string().lines().nth(3).unwrap(),
            "  ⚠️ The previous approval (on 'main') shares no history with 'orphan'. The \
             comparison is not meaningful; review the file directly."
        );
    }

    /// `round_basis` builds on the *newest* round. Every other fixture here has exactly
    /// one round, which makes newest and oldest the same value and leaves the choice
    /// untested — reading the oldest round would open round 2 forever and diff against
    /// Initial QC's approval no matter how many rounds had closed since.
    #[tokio::test]
    async fn the_next_round_builds_on_the_newest_closed_round() {
        // [Initial QC(closed at A), Gap, Round 2(closed at B), Gap(C)] — the shape I3
        // guarantees after two completed rounds, the second QC'd on another branch.
        let mut thread = thread(false);
        thread.segments = vec![
            Segment::Round(closed_round(1, A, "main", A, issue_commits(&[A]))),
            Segment::Gap(Gap {
                branch: "main".to_string(),
                commits: Vec::new(),
                continuity: GapContinuity::Linear,
                placement: Placement::Placed,
            }),
            Segment::Round(closed_round(2, A, "release/1.0", B, issue_commits(&[B]))),
            Segment::Gap(Gap {
                branch: "release/1.0".to_string(),
                commits: vec![commit(C)],
                continuity: GapContinuity::Linear,
                placement: Placement::Placed,
            }),
        ];

        let basis = round_basis(&thread, &MockGit::new()).expect("both rounds are closed");
        assert_eq!(
            basis.previous_approval,
            oid(B),
            "round 2's approval, not round 1's"
        );
        assert_eq!(basis.previous_branch, "release/1.0");

        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .withf(|round: &QCNewRound| {
                round.round == 3 && round.previous_approved_commit == oid(B)
            })
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        let result = start_round(&request(NotificationMode::None), &thread, &git)
            .await
            .expect("the round opens");
        assert_eq!(result.round, 3);
        assert_eq!(result.previous_approval, oid(B));
    }

    /// D5 says a branch is always declared, so an empty `previous_branch` means the
    /// previous round's comment was malformed — reachable, since the fold degrades such a
    /// round rather than failing. Saying so beats `(on '')`.
    #[test]
    fn an_undeclared_previous_branch_is_named_rather_than_rendered_empty() {
        let mut result = StartRoundResult {
            round: 2,
            round_comment_url: URL.to_string(),
            anchor: oid(C),
            branch: "feature/x".to_string(),
            previous_approval: oid(B),
            previous_branch: String::new(),
            comparison_base: oid(A),
            continuity: GapContinuity::Diverged { merge_base: oid(A) },
            reopened: StepOutcome::Done,
            body_marker: StepOutcome::Done,
            notification: StepOutcome::Skipped,
            impacted_issues: ImpactedIssues::None,
        };

        let display = result.to_string();
        assert!(
            display.contains("(on an undeclared branch)"),
            "unexpected: {display}"
        );
        assert!(!display.contains("(on '')"), "unexpected: {display}");

        result.continuity = GapContinuity::Unrelated;
        let display = result.to_string();
        assert!(
            display.contains("(on an undeclared branch)"),
            "unexpected: {display}"
        );
        assert!(!display.contains("(on '')"), "unexpected: {display}");
    }

    /// The two notes answer different questions — "why does this round exist" is a
    /// permanent record, "here is what you need to know" is addressed to a person —
    /// so neither comment may carry the other's text. A single shared `note` used to
    /// go to both, which is what made the distinction impossible to express.
    #[tokio::test]
    async fn each_note_reaches_only_its_own_comment() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .withf(|round: &QCNewRound| {
                round.note.as_deref() == Some("Second pass after the refactor.")
            })
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .withf(|comment: &QCComment| {
                comment.note.as_deref() == Some("Please re-check the covariate block.")
            })
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("both comments post");
        assert_eq!(result.notification, StepOutcome::Done);
    }

    /// A round can be opened with an audit reason and no message for the reviewer.
    /// The notification must then carry nothing rather than falling back to the
    /// reason: the API is explicit, and only the CLI substitutes one for the other.
    #[tokio::test]
    async fn an_absent_notification_note_leaves_the_notification_noteless() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .withf(|comment: &QCComment| comment.note.is_none())
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let mut request = request(NotificationMode::Full);
        request.notification_note = None;
        assert!(request.note.is_some(), "the round's own note is unaffected");

        let result = start_round(&request, &thread(false), &git)
            .await
            .expect("the round still opens");
        assert_eq!(result.notification, StepOutcome::Done);
    }

    #[tokio::test]
    async fn metadata_only_notification_suppresses_the_diff() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .withf(|comment: &QCComment| comment.no_diff)
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(
            &request(NotificationMode::MetadataOnly),
            &thread(false),
            &git,
        )
        .await
        .expect("steps succeed");
        assert_eq!(result.notification, StepOutcome::Done);
    }

    #[tokio::test]
    async fn no_notification_posts_exactly_one_comment() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        // Zero notification comments.
        git.writer.expect_post_comment::<QCComment>().times(0);
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        let result = start_round(&request(NotificationMode::None), &thread(false), &git)
            .await
            .expect("steps succeed");
        assert_eq!(result.notification, StepOutcome::Skipped);
        assert!(!result.needs_repair());
    }

    // ── Partial failure ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn reopen_failure_still_updates_the_body_and_notifies() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
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
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("a step-2 failure is not a hard error");

        assert!(result.reopened.failed());
        assert_eq!(result.body_marker, StepOutcome::Done);
        assert_eq!(result.notification, StepOutcome::Done);
        assert!(result.needs_repair());
        let display = result.to_string();
        assert!(display.contains("Round 2 started!"));
        assert!(display.contains("Issue set back to open failed"));
        assert!(display.contains("retried safely"));
    }

    #[tokio::test]
    async fn body_update_failure_still_notifies() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Err(api_error()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .returning(|_| Box::pin(async { Ok(format!("{URL}2")) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("a step-3 failure is not a hard error");

        assert_eq!(result.reopened, StepOutcome::Done);
        assert!(result.body_marker.failed());
        assert_eq!(result.notification, StepOutcome::Done);
    }

    #[tokio::test]
    async fn notification_failure_leaves_the_round_and_marker_intact() {
        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.writer
            .expect_post_comment::<QCComment>()
            .times(1)
            .returning(|_| Box::pin(async { Err(api_error()) }));

        let result = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect("a step-4 failure is not a hard error");

        assert_eq!(result.round_comment_url, URL);
        assert_eq!(result.reopened, StepOutcome::Done);
        assert_eq!(result.body_marker, StepOutcome::Done);
        assert!(result.notification.failed());
    }

    #[tokio::test]
    async fn round_comment_failure_is_a_hard_error_and_stops_everything() {
        let mut git = MockGit::new();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .times(1)
            .returning(|_| Box::pin(async { Err(api_error()) }));
        git.writer.expect_open_issue().times(0);
        git.writer.expect_update_issue().times(0);
        git.writer.expect_post_comment::<QCComment>().times(0);
        git.reader.expect_get_blocked_issues().times(0);

        let error = start_round(&request(NotificationMode::Full), &thread(false), &git)
            .await
            .expect_err("step 1 is the only hard failure");
        assert!(matches!(error, StartRoundError::GitHubApiError(_)));
    }

    // ── Downstream impact ────────────────────────────────────────────────────

    #[tokio::test]
    async fn downstream_issues_are_reported_one_layer_deep_without_writes() {
        let mut git = MockGit::new();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        // Exactly one lookup: no recursion into the blocked issue's own children.
        git.reader
            .expect_get_blocked_issues()
            .times(1)
            .returning(|_| Box::pin(async { Ok(vec![load_issue()]) }));

        let result = start_round(&request(NotificationMode::None), &thread(false), &git)
            .await
            .expect("steps succeed");
        match &result.impacted_issues {
            ImpactedIssues::Some(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert!(nodes[0].children.is_empty(), "impact must not recurse");
            }
            other => panic!("expected downstream issues, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unavailable_dependency_api_is_reported_not_fatal() {
        let mut git = MockGit::new();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(|_| Box::pin(async { Ok(URL.to_string()) }));
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        git.reader
            .expect_get_blocked_issues()
            .returning(|_| Box::pin(async { Err(api_error()) }));

        let result = start_round(&request(NotificationMode::None), &thread(false), &git)
            .await
            .expect("an unavailable dependency API is display-only");
        assert!(matches!(
            result.impacted_issues,
            ImpactedIssues::ApiUnavailable
        ));
        assert!(
            result
                .to_string()
                .contains("Could not check which QCs depend on this file")
        );
    }

    // ── The key test: what we write is what the model reads ──────────────────

    #[tokio::test]
    async fn the_round_comment_we_post_folds_into_a_correct_round_two() {
        let posted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&posted);

        let mut git = MockGit::new().without_downstream();
        git.writer
            .expect_post_comment::<QCNewRound>()
            .returning(move |round: &QCNewRound| {
                captured
                    .lock()
                    .unwrap()
                    .push(round.generate_body(&MockGitHelpers));
                Box::pin(async { Ok(URL.to_string()) })
            });
        git.writer
            .expect_open_issue()
            .returning(|_| Box::pin(async { Ok(()) }));
        git.writer
            .expect_update_issue()
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        start_round(&request(NotificationMode::None), &thread(false), &git)
            .await
            .expect("steps succeed");

        let body = posted.lock().unwrap()[0].clone();
        let comment = |body: &str| GitComment {
            body: body.to_string(),
            author_login: "author".to_string(),
            created_at: chrono::Utc::now(),
            id: None,
            html_url: None,
            html: None,
        };
        // The thread as it will look once the round comment lands.
        let comments = vec![
            comment(&format!(
                "# QC Approval\n\n## Metadata\napproved qc commit: {B}\n"
            )),
            comment(&body),
        ];
        let (raw, raw_anomalies) =
            fold_rounds_from_comments(A, Some("Code Review Checklist"), &comments);
        // Both rounds declare `main` — the mock's checked-out branch — so the whole
        // thread is one walk.
        let branches = round_branches(&raw, "main");
        let mut walks: BranchWalks = HashMap::new();
        walks.insert("main".to_string(), Ok(issue_commits(&[A, B, C])));
        let (segments, anomalies) =
            resolve_segments(raw, &branches, &walks, &|a, _| Some(*a), raw_anomalies);

        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");
        // Round 1, the gap it left behind, and the round we just opened.
        assert_eq!(segments.len(), 3);
        let second = segments[2].as_round().expect("round 2 is the last segment");
        assert_eq!(second.index, 2);
        // The anchor is HEAD at open time; the base is derived, not written.
        assert_eq!(second.opened_at, oid(C));
        assert_eq!(
            segments[0]
                .as_round()
                .and_then(Round::closing_commit)
                .copied(),
            Some(oid(B)),
            "the approval round 2 builds on is Initial QC's, derived not written"
        );
        assert!(second.is_open());
        assert_eq!(second.branch, "main");
        assert_eq!(
            second.checklist_name.as_deref(),
            Some("Code Review Checklist")
        );
        assert!(
            matches!(
                &second.checklist,
                ChecklistSource::Comment {
                    comment_index: 1,
                    ..
                }
            ),
            "unexpected checklist source: {:?}",
            second.checklist
        );
        assert!(matches!(
            &second.opened,
            RoundOpen::NewRound {
                comment_index: 1,
                note: Some(note),
                ..
            } if note == "Second pass after the refactor."
        ));
    }

    #[test]
    fn step_outcome_display_covers_every_variant() {
        let result = StartRoundResult {
            branch: "main".to_string(),
            previous_approval: oid(B),
            previous_branch: "main".to_string(),
            comparison_base: oid(B),
            continuity: GapContinuity::Linear,
            round: 3,
            round_comment_url: URL.to_string(),
            anchor: oid(C),
            reopened: StepOutcome::Done,
            body_marker: StepOutcome::Failed("boom".to_string()),
            notification: StepOutcome::Skipped,
            impacted_issues: ImpactedIssues::Some(vec![ImpactNode {
                issue_number: 30,
                file_name: PathBuf::from("a/b.R"),
                milestone: "Sprint 1".to_string(),
                relationship: BlockingRelationship::PreviousQC,
                children: Vec::new(),
                fetch_error: None,
            }]),
        };
        let display = result.to_string();
        assert!(display.contains("Round 3 started!"));
        assert!(display.contains("Issue set back to open"));
        assert!(display.contains("boom"));
        assert!(display.contains("(skipped)"));
        assert!(display.contains("#30 a/b.R (Sprint 1) (previous QC)"));
    }

    /// P4: a new round is an *append*. Its impact list is a notice — the previous
    /// approval still stands and nothing was written downstream — and must stay
    /// clearly distinct from the unapproval copy in [`crate::approve`], which is an
    /// amend and does read as invalidation.
    #[test]
    fn new_round_impact_reads_as_a_notice_not_as_invalidation() {
        let display = StartRoundResult {
            branch: "main".to_string(),
            previous_approval: oid(B),
            previous_branch: "main".to_string(),
            comparison_base: oid(B),
            continuity: GapContinuity::Linear,
            round: 2,
            round_comment_url: URL.to_string(),
            anchor: oid(C),
            reopened: StepOutcome::Done,
            body_marker: StepOutcome::Done,
            notification: StepOutcome::Done,
            impacted_issues: ImpactedIssues::Some(vec![ImpactNode {
                issue_number: 30,
                file_name: PathBuf::from("a/b.R"),
                milestone: "Sprint 1".to_string(),
                relationship: BlockingRelationship::PreviousQC,
                children: Vec::new(),
                fetch_error: None,
            }]),
        }
        .to_string();

        assert!(display.contains("For your information"));
        assert!(display.contains("approval still stands"));
        assert!(display.contains("nothing was written to them"));
        // Never the unapproval wording: nothing here was invalidated.
        assert!(!display.to_lowercase().contains("unapprove"));
        assert!(!display.contains("no longer be valid"));
        assert!(!display.contains("redone"));
        // "re-open" collides with GitHub's own issue reopen — never in user copy.
        assert!(!display.to_lowercase().contains("reopen"));
        assert!(!display.to_lowercase().contains("re-open"));
    }
}
