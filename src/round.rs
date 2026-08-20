//! QC Rounds: an issue's comment thread as a sequence of *segments* — rounds and the
//! drift between them.
//!
//! A QC issue is a sequence of *rounds*. Round 1 is "Initial QC": it opens at the
//! `initial qc commit:` recorded in the issue body and closes when an approval is
//! posted. A later round is opened explicitly by a round comment, which
//! carries its own anchor commit (`initial qc round commit:`) and its own checklist.
//!
//! The fold trusts written metadata only for values it cannot derive from the
//! comment log. The anchor is such a value, so it is authoritative; the round
//! number and the approval a round builds on are both derivable, so they are always
//! derived. A round comment that cannot open a new round — because the
//! current round is still open, or because its written `round:` disagrees with the
//! derived number — instead *extends* the current round, so its author-written
//! checklist is never discarded.
//!
//! Metadata keys are only read inside a comment's `## Metadata` section, and
//! markers are only recognised at the start of a line.
//!
//! The segment list is the **single** representation of round scope: a [`Segment`] owns
//! its commits, so no consumer re-derives which commits belong to which round.
//! [`crate::QCStatus`] reads it directly.
//!
//! The fold is deliberately split into two stages so the interesting logic is
//! testable without a git repository:
//!
//! 1. [`fold_rounds_from_comments`] is a pure function over `&[GitComment]` that
//!    works on SHA strings exactly as they appear in the comments.
//! 2. [`resolve_segments`] resolves those strings against per-branch commit walks,
//!    using the same short-SHA prefix matching as `IssueThread::from_issue_comments`,
//!    and places every commit in exactly one segment.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use gix::ObjectId;

use crate::git::GitComment;
use crate::issue::IssueCommit;

/// Heading that opens a round comment, written as `# QC Round <n>`.
///
/// The trailing number is for readers only — the round's identity always comes from
/// the `round:` metadata line, never from this heading.
pub(crate) const ROUND_HEADING: &str = "# QC Round";
/// Comment marker for a QC notification.
pub(crate) const NOTIFICATION_MARKER: &str = "# QC Notification";
/// Comment marker for a QC review.
pub(crate) const REVIEW_MARKER: &str = "# QC Review";
/// Comment marker retracting the standing approval.
pub(crate) const UNAPPROVAL_MARKER: &str = "# QC Un-Approval";

/// Metadata key holding the notification commit.
pub(crate) const CURRENT_COMMIT_KEY: &str = "current commit: ";
/// Metadata key holding the approved commit. Approval is detected by the presence
/// of this key, mirroring `parse_commits_from_comments`.
pub(crate) const APPROVED_COMMIT_KEY: &str = "approved qc commit: ";
/// Metadata key holding the commit a review compared against.
pub(crate) const COMPARING_COMMIT_KEY: &str = "comparing commit: ";
/// Metadata key holding the 1-based round number of a round comment — the round's
/// identity, and the only place it is read from.
pub(crate) const ROUND_KEY: &str = "round: ";
/// Metadata key holding the anchor commit of a round comment — the commit the round
/// opened at. Named to mirror the issue body's `initial qc commit`.
pub(crate) const INITIAL_ROUND_COMMIT_KEY: &str = "initial qc round commit: ";
/// Metadata key holding the approval the new round builds on.
pub(crate) const PREVIOUS_APPROVED_COMMIT_KEY: &str = "previous approved commit: ";
/// Metadata key holding a free-text note on a round comment.
pub(crate) const NOTE_KEY: &str = "note: ";
/// Metadata key holding the branch a round was opened on. Named to match the issue
/// body's `git branch`, which records the same thing for Initial QC.
///
/// A round can be QC'd on a different branch than the one the issue was created on —
/// analysis moves, and older work is often merged into the branch the current round
/// lives on. This is recorded per round rather than by rewriting the issue body, so
/// each round keeps the branch it was actually reviewed on.
pub(crate) const GIT_BRANCH_KEY: &str = "git branch: ";

/// How a round came into being.
#[derive(Debug, Clone, PartialEq)]
pub enum RoundOpen {
    /// Round 1 ("Initial QC"), opened when the issue was created.
    IssueCreated,
    /// Round N > 1, opened by a round comment.
    NewRound {
        /// Position of the opening comment in the folded comment slice. Always
        /// available, and how the fold addresses the slice.
        comment_index: usize,
        /// GitHub's id for the opening comment. `None` when the comment came from
        /// a cache written before ids were recorded.
        comment_id: Option<u64>,
        /// Permalink to the opening comment. `None` for the same reason as
        /// `comment_id`.
        comment_url: Option<String>,
        author: String,
        at: DateTime<Utc>,
        note: Option<String>,
        /// The branch this round was opened on. `None` for rounds recorded before
        /// the branch was written, in which case the issue body's branch is the
        /// only thing known about it.
        branch: Option<String>,
    },
}

/// A round comment that could not open a new round and therefore
/// extended the current one instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    pub comment_index: usize,
    /// GitHub's id for the extending comment; `None` for cache-loaded comments.
    pub comment_id: Option<u64>,
    /// Permalink to the extending comment; `None` for cache-loaded comments.
    pub comment_url: Option<String>,
    pub by: String,
    pub at: DateTime<Utc>,
    /// The written `round commit:`, if present and resolvable. The round's own
    /// `opened_at` is deliberately left alone by an extension.
    pub at_commit: Option<ObjectId>,
    pub note: Option<String>,
}

/// Why a round comment extended the current round instead of opening
/// a new one.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionReason {
    /// The current round was still `Open`.
    RoundStillOpen,
    /// The written `round:` disagreed with the derived round number.
    IndexMismatch { written: u32, derived: u32 },
}

/// Where the round's checklist lives.
#[derive(Debug, Clone, PartialEq)]
pub enum ChecklistSource {
    /// Initial QC: the checklist is in the issue body.
    IssueBody,
    /// Round N > 1: the checklist is in the round comment.
    Comment {
        comment_index: usize,
        /// GitHub's id for that comment; `None` for cache-loaded comments.
        comment_id: Option<u64>,
        /// Permalink to that comment; `None` for cache-loaded comments.
        comment_url: Option<String>,
    },
}

/// Whether a round is still being reviewed, or has been closed by an approval.
#[derive(Debug, Clone, PartialEq)]
pub enum RoundState {
    Open,
    Closed {
        commit: ObjectId,
        by: String,
        at: DateTime<Utc>,
        comment_index: usize,
        /// GitHub's id for the approving comment; `None` for cache-loaded comments.
        comment_id: Option<u64>,
        /// Permalink to the approving comment; `None` for cache-loaded comments.
        comment_url: Option<String>,
    },
}

impl RoundState {
    /// The commit an approval closed this round at, if it is currently closed.
    pub fn closing_commit(&self) -> Option<&ObjectId> {
        match self {
            RoundState::Closed { commit, .. } => Some(commit),
            RoundState::Open => None,
        }
    }
}

/// A `# QC Un-Approval` that took back a previously recorded approval.
#[derive(Debug, Clone, PartialEq)]
pub struct Retraction {
    pub retracted_commit: ObjectId,
    pub by: String,
    pub at: DateTime<Utc>,
    pub comment_index: usize,
    /// GitHub's id for the un-approval comment; `None` for cache-loaded comments.
    pub comment_id: Option<u64>,
    /// Permalink to the un-approval comment; `None` for cache-loaded comments.
    pub comment_url: Option<String>,
}

/// Something that happened inside a round without closing it.
#[derive(Debug, Clone, PartialEq)]
pub enum RoundEvent {
    Notification {
        commit: ObjectId,
        by: String,
        at: DateTime<Utc>,
        comment_index: usize,
        /// GitHub's id for the comment; `None` for cache-loaded comments.
        comment_id: Option<u64>,
        /// Permalink to the comment; `None` for cache-loaded comments.
        comment_url: Option<String>,
    },
    Review {
        commit: ObjectId,
        by: String,
        at: DateTime<Utc>,
        comment_index: usize,
        /// GitHub's id for the comment; `None` for cache-loaded comments.
        comment_id: Option<u64>,
        /// Permalink to the comment; `None` for cache-loaded comments.
        comment_url: Option<String>,
    },
}

impl RoundEvent {
    /// The commit this event refers to.
    pub fn commit(&self) -> &ObjectId {
        match self {
            RoundEvent::Notification { commit, .. } | RoundEvent::Review { commit, .. } => commit,
        }
    }
}

/// A single QC round.
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    /// 1-based round number, always **derived** as the previous round's index plus
    /// one (`1` for Initial QC). A written `round:` that disagrees is not trusted;
    /// it extends the current round instead — see [`RoundAnomaly::RoundExtended`].
    /// Indices are therefore unique and monotonic.
    pub index: u32,
    /// The commit the file was at when this round opened — the round's anchor. Taken
    /// from the comment's `round commit:` (the one value the comment log cannot
    /// derive), and never moved by a later extension.
    ///
    /// The anchor belongs to this round, not to the gap before it: Initial QC owns the
    /// `initial qc commit` that starts it, and every later round is symmetric with it.
    pub opened_at: ObjectId,
    /// The branch this round was reviewed on: the issue body's `git branch` for Initial
    /// QC, the round comment's for every later round. Always resolved — a round comment
    /// without one is malformed input, not a legacy case, and leaves the round
    /// [`Placement::Unplaceable`] with an empty `branch`.
    pub branch: String,
    pub opened: RoundOpen,
    pub checklist: ChecklistSource,
    /// Name of the checklist template this round was QC'd against. Read from the
    /// opening comment's `checklist:` metadata (or, for Initial QC, from the
    /// issue body's checklist heading). `None` when nothing recorded it.
    pub checklist_name: Option<String>,
    pub state: RoundState,
    pub events: Vec<RoundEvent>,
    pub retractions: Vec<Retraction>,
    /// round comments that extended this round rather than opening a
    /// new one, oldest first.
    pub extensions: Vec<Extension>,
    /// The commits this round owns, newest-first: from its closing commit (or the
    /// branch tip while open) back to `opened_at`, inclusive.
    pub commits: Vec<IssueCommit>,
    pub placement: Placement,
}

impl Round {
    /// Human-readable name of the round.
    pub fn name(&self) -> String {
        Self::name_of(self.index)
    }

    /// Human-readable name of the round at `index`, without a round to hand.
    ///
    /// **The one authority for the naming rule** — `1` is `Initial QC`, every later round
    /// is `Round n` — which is a spec-pinned string, not a formatting preference.
    ///
    /// It takes a bare index because legitimate callers have nothing else: provenance read
    /// back from `ghqc_archive_metadata.json` carries `round` and `approval.round` as
    /// `u32` with no thread in scope, so a `&Round` cannot be required. Rendering that
    /// through a local copy of the rule is how the same string ends up spelled two ways,
    /// so the index-only shape lives here beside [`Self::name`] rather than in each
    /// surface.
    pub fn name_of(index: u32) -> String {
        if index == 1 {
            "Initial QC".to_string()
        } else {
            format!("Round {index}")
        }
    }

    /// The commit that closed this round, if it is currently closed.
    pub fn closing_commit(&self) -> Option<&ObjectId> {
        self.state.closing_commit()
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, RoundState::Open)
    }

    /// Whether this round's commits could be located at all.
    pub fn is_placed(&self) -> bool {
        matches!(self.placement, Placement::Placed)
    }

    /// This round's newest commit carrying an action — its anchor (`opened_at`, which
    /// for Initial QC is the `initial qc commit`), a notification, or a review — by
    /// position in the round's own commits. Commits nobody acted on are not candidates,
    /// so drift on top of the last notification is never reported here.
    ///
    /// `None` exactly when the round is unplaceable: it then owns no commits and its
    /// anchor is a null-OID placeholder, which is not a commit anyone may address.
    ///
    /// Not `commits[0]`, which is the newest commit full stop, and not
    /// [`Self::closing_commit`]: an approval is not a [`RoundEvent`], so a closed round
    /// can report an actioned commit *older* than the commit it closed at.
    ///
    /// **This is the one implementation of "the round's latest update commit."**
    /// [`crate::IssueThread::next_notification_from`]'s round branch is the same
    /// computation and calls this so the two cannot drift apart (archive-rounds **M4**).
    pub fn latest_actioned_commit(&self) -> Option<&IssueCommit> {
        // Stated rather than left to fall out of the empty candidate set below: an
        // unplaceable round owns no commits today, so the filter would return `None`
        // anyway, and this is the claim being made rather than a coincidence of it.
        if !self.is_placed() {
            return None;
        }
        // {opened_at} ∪ {e.commit}, newest by position in this round's own commits.
        self.events
            .iter()
            .map(RoundEvent::commit)
            .chain(std::iter::once(&self.opened_at))
            .filter_map(|hash| self.commit_position(hash))
            .min()
            .map(|position| &self.commits[position])
    }

    /// Position of `hash` in this round's commits (newest-first), if it owns it.
    pub fn commit_position(&self, hash: &ObjectId) -> Option<usize> {
        self.commits.iter().position(|commit| commit.hash == *hash)
    }
}

// ── Segments ────────────────────────────────────────────────────────────────

/// One span of an issue's history: a QC round, or the drift between two rounds.
///
/// An issue is a strict alternation of rounds and gaps, starting at Initial QC and
/// ending in either an open round or a gap. Because a segment *owns* its commits,
/// "which commits does this round cover" is a property of the data rather than
/// something every consumer re-derives.
///
/// A `Round` is much larger than a `Gap`, which is deliberate: boxing it to even the
/// variants out would buy nothing but an indirection on the hot path, and there are a
/// handful of segments per issue.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Round(Round),
    Gap(Gap),
}

impl Segment {
    /// The branch this segment was walked on.
    pub fn branch(&self) -> &str {
        match self {
            Segment::Round(round) => &round.branch,
            Segment::Gap(gap) => &gap.branch,
        }
    }

    /// The commits this segment owns, newest-first. Always empty when the segment
    /// could not be placed.
    pub fn commits(&self) -> &[IssueCommit] {
        match self {
            Segment::Round(round) => &round.commits,
            Segment::Gap(gap) => &gap.commits,
        }
    }

    pub fn placement(&self) -> &Placement {
        match self {
            Segment::Round(round) => &round.placement,
            Segment::Gap(gap) => &gap.placement,
        }
    }

    pub fn is_placed(&self) -> bool {
        matches!(self.placement(), Placement::Placed)
    }

    pub fn as_round(&self) -> Option<&Round> {
        match self {
            Segment::Round(round) => Some(round),
            Segment::Gap(_) => None,
        }
    }

    pub fn as_gap(&self) -> Option<&Gap> {
        match self {
            Segment::Gap(gap) => Some(gap),
            Segment::Round(_) => None,
        }
    }
}

/// The commits between one round's approval and the next round's anchor — or, when it
/// trails the last round, everything committed on that round's branch since its
/// approval.
///
/// Gaps are implied by adjacency rather than written anywhere, carry no identity, and
/// are legitimately empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    /// The branch this gap was walked on: the branch of the round bounding its newer
    /// end, or — for a trailing gap, which has no newer round — the branch of the round
    /// bounding its older end. Never the viewer's checkout, so status stays
    /// reproducible.
    pub branch: String,
    /// The commits this gap owns, newest-first.
    pub commits: Vec<IssueCommit>,
    pub continuity: GapContinuity,
    pub placement: Placement,
}

impl Gap {
    pub fn is_placed(&self) -> bool {
        matches!(self.placement, Placement::Placed)
    }
}

/// How a gap's two bounding commits relate in history.
///
/// Divergence is a property of the gap *between* rounds, not a special case of
/// starting one: a round QC'd on another branch simply bounds a gap whose ends do not
/// sit on one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapContinuity {
    /// The bounds are ancestrally connected — the normal case.
    Linear,
    /// The bounds diverge; the histories meet at `merge_base`, which is where the
    /// gap's walk stops.
    Diverged { merge_base: ObjectId },
    /// The bounds share no history, so no diff between them is meaningful and the gap
    /// owns nothing.
    Unrelated,
}

/// Whether a segment's commits could be located.
///
/// One degradation path for every failure mode: an unresolvable segment owns no
/// commits and renders grayed, rather than failing the whole issue or — worse —
/// showing plausible commits from somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Placed,
    /// `commits` is empty; the segment renders grayed.
    Unplaceable(UnplaceableReason),
}

impl Placement {
    pub fn is_placed(&self) -> bool {
        matches!(self, Placement::Placed)
    }
}

/// Why a segment could not be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnplaceableReason {
    /// The round comment declared no `git branch` — malformed input.
    BranchNotDeclared,
    /// The branch was not fetched, or has been deleted.
    BranchUnavailable,
    /// A bounding commit is not on the branch: force-push, or gc.
    AnchorUnreachable,
    /// A gap whose bounds both resolved, but whose common ancestor lies outside the
    /// walked range. Distinct from [`Self::AnchorUnreachable`]: nothing is missing, so
    /// the remedy is not to go looking for a commit. Walks stop at Initial QC, so this
    /// is what an ordinary later-round branch that forked *before* Initial QC produces.
    MergeBaseUnreachable,
    /// A gap whose bounding round could not be placed. Deliberately *not* walked on
    /// the other round's branch: that would produce a real commit set which is not
    /// this gap, and plausible-but-wrong commits are worse than none.
    NeighbourUnplaceable,
}

impl UnplaceableReason {
    /// Why the commits could not be located, in the user's terms and as a clause that
    /// reads after the segment it describes: *"Round 2 — its branch is unavailable
    /// locally"*.
    ///
    /// One wording for every surface: `ghqc issue status`'s trust marker, the repair's
    /// per-step report, and the API's skip reason all delegate here, so a user never
    /// sees the same degradation explained two ways.
    pub fn describe(self) -> &'static str {
        match self {
            UnplaceableReason::BranchNotDeclared => "its round comment declared no branch",
            UnplaceableReason::BranchUnavailable => "its branch is unavailable locally",
            UnplaceableReason::AnchorUnreachable => "its commits are not on that branch",
            UnplaceableReason::MergeBaseUnreachable => {
                "the histories it spans meet before Initial QC"
            }
            UnplaceableReason::NeighbourUnplaceable => "the round bounding it could not be placed",
        }
    }
}

/// A problem found while folding rounds. The fold never fails; it accumulates
/// anomalies instead so callers can surface them without losing the derived state.
/// Anomalies are diagnostics, so they identify comments by index only — no
/// comment id or URL is carried here.
#[derive(Debug, Clone, PartialEq)]
pub enum RoundAnomaly {
    /// A round comment could not open a new round, so it extended the
    /// current one instead. The comment's checklist is never discarded.
    RoundExtended {
        comment_index: usize,
        reason: ExtensionReason,
    },
    /// A `# QC Un-Approval` arrived with no standing approval to retract. Ignored.
    RetractWithNothingClosed { comment_index: usize },
    /// The written `previous approved commit:` disagreed with the previous round's
    /// closing commit. The derived value wins; this is warn-only.
    BaseCommitMismatch {
        comment_index: usize,
        written: String,
        derived: String,
    },
    /// A SHA referenced by a comment could not be resolved against the issue's
    /// commit list (stage 2 only).
    AnchorUnreachable { comment_index: usize, sha: String },
    /// A new round's resolved anchor was older than the approval it builds on,
    /// which would make adjacent rounds' memberships overlap. The anchor is
    /// replaced by `previous_approval` (stage 2 only).
    AnchorOlderThanBase {
        comment_index: usize,
        anchor: String,
        base: String,
    },
    /// A notification or review arrived after the round had already been closed.
    /// The event is still recorded.
    EventAfterClose { comment_index: usize },
    /// A round comment declared no `git branch`. Every round declares one, so this is
    /// malformed input rather than a legacy thread: nothing before round comments
    /// existed could produce it.
    BranchNotDeclared { comment_index: usize },
    /// A segment's commits could not be located, so it owns none. `position` is its
    /// index in the segment list.
    SegmentUnplaceable {
        position: usize,
        reason: UnplaceableReason,
    },
}

/// Stage 1 anomalies use the same shape as stage 2 ones.
///
/// Stage 1 never emits [`RoundAnomaly::AnchorUnreachable`]: resolving SHAs is
/// exclusively stage 2's job.
pub(crate) type RawAnomaly = RoundAnomaly;

// ── Stage 1: pure fold over comments ────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawRoundOpen<'a> {
    IssueCreated,
    NewRound {
        comment_index: usize,
        comment_id: Option<u64>,
        comment_url: Option<&'a str>,
        author: &'a str,
        at: DateTime<Utc>,
        note: Option<&'a str>,
        branch: Option<&'a str>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawRoundState<'a> {
    Open,
    Closed {
        commit: &'a str,
        by: &'a str,
        at: DateTime<Utc>,
        comment_index: usize,
        comment_id: Option<u64>,
        comment_url: Option<&'a str>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawRetraction<'a> {
    pub retracted_commit: &'a str,
    pub by: &'a str,
    pub at: DateTime<Utc>,
    pub comment_index: usize,
    pub comment_id: Option<u64>,
    pub comment_url: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawRoundEvent<'a> {
    Notification {
        commit: &'a str,
        by: &'a str,
        at: DateTime<Utc>,
        comment_index: usize,
        comment_id: Option<u64>,
        comment_url: Option<&'a str>,
    },
    Review {
        commit: &'a str,
        by: &'a str,
        at: DateTime<Utc>,
        comment_index: usize,
        comment_id: Option<u64>,
        comment_url: Option<&'a str>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawExtension<'a> {
    pub comment_index: usize,
    pub comment_id: Option<u64>,
    pub comment_url: Option<&'a str>,
    pub by: &'a str,
    pub at: DateTime<Utc>,
    pub at_commit: Option<&'a str>,
    pub note: Option<&'a str>,
}

/// The branch each stage-1 round declares, in round order: the issue body's for Initial
/// QC, the round comment's `git branch` for every later round.
///
/// `None` means the comment declared nothing, which is malformed rather than legacy.
///
/// Computed from stage-1 output *before* stage 2 runs, which is the only order that
/// works: resolving a round's anchor needs its branch walked, and the branch is only
/// known once the comments have been folded.
pub(crate) fn round_branches(
    raw_rounds: &[RawRound<'_>],
    issue_branch: &str,
) -> Vec<Option<String>> {
    raw_rounds
        .iter()
        .map(|round| match &round.opened {
            RawRoundOpen::IssueCreated => Some(issue_branch.to_string()),
            RawRoundOpen::NewRound { branch, .. } => branch.map(|b| b.to_string()),
        })
        .collect()
}

/// [`Round`] before any SHA has been resolved to an [`ObjectId`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawRound<'a> {
    pub index: u32,
    pub opened_at: &'a str,
    pub previous_approval: Option<&'a str>,
    pub opened: RawRoundOpen<'a>,
    pub checklist: ChecklistSource,
    pub checklist_name: Option<&'a str>,
    pub state: RawRoundState<'a>,
    pub events: Vec<RawRoundEvent<'a>>,
    pub retractions: Vec<RawRetraction<'a>>,
    pub extensions: Vec<RawExtension<'a>>,
}

/// Heading that opens the metadata block every QC comment renders.
const METADATA_HEADING: &str = "## Metadata";

/// Whether `line` opens an ATX heading of any level.
///
/// Metadata sections hold nothing but `* key: value` lines, so *any* heading ends
/// one. Level matters: a round comment names its checklist with a
/// level-1 heading, so stopping only at `## ` would pull the whole checklist into
/// the metadata slice and let its prose be read as metadata.
fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ')
}

/// The slice of `body` that belongs to its `## Metadata` section: everything after
/// a line equal to `## Metadata` up to the next heading of any level (or the end of
/// the body). Empty if the body has no metadata section.
///
/// Metadata keys are only ever looked up inside this slice, so text that merely
/// *looks* like metadata elsewhere in the comment — a `current commit: ` line
/// inside an inlined `## File Difference` diff, or inside a checklist, for
/// instance — is never parsed.
fn metadata_section(body: &str) -> &str {
    let mut start: Option<usize> = None;
    let mut offset = 0usize;
    for raw_line in body.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        match start {
            None => {
                if line.trim() == METADATA_HEADING {
                    start = Some(offset + raw_line.len());
                }
            }
            Some(begin) => {
                if is_heading(line) {
                    return &body[begin..offset];
                }
            }
        }
        offset += raw_line.len();
    }
    match start {
        Some(begin) => &body[begin..],
        None => "",
    }
}

/// Heading of the generic checklist section, used when no template name is recorded.
pub(crate) const UNNAMED_CHECKLIST_HEADING: &str = "## Checklist";

/// The heading that introduces a round comment's checklist.
pub(crate) enum ChecklistHeading<'a> {
    /// `# <name>` — named by its template, exactly as in an issue body.
    Named { name: &'a str, start: usize },
    /// `## Checklist` — no template name was recorded.
    Unnamed { start: usize },
}

impl ChecklistHeading<'_> {
    /// Byte offset just past the heading line: where the checklist content begins.
    pub(crate) fn start(&self) -> usize {
        match *self {
            ChecklistHeading::Named { start, .. } | ChecklistHeading::Unnamed { start } => start,
        }
    }
}

/// Where a round comment's checklist begins, and whether its heading names it.
///
/// One pass, one rule: the checklist is introduced by the first line that is either a
/// level-1 heading other than the round heading, or exactly `## Checklist`. Because
/// whichever comes *first* wins, a level-1 heading inside an unnamed checklist's
/// content can never be mistaken for the checklist's name.
///
/// The round heading is matched by [`is_round_heading`] rather than by prefix, so a
/// checklist whose name merely starts with "QC Round" is still found.
pub(crate) fn find_checklist_heading(body: &str) -> Option<ChecklistHeading<'_>> {
    let mut offset = 0usize;
    for raw_line in body.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = line.trim();
        let start = offset + raw_line.len();

        if trimmed == UNNAMED_CHECKLIST_HEADING {
            return Some(ChecklistHeading::Unnamed { start });
        }
        if !is_round_heading(trimmed)
            && let Some(name) = trimmed.strip_prefix("# ")
        {
            let name = name.trim();
            if !name.is_empty() {
                return Some(ChecklistHeading::Named { name, start });
            }
        }
        offset += raw_line.len();
    }
    None
}

/// The checklist template name a round comment records: its `# <name>` heading, or
/// `None` when the checklist is under the generic `## Checklist` heading.
pub(crate) fn checklist_name_from_body(body: &str) -> Option<&str> {
    match find_checklist_heading(body)? {
        ChecklistHeading::Named { name, .. } => Some(name),
        ChecklistHeading::Unnamed { .. } => None,
    }
}

/// Everything after `key` on the first metadata line carrying it, trimmed.
///
/// A line matches when — after optional leading whitespace and an optional `* ` or
/// `- ` bullet — it starts with `key`. Both the bulleted form real comments render
/// and the bare form used in tests therefore parse.
fn metadata_line<'a>(section: &'a str, key: &str) -> Option<&'a str> {
    for line in section.lines() {
        let mut rest = line.trim_start();
        if let Some(stripped) = rest.strip_prefix("* ").or_else(|| rest.strip_prefix("- ")) {
            rest = stripped.trim_start();
        }
        if let Some(value) = rest.strip_prefix(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// The first whitespace-delimited token after `key` in the metadata section, which
/// is how every commit-valued key is read.
fn metadata_commit<'a>(section: &'a str, key: &str) -> Option<&'a str> {
    metadata_line(section, key)?.split_whitespace().next()
}

/// Whether `line` is the heading that opens a round comment: `# QC Round <n>`.
///
/// The heading carries the round number, so it is matched as the prefix followed by
/// digits only — a checklist named "QC Rounds" is not a round heading.
pub(crate) fn is_round_heading(line: &str) -> bool {
    let trimmed = line.trim();
    match trimmed.strip_prefix(ROUND_HEADING) {
        Some(rest) => rest.trim().chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// Whether any line of `body` begins with `marker`, ignoring leading whitespace.
///
/// Anchoring at the start of a line means a quoted `> # QC Round` does not
/// count, while a split header such as `# QC Notification (2/3)` still does.
fn has_marker(body: &str, marker: &str) -> bool {
    body.lines()
        .any(|line| line.trim_start().starts_with(marker))
}

/// Whether two comment SHAs denote the same commit, allowing either side to be an
/// abbreviated SHA. Uses the same minimum-7-character rule as
/// `IssueThread::from_issue_comments`, so a comment writing a short
/// `previous approved commit:` is not reported as a mismatch against the full SHA
/// derived from the fold.
fn sha_equivalent(a: &str, b: &str) -> bool {
    a == b || (a.len() >= 7 && b.starts_with(a)) || (b.len() >= 7 && a.starts_with(b))
}

/// Fold an issue's comment thread into a sequence of rounds, without touching git.
///
/// The returned rounds are ordered oldest-first (Initial QC is always index 0 of
/// the vec). This function never fails; every problem is reported as an anomaly.
///
/// A thread with no round headings always folds to exactly one round.
/// There is deliberately no legacy reinterpretation: un-approvals in such a thread
/// are plain retractions on round 1.
///
/// `initial_checklist_name` is the checklist template name Initial QC used, which
/// lives in the issue body rather than in any comment (see
/// [`crate::new_round::checklist_from_issue_body`]); later rounds carry their own
/// name in the opening comment's `checklist:` metadata.
pub(crate) fn fold_rounds_from_comments<'a>(
    initial_commit_sha: &'a str,
    initial_checklist_name: Option<&'a str>,
    comments: &'a [GitComment],
) -> (Vec<RawRound<'a>>, Vec<RawAnomaly>) {
    let mut anomalies: Vec<RawAnomaly> = Vec::new();
    let mut rounds: Vec<RawRound<'a>> = vec![RawRound {
        index: 1,
        opened_at: initial_commit_sha,
        previous_approval: None,
        opened: RawRoundOpen::IssueCreated,
        checklist: ChecklistSource::IssueBody,
        checklist_name: initial_checklist_name,
        state: RawRoundState::Open,
        events: Vec::new(),
        retractions: Vec::new(),
        extensions: Vec::new(),
    }];

    for (comment_index, comment) in comments.iter().enumerate() {
        let body = comment.body.as_str();
        let metadata = metadata_section(body);
        let author = comment.author_login.as_str();
        let at = comment.created_at;
        // Real comment identity, when the comment carries it. Cache-loaded
        // comments legitimately have neither, hence `Option`.
        let comment_id = comment.id;
        let comment_url = comment.html_url.as_deref();

        // F4: a new round opens only on top of a closed one whose derived round
        // number the comment agrees with. Otherwise the comment extends the current
        // round, so its author-written checklist is never silently discarded.
        if body.lines().any(is_round_heading) {
            let cur = rounds.last().expect("rounds is never empty");
            let derived_index = cur.index.saturating_add(1);
            let written_index =
                metadata_commit(metadata, ROUND_KEY).and_then(|value| value.parse::<u32>().ok());

            let extension_reason = match &cur.state {
                RawRoundState::Open => Some(ExtensionReason::RoundStillOpen),
                RawRoundState::Closed { .. } => match written_index {
                    Some(written) if written != derived_index => {
                        Some(ExtensionReason::IndexMismatch {
                            written,
                            derived: derived_index,
                        })
                    }
                    _ => None,
                },
            };

            match extension_reason {
                Some(reason) => {
                    let cur = rounds.last_mut().expect("rounds is never empty");
                    log::debug!(
                        "comment {comment_index}: round heading extends round {} ({reason:?})",
                        cur.index
                    );
                    cur.state = RawRoundState::Open;
                    cur.checklist = ChecklistSource::Comment {
                        comment_index,
                        comment_id,
                        comment_url: comment_url.map(|url| url.to_string()),
                    };
                    // The checklist name travels with the checklist it names: the
                    // round is now QC'd against this comment's checklist, so an
                    // unnamed one legitimately leaves the round unnamed.
                    cur.checklist_name = checklist_name_from_body(body);
                    cur.extensions.push(RawExtension {
                        comment_index,
                        comment_id,
                        comment_url,
                        by: author,
                        at,
                        at_commit: metadata_commit(metadata, INITIAL_ROUND_COMMIT_KEY),
                        note: metadata_line(metadata, NOTE_KEY),
                    });
                    anomalies.push(RoundAnomaly::RoundExtended {
                        comment_index,
                        reason,
                    });
                }
                None => {
                    let derived_base = match &cur.state {
                        RawRoundState::Closed { commit, .. } => *commit,
                        RawRoundState::Open => unreachable!("an open round always extends"),
                    };

                    // The base is derivable, so the written value is only checked,
                    // never used: round N's base must stay round N-1's approval.
                    if let Some(written) = metadata_commit(metadata, PREVIOUS_APPROVED_COMMIT_KEY)
                        && !sha_equivalent(written, derived_base)
                    {
                        anomalies.push(RoundAnomaly::BaseCommitMismatch {
                            comment_index,
                            written: written.to_string(),
                            derived: derived_base.to_string(),
                        });
                    }

                    // The anchor is the one value the comment log cannot derive.
                    let opened_at =
                        metadata_commit(metadata, INITIAL_ROUND_COMMIT_KEY).unwrap_or(derived_base);

                    log::debug!(
                        "comment {comment_index}: opening round {derived_index} at {opened_at}"
                    );
                    rounds.push(RawRound {
                        index: derived_index,
                        opened_at,
                        previous_approval: Some(derived_base),
                        opened: RawRoundOpen::NewRound {
                            comment_index,
                            comment_id,
                            comment_url,
                            author,
                            at,
                            note: metadata_line(metadata, NOTE_KEY),
                            branch: metadata_line(metadata, GIT_BRANCH_KEY),
                        },
                        checklist: ChecklistSource::Comment {
                            comment_index,
                            comment_id,
                            comment_url: comment_url.map(|url| url.to_string()),
                        },
                        checklist_name: checklist_name_from_body(body),
                        state: RawRoundState::Open,
                        events: Vec::new(),
                        retractions: Vec::new(),
                        extensions: Vec::new(),
                    });
                }
            }
            continue;
        }

        // F5: an un-approval reopens the current round, or is a no-op.
        if has_marker(body, UNAPPROVAL_MARKER) {
            let cur = rounds.last_mut().expect("rounds is never empty");
            match &cur.state {
                RawRoundState::Closed { commit, .. } => {
                    log::debug!(
                        "comment {comment_index}: retracting approval of {commit} on round {}",
                        cur.index
                    );
                    cur.retractions.push(RawRetraction {
                        retracted_commit: commit,
                        by: author,
                        at,
                        comment_index,
                        comment_id,
                        comment_url,
                    });
                    cur.state = RawRoundState::Open;
                }
                RawRoundState::Open => {
                    anomalies.push(RoundAnomaly::RetractWithNothingClosed { comment_index });
                }
            }
            continue;
        }

        // F2: notification / review events land on the current round.
        let notification = if has_marker(body, NOTIFICATION_MARKER) {
            metadata_commit(metadata, CURRENT_COMMIT_KEY)
        } else {
            None
        };
        let review = if has_marker(body, REVIEW_MARKER) {
            metadata_commit(metadata, COMPARING_COMMIT_KEY)
        } else {
            None
        };

        for event in [
            notification.map(|commit| RawRoundEvent::Notification {
                commit,
                by: author,
                at,
                comment_index,
                comment_id,
                comment_url,
            }),
            review.map(|commit| RawRoundEvent::Review {
                commit,
                by: author,
                at,
                comment_index,
                comment_id,
                comment_url,
            }),
        ]
        .into_iter()
        .flatten()
        {
            let cur = rounds.last_mut().expect("rounds is never empty");
            if matches!(cur.state, RawRoundState::Closed { .. }) {
                anomalies.push(RoundAnomaly::EventAfterClose { comment_index });
            }
            cur.events.push(event);
        }

        // F3: an approval closes the current round; a second approval corrects the
        // closing commit rather than opening a new round.
        if let Some(commit) = metadata_commit(metadata, APPROVED_COMMIT_KEY) {
            let cur = rounds.last_mut().expect("rounds is never empty");
            log::debug!(
                "comment {comment_index}: closing round {} at {commit}",
                cur.index
            );
            cur.state = RawRoundState::Closed {
                commit,
                by: author,
                at,
                comment_index,
                comment_id,
                comment_url,
            };
        }
    }

    (rounds, anomalies)
}

// ── Stage 2: place every commit in exactly one segment ──────────────────────

/// A branch's commit walk (newest-first), or the reason it could not be walked.
pub(crate) type BranchWalk = Result<Vec<IssueCommit>, UnplaceableReason>;

/// Every branch any round declared, walked once each. Cost is bounded by *distinct
/// branches*, not by segments, so a single-branch issue does exactly one walk.
pub(crate) type BranchWalks = HashMap<String, BranchWalk>;

/// Resolve a comment SHA against a commit walk using the same matching semantics as
/// `IssueThread::from_issue_comments`: exact match, or a prefix match for abbreviated
/// SHAs of at least 7 characters.
fn resolve_sha(sha: &str, commits: &[IssueCommit]) -> Option<ObjectId> {
    commits
        .iter()
        .find(|commit| {
            let full = commit.hash.to_string();
            full == sha || (sha.len() >= 7 && full.starts_with(sha))
        })
        .map(|commit| commit.hash)
}

/// Resolve a comment SHA against *any* branch we walked.
///
/// A commit that is not on a segment's own branch still has an identity worth
/// recording — the segment is unplaceable, not unknown — and callers that only need
/// the pair of commits to diff can still use it.
fn resolve_anywhere(sha: &str, walks: &BranchWalks) -> Option<ObjectId> {
    walks
        .values()
        .filter_map(|walk| walk.as_ref().ok())
        .find_map(|commits| resolve_sha(sha, commits))
}

/// Position of a commit in a walk (newest-first): a smaller index is more recent.
fn position_of(commits: &[IssueCommit], hash: &ObjectId) -> Option<usize> {
    commits.iter().position(|commit| commit.hash == *hash)
}

/// The unresolvable-SHA placeholder for an anchor or closing commit that is not in any
/// walk. The segment carrying it is always `Unplaceable`, so the value is never used to
/// address a commit; it exists because a round always has an anchor.
fn unknown_commit() -> ObjectId {
    ObjectId::null(gix::hash::Kind::Sha1)
}

/// Resolve stage-1 rounds into the issue's segment list, placing every commit.
///
/// `branches` is parallel to `raw_rounds` (see [`round_branches`]); `walks` holds one
/// walk per distinct declared branch; `merge_base` answers ancestry for gaps whose
/// bounds do not sit on one line.
///
/// Gaps are not written anywhere — they are implied by adjacency and inserted between
/// rounds here, with a trailing gap after a closed final round so the last segment is
/// always an open round or a gap. That is what makes status total: it never has to
/// consider a closed round as the active one.
///
/// Nothing here fails. Every unresolvable segment degrades to
/// [`Placement::Unplaceable`], owning no commits, and reports a
/// [`RoundAnomaly::SegmentUnplaceable`].
pub(crate) fn resolve_segments(
    raw_rounds: Vec<RawRound<'_>>,
    branches: &[Option<String>],
    walks: &BranchWalks,
    merge_base: &dyn Fn(&ObjectId, &ObjectId) -> Option<ObjectId>,
    mut anomalies: Vec<RawAnomaly>,
) -> (Vec<Segment>, Vec<RoundAnomaly>) {
    let mut rounds: Vec<Round> = Vec::with_capacity(raw_rounds.len());
    // The previous round's closing commit: the round's base as the fold derives it,
    // and the lower bound of the gap that follows it.
    let mut previous_approval: Option<ObjectId> = None;

    for (round_position, raw) in raw_rounds.into_iter().enumerate() {
        // Round N sits at segment position 2N (rounds and gaps strictly alternate).
        let segment_position = round_position * 2;
        let round = resolve_round(
            raw,
            branches.get(round_position).cloned().flatten(),
            segment_position,
            previous_approval,
            walks,
            &mut anomalies,
        );
        previous_approval = round.closing_commit().copied();
        rounds.push(round);
    }

    let mut segments: Vec<Segment> = Vec::with_capacity(rounds.len() * 2);
    for round in rounds {
        if let Some(Segment::Round(older)) = segments.last() {
            let gap = build_gap(
                older,
                Some(&round),
                segments.len(),
                walks,
                merge_base,
                &mut anomalies,
            );
            segments.push(Segment::Gap(gap));
        }
        segments.push(Segment::Round(round));
    }

    // A closed round is never the last segment: the drift since its approval is a
    // segment of its own, even when it is empty.
    if let Some(Segment::Round(last)) = segments.last()
        && !last.is_open()
    {
        let gap = build_gap(
            last,
            None,
            segments.len(),
            walks,
            merge_base,
            &mut anomalies,
        );
        segments.push(Segment::Gap(gap));
    }

    debug_assert!(
        segment_invariants(&segments, Some(walks)).is_ok(),
        "segment invariants violated: {:?}",
        segment_invariants(&segments, Some(walks))
    );

    (segments, anomalies)
}

/// Resolve one stage-1 round: its branch, its anchor, its state, its events, and the
/// commits it owns.
fn resolve_round(
    raw: RawRound<'_>,
    declared_branch: Option<String>,
    segment_position: usize,
    previous_approval: Option<ObjectId>,
    walks: &BranchWalks,
    anomalies: &mut Vec<RoundAnomaly>,
) -> Round {
    let anchor_comment_index = match &raw.opened {
        RawRoundOpen::NewRound { comment_index, .. } => *comment_index,
        RawRoundOpen::IssueCreated => 0,
    };

    // Every round declares a branch. One that does not is malformed input, so the
    // round is grayed rather than silently walked on somebody else's branch.
    let mut unplaceable: Option<UnplaceableReason> = None;
    let branch = match declared_branch {
        Some(branch) => branch,
        None => {
            anomalies.push(RoundAnomaly::BranchNotDeclared {
                comment_index: anchor_comment_index,
            });
            unplaceable = Some(UnplaceableReason::BranchNotDeclared);
            String::new()
        }
    };

    let walk: &[IssueCommit] = if unplaceable.is_some() {
        &[]
    } else {
        match walks.get(&branch) {
            Some(Ok(commits)) => commits.as_slice(),
            Some(Err(reason)) => {
                unplaceable = Some(*reason);
                &[]
            }
            None => {
                unplaceable = Some(UnplaceableReason::BranchUnavailable);
                &[]
            }
        }
    };

    // The anchor is authoritative, so it is resolved against the round's own branch
    // first — a better input than one arbitrary thread-wide reference commit — and only
    // then against anything else we walked, which identifies the commit while still
    // leaving the round unplaceable.
    let opened_at = match resolve_sha(raw.opened_at, walk) {
        Some(id) => id,
        None => match resolve_anywhere(raw.opened_at, walks) {
            Some(id) => {
                unplaceable.get_or_insert(UnplaceableReason::AnchorUnreachable);
                id
            }
            None => {
                anomalies.push(RoundAnomaly::AnchorUnreachable {
                    comment_index: anchor_comment_index,
                    sha: raw.opened_at.to_string(),
                });
                unplaceable.get_or_insert(UnplaceableReason::AnchorUnreachable);
                unknown_commit()
            }
        },
    };

    let state = match raw.state {
        RawRoundState::Open => RoundState::Open,
        RawRoundState::Closed {
            commit,
            by,
            at,
            comment_index,
            comment_id,
            comment_url,
        } => match resolve_sha(commit, walk).or_else(|| resolve_anywhere(commit, walks)) {
            Some(id) => RoundState::Closed {
                commit: id,
                by: by.to_string(),
                at,
                comment_index,
                comment_id,
                comment_url: comment_url.map(|url| url.to_string()),
            },
            // The approval happened; we simply cannot say where. Reopening the round
            // without graying it would report the issue as awaiting review, so the round
            // takes the one degradation path instead.
            None => {
                anomalies.push(RoundAnomaly::AnchorUnreachable {
                    comment_index,
                    sha: commit.to_string(),
                });
                unplaceable.get_or_insert(UnplaceableReason::AnchorUnreachable);
                RoundState::Open
            }
        },
    };

    // Sanity guard: the anchor is authoritative, but an anchor older than the round's
    // base (a larger index in the newest-first walk) would make this round's commits
    // overlap the previous round's. Falling back to the base leaves the two rounds
    // sharing one boundary commit, which is legal, instead of overlapping.
    //
    // Both ends must be on this walk for the comparison to mean anything: an anchor that
    // is off the round's own branch is not "older" than anything here, and rewriting it
    // to the base would destroy the identity the round is unplaceable *for*.
    let opened_at = match (previous_approval, position_of(walk, &opened_at)) {
        (Some(base), Some(anchor))
            if position_of(walk, &base).is_some_and(|base_position| anchor > base_position) =>
        {
            anomalies.push(RoundAnomaly::AnchorOlderThanBase {
                comment_index: anchor_comment_index,
                anchor: opened_at.to_string(),
                base: base.to_string(),
            });
            base
        }
        _ => opened_at,
    };

    let mut events = Vec::with_capacity(raw.events.len());
    for event in raw.events {
        let (commit, by, at, comment_index, comment_id, comment_url, is_notification) = match event
        {
            RawRoundEvent::Notification {
                commit,
                by,
                at,
                comment_index,
                comment_id,
                comment_url,
            } => (commit, by, at, comment_index, comment_id, comment_url, true),
            RawRoundEvent::Review {
                commit,
                by,
                at,
                comment_index,
                comment_id,
                comment_url,
            } => (
                commit,
                by,
                at,
                comment_index,
                comment_id,
                comment_url,
                false,
            ),
        };
        match resolve_sha(commit, walk).or_else(|| resolve_anywhere(commit, walks)) {
            Some(id) if is_notification => events.push(RoundEvent::Notification {
                commit: id,
                by: by.to_string(),
                at,
                comment_index,
                comment_id,
                comment_url: comment_url.map(|url| url.to_string()),
            }),
            Some(id) => events.push(RoundEvent::Review {
                commit: id,
                by: by.to_string(),
                at,
                comment_index,
                comment_id,
                comment_url: comment_url.map(|url| url.to_string()),
            }),
            None => anomalies.push(RoundAnomaly::AnchorUnreachable {
                comment_index,
                sha: commit.to_string(),
            }),
        }
    }

    let mut retractions = Vec::with_capacity(raw.retractions.len());
    for retraction in raw.retractions {
        match resolve_sha(retraction.retracted_commit, walk)
            .or_else(|| resolve_anywhere(retraction.retracted_commit, walks))
        {
            Some(id) => retractions.push(Retraction {
                retracted_commit: id,
                by: retraction.by.to_string(),
                at: retraction.at,
                comment_index: retraction.comment_index,
                comment_id: retraction.comment_id,
                comment_url: retraction.comment_url.map(|url| url.to_string()),
            }),
            None => anomalies.push(RoundAnomaly::AnchorUnreachable {
                comment_index: retraction.comment_index,
                sha: retraction.retracted_commit.to_string(),
            }),
        }
    }

    let opened = match raw.opened {
        RawRoundOpen::IssueCreated => RoundOpen::IssueCreated,
        RawRoundOpen::NewRound {
            comment_index,
            comment_id,
            comment_url,
            author,
            at,
            note,
            branch,
        } => RoundOpen::NewRound {
            comment_index,
            comment_id,
            comment_url: comment_url.map(|url| url.to_string()),
            author: author.to_string(),
            at,
            note: note.map(|n| n.to_string()),
            branch: branch.map(|b| b.to_string()),
        },
    };

    // An extension's `at_commit` is informational only, so an unresolvable one is
    // simply dropped rather than reported.
    let extensions = raw
        .extensions
        .into_iter()
        .map(|extension| Extension {
            comment_index: extension.comment_index,
            comment_id: extension.comment_id,
            comment_url: extension.comment_url.map(|url| url.to_string()),
            by: extension.by.to_string(),
            at: extension.at,
            at_commit: extension
                .at_commit
                .and_then(|sha| resolve_sha(sha, walk).or_else(|| resolve_anywhere(sha, walks))),
            note: extension.note.map(|note| note.to_string()),
        })
        .collect();

    // The round's own walk, bounded: newest end is its closing commit, or the branch
    // tip while it is still open; oldest end is its anchor, inclusive.
    let (commits, placement) = match unplaceable {
        Some(reason) => (Vec::new(), Placement::Unplaceable(reason)),
        None => {
            let newest = match state.closing_commit() {
                Some(commit) => position_of(walk, commit),
                None if walk.is_empty() => None,
                None => Some(0),
            };
            match (newest, position_of(walk, &opened_at)) {
                (Some(newest), Some(anchor)) if newest <= anchor => {
                    (walk[newest..=anchor].to_vec(), Placement::Placed)
                }
                // A closing commit older than its own anchor is malformed; the round
                // then owns only the commit it closed at, so nothing overlaps.
                (Some(newest), Some(_)) => (walk[newest..=newest].to_vec(), Placement::Placed),
                _ => (
                    Vec::new(),
                    Placement::Unplaceable(UnplaceableReason::AnchorUnreachable),
                ),
            }
        }
    };
    if let Placement::Unplaceable(reason) = placement {
        anomalies.push(RoundAnomaly::SegmentUnplaceable {
            position: segment_position,
            reason,
        });
    }

    Round {
        index: raw.index,
        opened_at,
        branch,
        opened,
        checklist: raw.checklist,
        checklist_name: raw.checklist_name.map(|name| name.to_string()),
        state,
        events,
        retractions,
        extensions,
        commits,
        placement,
    }
}

/// Build the gap between `older` and `newer`, or the gap trailing `older` when there is
/// no newer round.
///
/// One rule picks the branch: a gap walks on the branch of the round bounding its newer
/// end, and on the older round's branch only when there is no newer round. Never the
/// viewer's checkout — two people on different branches must see the same segments.
fn build_gap(
    older: &Round,
    newer: Option<&Round>,
    position: usize,
    walks: &BranchWalks,
    merge_base: &dyn Fn(&ObjectId, &ObjectId) -> Option<ObjectId>,
    anomalies: &mut Vec<RoundAnomaly>,
) -> Gap {
    let branch = match newer {
        Some(round) => round.branch.clone(),
        None => older.branch.clone(),
    };
    let mut unplaceable = |reason: UnplaceableReason| -> Gap {
        anomalies.push(RoundAnomaly::SegmentUnplaceable { position, reason });
        Gap {
            branch: branch.clone(),
            commits: Vec::new(),
            continuity: GapContinuity::Linear,
            placement: Placement::Unplaceable(reason),
        }
    };

    // A gap bounded by an unplaceable round is itself unplaceable. Substituting the
    // other round's branch would yield a real commit set that is not this gap.
    if !older.is_placed() || newer.is_some_and(|round| !round.is_placed()) {
        return unplaceable(UnplaceableReason::NeighbourUnplaceable);
    }
    let Some(lower) = older.closing_commit().copied() else {
        return unplaceable(UnplaceableReason::NeighbourUnplaceable);
    };
    let walk: &[IssueCommit] = match walks.get(&branch) {
        Some(Ok(commits)) => commits,
        Some(Err(reason)) => return unplaceable(*reason),
        None => return unplaceable(UnplaceableReason::BranchUnavailable),
    };

    // Newer end: the next round's anchor, exclusive — the anchor belongs to that round
    // — or the branch tip, inclusive, when the gap trails the last round.
    let (start, upper) = match newer {
        Some(round) => match position_of(walk, &round.opened_at) {
            Some(anchor) => (anchor + 1, round.opened_at),
            None => return unplaceable(UnplaceableReason::AnchorUnreachable),
        },
        None => match walk.first() {
            Some(tip) => (0, tip.hash),
            // An empty walk has no tip to bound the gap at. Nothing is unreachable *on*
            // the branch — the branch itself gave us nothing.
            None => return unplaceable(UnplaceableReason::BranchUnavailable),
        },
    };

    // Older end: the previous round's approval, exclusive. When it is not on this
    // branch the two ends diverge, and the merge-base becomes the gap's natural lower
    // bound rather than a fallback.
    let (end, continuity) = match position_of(walk, &lower) {
        Some(approval) => (approval, GapContinuity::Linear),
        None => match merge_base(&lower, &upper) {
            // The merge-base *is* the gap's lower bound, so one that is not on this walk
            // leaves the gap unbounded: running back to the walk's oldest commit instead
            // would claim commits the older round already owns.
            Some(base) => match position_of(walk, &base) {
                Some(position) => (position, GapContinuity::Diverged { merge_base: base }),
                // Both bounds resolved; it is their common ancestor that sits outside the
                // walked range. Reporting an unreachable *anchor* here would send the user
                // hunting a commit that is not missing.
                None => return unplaceable(UnplaceableReason::MergeBaseUnreachable),
            },
            // Unrelated histories: no diff between the ends is meaningful, so the gap
            // claims nothing rather than claiming a whole branch.
            None => (start, GapContinuity::Unrelated),
        },
    };

    let commits = if end > start {
        walk[start..end].to_vec()
    } else {
        Vec::new()
    };

    Gap {
        branch,
        commits,
        continuity,
        placement: Placement::Placed,
    }
}

/// Check the segment-list invariants that hold after every fold.
///
/// Asserted in debug builds by [`resolve_segments`], and called directly by tests. The
/// interesting ones are ownership: every commit the segments know about belongs to
/// exactly one of them — except a boundary commit that is both one round's closing
/// commit and the next round's anchor, which is legitimate when HEAD has not moved
/// since the approval — and an unplaceable segment owns nothing at all.
///
/// `walks` adds the other half of **I4**/**I5**: not merely that no commit is owned
/// twice, but that the owned commits leave no hole in the walk (see
/// [`walk_is_contiguous`]). Pass `None` where there are no walks to check against —
/// hand-built fixtures, and threads whose walks the fold already asserted against.
pub(crate) fn segment_invariants(
    segments: &[Segment],
    walks: Option<&BranchWalks>,
) -> Result<(), String> {
    match segments.first() {
        Some(Segment::Round(_)) => {}
        Some(Segment::Gap(_)) => return Err("segments[0] is a Gap, not Initial QC".to_string()),
        None => return Err("segment list is empty".to_string()),
    }
    for (position, segment) in segments.iter().enumerate() {
        let expects_round = position % 2 == 0;
        if expects_round != matches!(segment, Segment::Round(_)) {
            return Err(format!("segment {position} breaks Round/Gap alternation"));
        }
        if !segment.is_placed() && !segment.commits().is_empty() {
            return Err(format!(
                "unplaceable segment {position} owns {} commit(s)",
                segment.commits().len()
            ));
        }
    }
    if let Some(Segment::Round(last)) = segments.last()
        && !last.is_open()
    {
        return Err("the last segment is a closed Round".to_string());
    }

    let mut owners: HashMap<ObjectId, Vec<usize>> = HashMap::new();
    for (position, segment) in segments.iter().enumerate() {
        for commit in segment.commits() {
            owners.entry(commit.hash).or_default().push(position);
        }
    }
    // `owners` is a `HashMap`, so collect every violation and report the first in a
    // deterministic order rather than whichever one iteration reached first: an assertion
    // that names a different commit from run to run is unreproducible in exactly the
    // situation it exists to diagnose.
    let mut overlaps: Vec<(usize, &ObjectId, &Vec<usize>)> = Vec::new();
    for (hash, positions) in &owners {
        if positions.len() == 1 {
            continue;
        }
        // The one legal sharing: `hash` closes the round at `first` and anchors the
        // round at `first + 2`.
        let shared_boundary = positions.len() == 2
            && positions[1] == positions[0] + 2
            && matches!(
                (segments.get(positions[0]), segments.get(positions[1])),
                (Some(Segment::Round(closed)), Some(Segment::Round(opened)))
                    if closed.closing_commit() == Some(hash) && opened.opened_at == *hash
            );
        if !shared_boundary {
            overlaps.push((positions[0], hash, positions));
        }
    }
    overlaps.sort_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)));
    if let Some((_, hash, positions)) = overlaps.first() {
        return Err(format!("commit {hash} is owned by segments {positions:?}"));
    }

    if let Some(walks) = walks {
        walks_are_contiguous(segments, walks, &owners)?;
    }

    Ok(())
}

/// The other half of **I4**/**I5**: the commits the placed segments own must form a
/// *contiguous run* of each walk, not merely avoid overlapping on it. Without this, a gap
/// that silently owns too *few* commits passes every check above.
///
/// Note the asymmetry, and what it does **not** cover: a gap that owns too *many* commits
/// is caught only when it over-claims commits some other segment also owns, which is the
/// overlap clause's job rather than this one's. A gap over-claiming commits **nobody
/// else** owns is invisible to every clause here and is pinned by direct regression tests
/// alone.
///
/// Contiguity rather than totality, for two reasons, both of them about what a walk
/// contains that no segment is responsible for:
///
/// * Commits at the two *ends*. The newest end is branch tip beyond every segment's
///   range; the oldest end is history from before Initial QC's anchor. Each walk is
///   therefore windowed to its owned span before anything is checked.
/// * Commits an `Unplaceable` segment would have owned. Its range is unknown by
///   definition (**I5**), so nothing can say which commits should have been in it — but
///   *which* holes it can account for is knowable: only those spanned by it in the
///   segment list. A hole bounded by two segments with nothing unplaceable between them
///   is a violation, because no segment's range can explain it.
///
/// Run once per walk. The multi-branch shapes are the ones that need it: an interior gap
/// on a second branch owning too few commits is invisible to a check that looks at one
/// walk only, and that is exactly where a bad `Diverged` bound lives.
///
/// Ownership is read per walk but *not* per branch. A commit owned by a segment on
/// another branch is still owned, and where two branches share history it can sit in the
/// middle of this walk's owned run — see
/// `a_commit_owned_on_another_branch_is_not_a_hole_in_this_walk`, where grouping by each
/// segment's own `branch` would report a legitimate thread as holed.
///
/// [`BranchWalks`] is a `HashMap`, so the walks are visited in sorted branch order: an
/// assertion that fires on a different walk from run to run is not a usable diagnostic.
fn walks_are_contiguous(
    segments: &[Segment],
    walks: &BranchWalks,
    owners: &HashMap<ObjectId, Vec<usize>>,
) -> Result<(), String> {
    let owned = |commit: &IssueCommit| owners.contains_key(&commit.hash);
    // A segment whose range is unknowable, and which can therefore account for a hole it
    // spans. An `Unplaceable` segment owns nothing by **I5**. A `Placed` `Unrelated` gap
    // owns nothing either: no bound-to-bound range is meaningful when its ends share no
    // history (**M4**/**D15**), so it stands outside the partition just as an unplaceable
    // segment does — and unlike one, every segment around it may be perfectly placed.
    let range_unknowable = |segment: &Segment| match segment {
        Segment::Gap(gap) => !gap.is_placed() || matches!(gap.continuity, GapContinuity::Unrelated),
        Segment::Round(round) => !round.is_placed(),
    };
    let mut branches: Vec<&String> = walks.keys().collect();
    branches.sort();

    for branch in branches {
        let Some(Ok(walk)) = walks.get(branch) else {
            continue;
        };
        let Some(newest) = walk.iter().position(owned) else {
            continue;
        };
        let oldest = walk.iter().rposition(owned).unwrap_or(newest);

        for index in newest..=oldest {
            if owned(&walk[index]) {
                continue;
            }
            // The owned commits bounding this hole, and the segments they belong to.
            // Both searches succeed: the window's ends are owned by construction.
            let above = walk[..index].iter().rposition(owned).unwrap_or(newest);
            let below = index + 1 + walk[index + 1..].iter().position(owned).unwrap_or(0);
            let bounds = [above, below]
                .into_iter()
                .flat_map(|at| owners.get(&walk[at].hash).into_iter().flatten().copied());
            // A shared boundary commit (**D1**) has two owners, so take the widest span
            // the bounding commits allow — the aim is to find any unplaceable segment
            // that could account for the hole, not to pin down one owner.
            let (Some(first), Some(last)) = (bounds.clone().min(), bounds.max()) else {
                continue;
            };
            // One segment on both sides of the hole means this walk orders its commits
            // differently than the walk they were taken from, which is topology rather
            // than a hole — and the span below would be empty or inverted anyway.
            if first >= last {
                continue;
            }
            if segments
                .get(first + 1..last)
                .is_some_and(|span| span.iter().any(range_unknowable))
            {
                continue;
            }
            return Err(format!(
                "commit {} is owned by no segment, between commits that are (walk {branch})",
                walk[index].hash
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::IssueThread;
    use std::path::PathBuf;
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";
    const D: &str = "ddddddd000000000000000000000000000000004";
    const E: &str = "eeeeeee000000000000000000000000000000005";
    const F: &str = "fffffff000000000000000000000000000000006";

    /// The issue body's branch, i.e. Initial QC's.
    const MAIN: &str = "main";
    /// A branch a later round moved to.
    const FEATURE: &str = "feature/x";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    /// A comment as loaded from the on-disk cache: no id, no URL.
    fn comment(body: &str) -> GitComment {
        GitComment {
            body: body.to_string(),
            author_login: "tester".to_string(),
            created_at: Utc::now(),
            id: None,
            html_url: None,
            html: None,
        }
    }

    /// A comment carrying real GitHub identity, as fetched fresh from the API.
    fn identified(body: &str, id: u64, url: &str) -> GitComment {
        GitComment {
            id: Some(id),
            html_url: Some(url.to_string()),
            ..comment(body)
        }
    }

    fn notification(sha: &str) -> GitComment {
        comment(&format!(
            "# QC Notification\n\n@reviewer\n\n## Metadata\ncurrent commit: {sha}\n"
        ))
    }

    fn review(sha: &str) -> GitComment {
        comment(&format!(
            "# QC Review\n\n@author\n\n## Metadata\ncomparing commit: {sha}\n"
        ))
    }

    fn approval(sha: &str) -> GitComment {
        comment(&format!(
            "# QC Approval\n\n## Metadata\napproved qc commit: {sha}\n"
        ))
    }

    fn unapproval() -> GitComment {
        comment("# QC Un-Approval\n\nWithdrawing approval.\n")
    }

    fn new_round(round: u32, round_commit: &str, previous: &str) -> GitComment {
        new_round_raw(&round.to_string(), round_commit, previous)
    }

    /// `# QC Round` with an arbitrary (possibly non-numeric) `round:` value.
    fn new_round_raw(round: &str, round_commit: &str, previous: &str) -> GitComment {
        comment(&format!(
            "# QC Round\n\n## Metadata\nround: {round}\ninitial qc round commit: {round_commit}\nprevious approved commit: {previous}\nnote: second pass\n\n# Checklist\n- [ ] item\n"
        ))
    }

    /// A round comment that declares the branch it was opened on.
    fn new_round_on(round: u32, round_commit: &str, previous: &str, branch: &str) -> GitComment {
        comment(&format!(
            "# QC Round\n\n## Metadata\nround: {round}\ninitial qc round commit: {round_commit}\nprevious approved commit: {previous}\ngit branch: {branch}\n\n# Checklist\n- [ ] item\n"
        ))
    }

    /// A branch walk. `shas` is oldest-first here for readability and reversed into the
    /// newest-first order every walk uses.
    fn walk(shas: &[&str]) -> Vec<IssueCommit> {
        shas.iter()
            .rev()
            .map(|sha| IssueCommit {
                hash: oid(sha),
                message: format!("commit {sha}"),
                file_changed: true,
            })
            .collect()
    }

    /// Walks for a single-branch issue — the shape every existing issue has.
    fn one_branch(shas: &[&str]) -> BranchWalks {
        BranchWalks::from([(MAIN.to_string(), Ok(walk(shas)))])
    }

    /// No merge-base is reachable. Gaps whose ends sit on one branch never ask.
    fn no_merge_base(_: &ObjectId, _: &ObjectId) -> Option<ObjectId> {
        None
    }

    /// Build an `IssueThread` from comments and pre-canned walks: segments fold over
    /// comment text, so no git is needed.
    fn thread_with(
        walks: &BranchWalks,
        initial: &str,
        comments: &[GitComment],
        merge_base: &dyn Fn(&ObjectId, &ObjectId) -> Option<ObjectId>,
    ) -> IssueThread {
        let (raw, raw_anomalies) = fold_rounds_from_comments(initial, None, comments);
        let branches = round_branches(&raw, MAIN);
        let (segments, anomalies) =
            resolve_segments(raw, &branches, walks, merge_base, raw_anomalies);
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            open: true,
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies,
        }
    }

    /// The single-branch case: `shas` oldest-first.
    fn thread(shas: &[&str], initial: &str, comments: &[GitComment]) -> IssueThread {
        thread_with(&one_branch(shas), initial, comments, &no_merge_base)
    }

    fn round_at(thread: &IssueThread, position: usize) -> &Round {
        thread.segments[position]
            .as_round()
            .expect("expected a Round")
    }

    fn gap_at(thread: &IssueThread, position: usize) -> &Gap {
        thread.segments[position].as_gap().expect("expected a Gap")
    }

    /// Owned commits of a segment, newest-first, as SHA strings.
    fn owned(segment: &Segment) -> Vec<String> {
        segment
            .commits()
            .iter()
            .map(|commit| commit.hash.to_string())
            .collect()
    }

    // ── Stage 1 fold rules ───────────────────────────────────────────────────

    #[test]
    fn legacy_notifications_only_folds_to_one_open_round() {
        let comments = vec![notification(B), notification(C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert!(anomalies.is_empty());
        let round = &rounds[0];
        assert_eq!(round.index, 1);
        assert_eq!(round.opened_at, A);
        assert_eq!(round.previous_approval, None);
        assert_eq!(round.opened, RawRoundOpen::IssueCreated);
        assert_eq!(round.checklist, ChecklistSource::IssueBody);
        assert_eq!(round.state, RawRoundState::Open);
        assert_eq!(round.events.len(), 2);
        assert!(matches!(
            round.events[0],
            RawRoundEvent::Notification { commit, .. } if commit == B
        ));
        assert!(matches!(
            round.events[1],
            RawRoundEvent::Notification { commit, .. } if commit == C
        ));
        assert!(round.retractions.is_empty());
    }

    #[test]
    fn legacy_approval_closes_the_single_round() {
        let comments = vec![notification(B), approval(B)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert!(anomalies.is_empty());
        assert!(matches!(
            rounds[0].state,
            RawRoundState::Closed { commit, .. } if commit == B
        ));
    }

    #[test]
    fn unapproval_reopens_round_and_records_retraction() {
        let comments = vec![notification(B), approval(B), unapproval()];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert!(anomalies.is_empty());
        assert_eq!(rounds[0].state, RawRoundState::Open);
        assert_eq!(rounds[0].retractions.len(), 1);
        assert_eq!(rounds[0].retractions[0].retracted_commit, B);
        assert_eq!(rounds[0].retractions[0].comment_index, 2);

        // The reopened round is the last segment, so there is no trailing gap and
        // nothing stands approved.
        let thread = thread(&[A, B], A, &comments);
        assert_eq!(thread.segments.len(), 1);
        assert_eq!(thread.standing_approval(), None);
        assert!(
            thread
                .active_segment()
                .as_round()
                .is_some_and(Round::is_open)
        );
    }

    #[test]
    fn second_unapproval_is_idempotent_and_flagged() {
        let comments = vec![approval(B), unapproval(), unapproval()];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].retractions.len(), 1);
        assert_eq!(
            anomalies,
            vec![RoundAnomaly::RetractWithNothingClosed { comment_index: 2 }]
        );
    }

    #[test]
    fn second_approval_overwrites_closing_commit_without_new_round() {
        let comments = vec![approval(B), approval(C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert!(anomalies.is_empty());
        assert!(matches!(
            rounds[0].state,
            RawRoundState::Closed { commit, .. } if commit == C
        ));
    }

    #[test]
    fn new_round_after_close_opens_round_two() {
        let comments = vec![notification(B), approval(B), new_round(2, C, B)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert!(anomalies.is_empty());
        assert_eq!(rounds.len(), 2);
        let second = &rounds[1];
        assert_eq!(second.index, 2);
        assert_eq!(second.opened_at, C);
        assert_eq!(second.previous_approval, Some(B));
        assert_eq!(second.state, RawRoundState::Open);
        assert!(second.events.is_empty());
        assert_eq!(
            second.checklist,
            ChecklistSource::Comment {
                comment_index: 2,
                comment_id: None,
                comment_url: None,
            }
        );
        assert!(matches!(
            &second.opened,
            RawRoundOpen::NewRound {
                comment_index: 2,
                note: Some("second pass"),
                ..
            }
        ));
    }

    #[test]
    fn new_round_while_open_extends_the_current_round() {
        let comments = vec![notification(B), new_round(2, C, B)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert_eq!(
            anomalies,
            vec![RoundAnomaly::RoundExtended {
                comment_index: 1,
                reason: ExtensionReason::RoundStillOpen,
            }]
        );
        let round = &rounds[0];
        assert_eq!(round.index, 1);
        assert_eq!(round.state, RawRoundState::Open);
        // The comment's checklist is adopted; the anchor stays where it was.
        assert_eq!(
            round.checklist,
            ChecklistSource::Comment {
                comment_index: 1,
                comment_id: None,
                comment_url: None,
            }
        );
        assert_eq!(round.opened_at, A);
        assert_eq!(round.extensions.len(), 1);
        assert_eq!(round.extensions[0].comment_index, 1);
        assert_eq!(round.extensions[0].at_commit, Some(C));
        assert_eq!(round.extensions[0].note, Some("second pass"));
    }

    #[test]
    fn new_round_with_mismatched_written_index_extends_instead_of_opening() {
        let comments = vec![approval(B), new_round(7, D, C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1, "a mismatched index must not open a round");
        assert_eq!(
            anomalies,
            vec![RoundAnomaly::RoundExtended {
                comment_index: 1,
                reason: ExtensionReason::IndexMismatch {
                    written: 7,
                    derived: 2,
                },
            }]
        );
        assert_eq!(rounds[0].state, RawRoundState::Open);
        assert_eq!(
            rounds[0].checklist,
            ChecklistSource::Comment {
                comment_index: 1,
                comment_id: None,
                comment_url: None,
            }
        );
        assert_eq!(rounds[0].opened_at, A);

        // Reopening the round clears its standing approval.
        let thread = thread(&[A, B, C, D], A, &comments);
        assert_eq!(thread.rounds().count(), 1);
        assert_eq!(thread.standing_approval(), None);
        assert_eq!(round_at(&thread, 0).extensions.len(), 1);
        assert_eq!(round_at(&thread, 0).extensions[0].at_commit, Some(oid(D)));
    }

    #[test]
    fn new_round_base_is_derived_and_written_mismatch_is_warn_only() {
        // Matching `round:`, but the comment wrote the wrong previous approval.
        let comments = vec![approval(B), new_round(2, D, C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[1].index, 2);
        // Derived, not written: round 2's base is round 1's closing commit.
        assert_eq!(rounds[1].previous_approval, Some(B));
        assert_eq!(rounds[1].opened_at, D);
        assert_eq!(
            anomalies,
            vec![RoundAnomaly::BaseCommitMismatch {
                comment_index: 1,
                written: C.to_string(),
                derived: B.to_string(),
            }]
        );
    }

    #[test]
    fn new_round_without_written_index_opens_the_derived_round() {
        let body = format!("# QC Round\n\n## Metadata\ninitial qc round commit: {D}\n");
        let comments = vec![approval(B), comment(&body)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");
        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[1].index, 2);
        assert_eq!(rounds[1].previous_approval, Some(B));
        assert_eq!(rounds[1].opened_at, D);
    }

    #[test]
    fn event_after_close_is_recorded_and_flagged() {
        let comments = vec![approval(B), notification(C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].events.len(), 1);
        assert_eq!(
            anomalies,
            vec![RoundAnomaly::EventAfterClose { comment_index: 1 }]
        );
    }

    #[test]
    fn review_events_are_folded_onto_the_current_round() {
        let comments = vec![notification(B), review(B)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);
        assert_eq!(rounds[0].events.len(), 2);
        assert!(matches!(
            rounds[0].events[1],
            RawRoundEvent::Review { commit, .. } if commit == B
        ));
    }

    /// Round events are the only record of what the comments said, so one commit
    /// legitimately carries several of them — a review, more reviews, then the approval
    /// that closes the round at it.
    #[test]
    fn one_commit_can_carry_several_events_and_then_close_the_round() {
        let comments = vec![review(B), review(B), approval(B)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].events.len(), 2);
        assert!(
            rounds[0]
                .events
                .iter()
                .all(|event| matches!(event, RawRoundEvent::Review { commit, .. } if *commit == B))
        );
        assert!(matches!(
            rounds[0].state,
            RawRoundState::Closed { commit, .. } if commit == B
        ));

        let thread = thread(&[A, B], A, &comments);
        assert_eq!(round_at(&thread, 0).events.len(), 2);
        assert_eq!(thread.standing_approval(), Some(&oid(B)));
    }

    // ── Stage 2: segments own their commits ──────────────────────────────────

    /// The ownership rule, end to end: a round owns its anchor through its closing
    /// commit, and the gap owns only what sits strictly between two rounds. No commit
    /// is left owned by nothing — which is exactly what the anchors used to be.
    #[test]
    fn segments_own_their_anchors_and_gaps_own_the_drift_between_rounds() {
        let comments = vec![notification(B), approval(B), new_round_on(2, D, B, MAIN)];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        assert_eq!(thread.segments.len(), 3);

        // Initial QC: from its closing commit B back to its anchor A, inclusive.
        assert_eq!(
            owned(&thread.segments[0]),
            vec![B.to_string(), A.to_string()]
        );
        // The gap: strictly between round 1's approval and round 2's anchor.
        assert_eq!(owned(&thread.segments[1]), vec![C.to_string()]);
        assert_eq!(gap_at(&thread, 1).continuity, GapContinuity::Linear);
        // Round 2 is open: from the branch tip back to its own anchor, inclusive. The
        // anchor belongs to the round that opened at it, not to the gap before it.
        assert_eq!(
            owned(&thread.segments[2]),
            vec![E.to_string(), D.to_string()]
        );
        assert_eq!(round_at(&thread, 2).opened_at, oid(D));

        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    #[test]
    fn a_closed_final_round_is_followed_by_a_gap_even_when_it_is_empty() {
        let comments = vec![notification(B), approval(B)];
        let thread = thread(&[A, B], A, &comments);

        assert_eq!(thread.segments.len(), 2);
        assert!(thread.segments[1].as_gap().is_some());
        assert!(thread.segments[1].commits().is_empty());
        assert_eq!(thread.segments[1].branch(), MAIN);
        assert_eq!(thread.standing_approval(), Some(&oid(B)));
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// A retraction reopens the round, which absorbs the gap that followed it: those
    /// commits become drift arriving during review rather than drift after approval.
    #[test]
    fn retraction_absorbs_the_gap_that_followed_the_round() {
        let approved = vec![approval(B)];
        let before = thread(&[A, B, C], A, &approved);
        assert_eq!(before.segments.len(), 2);
        assert_eq!(owned(&before.segments[1]), vec![C.to_string()]);

        let retracted = vec![approval(B), unapproval()];
        let after = thread(&[A, B, C], A, &retracted);
        assert_eq!(after.segments.len(), 1);
        assert_eq!(
            owned(&after.segments[0]),
            vec![C.to_string(), B.to_string(), A.to_string()],
            "the gap's commits become the reopened round's"
        );
        assert_eq!(segment_invariants(&after.segments, None), Ok(()));
    }

    /// A round opened without HEAD having moved since the approval anchors *at* that
    /// approval. Sharing that one boundary commit is legitimate, and the only sharing
    /// the ownership invariant allows.
    #[test]
    fn a_round_anchored_at_the_previous_approval_shares_only_that_boundary() {
        let comments = vec![approval(B), new_round_on(2, B, B, MAIN)];
        let thread = thread(&[A, B, C], A, &comments);

        assert_eq!(thread.segments.len(), 3);
        assert_eq!(
            owned(&thread.segments[0]),
            vec![B.to_string(), A.to_string()]
        );
        assert!(thread.segments[1].commits().is_empty());
        assert_eq!(
            owned(&thread.segments[2]),
            vec![C.to_string(), B.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    #[test]
    fn short_shas_resolve_in_stage_two() {
        let short = &B[..7];
        let comments = vec![notification(short), approval(short)];
        let thread = thread(&[A, B], A, &comments);

        assert_eq!(thread.rounds().count(), 1);
        assert!(thread.anomalies.is_empty());
        assert_eq!(thread.standing_approval(), Some(&oid(B)));
        assert_eq!(
            round_at(&thread, 0)
                .latest_actioned_commit()
                .map(|commit| commit.hash),
            Some(oid(B)),
            "the notification's short sha resolved to the full commit"
        );
    }

    #[test]
    fn single_commit_thread_has_one_open_round_owning_it() {
        let comments: Vec<GitComment> = vec![];
        let thread = thread(&[A], A, &comments);

        assert_eq!(thread.segments.len(), 1);
        assert_eq!(owned(&thread.segments[0]), vec![A.to_string()]);
        assert!(round_at(&thread, 0).is_open());
        assert_eq!(thread.standing_approval(), None);
        assert_eq!(thread.last_approved_commit(), None);
        assert_eq!(thread.next_notification_from(), Some(oid(A)));
    }

    /// One function answers for both call shapes: a caller holding a `&Round`, and one
    /// holding only the bare index a metadata file gives it. The `Initial QC` string is
    /// spec-pinned, so a divergence between the two spellings must fail the suite.
    #[test]
    fn the_naming_rule_has_one_home_for_both_call_shapes() {
        assert_eq!(Round::name_of(1), "Initial QC");
        assert_eq!(Round::name_of(2), "Round 2");
        assert_eq!(Round::name_of(7), "Round 7");

        let thread = thread(
            &[A, B, C, D, E],
            A,
            &[
                approval(B),
                new_round(2, C, B),
                approval(D),
                new_round(3, E, D),
            ],
        );
        assert_eq!(thread.rounds().count(), 3);
        for round in thread.rounds() {
            assert_eq!(
                round.name(),
                Round::name_of(round.index),
                "a round's own name must be what the index-only caller renders"
            );
        }
    }

    // ── The round's latest actioned commit (archive M4) ──────────────────────

    /// Drift on top of the newest notification is not an actioned commit: nobody put it
    /// up for review. This is the whole difference between this accessor and
    /// `commits[0]`.
    #[test]
    fn the_latest_actioned_commit_ignores_drift_nobody_acted_on() {
        let thread = thread(&[A, B, C, D], A, &[notification(B), review(C)]);
        let round = round_at(&thread, 0);

        assert_eq!(owned(&thread.segments[0])[0], D.to_string());
        assert_eq!(
            round.latest_actioned_commit().map(|commit| commit.hash),
            Some(oid(C))
        );
    }

    /// With nothing acted on yet, the round's own anchor is the answer.
    #[test]
    fn a_round_with_no_events_reports_its_anchor_as_actioned() {
        let thread = thread(&[A, B], A, &[]);

        assert_eq!(
            round_at(&thread, 0)
                .latest_actioned_commit()
                .map(|commit| commit.hash),
            Some(oid(A))
        );
    }

    /// An approval is not a `RoundEvent`, so a closed round can report an actioned
    /// commit *older* than the commit it closed at. A consumer wanting the approval
    /// reads `closing_commit`.
    #[test]
    fn a_closed_rounds_latest_actioned_commit_can_predate_its_approval() {
        let thread = thread(&[A, B, C], A, &[notification(B), approval(C)]);
        let round = round_at(&thread, 0);

        assert_eq!(round.closing_commit(), Some(&oid(C)));
        assert_eq!(
            round.latest_actioned_commit().map(|commit| commit.hash),
            Some(oid(B))
        );
    }

    /// An unplaceable round owns no commits and its anchor is a placeholder, so there is
    /// no commit to report — the same gate `next_notification_from` applies.
    #[test]
    fn an_unplaceable_round_has_no_actioned_commit() {
        let missing = "fffffff000000000000000000000000000000009";
        let thread = thread(&[A, B], missing, &[]);

        assert!(!round_at(&thread, 0).is_placed());
        assert_eq!(round_at(&thread, 0).latest_actioned_commit(), None);
        assert_eq!(thread.next_notification_from(), None);
    }

    /// The identity `next_notification_from` is built on: an open round's notification
    /// base *is* its latest actioned commit.
    #[test]
    fn the_notification_base_of_an_open_round_is_its_latest_actioned_commit() {
        let thread = thread(&[A, B, C, D], A, &[notification(B), review(C)]);

        assert_eq!(
            thread.next_notification_from(),
            round_at(&thread, 0)
                .latest_actioned_commit()
                .map(|commit| commit.hash)
        );
    }

    // ── Degradation: unplaceable segments ────────────────────────────────────

    /// An anchor that is on no branch we walked grays its round rather than widening it
    /// to the whole history — showing plausible-but-wrong commits is worse than none.
    #[test]
    fn an_unresolvable_anchor_grays_the_round() {
        let missing = "fffffff000000000000000000000000000000009";
        let thread = thread(&[A, B], missing, &[]);

        assert_eq!(thread.segments.len(), 1);
        assert_eq!(
            round_at(&thread, 0).placement,
            Placement::Unplaceable(UnplaceableReason::AnchorUnreachable)
        );
        assert!(thread.segments[0].commits().is_empty());
        assert!(thread.anomalies.contains(&RoundAnomaly::AnchorUnreachable {
            comment_index: 0,
            sha: missing.to_string(),
        }));
        assert!(
            thread
                .anomalies
                .contains(&RoundAnomaly::SegmentUnplaceable {
                    position: 0,
                    reason: UnplaceableReason::AnchorUnreachable,
                })
        );
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// A round comment with no `git branch` is malformed input, not a legacy case:
    /// nothing predating round comments can produce one.
    #[test]
    fn a_round_without_a_declared_branch_is_unplaceable() {
        let comments = vec![approval(B), new_round(2, C, B)];
        let thread = thread(&[A, B, C, D], A, &comments);

        assert_eq!(thread.segments.len(), 3);
        assert_eq!(
            round_at(&thread, 2).placement,
            Placement::Unplaceable(UnplaceableReason::BranchNotDeclared)
        );
        assert!(thread.segments[2].commits().is_empty());
        assert!(
            thread
                .anomalies
                .contains(&RoundAnomaly::BranchNotDeclared { comment_index: 1 })
        );
        assert!(
            thread
                .anomalies
                .contains(&RoundAnomaly::SegmentUnplaceable {
                    position: 2,
                    reason: UnplaceableReason::BranchNotDeclared,
                })
        );
        // W6: the gap beside it cannot be placed either, and is *not* substituted onto
        // round 1's branch — that would be a real commit set which is not this gap.
        assert_eq!(
            gap_at(&thread, 1).placement,
            Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable)
        );
        assert!(thread.segments[1].commits().is_empty());
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// A branch that was never fetched, or has been deleted, grays every segment on it.
    #[test]
    fn an_unavailable_branch_grays_its_segments() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B]))),
            (
                FEATURE.to_string(),
                Err(UnplaceableReason::BranchUnavailable),
            ),
        ]);
        let comments = vec![approval(B), new_round_on(2, B, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert_eq!(thread.segments.len(), 3);
        assert_eq!(
            round_at(&thread, 2).placement,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable)
        );
        assert_eq!(round_at(&thread, 2).branch, FEATURE);
        assert_eq!(
            gap_at(&thread, 1).placement,
            Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable)
        );
        // Round 1 is untouched: its own branch was walked.
        assert!(round_at(&thread, 0).is_placed());
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// An anchor that resolves on another branch but not on the round's own is still
    /// identified — the round is unplaceable, not unknown.
    #[test]
    fn an_anchor_off_its_own_branch_is_identified_but_unplaceable() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (FEATURE.to_string(), Ok(walk(&[A, D]))),
        ]);
        // Round 2 declares `feature/x` but anchors at C, which only exists on main.
        let comments = vec![approval(B), new_round_on(2, C, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        let round_two = round_at(&thread, 2);
        assert_eq!(
            round_two.opened_at,
            oid(C),
            "the anchor is still identified"
        );
        assert_eq!(
            round_two.placement,
            Placement::Unplaceable(UnplaceableReason::AnchorUnreachable)
        );
        assert!(round_two.commits.is_empty());
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// The anchor-older-than-base guard compares positions *within one walk*. An anchor
    /// that is off the round's own branch has no position there, so it is not "older"
    /// than the base: rewriting it to the base would emit a false anomaly and throw away
    /// the one thing the round still knows about itself.
    #[test]
    fn an_off_walk_anchor_is_not_treated_as_older_than_its_base() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, D, E]))),
        ]);
        // Round 1 closes at B; round 2 declares `feature/x` but anchors at C, which only
        // exists on main — so its anchor is off its own walk while the base, B, is on it.
        let comments = vec![approval(B), new_round_on(2, C, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        let round_two = round_at(&thread, 2);
        assert_eq!(
            round_two.opened_at,
            oid(C),
            "the anchor is still identified"
        );
        assert!(
            !thread
                .anomalies
                .iter()
                .any(|anomaly| matches!(anomaly, RoundAnomaly::AnchorOlderThanBase { .. })),
            "anomalies: {:?}",
            thread.anomalies
        );
        assert_eq!(
            round_two.placement,
            Placement::Unplaceable(UnplaceableReason::AnchorUnreachable)
        );
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// An approval whose commit is on no branch we walked is still an approval that
    /// happened — we simply cannot say where. Silently reopening the round would report
    /// the issue as awaiting review, so it grays instead (**M5**, **D4**).
    #[test]
    fn an_unresolvable_approval_grays_the_round_instead_of_reopening_it() {
        let missing = "fffffff000000000000000000000000000000009";
        let thread = thread(&[A, B], A, &[approval(missing)]);

        let round_one = round_at(&thread, 0);
        assert_eq!(
            round_one.placement,
            Placement::Unplaceable(UnplaceableReason::AnchorUnreachable)
        );
        assert!(round_one.commits.is_empty());
        assert!(round_one.status().is_none(), "a grayed round has no status");
        assert!(thread.anomalies.contains(&RoundAnomaly::AnchorUnreachable {
            comment_index: 0,
            sha: missing.to_string(),
        }));
        assert_eq!(
            segment_invariants(&thread.segments, Some(&one_branch(&[A, B]))),
            Ok(())
        );
    }

    #[test]
    fn walking_nothing_at_all_still_yields_initial_qc_grayed() {
        let walks = BranchWalks::new();
        let thread = thread_with(&walks, A, &[], &no_merge_base);

        assert_eq!(thread.segments.len(), 1);
        assert_eq!(
            round_at(&thread, 0).placement,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable)
        );
        assert!(thread.segments[0].commits().is_empty());
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    // ── Gap continuity ──────────────────────────────────────────────────────

    /// Round 2 on another branch: the gap's ends do not sit on one line, so it walks
    /// back to the merge-base instead. Divergence is a property of the gap, not a
    /// special case of starting a round.
    #[test]
    fn a_diverged_gap_walks_back_to_the_merge_base() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (FEATURE.to_string(), Ok(walk(&[A, D, E, F]))),
        ]);
        let merge_base = |_: &ObjectId, _: &ObjectId| Some(oid(A));
        let comments = vec![approval(B), new_round_on(2, E, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &merge_base);

        assert_eq!(thread.segments.len(), 3);
        assert_eq!(gap_at(&thread, 1).branch, FEATURE, "W1: the newer round's");
        assert_eq!(
            gap_at(&thread, 1).continuity,
            GapContinuity::Diverged { merge_base: oid(A) }
        );
        // From round 2's anchor (exclusive) back to the merge-base (exclusive).
        assert_eq!(owned(&thread.segments[1]), vec![D.to_string()]);
        assert_eq!(
            owned(&thread.segments[2]),
            vec![F.to_string(), E.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// A merge-base that is not on the gap's own walk cannot bound it — walks are
    /// bounded, so a merge-base older than the walked range simply is not there. Running
    /// back to the walk's oldest commit instead would claim commits the older round owns,
    /// double-counting them (**I4**, **I5**, **W5**), so the gap is grayed.
    #[test]
    fn a_merge_base_off_the_gaps_own_walk_grays_the_gap() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, D, E]))),
        ]);
        // The merge-base is a commit neither walk reaches.
        let merge_base = |_: &ObjectId, _: &ObjectId| Some(oid(F));
        // Round 1 closes at C, which is only on main; round 2 anchors at E on the feature
        // branch, so the gap between them has to look for a lower bound.
        let comments = vec![approval(C), new_round_on(2, E, C, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &merge_base);

        assert_eq!(
            gap_at(&thread, 1).placement,
            Placement::Unplaceable(UnplaceableReason::MergeBaseUnreachable),
            "both bounds resolved; it is their common ancestor that is off the walk"
        );
        assert!(thread.segments[1].commits().is_empty());
        assert!(
            thread
                .anomalies
                .contains(&RoundAnomaly::SegmentUnplaceable {
                    position: 1,
                    reason: UnplaceableReason::MergeBaseUnreachable,
                })
        );
        // Round 1 keeps every commit it owns; nothing is owned twice.
        assert_eq!(
            owned(&thread.segments[0]),
            vec![C.to_string(), B.to_string(), A.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// Histories that share nothing have no meaningful diff between them, so the gap
    /// claims nothing rather than claiming a whole branch.
    #[test]
    fn an_unrelated_gap_owns_nothing() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B]))),
            (FEATURE.to_string(), Ok(walk(&[D, E, F]))),
        ]);
        let comments = vec![approval(B), new_round_on(2, E, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert_eq!(gap_at(&thread, 1).continuity, GapContinuity::Unrelated);
        assert!(gap_at(&thread, 1).is_placed());
        assert!(thread.segments[1].commits().is_empty());
        assert_eq!(
            owned(&thread.segments[2]),
            vec![F.to_string(), E.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// A trailing gap walks on the previous round's branch — never on the viewer's
    /// checkout, so two people on different branches see the same segments.
    #[test]
    fn a_trailing_gap_walks_on_the_previous_rounds_branch() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, D, E]))),
        ]);
        let comments = vec![approval(B), new_round_on(2, D, B, FEATURE), approval(D)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert_eq!(thread.segments.len(), 4);
        assert_eq!(gap_at(&thread, 3).branch, FEATURE);
        assert_eq!(owned(&thread.segments[3]), vec![E.to_string()]);
        assert_eq!(gap_at(&thread, 3).continuity, GapContinuity::Linear);
        // The standing approval is the round bounding this gap's older end, not the
        // one two positions back — the offset only holds between rounds.
        assert_eq!(thread.standing_approval(), Some(&oid(D)));
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    /// A trailing gap is *always* `Linear`: it walks the branch of the placed, closed
    /// round bounding it, and that round's closing commit is by construction on that
    /// walk. So it never asks for a merge-base, and can never report `Diverged` or
    /// `Unrelated` — the shapes that would let a trailing gap misreport `Approved`.
    #[test]
    fn a_trailing_gap_is_always_linear() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, D, E]))),
        ]);
        // Asking for a merge-base at all would be a bug: fail loudly if the trailing gap
        // ever reaches for one. The feature walk contains main's history through B, so
        // nothing diverges here — the interior gap finds its own lower bound, B, by
        // position on the feature walk. Near-duplicate of
        // `a_trailing_gap_walks_on_the_previous_rounds_branch`; both are kept because that
        // one pins *which branch* the gap walks and this one pins that it never needs a
        // merge-base to do it.
        let merge_base = |_: &ObjectId, _: &ObjectId| -> Option<ObjectId> {
            panic!("a trailing gap's lower bound is on its own walk; no merge-base needed")
        };
        let comments = vec![approval(B), new_round_on(2, D, B, FEATURE), approval(D)];
        let thread = thread_with(&walks, A, &comments, &merge_base);

        let trailing = gap_at(&thread, 3);
        assert_eq!(trailing.continuity, GapContinuity::Linear);
        assert!(trailing.is_placed());
        assert_eq!(owned(&thread.segments[3]), vec![E.to_string()]);
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    // ── Invariants ──────────────────────────────────────────────────────────

    #[test]
    fn segment_invariants_reject_every_shape_the_model_forbids() {
        // `resolve_segments` asserts all of this after every fold, which is a guard
        // wherever debug assertions are on — every test and dev build.
        let base = thread(&[A, B, C, D, E], A, &[approval(B), new_round(2, D, B)]);
        // Round 2 declares no branch in that thread, so build a placed one by hand.
        let placed = thread(&[A, B, C], A, &[approval(B)]);
        assert_eq!(segment_invariants(&placed.segments, None), Ok(()));

        // A Gap can never be first: Initial QC opens the thread.
        assert!(segment_invariants(&placed.segments[1..], None).is_err());
        // A closed Round can never be last: the drift after it is a segment of its own.
        assert!(segment_invariants(&placed.segments[..1], None).is_err());
        // Nothing at all is not a thread.
        assert!(segment_invariants(&[], None).is_err());

        // An unplaceable segment owns nothing.
        let mut owning = placed.segments.clone();
        if let Segment::Round(round) = &mut owning[0] {
            round.placement = Placement::Unplaceable(UnplaceableReason::AnchorUnreachable);
        }
        assert!(segment_invariants(&owning, None).is_err());

        // Two segments claiming the same commit, and not as a shared boundary.
        let mut overlapping = placed.segments.clone();
        let stolen = placed.segments[0].commits()[0].clone();
        if let Segment::Gap(gap) = &mut overlapping[1] {
            gap.commits.push(stolen);
        }
        assert!(segment_invariants(&overlapping, None).is_err());

        // Alternation is strict.
        let mut doubled = base.segments.clone();
        doubled.insert(1, base.segments[0].clone());
        assert!(segment_invariants(&doubled, None).is_err());
    }

    /// Given the walks, the commits the placed segments own must be *contiguous*: a
    /// segment that owns too few commits is as much an **I4**/**I5** violation as one
    /// that owns a commit twice, and it is the failure mode an unbounded gap produces.
    #[test]
    fn segment_invariants_reject_a_hole_between_owned_walk_commits() {
        let walks = one_branch(&[A, B, C, D, E]);
        let comments = vec![approval(B), new_round_on(2, D, B, MAIN)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        // Round 1 owns B..A, the gap owns C, round 2 owns E..D.
        assert_eq!(owned(&thread.segments[1]), vec![C.to_string()]);
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));

        // Shrink the gap: nothing overlaps, so only the contiguity check can see that C
        // has become a hole between round 1's B..A and round 2's E..D.
        let mut shrunk = thread.segments.clone();
        if let Segment::Gap(gap) = &mut shrunk[1] {
            gap.commits.clear();
        }
        assert_eq!(segment_invariants(&shrunk, None), Ok(()));
        assert_eq!(
            segment_invariants(&shrunk, Some(&walks)),
            Err(format!(
                "commit {C} is owned by no segment, between commits that are (walk {MAIN})"
            ))
        );

        // Unowned commits at the *newest* end are not a hole: that is what an unplaceable
        // segment leaves behind, and its would-be range is unknown, so nothing can say
        // those commits should have been owned.
        let mut trimmed = thread.segments.clone();
        if let Segment::Round(round) = &mut trimmed[2] {
            round.commits.clear();
            round.placement = Placement::Unplaceable(UnplaceableReason::AnchorUnreachable);
        }
        assert_eq!(segment_invariants(&trimmed, Some(&walks)), Ok(()));

        // And an unplaceable segment elsewhere in the thread does not excuse a hole: this
        // is what a check gated on *every* segment being placed cannot see.
        let closed = vec![approval(B), new_round_on(2, D, B, MAIN), approval(D)];
        let mut mixed = thread_with(&walks, A, &closed, &no_merge_base).segments;
        assert_eq!(owned(&mixed[3]), vec![E.to_string()]);
        if let Segment::Gap(gap) = &mut mixed[3] {
            gap.commits.clear();
            gap.placement = Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable);
        }
        if let Segment::Gap(gap) = &mut mixed[1] {
            gap.commits.clear();
        }
        assert_eq!(
            segment_invariants(&mixed, Some(&walks)),
            Err(format!(
                "commit {C} is owned by no segment, between commits that are (walk {MAIN})"
            ))
        );
    }

    /// Which overlap is reported must not depend on `HashMap` iteration order. `owners` is
    /// rebuilt — with a fresh `RandomState` — on every call, so a thread carrying two
    /// violations reports whichever the iteration happened to reach first unless the
    /// violations are ordered explicitly. An invariant message that names a different
    /// commit from run to run is unreproducible in exactly the situation it diagnoses.
    #[test]
    fn the_reported_overlap_does_not_depend_on_hashmap_order() {
        // The trailing gap illegally re-claims both of round 1's commits, so A and B are
        // each owned by segments [0, 1] and either could be reported.
        let thread = thread(&[A, B, C], A, &[approval(B)]);
        let mut overlapping = thread.segments.clone();
        if let Segment::Gap(gap) = &mut overlapping[1] {
            gap.commits.extend(
                thread.segments[0]
                    .commits()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            );
        }

        // Lowest position first, then lowest hash: A precedes B.
        let expected = Err(format!("commit {A} is owned by segments [0, 1]"));
        for _ in 0..64 {
            assert_eq!(segment_invariants(&overlapping, None), expected);
        }
    }

    /// Contiguity is checked on *every* walk, not just on threads that have one. A
    /// genuinely diverged thread — round 2 on another branch, its gap bounded by the
    /// merge-base rather than by the previous approval (**W5**) — leaves each walk with a
    /// clean run, so the stronger check does not gray anything legitimate.
    #[test]
    fn a_diverged_multi_branch_thread_is_contiguous_on_every_walk() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (FEATURE.to_string(), Ok(walk(&[A, D, E, F]))),
        ]);
        // Feature branched from main at A, so the gap between round 1's approval (B, on
        // main only) and round 2's anchor (E, on feature) has to fall back to the
        // merge-base for its lower bound.
        let merge_base = |_: &ObjectId, _: &ObjectId| Some(oid(A));
        let comments = vec![approval(B), new_round_on(2, E, B, FEATURE), approval(F)];
        let thread = thread_with(&walks, A, &comments, &merge_base);

        // main:    C   B   A       feature:  F   E   D   A
        //              └R1┘                  └R2─┘  Gap └R1
        assert_eq!(thread.segments.len(), 4);
        assert_eq!(
            gap_at(&thread, 1).continuity,
            GapContinuity::Diverged { merge_base: oid(A) }
        );
        assert_eq!(owned(&thread.segments[1]), vec![D.to_string()]);
        assert_eq!(
            owned(&thread.segments[2]),
            vec![F.to_string(), E.to_string()]
        );
        // The trailing gap is empty: round 2 closed at the feature tip.
        assert!(thread.segments[3].commits().is_empty());
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// The reason ownership is read per *walk* and not per *branch*: round 3 returns to
    /// main, so main's walk holds an owned run that C interrupts — and C is owned by the
    /// gap before round 2, which walks `feature/x` (**W1**). Grouping the owned commits by
    /// each segment's own branch would call C a hole in main's walk, graying a thread in
    /// which every commit is owned exactly once.
    #[test]
    fn a_commit_owned_on_another_branch_is_not_a_hole_in_this_walk() {
        /// A sixth commit, so main and feature can each be five long.
        const G: &str = "9999999000000000000000000000000000000007";
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C, F, G]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, C, D, E]))),
        ]);
        // Round 2's approval E is on feature only, so the gap that follows it looks for a
        // merge-base; feature left main at C.
        let merge_base = |_: &ObjectId, _: &ObjectId| Some(oid(C));
        let comments = vec![
            approval(B),
            new_round_on(2, D, B, FEATURE),
            approval(E),
            new_round_on(3, F, E, MAIN),
        ];
        let thread = thread_with(&walks, A, &comments, &merge_base);

        // main:    G   F   C   B   A       feature:  E   D   C   B   A
        //          └R3─┘  ?   └R1┘                   └R2─┘ Gap  └R1─┘
        assert_eq!(thread.segments.len(), 5);
        assert_eq!(gap_at(&thread, 1).branch, FEATURE, "W1: round 2's branch");
        assert_eq!(
            owned(&thread.segments[1]),
            vec![C.to_string()],
            "C is the gap's, and the gap is on feature/x"
        );
        assert_eq!(gap_at(&thread, 3).branch, MAIN, "W1: round 3's branch");
        assert!(
            thread.segments[3].commits().is_empty(),
            "round 3 anchors at the merge-base's child, so nothing sits between them"
        );
        assert_eq!(
            owned(&thread.segments[4]),
            vec![G.to_string(), F.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// The hole the single-walk guard could not see: it lives on the second branch's walk,
    /// which a check that ran only when there was exactly one walk skipped entirely.
    #[test]
    fn a_hole_on_a_second_branchs_walk_is_rejected() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, C, D, E]))),
        ]);
        // Round 2 on feature anchors at D, so the gap before it owns C.
        let comments = vec![approval(B), new_round_on(2, D, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert_eq!(owned(&thread.segments[1]), vec![C.to_string()]);
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));

        // Shrink that gap. Nothing overlaps, and main's walk is untouched and still
        // clean — only the feature walk shows C as a hole between round 1's B and round
        // 2's D.
        let mut shrunk = thread.segments.clone();
        if let Segment::Gap(gap) = &mut shrunk[1] {
            gap.commits.clear();
        }
        assert_eq!(
            segment_invariants(&shrunk, Some(&walks)),
            Err(format!(
                "commit {C} is owned by no segment, between commits that are (walk {FEATURE})"
            ))
        );
    }

    /// The UI keeps its own copy of these strings (`unplaceableReasonText` in
    /// `ui/src/utils/rounds.ts`), because it renders a reason for any segment the wire
    /// describes only by `placement.reason`. The **backend's** wording is the one on the
    /// wire — a repair's `skipped_reason` carries it verbatim and `ghqc issue status`
    /// prints it — so the two copies must agree, or a card and a modal explain the same
    /// degradation two different ways in one view.
    ///
    /// A new variant needs adding to the list below as well as to `describe()`.
    #[test]
    fn every_reason_wording_matches_the_ui_copy() {
        let ui = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src/utils/rounds.ts"),
        )
        .expect("the UI's reason map is readable");

        for reason in [
            UnplaceableReason::BranchNotDeclared,
            UnplaceableReason::BranchUnavailable,
            UnplaceableReason::AnchorUnreachable,
            UnplaceableReason::MergeBaseUnreachable,
            UnplaceableReason::NeighbourUnplaceable,
        ] {
            let wording = reason.describe();
            assert!(
                ui.contains(&format!("'{wording}'")),
                "{reason:?}: `{wording}` is not in ui/src/utils/rounds.ts — the Rust and TS \
                 copies of this wording have drifted"
            );
        }
    }

    /// An empty walk leaves a trailing gap no tip to bound itself at. Nothing is missing
    /// *on* the branch — the branch yielded nothing at all — so `BranchUnavailable` is the
    /// true reason, where `AnchorUnreachable` would send the user hunting a commit that is
    /// not lost.
    ///
    /// Exercised through [`build_gap`] directly, because via the fold this arm is
    /// unreachable: a trailing gap walks its own round's branch (**W1**), so an empty walk
    /// would already have left that round `Unplaceable` and the gap
    /// `NeighbourUnplaceable` before this bound is ever computed.
    #[test]
    fn an_empty_walk_reports_the_branch_as_unavailable() {
        let placed = thread(&[A, B, C], A, &[approval(B)]);
        let older = round_at(&placed, 0);
        assert!(older.is_placed(), "the older round must be placed");

        let empty = BranchWalks::from([(MAIN.to_string(), Ok(Vec::new()))]);
        let mut anomalies = Vec::new();
        let gap = build_gap(older, None, 1, &empty, &no_merge_base, &mut anomalies);

        assert_eq!(
            gap.placement,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable)
        );
        assert!(gap.commits.is_empty());
        assert_eq!(
            anomalies,
            vec![RoundAnomaly::SegmentUnplaceable {
                position: 1,
                reason: UnplaceableReason::BranchUnavailable,
            }]
        );
    }

    /// An interior `Unrelated` gap owns nothing (**M4**/**D15**), so the commits between
    /// its bounds are owned by nobody — and unlike the `Unplaceable` case, every segment
    /// here is `Placed`, so the unplaceable-span excuse cannot account for the hole.
    ///
    /// Round 2 sits on a branch sharing no history with `main`, so the gap before round 3
    /// walks `main` (**W1**) with a lower bound — round 2's approval — that `main` does not
    /// contain and no merge-base can replace. C is left unowned, interior to `main`'s owned
    /// window. Treated as a hole this fires the fold's `debug_assert` on entirely valid
    /// input, which is worse than not checking at all; an `Unrelated` gap's range is as
    /// unknowable as an unplaceable segment's, so it excuses the hole it spans.
    #[test]
    fn an_interior_unrelated_gap_excuses_the_hole_it_spans() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C, E]))),
            (FEATURE.to_string(), Ok(walk(&[D, F]))),
        ]);
        // Round 2's approval F is on feature, whose history is unrelated to main's.
        let comments = vec![
            approval(B),
            new_round_on(2, D, B, FEATURE),
            approval(F),
            new_round_on(3, E, F, MAIN),
        ];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        // main:    E   C   B   A          feature:  F   D
        //          └R3┘  ?   └R1┘                   └R2─┘
        assert_eq!(thread.segments.len(), 5);
        assert_eq!(gap_at(&thread, 3).branch, MAIN, "W1: round 3's branch");
        assert_eq!(
            gap_at(&thread, 3).continuity,
            GapContinuity::Unrelated,
            "main cannot reach round 2's approval and there is no merge-base"
        );
        assert!(
            thread.segments[3].is_placed(),
            "D15: an Unrelated gap is Placed-but-empty, not Unplaceable"
        );
        assert!(thread.segments[3].commits().is_empty());
        // Every segment is placed, so only the Unrelated gap can excuse C.
        assert!(thread.segments.iter().all(|segment| segment.is_placed()));
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// One segment owning commits on **both** sides of a hole means this walk orders its
    /// commits differently from the walk they were taken from — topology, not a hole. The
    /// `first >= last` guard is what keeps that from being reported.
    ///
    /// `feature/x` interleaves D between round 1's A and B, which is what a date-ordered
    /// `rev-list` over a merge produces. Round 1 then owns commits either side of D on that
    /// walk, so the hole's bounding commits resolve to the *same* segment and the span
    /// between them is empty or inverted. Without the guard this is a false positive; with
    /// the guard and direct indexing instead of `.get(..)` it is a panic.
    #[test]
    fn a_segment_owning_both_sides_of_a_hole_is_topology_not_a_hole() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            // D interleaved between A and B, as a merge's date ordering can place it.
            (FEATURE.to_string(), Ok(walk(&[A, D, B, C]))),
        ]);
        let comments = vec![approval(B), new_round_on(2, C, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        // feature:  C   B   D   A
        //           └R2┘  ?  └R1─┘   — R1 owns B and A, either side of D
        assert_eq!(
            owned(&thread.segments[0]),
            vec![B.to_string(), A.to_string()]
        );
        assert_eq!(owned(&thread.segments[2]), vec![C.to_string()]);
        assert!(thread.segments.iter().all(|segment| segment.is_placed()));
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// The `Diverged` lower bound is the merge-base *on this walk*, and an off-walk
    /// merge-base grays the gap — even when the older round's commits are nowhere on that
    /// walk, so nothing the gap over-claims would collide with them.
    ///
    /// This is the one shape of that defect the invariants cannot see, and the reason it
    /// is pinned here directly. Bounding the gap at the walk's oldest end instead of
    /// graying it makes it own more, never less, so contiguity — which only detects
    /// owning *too few* — is blind by construction; and with the older round's history
    /// absent from this walk there is no double-ownership for the overlap check to catch
    /// either. Every clause of [`segment_invariants`] passes on the wrong answer.
    #[test]
    fn an_off_walk_merge_base_grays_the_gap_even_with_nothing_to_overlap() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B]))),
            (FEATURE.to_string(), Ok(walk(&[C, D, E]))),
        ]);
        // A merge-base on neither walk: the gap has no bound it can use.
        let merge_base = |_: &ObjectId, _: &ObjectId| Some(oid(F));
        let comments = vec![approval(B), new_round_on(2, D, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &merge_base);

        assert_eq!(
            gap_at(&thread, 1).placement,
            Placement::Unplaceable(UnplaceableReason::MergeBaseUnreachable)
        );
        assert!(
            thread.segments[1].commits().is_empty(),
            "an unbounded gap must claim nothing, not everything older on its walk"
        );
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// A hole an `Unplaceable` segment *spans* is not a violation: its range is unknown
    /// (**I5**), so the commits it would have owned are legitimately owned by nobody, and
    /// here they are interior rather than at either end of the walk. Round 2 declares no
    /// branch (**D5** — malformed input, a supported degradation path per **M5**), so it
    /// and both gaps around it gray, leaving C and D owned by nothing between round 1's
    /// approval and round 3's anchor.
    #[test]
    fn a_hole_an_unplaceable_segment_spans_is_not_a_violation() {
        let walks = one_branch(&[A, B, C, D, E, F]);
        let comments = vec![
            approval(B),
            new_round(2, C, B),
            approval(D),
            new_round_on(3, E, D, MAIN),
        ];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert_eq!(thread.segments.len(), 5);
        for position in 1..4 {
            assert!(
                !thread.segments[position].is_placed(),
                "segment {position} should be grayed"
            );
        }
        assert_eq!(
            owned(&thread.segments[0]),
            vec![B.to_string(), A.to_string()]
        );
        assert_eq!(
            owned(&thread.segments[4]),
            vec![F.to_string(), E.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));

        // But it is only excused where the unplaceable segment sits: place the middle of
        // the thread back and the same two commits become a real hole.
        let mut placed = thread.segments.clone();
        for segment in &mut placed[1..4] {
            match segment {
                Segment::Round(round) => round.placement = Placement::Placed,
                Segment::Gap(gap) => gap.placement = Placement::Placed,
            }
        }
        assert_eq!(
            segment_invariants(&placed, Some(&walks)),
            Err(format!(
                "commit {D} is owned by no segment, between commits that are (walk {MAIN})"
            ))
        );
    }

    /// The walks live in a `HashMap`, so which one a violation is reported against would
    /// otherwise vary run to run when more than one is holed — and an invariant assertion
    /// nobody can reproduce is worse than none. Visiting them in sorted branch order pins
    /// it: `feature/x` sorts before `main`, so `feature/x` is always the one named.
    #[test]
    fn a_violation_is_reported_against_the_same_walk_every_run() {
        // Two walks with identical history, so the same hole exists on both.
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C, D, E]))),
            (FEATURE.to_string(), Ok(walk(&[A, B, C, D, E]))),
        ]);
        let comments = vec![approval(B), new_round_on(2, D, B, FEATURE)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        let mut shrunk = thread.segments.clone();
        if let Segment::Gap(gap) = &mut shrunk[1] {
            gap.commits.clear();
        }
        assert_eq!(
            segment_invariants(&shrunk, Some(&walks)),
            Err(format!(
                "commit {C} is owned by no segment, between commits that are (walk {FEATURE})"
            ))
        );
    }

    /// Unowned commits at *both* ends of a walk are legitimate and must not read as
    /// holes: the newest end is branch tip past every segment's range, and the oldest is
    /// history from before Initial QC's anchor.
    #[test]
    fn unowned_commits_at_both_ends_of_a_walk_are_accepted() {
        // The issue opens at C, so A and B predate it; round 1 closes at D, so E is drift
        // the trailing gap would own — clear it to leave the newest end unowned too.
        let walks = one_branch(&[A, B, C, D, E]);
        let thread = thread_with(&walks, C, &[approval(D)], &no_merge_base);

        assert_eq!(
            owned(&thread.segments[0]),
            vec![D.to_string(), C.to_string()]
        );
        assert_eq!(owned(&thread.segments[1]), vec![E.to_string()]);
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));

        let mut trimmed = thread.segments.clone();
        if let Segment::Gap(gap) = &mut trimmed[1] {
            gap.commits.clear();
        }
        assert_eq!(
            segment_invariants(&trimmed, Some(&walks)),
            Ok(()),
            "only D and C are owned; A, B at the oldest end and E at the newest are not"
        );
    }

    /// A closed final round that could not be placed still gets its trailing gap, which
    /// is unplaceable beside it (**W6**) — so the issue has no status rather than reading
    /// as approved off a gap that walked nothing.
    #[test]
    fn an_unplaceable_closed_final_round_trails_an_unplaceable_gap() {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(&[A, B, C]))),
            (
                FEATURE.to_string(),
                Err(UnplaceableReason::BranchUnavailable),
            ),
        ]);
        // Round 2 is on a branch we cannot walk, and closed at a commit main can identify.
        let comments = vec![approval(B), new_round_on(2, C, B, FEATURE), approval(C)];
        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert_eq!(thread.segments.len(), 4);
        let round_two = round_at(&thread, 2);
        assert_eq!(
            round_two.placement,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable)
        );
        assert_eq!(round_two.closing_commit(), Some(&oid(C)));
        assert_eq!(
            gap_at(&thread, 3).placement,
            Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable)
        );
        assert!(thread.segments[3].commits().is_empty());
        assert!(
            crate::qc_status::QCStatus::determine_status(&thread).is_none(),
            "an unplaceable trailing gap must not report as approved"
        );
        assert_eq!(segment_invariants(&thread.segments, Some(&walks)), Ok(()));
    }

    /// The anchor is authoritative, but an anchor older than the round's base would
    /// make two rounds' commits overlap. Falling back to the base leaves them sharing
    /// one legal boundary commit instead.
    #[test]
    fn an_anchor_older_than_its_base_falls_back_and_nothing_overlaps() {
        let walks = BranchWalks::from([(MAIN.to_string(), Ok(walk(&[A, B, C, D, E])))]);
        // Round 1 closes at D; the new round writes an anchor (B) older than that. The
        // branch is declared so placement, not D5, is what is under test.
        let comments = vec![approval(D), new_round_on(2, B, D, MAIN)];

        let thread = thread_with(&walks, A, &comments, &no_merge_base);

        assert!(
            thread
                .anomalies
                .contains(&RoundAnomaly::AnchorOlderThanBase {
                    comment_index: 1,
                    anchor: B.to_string(),
                    base: D.to_string(),
                })
        );
        assert_eq!(round_at(&thread, 2).opened_at, oid(D));
        assert_eq!(
            owned(&thread.segments[0]),
            vec![D.to_string(), C.to_string(), B.to_string(), A.to_string()]
        );
        assert!(thread.segments[1].commits().is_empty());
        assert_eq!(
            owned(&thread.segments[2]),
            vec![E.to_string(), D.to_string()]
        );
        assert_eq!(segment_invariants(&thread.segments, None), Ok(()));
    }

    // ── Derived accessors ────────────────────────────────────────────────────

    #[test]
    fn next_notification_from_falls_back_to_the_rounds_own_anchor() {
        let thread = thread(&[A, B], A, &[]);
        assert_eq!(thread.next_notification_from(), Some(oid(A)));
    }

    #[test]
    fn next_notification_from_uses_the_notified_commit() {
        let thread = thread(&[A, B, C], A, &[notification(B)]);
        assert_eq!(thread.next_notification_from(), Some(oid(B)));
    }

    #[test]
    fn next_notification_from_prefers_the_newer_review() {
        let thread = thread(&[A, B, C, D], A, &[notification(B), review(C)]);
        assert_eq!(thread.next_notification_from(), Some(oid(C)));
        assert_eq!(
            round_at(&thread, 0)
                .latest_actioned_commit()
                .map(|commit| commit.hash),
            Some(oid(C)),
            "the review outranks the older notification"
        );
    }

    /// An open round with nothing notified yet diffs against its own anchor: the round
    /// exists to review what has happened since it opened, and the anchor is where it
    /// opened.
    #[test]
    fn next_notification_from_an_open_round_with_no_events_is_its_anchor() {
        let comments = vec![notification(B), approval(B), new_round_on(2, D, B, MAIN)];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        assert_eq!(thread.segments.len(), 3);
        assert!(round_at(&thread, 2).is_open());
        assert_eq!(thread.next_notification_from(), Some(oid(D)));
        assert_eq!(thread.previous_approval_of(2), Some(&oid(B)));
    }

    /// With a gap active there is no round to notify, so the standing approval is the
    /// only sensible base — even when a stray notification landed after the approval.
    #[test]
    fn next_notification_from_a_trailing_gap_is_the_standing_approval() {
        let thread = thread(&[A, B, C, D], A, &[approval(B), notification(D)]);

        assert!(
            thread
                .anomalies
                .contains(&RoundAnomaly::EventAfterClose { comment_index: 1 })
        );
        assert_eq!(thread.standing_approval(), Some(&oid(B)));
        assert_eq!(thread.next_notification_from(), Some(oid(B)));
    }

    #[test]
    fn standing_approval_ignores_retracted_and_earlier_rounds() {
        let comments = vec![
            approval(B),
            new_round_on(2, C, B, MAIN),
            approval(E),
            unapproval(),
        ];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        assert_eq!(thread.rounds().count(), 2);
        // Round 2 was reopened, so it is the active segment and nothing stands.
        assert_eq!(thread.standing_approval(), None);
        // But something *was* approved, and the archive needs to be able to see it.
        assert_eq!(thread.last_approved_commit(), Some(&oid(B)));
        assert_eq!(round_at(&thread, 2).index, 2);
        assert!(round_at(&thread, 2).is_open());
    }

    #[test]
    fn reapproval_after_retraction_closes_at_the_newer_commit() {
        let comments = vec![approval(B), unapproval(), approval(D)];
        let thread = thread(&[A, B, C, D], A, &comments);

        assert_eq!(thread.rounds().count(), 1);
        assert_eq!(round_at(&thread, 0).retractions.len(), 1);
        assert_eq!(round_at(&thread, 0).retractions[0].retracted_commit, oid(B));
        assert!(!round_at(&thread, 0).is_open());
        assert_eq!(thread.standing_approval(), Some(&oid(D)));
        assert_eq!(thread.last_approved_commit(), Some(&oid(D)));
    }

    #[test]
    fn round_indices_are_unique_and_monotonic_despite_garbage_written_values() {
        let comments = vec![
            approval(B),
            // Unparsable `round:` — absent, so the derived index 2 opens round 2.
            new_round_raw("two", C, B),
            approval(C),
            // Garbage index — extends round 2 rather than opening round 3.
            new_round(u32::MAX, D, C),
            approval(D),
            // Agrees with the derived index — opens round 3.
            new_round(3, E, D),
        ];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);

        let indices: Vec<u32> = rounds.iter().map(|r| r.index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
        assert!(indices.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(rounds[1].extensions.len(), 1);
    }

    #[test]
    fn extending_a_closed_round_does_not_record_a_retraction() {
        let comments = vec![approval(B), new_round(9, D, B)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 1);
        assert!(rounds[0].retractions.is_empty());
        assert_eq!(rounds[0].extensions.len(), 1);
    }

    // ── Metadata section parsing ─────────────────────────────────────────────

    #[test]
    fn metadata_key_inside_a_file_difference_section_is_not_parsed() {
        // A diff line inside `## File Difference` happens to contain a metadata key.
        let body = format!(
            "# QC Notification\n\n## Metadata\ncurrent commit: {B}\n\n## File Difference\n```diff\n+ message <- \"current commit: {C}\"\n```\n"
        );
        let comments = vec![comment(&body)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert!(anomalies.is_empty());
        assert_eq!(rounds[0].events.len(), 1);
        assert!(matches!(
            rounds[0].events[0],
            RawRoundEvent::Notification { commit, .. } if commit == B
        ));
    }

    #[test]
    fn body_without_metadata_section_yields_no_metadata_keys() {
        let body = format!("# QC Notification\n\n@reviewer\n\ncurrent commit: {B}\n");
        assert_eq!(metadata_section(&body), "");
        assert_eq!(
            metadata_commit(metadata_section(&body), CURRENT_COMMIT_KEY),
            None
        );

        let comments = vec![comment(&body)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);
        assert!(rounds[0].events.is_empty());
    }

    #[test]
    fn bulleted_and_unbulleted_metadata_lines_both_parse() {
        let bulleted = format!(
            "# QC Notification\n\n## Metadata\n* current commit: {B}\n* previous commit: {A}\n* [commit comparison](https://example.com)\n"
        );
        let unbulleted = format!("# QC Notification\n\n## Metadata\ncurrent commit: {B}\n");
        for body in [bulleted, unbulleted] {
            assert_eq!(
                metadata_commit(metadata_section(&body), CURRENT_COMMIT_KEY),
                Some(B),
                "failed for body: {body}"
            );
        }
        // A `- ` bullet is accepted too.
        let dashed = format!("## Metadata\n- current commit: {B}\n");
        assert_eq!(
            metadata_commit(metadata_section(&dashed), CURRENT_COMMIT_KEY),
            Some(B)
        );
    }

    /// The branch a round declares is read from its metadata like every other key.
    #[test]
    fn a_round_comments_branch_is_read_from_its_metadata() {
        let comments = vec![approval(B), new_round_on(2, C, B, FEATURE)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(
            round_branches(&rounds, MAIN),
            vec![Some(MAIN.to_string()), Some(FEATURE.to_string())]
        );
    }

    // ── Marker matching ──────────────────────────────────────────────────────

    #[test]
    fn quoted_marker_does_not_trigger_and_split_header_does() {
        let quoted = format!(
            "Replying:\n\n> # QC Round 2\n\n## Metadata\nround: 2\ninitial qc round commit: {C}\n"
        );
        // Anchored at the start of a line, so a quoted heading is not a round heading.
        assert!(!quoted.lines().any(is_round_heading));
        let comments = vec![approval(B), comment(&quoted)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);
        assert_eq!(rounds.len(), 1, "a quoted marker must not open a round");
        assert!(anomalies.is_empty());

        let split = format!("# QC Notification (2/3)\n\n## Metadata\ncurrent commit: {B}\n");
        assert!(has_marker(&split, NOTIFICATION_MARKER));
        let split_comments = vec![comment(&split)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &split_comments);
        assert_eq!(rounds[0].events.len(), 1);
    }

    #[test]
    fn short_previous_approved_commit_is_not_a_base_mismatch() {
        let body = format!(
            "# QC Round\n\n## Metadata\nround: 2\ninitial qc round commit: {}\nprevious approved commit: {}\ngit branch: {MAIN}\n",
            &C[..7],
            &B[..7]
        );
        let comments = vec![approval(B), comment(&body)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 2);
        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");

        // The short SHAs still resolve to the full commits in stage 2.
        let thread = thread(&[A, B, C], A, &comments);
        assert!(thread.anomalies.is_empty());
        assert_eq!(thread.previous_approval_of(2), Some(&oid(B)));
        assert_eq!(round_at(&thread, 2).opened_at, oid(C));
    }

    // ── Comment identity (id / URL) ──────────────────────────────────────────

    #[test]
    fn comment_id_and_url_reach_every_record_that_names_a_comment() {
        const URL: &str = "https://github.com/o/r/issues/1#issuecomment-";
        let comments = vec![
            identified(
                &format!("# QC Notification\n\n## Metadata\ncurrent commit: {B}\n"),
                10,
                &format!("{URL}10"),
            ),
            identified(
                &format!("# QC Review\n\n## Metadata\ncomparing commit: {B}\n"),
                11,
                &format!("{URL}11"),
            ),
            identified(
                &format!("# QC Approval\n\n## Metadata\napproved qc commit: {B}\n"),
                12,
                &format!("{URL}12"),
            ),
            identified("# QC Un-Approval\n", 13, &format!("{URL}13")),
            identified(
                &format!("# QC Approval\n\n## Metadata\napproved qc commit: {B}\n"),
                14,
                &format!("{URL}14"),
            ),
            identified(
                &format!(
                    "# QC Round\n\n## Metadata\nround: 2\ninitial qc round commit: {C}\nprevious approved commit: {B}\ngit branch: {MAIN}\nnote: second pass\n"
                ),
                15,
                &format!("{URL}15"),
            ),
            // Round 2 is open, so this one extends it rather than opening round 3.
            identified(
                &format!("# QC Round\n\n## Metadata\nround: 3\ninitial qc round commit: {D}\n"),
                16,
                &format!("{URL}16"),
            ),
        ];
        let thread = thread(&[A, B, C, D, E], A, &comments);
        assert_eq!(thread.rounds().count(), 2);

        let first = round_at(&thread, 0);
        assert!(matches!(
            &first.events[0],
            RoundEvent::Notification { comment_id: Some(10), comment_url: Some(url), .. }
                if url == &format!("{URL}10")
        ));
        assert!(matches!(
            &first.events[1],
            RoundEvent::Review { comment_id: Some(11), comment_url: Some(url), .. }
                if url == &format!("{URL}11")
        ));
        assert_eq!(first.retractions[0].comment_id, Some(13));
        assert_eq!(
            first.retractions[0].comment_url.as_deref(),
            Some(format!("{URL}13").as_str())
        );
        assert!(matches!(
            &first.state,
            RoundState::Closed { comment_id: Some(14), comment_url: Some(url), .. }
                if url == &format!("{URL}14")
        ));

        let second = round_at(&thread, 2);
        assert!(matches!(
            &second.opened,
            RoundOpen::NewRound { comment_index: 5, comment_id: Some(15), comment_url: Some(url), .. }
                if url == &format!("{URL}15")
        ));
        // The extension re-points the checklist, so the checklist names comment 6.
        assert_eq!(
            second.checklist,
            ChecklistSource::Comment {
                comment_index: 6,
                comment_id: Some(16),
                comment_url: Some(format!("{URL}16")),
            }
        );
        assert_eq!(second.extensions[0].comment_id, Some(16));
        assert_eq!(
            second.extensions[0].comment_url.as_deref(),
            Some(format!("{URL}16").as_str())
        );
    }

    #[test]
    fn cache_loaded_comments_without_identity_fold_cleanly_with_none() {
        // Every comment here is built by `comment()`, i.e. id/url are `None`.
        let comments = vec![
            notification(B),
            approval(B),
            new_round_on(2, C, B, MAIN),
            notification(D),
            approval(D),
            unapproval(),
        ];
        let thread = thread(&[A, B, C, D], A, &comments);
        assert!(
            thread.anomalies.is_empty(),
            "unexpected anomalies: {:?}",
            thread.anomalies
        );
        assert_eq!(thread.rounds().count(), 2);

        assert!(matches!(
            &round_at(&thread, 0).state,
            RoundState::Closed {
                comment_id: None,
                comment_url: None,
                ..
            }
        ));
        assert!(matches!(
            &round_at(&thread, 0).events[0],
            RoundEvent::Notification {
                comment_id: None,
                comment_url: None,
                ..
            }
        ));
        let second = round_at(&thread, 2);
        assert!(matches!(
            &second.opened,
            RoundOpen::NewRound {
                comment_id: None,
                comment_url: None,
                ..
            }
        ));
        assert_eq!(
            second.checklist,
            ChecklistSource::Comment {
                comment_index: 2,
                comment_id: None,
                comment_url: None,
            }
        );
        assert_eq!(second.retractions[0].comment_id, None);
        assert_eq!(second.retractions[0].comment_url, None);
    }
}
