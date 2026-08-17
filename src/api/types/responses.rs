//! API response types.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use gix::ObjectId;
use octocrab::models::IssueState;
use serde::{Deserialize, Serialize};

use crate::{
    FileRenameEvent, GitHubApiError, GitProvider, IssueThread, ReviewStashResult,
    analyze_issue_checklists, api::ApiError, create::CreateResult, get_git_status,
    parse_blocking_qcs, parse_file_history,
};

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Milestone information.
#[derive(Debug, Serialize, Deserialize)]
pub struct Milestone {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub description: Option<String>,
    pub open_issues: u64,
    pub closed_issues: u64,
}

impl From<octocrab::models::Milestone> for Milestone {
    fn from(milestone: octocrab::models::Milestone) -> Self {
        Self {
            number: milestone.number as u64,
            title: milestone.title.to_string(),
            state: milestone.state.as_deref().unwrap_or("unknown").to_string(),
            description: milestone.description.clone(),
            open_issues: milestone.open_issues.unwrap_or_default() as u64,
            closed_issues: milestone.closed_issues.unwrap_or_default() as u64,
        }
    }
}

/// Kind of a relevant file entry in an issue body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RelevantFileKind {
    /// Gating QC or Previous QC — must be approved before this issue
    BlockingQc,
    /// Relevant QC — informational only
    RelevantQc,
    /// Plain file with no associated issue
    File,
}

/// A single entry from the "## Relevant Files" section of an issue body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantFileInfo {
    pub file_name: String,
    pub kind: RelevantFileKind,
    /// GitHub issue URL — present for BlockingQc and RelevantQc kinds, None for File
    pub issue_url: Option<String>,
}

/// A detected rename of a file that has an open QC issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedRename {
    pub issue_number: u64,
    pub old_path: String,
    pub new_path: String,
}

/// Issue information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub branch: Option<String>,
    pub checklist_name: Option<String>,
    pub relevant_files: Vec<RelevantFileInfo>,
    pub file_history: Vec<FileRenameEvent>,
}

impl From<octocrab::models::issues::Issue> for Issue {
    fn from(issue: octocrab::models::issues::Issue) -> Self {
        Issue {
            number: issue.number as u64,
            title: issue.title,
            state: match issue.state {
                IssueState::Closed => "closed",
                IssueState::Open => "open",
                _ => "unknown",
            }
            .to_string(),
            html_url: issue.html_url.to_string(),
            assignees: issue.assignees.iter().map(|a| a.login.clone()).collect(),
            labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
            milestone: issue.milestone.map(|m| m.title),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            closed_at: issue.closed_at,
            created_by: issue.user.login.clone(),
            branch: issue
                .body
                .as_deref()
                .and_then(parse_branch_from_body_simple),
            checklist_name: issue.body.as_deref().and_then(parse_checklist_name),
            relevant_files: issue
                .body
                .as_deref()
                .map(parse_relevant_file_infos)
                .unwrap_or_default(),
            file_history: issue
                .body
                .as_deref()
                .map(parse_file_history)
                .unwrap_or_default(),
        }
    }
}

/// QC status information.
///
/// Every sha here is nullable, and each null is a legitimate state rather than an
/// edge case — see the individual fields. `approved_commit` is gone: it silently
/// meant both `standing_approval` and `last_approved_commit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QCStatus {
    pub status: QCStatusEnum,
    pub status_detail: String,
    /// The approval that currently stands: the previous round's closing commit while
    /// the active segment is a gap. `None` while a round is open — i.e. exactly while
    /// the file is back under review.
    pub standing_approval: Option<String>,
    /// The newest closing commit across all rounds, ungated. `None` when nothing was
    /// ever approved.
    pub last_approved_commit: Option<String>,
    /// Initial QC's anchor. `None` when Initial QC could not be placed: such a
    /// segment owns no commits, so its anchor is not a commit we can point at.
    pub initial_commit: Option<String>,
    /// Newest commit of the **active segment**, not of the thread. `None` whenever
    /// that segment owns none — which includes the steady approved state, whose
    /// trailing gap is legitimately empty.
    pub latest_commit: Option<String>,
    /// **The newest file-changing commit of the trailing gap** — the sha
    /// `ChangesAfterApproval` carries. `None` in every other state, including
    /// `Approved`: a gap with no file-changing commit is approved, not changed.
    ///
    /// Distinct from [`Self::latest_commit`], and deliberately so. **S1** picks the
    /// newest commit of the gap that *touched the file*, because drift that never
    /// touched it is not something a reviewer is asked to comment on — so for a gap
    /// like `[X(file_changed: false), Y(file_changed: true)]` the status names `Y`
    /// while `latest_commit` is `X`. Without this field a client rendering "changed at"
    /// from `latest_commit` names a commit that never touched the file, which is the
    /// backend-knows/frontend-guesses split D12 exists to close.
    pub changed_commit: Option<String>,
    /// The newest commit the active round reviewed. `None` when the active segment is
    /// a gap, when the round has posted no review, or when every review it posted
    /// names a commit the round does not own — coverage is scoped to the round's own
    /// commits (S3), so an event outside them reports nothing.
    ///
    /// Round-scoped on purpose: the card labels a row *Reviewed*, and
    /// [`Self::latest_commit`] now means the branch tip, so rendering that under the
    /// label would call unreviewed drift reviewed.
    pub last_reviewed_commit: Option<String>,
    /// The newest commit the active round notified, on the same terms as
    /// [`Self::last_reviewed_commit`] — the card's *Last Posted* row, and `None` on all
    /// three of that field's null cases.
    pub last_notified_commit: Option<String>,
}

impl From<&IssueThread> for QCStatus {
    fn from(issue: &IssueThread) -> Self {
        let status = crate::QCStatus::determine_status(issue);
        // Notifications and reviews of the *active* round only. The projection lives
        // here rather than on `IssueThread` because nothing else needs it yet (A4).
        let active_round = issue.active_segment().as_round();
        let newest_event = |want_review: bool| -> Option<String> {
            let round = active_round?;
            round
                .events
                .iter()
                .filter(|event| matches!(event, crate::RoundEvent::Review { .. }) == want_review)
                .filter_map(|event| {
                    round
                        .commit_position(event.commit())
                        .map(|position| (position, event.commit()))
                })
                .min_by_key(|(position, _)| *position)
                .map(|(_, commit)| commit.to_string())
        };
        // The status already selected the trailing gap's newest *file-changing* commit
        // (S1); the enum projection throws that sha away, so capture it before the
        // conversion rather than leaving the client to re-derive it from the gap's
        // commits — it would have to reimplement the `file_changed` scan to get it right.
        let changed_commit = match status.as_ref() {
            Some(crate::QCStatus::ChangesAfterApproval(hash)) => Some(hash.to_string()),
            _ => None,
        };
        Self {
            // An unplaceable active segment yields no status at all (S4), which the
            // enum reports as its eighth value, `unknown`. The detail says the same
            // thing in words, mirroring the CLI's "Unknown".
            status_detail: status
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "Unknown".to_string()),
            status: status.into(),
            standing_approval: issue.standing_approval().map(ObjectId::to_string),
            last_approved_commit: issue.last_approved_commit().map(ObjectId::to_string),
            initial_commit: issue
                .segments
                .first()
                .and_then(crate::Segment::as_round)
                .filter(|round| round.is_placed())
                .map(|round| round.opened_at.to_string()),
            latest_commit: issue.latest_commit().map(|c| c.hash.to_string()),
            changed_commit,
            last_reviewed_commit: newest_event(true),
            last_notified_commit: newest_event(false),
        }
    }
}

/// QC status enum values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QCStatusEnum {
    Approved,
    ChangesAfterApproval,
    AwaitingReview,
    ChangeRequested,
    InProgress,
    ApprovalRequired,
    ChangesToComment,
    /// **The absence of a status, not an activity.** Emitted for — and only for — the
    /// `None` the domain returns when the active segment could not be placed (S4).
    ///
    /// It is the eighth value on the wire and has no domain counterpart: the domain's
    /// seven are the statuses a *placed* segment can report. It co-occurs with the
    /// card's graying through D15's clause 2 (active segment `Unplaceable`), which is
    /// the only clause that accompanies the absence of a status.
    Unknown,
}

impl From<Option<crate::QCStatus>> for QCStatusEnum {
    /// `None` — an unplaceable active segment (S4) — maps to [`QCStatusEnum::Unknown`].
    ///
    /// It deliberately does **not** map to `in_progress`, which it used to: that is a
    /// real status a *placed* round reports when it owns commits, none of which touched
    /// the QC'd file, and it announced nothing (S2's last row,
    /// `crate::QCStatus::InProgress`). Collapsing the two would tell a client "nothing
    /// can be asserted" about a round that is perfectly placeable and fully understood,
    /// and would tie graying to a status that D15's second addendum reserves for the
    /// absence of one.
    fn from(value: Option<crate::QCStatus>) -> Self {
        match value {
            None => QCStatusEnum::Unknown,
            Some(status) => status.into(),
        }
    }
}

impl From<crate::QCStatus> for QCStatusEnum {
    fn from(value: crate::QCStatus) -> Self {
        match value {
            crate::QCStatus::Approved => QCStatusEnum::Approved,
            crate::QCStatus::ChangesAfterApproval(_) => QCStatusEnum::ChangesAfterApproval,
            crate::QCStatus::AwaitingReview => QCStatusEnum::AwaitingReview,
            crate::QCStatus::ChangeRequested => QCStatusEnum::ChangeRequested,
            crate::QCStatus::InProgress => QCStatusEnum::InProgress,
            crate::QCStatus::ApprovalRequired => QCStatusEnum::ApprovalRequired,
            crate::QCStatus::ChangesToComment(_) => QCStatusEnum::ChangesToComment,
        }
    }
}

/// Git status information.
#[derive(Debug, Clone, Serialize)]
pub struct GitStatus {
    pub status: GitStatusEnum,
    pub detail: String,
    pub ahead_commits: Vec<String>,
    pub behind_commits: Vec<String>,
}

impl From<crate::GitState> for GitStatus {
    fn from(status: crate::GitState) -> Self {
        let mut res = GitStatus {
            status: GitStatusEnum::Clean,
            detail: status.to_string(),
            ahead_commits: Vec::new(),
            behind_commits: Vec::new(),
        };

        let convert_commits = |commits: Vec<ObjectId>| -> Vec<String> {
            commits.iter().map(ObjectId::to_string).collect()
        };

        match status {
            crate::GitState::Clean => (),
            crate::GitState::Ahead(ahead_commits) => {
                res.status = GitStatusEnum::Ahead;
                res.ahead_commits = convert_commits(ahead_commits);
            }
            crate::GitState::Behind(behind_commits) => {
                res.status = GitStatusEnum::Behind;
                res.behind_commits = convert_commits(behind_commits);
            }
            crate::GitState::Diverged { ahead, behind } => {
                res.status = GitStatusEnum::Diverged;
                res.ahead_commits = convert_commits(ahead);
                res.behind_commits = convert_commits(behind);
            }
        }

        res
    }
}

/// Git status enum values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitStatusEnum {
    Clean,
    Ahead,
    Behind,
    Diverged,
}

/// A commit owned by one segment.
///
/// The wire shape is unchanged, but `statuses` is now a projection of the owning
/// segment's events and state rather than a stored parse of the comment thread (A4):
/// the comments are folded once, into rounds, and this reads that fold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueCommit {
    pub hash: String,
    pub message: String,
    pub statuses: Vec<CommitStatusEnum>,
    pub file_changed: bool,
}

impl IssueCommit {
    /// Project one commit as its **owning segment** sees it.
    ///
    /// `initial_anchor` is Initial QC's anchor — the only source of
    /// [`CommitStatusEnum::Initial`], and the only status that is not a fact about the
    /// owning segment.
    ///
    /// A gap has no events and no state, so a gap's commits can only ever carry
    /// `initial`. An event naming a commit its own round does not own therefore loses
    /// its status: coverage is scoped to the round's own commits (S3).
    ///
    /// **The two copies of a shared boundary commit disagree, deliberately.** A round's
    /// `opened_at` may equal the previous round's closing commit (D1), so the same hash
    /// appears in both segments' `commits`. The older round's copy carries `approved`
    /// and the newer round's copy carries nothing, because in the newer round's frame
    /// that commit genuinely is not its approval (D14). This is not a bug to fix by
    /// unioning the two.
    ///
    /// **`approved` is now per round, and that is an undocumented behaviour change.**
    /// The parse this replaced tracked a single `approved_commit` and stripped
    /// `Approved` from every other commit, so exactly one `approved` dot existed
    /// thread-wide. This projection dots *every* closed round's closing commit, so a
    /// thread like `[R1(closed@b), Gap(c), R2(closed@d), Gap()]` emits `["approved"]`
    /// on both `b` and `d`. Arguably a fix — each round's approval belongs in its own
    /// frame, which is the same reasoning as D14 — but neither the spec, the wire
    /// contract nor the old code says so, so it is recorded here rather than assumed.
    fn project(
        commit: &crate::IssueCommit,
        segment: &crate::Segment,
        initial_anchor: Option<&ObjectId>,
    ) -> Self {
        // Fixed order, deduplicated: the old field came from a `HashSet` and so had
        // non-deterministic order, which every consumer had to re-sort.
        let mut statuses = Vec::new();
        if initial_anchor == Some(&commit.hash) {
            statuses.push(CommitStatusEnum::Initial);
        }
        if let Some(round) = segment.as_round() {
            let names = |want_review: bool| {
                round.events.iter().any(|event| {
                    *event.commit() == commit.hash
                        && matches!(event, crate::RoundEvent::Review { .. }) == want_review
                })
            };
            if names(false) {
                statuses.push(CommitStatusEnum::Notification);
            }
            if round.closing_commit() == Some(&commit.hash) {
                statuses.push(CommitStatusEnum::Approved);
            }
            if names(true) {
                statuses.push(CommitStatusEnum::Reviewed);
            }
        }
        Self {
            hash: commit.hash.to_string(),
            message: commit.message.to_string(),
            statuses,
            file_changed: commit.file_changed,
        }
    }
}

/// Commit status enum values. Serialized in the fixed order they are declared in —
/// see [`IssueCommit::project`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatusEnum {
    Initial,
    Notification,
    Approved,
    Reviewed,
}

/// Checklist completion summary.
#[derive(Debug, Clone, Serialize)]
pub struct ChecklistSummary {
    pub completed: u32,
    pub total: u32,
    pub percentage: f32,
}

impl From<Vec<(String, crate::ChecklistSummary)>> for ChecklistSummary {
    fn from(checklists: Vec<(String, crate::ChecklistSummary)>) -> Self {
        let sum = crate::ChecklistSummary::sum(checklists.iter().map(|(_, c)| c));
        Self {
            completed: sum.completed as u32,
            total: sum.total as u32,
            percentage: if sum.total == 0 {
                0.0
            } else {
                sum.completed as f32 / sum.total as f32
            },
        }
    }
}

/// Whether a QC round is still being reviewed, or was closed by an approval.
///
/// Mirrors [`crate::RoundState`]'s discriminant; the closing details live in
/// [`RoundInfo`]'s flat `closing_*` fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoundStateEnum {
    Open,
    Closed,
}

/// Where a round's checklist lives. Mirrors [`crate::ChecklistSource`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistSourceKind {
    /// Initial QC: the checklist is in the issue body.
    IssueBody,
    /// Round N > 1: the checklist is in the round comment.
    Comment,
}

/// The checklist source of a round, with the comment it lives in when known.
#[derive(Debug, Clone, Serialize)]
pub struct RoundChecklistSource {
    pub kind: ChecklistSourceKind,
    /// GitHub's comment id; `None` for `issue_body` and for cache-loaded comments.
    pub comment_id: Option<u64>,
    /// Permalink to the comment; `None` for the same reasons as `comment_id`.
    pub comment_url: Option<String>,
}

impl From<&crate::ChecklistSource> for RoundChecklistSource {
    fn from(source: &crate::ChecklistSource) -> Self {
        match source {
            crate::ChecklistSource::IssueBody => Self {
                kind: ChecklistSourceKind::IssueBody,
                comment_id: None,
                comment_url: None,
            },
            crate::ChecklistSource::Comment {
                comment_id,
                comment_url,
                ..
            } => Self {
                kind: ChecklistSourceKind::Comment,
                comment_id: *comment_id,
                comment_url: comment_url.clone(),
            },
        }
    }
}

/// Whether a round was opened by creating the issue, or by a round comment.
/// Mirrors [`crate::RoundOpen`]'s discriminant.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoundOpenKind {
    IssueCreated,
    NewRound,
}

/// What opened a round.
///
/// Projected as a struct with a `kind` field rather than a tagged enum so all six keys
/// are always present and a client can read `comment_url` without narrowing. The
/// fold-internal `comment_index` is deliberately not exposed.
#[derive(Debug, Clone, Serialize)]
pub struct RoundOpenInfo {
    pub kind: RoundOpenKind,
    /// GitHub's comment id; `None` for `issue_created` and for cache-loaded comments.
    pub comment_id: Option<u64>,
    /// Permalink to the opening comment; `None` for the same reasons as `comment_id`.
    pub comment_url: Option<String>,
    /// `None` for `issue_created`.
    pub author: Option<String>,
    /// `None` for `issue_created`.
    pub at: Option<DateTime<Utc>>,
    /// The `note:` the round comment recorded, when it recorded one.
    pub note: Option<String>,
}

impl From<&crate::RoundOpen> for RoundOpenInfo {
    fn from(opened: &crate::RoundOpen) -> Self {
        match opened {
            crate::RoundOpen::IssueCreated => Self {
                kind: RoundOpenKind::IssueCreated,
                comment_id: None,
                comment_url: None,
                author: None,
                at: None,
                note: None,
            },
            crate::RoundOpen::NewRound {
                comment_id,
                comment_url,
                author,
                at,
                note,
                ..
            } => Self {
                kind: RoundOpenKind::NewRound,
                comment_id: *comment_id,
                comment_url: comment_url.clone(),
                author: Some(author.clone()),
                at: Some(*at),
                note: note.clone(),
            },
        }
    }
}

/// Whether a round event was a notification or a review.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoundEventKind {
    Notification,
    Review,
}

/// A notification or review posted inside a round. Mirrors [`crate::RoundEvent`],
/// flattened the same way [`RoundOpenInfo`] is.
#[derive(Debug, Clone, Serialize)]
pub struct RoundEventInfo {
    pub kind: RoundEventKind,
    /// The commit this event names.
    pub commit: String,
    pub by: String,
    pub at: DateTime<Utc>,
    /// `None` for cache-loaded comments, which carry no identity.
    pub comment_id: Option<u64>,
    /// `None` for the same reason as `comment_id`.
    pub comment_url: Option<String>,
}

impl From<&crate::RoundEvent> for RoundEventInfo {
    fn from(event: &crate::RoundEvent) -> Self {
        let (kind, commit, by, at, comment_id, comment_url) = match event {
            crate::RoundEvent::Notification {
                commit,
                by,
                at,
                comment_id,
                comment_url,
                ..
            } => (
                RoundEventKind::Notification,
                commit,
                by,
                at,
                comment_id,
                comment_url,
            ),
            crate::RoundEvent::Review {
                commit,
                by,
                at,
                comment_id,
                comment_url,
                ..
            } => (
                RoundEventKind::Review,
                commit,
                by,
                at,
                comment_id,
                comment_url,
            ),
        };
        Self {
            kind,
            commit: commit.to_string(),
            by: by.clone(),
            at: *at,
            comment_id: *comment_id,
            comment_url: comment_url.clone(),
        }
    }
}

/// A `# QC Un-Approval` that took back a round's approval. Mirrors
/// [`crate::Retraction`].
#[derive(Debug, Clone, Serialize)]
pub struct RetractionInfo {
    pub retracted_commit: String,
    pub by: String,
    pub at: DateTime<Utc>,
    /// `None` for cache-loaded comments.
    pub comment_id: Option<u64>,
    /// `None` for the same reason as `comment_id`.
    pub comment_url: Option<String>,
}

impl From<&crate::Retraction> for RetractionInfo {
    fn from(retraction: &crate::Retraction) -> Self {
        Self {
            retracted_commit: retraction.retracted_commit.to_string(),
            by: retraction.by.clone(),
            at: retraction.at,
            comment_id: retraction.comment_id,
            comment_url: retraction.comment_url.clone(),
        }
    }
}

/// A round comment that extended an open round instead of opening a new one.
/// Mirrors [`crate::Extension`].
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionInfo {
    pub by: String,
    pub at: DateTime<Utc>,
    /// The written `round commit:`, when present and resolvable. The round's own
    /// anchor is deliberately left alone by an extension.
    pub at_commit: Option<String>,
    pub note: Option<String>,
    /// `None` for cache-loaded comments.
    pub comment_id: Option<u64>,
    /// `None` for the same reason as `comment_id`.
    pub comment_url: Option<String>,
}

impl From<&crate::Extension> for ExtensionInfo {
    fn from(extension: &crate::Extension) -> Self {
        Self {
            by: extension.by.clone(),
            at: extension.at,
            at_commit: extension.at_commit.map(|c| c.to_string()),
            note: extension.note.clone(),
            comment_id: extension.comment_id,
            comment_url: extension.comment_url.clone(),
        }
    }
}

/// Why a segment's commits could not be located. Mirrors
/// [`crate::UnplaceableReason`].
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnplaceableReasonEnum {
    BranchNotDeclared,
    BranchUnavailable,
    AnchorUnreachable,
    MergeBaseUnreachable,
    NeighbourUnplaceable,
}

impl From<&crate::UnplaceableReason> for UnplaceableReasonEnum {
    fn from(reason: &crate::UnplaceableReason) -> Self {
        match reason {
            crate::UnplaceableReason::BranchNotDeclared => Self::BranchNotDeclared,
            crate::UnplaceableReason::BranchUnavailable => Self::BranchUnavailable,
            crate::UnplaceableReason::AnchorUnreachable => Self::AnchorUnreachable,
            crate::UnplaceableReason::MergeBaseUnreachable => Self::MergeBaseUnreachable,
            crate::UnplaceableReason::NeighbourUnplaceable => Self::NeighbourUnplaceable,
        }
    }
}

/// Whether a segment's commits could be located. Mirrors [`crate::Placement`], but as
/// a **struct** variant rather than M5's newtype.
///
/// That restructuring is load-bearing, not cosmetic: internally tagging a newtype
/// variant around a unit-only enum compiles *and* serializes, silently emitting
/// `{"kind":"unplaceable","branch_unavailable":null}` — the reason becomes a key. The
/// domain type must therefore never derive `Serialize`; it reaches the wire only here.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlacementInfo {
    Placed,
    Unplaceable { reason: UnplaceableReasonEnum },
}

impl From<&crate::Placement> for PlacementInfo {
    fn from(placement: &crate::Placement) -> Self {
        match placement {
            crate::Placement::Placed => Self::Placed,
            crate::Placement::Unplaceable(reason) => Self::Unplaceable {
                reason: reason.into(),
            },
        }
    }
}

/// How a gap's two bounding commits relate. Mirrors [`crate::GapContinuity`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GapContinuityInfo {
    Linear,
    Diverged { merge_base: String },
    Unrelated,
}

impl From<&crate::GapContinuity> for GapContinuityInfo {
    fn from(continuity: &crate::GapContinuity) -> Self {
        match continuity {
            crate::GapContinuity::Linear => Self::Linear,
            crate::GapContinuity::Diverged { merge_base } => Self::Diverged {
                merge_base: merge_base.to_string(),
            },
            crate::GapContinuity::Unrelated => Self::Unrelated,
        }
    }
}

impl GapContinuityInfo {
    /// The same fact as a round-start response's `divergence`, where `null` already
    /// means "no divergence". [`crate::GapContinuity::Linear`] therefore maps to
    /// `None`: emitting both encodings of one fact would let them disagree, so
    /// `{"kind":"linear"}` is unrepresentable on those two fields.
    pub fn divergence(continuity: &crate::GapContinuity) -> Option<Self> {
        match continuity {
            crate::GapContinuity::Linear => None,
            other => Some(other.into()),
        }
    }
}

/// One QC round and the commits it owns. Mirrors [`crate::Round`].
///
/// `previous_approval` is gone: it is the closing commit of the round two positions
/// back, which a consumer reads positionally (or, for the round it is rendering, from
/// `qc_status.standing_approval`). The three `*_count` fields are gone too — the lists
/// themselves are projected, and a count beside its list is data that can disagree.
#[derive(Debug, Clone, Serialize)]
pub struct RoundSegment {
    /// 1-based, derived round number. `1` is Initial QC.
    pub index: u32,
    /// Human-readable name: `"Initial QC"` or `"Round N"`.
    pub name: String,
    /// The commit the round opened at — its anchor, owned by this round and not by the
    /// gap before it.
    ///
    /// `None` when the round could not be placed. The fold leaves an unresolved anchor as
    /// the **all-zero OID**, which is not a commit anyone can address; emitting it would
    /// put a fake sha on an audit tool's wire, where every other unresolvable sha on this
    /// response is already null. Nulled at the source rather than guarded downstream.
    pub opened_at: Option<String>,
    /// The branch this round was reviewed on. Always present: a round comment without
    /// one is malformed input, reported as `placement.reason == branch_not_declared`
    /// rather than as a null branch.
    pub branch: String,
    pub opened: RoundOpenInfo,
    /// Projects the model's `checklist`; the wire name is unchanged.
    pub checklist_source: RoundChecklistSource,
    pub checklist_name: Option<String>,
    pub state: RoundStateEnum,
    /// Set only when `state == closed`: the approved commit.
    pub closing_commit: Option<String>,
    /// Set only when `state == closed`: who approved.
    pub closed_by: Option<String>,
    /// Set only when `state == closed`: when the approval landed.
    pub closed_at: Option<DateTime<Utc>>,
    /// Notifications and reviews inside this round, oldest first.
    pub events: Vec<RoundEventInfo>,
    /// `# QC Un-Approval` comments that took back this round's approval, oldest first.
    pub retractions: Vec<RetractionInfo>,
    /// Round comments that extended this round instead of opening one, oldest first.
    pub extensions: Vec<ExtensionInfo>,
    /// The commits this round owns, newest first. Empty when unplaceable.
    pub commits: Vec<IssueCommit>,
    pub placement: PlacementInfo,
}

/// The drift between two rounds, or after the last one. Mirrors [`crate::Gap`].
///
/// Gaps carry no index, name or id: they are addressed positionally.
#[derive(Debug, Clone, Serialize)]
pub struct GapSegment {
    /// The branch this gap was walked on: the bounding newer round's, or — for a
    /// trailing gap — the bounding older round's. Never the viewer's checkout.
    pub branch: String,
    /// The commits this gap owns, newest first. Legitimately empty.
    pub commits: Vec<IssueCommit>,
    pub continuity: GapContinuityInfo,
    /// The older bounding commit — the previous round's closing commit, exclusive.
    /// `None` when it does not resolve.
    pub lower_bound: Option<String>,
    /// The newer bounding commit — the next round's anchor, exclusive, or the branch
    /// tip for a trailing gap. `None` when it does not resolve.
    pub upper_bound: Option<String>,
    pub placement: PlacementInfo,
}

/// One segment of a thread: a QC round, or the drift between two of them.
///
/// Internally tagged on `kind`, which is what makes `segments.at(-1).kind == "round"`
/// the test for "a round is open" and what a TypeScript discriminated union consumes
/// unwrapped. Both variants wrap structs — an internally-tagged newtype variant
/// around anything that does not serialize as a map fails at runtime.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum SegmentInfo {
    Round(RoundSegment),
    Gap(GapSegment),
}

impl SegmentInfo {
    /// Project the segment at `position`.
    ///
    /// The whole list is passed because a gap's bounds are positional facts about its
    /// neighbours, and `initial` statuses are a fact about `segments[0]`.
    fn project(segments: &[crate::Segment], position: usize) -> Self {
        let initial_anchor = segments
            .first()
            .and_then(crate::Segment::as_round)
            .map(|round| &round.opened_at);
        let segment = &segments[position];
        let commits = segment
            .commits()
            .iter()
            .map(|commit| IssueCommit::project(commit, segment, initial_anchor))
            .collect();

        match segment {
            crate::Segment::Round(round) => {
                let (state, closing_commit, closed_by, closed_at) = match &round.state {
                    crate::RoundState::Open => (RoundStateEnum::Open, None, None, None),
                    crate::RoundState::Closed { commit, by, at, .. } => (
                        RoundStateEnum::Closed,
                        Some(commit.to_string()),
                        Some(by.clone()),
                        Some(*at),
                    ),
                };
                Self::Round(RoundSegment {
                    index: round.index,
                    name: round.name(),
                    opened_at: round.is_placed().then(|| round.opened_at.to_string()),
                    branch: round.branch.clone(),
                    opened: (&round.opened).into(),
                    checklist_source: (&round.checklist).into(),
                    checklist_name: round.checklist_name.clone(),
                    state,
                    closing_commit,
                    closed_by,
                    closed_at,
                    events: round.events.iter().map(RoundEventInfo::from).collect(),
                    retractions: round.retractions.iter().map(RetractionInfo::from).collect(),
                    extensions: round.extensions.iter().map(ExtensionInfo::from).collect(),
                    commits,
                    placement: (&round.placement).into(),
                })
            }
            crate::Segment::Gap(gap) => {
                // Positional, and the offsets differ from a round's: the rounds
                // bounding a gap at `position` are its immediate neighbours, at
                // `position - 1` (older) and `position + 1` (newer). `position - 2`
                // is another gap.
                let bounding = |at: Option<usize>| {
                    at.and_then(|at| segments.get(at))
                        .and_then(crate::Segment::as_round)
                        // An unplaceable round's commits were never resolved, so its
                        // anchor is not a commit we can hand a consumer as a bound.
                        .filter(|round| round.is_placed())
                };
                let older = bounding(position.checked_sub(1));
                // "Trailing" is a structural fact about the position, not the absence
                // of a *placeable* newer round: an interior gap whose newer round is
                // unplaceable must lose that bound (W6), not silently fall through to
                // the trailing rule and claim its own newest commit.
                let trailing = position + 1 >= segments.len();
                let newer = bounding(Some(position + 1));

                // The bounding *approval*, not the merge-base the walk stopped at.
                // For a diverged gap the two differ — which is exactly the case this
                // field exists to render — and the merge-base is already on the wire
                // as `continuity.merge_base`.
                let lower_bound = older.and_then(crate::Round::closing_commit).copied();
                let upper_bound = if trailing {
                    // A trailing gap's newer bound is the branch tip, inclusive: its
                    // own newest commit. With nothing since the approval the tip *is*
                    // that approval, so both bounds are the same commit — expected,
                    // not a bug. Only claimable when the walk actually reached back to
                    // it, i.e. a placed, linear gap. The non-linear fallback is
                    // unreachable from the fold today (D15's addendum: a trailing gap
                    // walks the previous round's branch, on which a placed round's
                    // closing commit necessarily sits, so continuity is always
                    // `Linear`); it guards a future model change.
                    gap.commits.first().map(|commit| commit.hash).or_else(|| {
                        (gap.is_placed() && gap.continuity == crate::GapContinuity::Linear)
                            .then_some(lower_bound)
                            .flatten()
                    })
                } else {
                    // An interior gap's newer bound is the next round's anchor — and
                    // nothing else. `bounding` has already nulled it if that round
                    // could not be placed (W6: no substitution).
                    newer.map(|round| round.opened_at)
                };

                // A gap that could not be placed *itself* has no range, so neither bound
                // is a handle anyone may draw on — even when both neighbours are placed
                // and the bounds therefore resolve. Reachable through `build_gap`'s
                // `BranchUnavailable` / `AnchorUnreachable` / `MergeBaseUnreachable`
                // paths, where the *gap* is grayed while the rounds around it are fine.
                // Handing U1 a real handle to draw across a segment it is graying is
                // W6's plausible-but-wrong, which is worse than nothing.
                let (lower_bound, upper_bound) = if gap.is_placed() {
                    (lower_bound, upper_bound)
                } else {
                    (None, None)
                };

                Self::Gap(GapSegment {
                    branch: gap.branch.clone(),
                    commits,
                    continuity: (&gap.continuity).into(),
                    lower_bound: lower_bound.map(|c| c.to_string()),
                    upper_bound: upper_bound.map(|c| c.to_string()),
                    placement: (&gap.placement).into(),
                })
            }
        }
    }
}

/// Blocking QC item (approved).
#[derive(Debug, Clone, Serialize)]
pub struct BlockingQCItem {
    pub issue_number: u64,
    pub file_name: String,
}

/// Blocking QC item with status (not approved).
#[derive(Debug, Clone, Serialize)]
pub struct BlockingQCItemWithStatus {
    pub issue_number: u64,
    pub file_name: String,
    pub status: String,
}

/// Blocking QC error.
#[derive(Debug, Clone, Serialize)]
pub struct BlockingQCError {
    pub issue_number: u64,
    pub error: String,
    pub kind: IssueStatusErrorKind,
    /// Title of the QC'd file, when known. Absent for fetch-failed errors
    /// since we never retrieved the issue body. Present for processing
    /// failures (incl. `BranchNotLocal`) since the issue itself was fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// Set when `kind == BranchNotLocal` — the branch ref that needs to be
    /// fetched / checked out locally so the UI can offer a copy-pasteable fix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl From<(u64, GitHubApiError)> for BlockingQCError {
    fn from(value: (u64, GitHubApiError)) -> Self {
        Self {
            issue_number: value.0,
            error: value.1.to_string(),
            kind: IssueStatusErrorKind::FetchFailed,
            file_name: None,
            branch: None,
        }
    }
}

/// Blocking QC status summary.
#[derive(Debug, Clone, Serialize, Default)]
pub struct BlockingQCStatus {
    pub total: u32,
    pub approved_count: u32,
    pub summary: String,
    pub approved: Vec<BlockingQCItem>,
    pub not_approved: Vec<BlockingQCItemWithStatus>,
    pub errors: Vec<BlockingQCError>,
}

/// Full issue status response.
#[derive(Debug, Clone, Serialize)]
pub struct IssueStatusResponse {
    pub issue: Issue,
    pub qc_status: QCStatus,
    pub dirty: bool,
    /// The branch the status was computed on: the active segment's. Never the
    /// viewer's checkout, so two people on different branches see the same status.
    /// A client compares this with `/api/repo`'s branch at render time — the compare
    /// is a join of stable data with a polled fact, not derivation.
    pub active_branch: String,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_status: BlockingQCStatus,
    /// The thread as a strict alternation of rounds and the drift between them,
    /// oldest first. Replaces **both** the round list and the thread-wide commit
    /// list: every commit is owned by the segment that covers it.
    ///
    /// `segments[0]` is Initial QC and the last segment is an open round or a gap, so
    /// `segments.at(-1).kind == "round"` replaces the removed `open_round_index`.
    pub segments: Vec<SegmentInfo>,
    /// The commit a new notification would diff against — the UI's default
    /// comparison base for its commit picker. `None` when the active segment can
    /// supply none: an unplaceable segment owns no commits, so there is neither a
    /// newest event commit nor a standing approval to fall back to.
    pub next_notification_from: Option<String>,
    /// Which of the open round's follow-up steps are incomplete, so a client can
    /// offer a repair without asking. `null` when no round is open, or when the
    /// open round is Initial QC (which no start-round action opened).
    ///
    /// This lives here rather than on the round seed because the status surface
    /// renders many issues at once and must decide per card without a second
    /// request, and because it is a pure function of the rounds already folded for
    /// this response: no extra API call, no writes.
    pub round_repair: Option<RoundRepairStatus>,
}

impl IssueStatusResponse {
    pub fn new(
        issue: &octocrab::models::issues::Issue,
        issue_thread: &IssueThread,
        dirty_files: &[PathBuf],
    ) -> Self {
        Self {
            dirty: dirty_files.contains(&PathBuf::from(&issue.title)),
            issue: issue.clone().into(),
            qc_status: issue_thread.into(),
            active_branch: issue_thread.active_branch().to_string(),
            checklist_summary: analyze_issue_checklists(issue.body.as_deref()).into(),
            blocking_qc_status: BlockingQCStatus::default(),
            segments: (0..issue_thread.segments.len())
                .map(|position| SegmentInfo::project(&issue_thread.segments, position))
                .collect(),
            next_notification_from: issue_thread
                .next_notification_from()
                .map(|commit| commit.to_string()),
            // By I3 a closed round is never last, so the only round that can be open
            // is the active segment.
            round_repair: issue_thread
                .active_segment()
                .as_round()
                .and_then(|round| RoundRepairStatus::derive(issue, round)),
        }
    }

    pub fn blocking_qc_numbers(issue: &octocrab::models::issues::Issue) -> Vec<u64> {
        issue
            .body
            .as_deref()
            .map(|body| {
                parse_blocking_qcs(body)
                    .iter()
                    .map(|b| b.issue_number)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Blocked issue with status.
#[derive(Debug, Serialize, Deserialize)]
pub struct BlockedIssueStatus {
    pub issue: Issue,
    pub qc_status: QCStatus,
}

/// Response for issue creation.
#[derive(Debug, Serialize)]
pub struct CreateIssueResponse {
    pub issue_url: String,
    pub blocking_created: Vec<u64>,
    pub blocking_errors: Vec<BlockingQCError>,
}

impl From<CreateResult> for CreateIssueResponse {
    fn from(res: CreateResult) -> Self {
        Self {
            issue_url: res.issue_url,
            blocking_created: res.successful_blocking,
            blocking_errors: res
                .blocking_errors
                .into_iter()
                .map(BlockingQCError::from)
                .collect(),
        }
    }
}

/// Error kind for batch issue status.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatusErrorKind {
    FetchFailed,
    ProcessingFailed,
    /// The issue's branch ref isn't reachable locally — the user likely needs
    /// to `git checkout` it once to populate the local ref.
    BranchNotLocal,
}

/// Error entry for batch issue status.
#[derive(Debug, Serialize)]
pub struct IssueStatusError {
    pub issue_number: u64,
    pub kind: IssueStatusErrorKind,
    pub error: String,
    /// Set when `kind == BranchNotLocal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Envelope response for batch issue status.
#[derive(Debug, Serialize)]
pub struct BatchIssueStatusResponse {
    pub results: Vec<IssueStatusResponse>,
    pub errors: Vec<IssueStatusError>,
}

/// Response for comment creation.
#[derive(Debug, Serialize)]
pub struct CommentResponse {
    pub comment_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResponse {
    pub comment_url: String,
    pub stash: ReviewStashResult,
}

/// Response for issue approval.
#[derive(Debug, Serialize)]
pub struct ApprovalResponse {
    pub approval_url: String,
    pub skipped_unapproved: Vec<u64>,
    pub skipped_errors: Vec<BlockingQCError>,
    pub closed: bool,
}

/// Response for issue unapproval.
#[derive(Debug, Serialize)]
pub struct UnapprovalResponse {
    pub unapproval_url: String,
    pub opened: bool,
}

/// How one of a round start's recoverable steps turned out. Mirrors
/// [`crate::StepOutcome`], whose failure message becomes `error`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatusEnum {
    Done,
    /// Not attempted because it was not requested.
    Skipped,
    Failed,
}

/// Outcome of one recoverable step of a round start.
#[derive(Debug, Clone, Serialize)]
pub struct StepOutcomeResponse {
    pub status: StepStatusEnum,
    /// Present only when `status == failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why the step was skipped, when the reason is not simply that there was nothing
    /// to do. Present only when `status == skipped` *and* such a reason exists, exactly
    /// as `error` is present only on `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

impl From<&crate::StepOutcome> for StepOutcomeResponse {
    fn from(outcome: &crate::StepOutcome) -> Self {
        match outcome {
            crate::StepOutcome::Done => Self {
                status: StepStatusEnum::Done,
                error: None,
                skipped_reason: None,
            },
            crate::StepOutcome::Skipped => Self {
                status: StepStatusEnum::Skipped,
                error: None,
                skipped_reason: None,
            },
            crate::StepOutcome::Failed(error) => Self {
                status: StepStatusEnum::Failed,
                error: Some(error.clone()),
                skipped_reason: None,
            },
        }
    }
}

impl StepOutcomeResponse {
    /// Attach why this step was skipped.
    ///
    /// A reason on a step that ran is dropped rather than reported: `done` was not
    /// skipped, and `failed` already says why in `error`. The domain type carries no
    /// reason of its own (`crate::StepOutcome::Skipped` is a unit variant shared with
    /// the start path), so the caller supplies it — see
    /// [`crate::RepairRoundResult::notification_skip_reason`].
    pub fn skipped_because(mut self, reason: Option<&str>) -> Self {
        if self.status == StepStatusEnum::Skipped {
            self.skipped_reason = reason.map(str::to_string);
        }
        self
    }
}

/// A downstream issue a new round may have invalidated. Display only — the round
/// action never writes to a downstream issue.
#[derive(Debug, Clone, Serialize)]
pub struct ImpactedIssueItem {
    pub issue_number: u64,
    pub file_name: String,
    pub milestone: String,
    /// Human-readable relationship, e.g. `"previous QC"`.
    pub relationship: String,
}

/// Direct downstream issues, one layer deep. Mirrors [`crate::ImpactedIssues`].
#[derive(Debug, Clone, Serialize)]
pub struct ImpactedIssuesResponse {
    /// `false` when the dependency API is unavailable on this GitHub instance, in
    /// which case `issues` is empty because nothing could be checked.
    pub api_available: bool,
    pub issues: Vec<ImpactedIssueItem>,
}

impl From<&crate::ImpactedIssues> for ImpactedIssuesResponse {
    fn from(impacted: &crate::ImpactedIssues) -> Self {
        match impacted {
            crate::ImpactedIssues::None => Self {
                api_available: true,
                issues: Vec::new(),
            },
            crate::ImpactedIssues::ApiUnavailable => Self {
                api_available: false,
                issues: Vec::new(),
            },
            crate::ImpactedIssues::Some(nodes) => Self {
                api_available: true,
                issues: nodes
                    .iter()
                    .map(|node| ImpactedIssueItem {
                        issue_number: node.issue_number,
                        file_name: node.file_name.display().to_string(),
                        milestone: node.milestone.clone(),
                        relationship: node.relationship.to_string(),
                    })
                    .collect(),
            },
        }
    }
}

/// Response for starting a new QC round.
///
/// The round exists as soon as `round_comment_url` is set; the three step fields
/// report what else landed. A `failed` step is **not** an error response: each
/// step is idempotent and independently retryable, which is what `needs_repair`
/// flags.
#[derive(Debug, Clone, Serialize)]
pub struct StartRoundResponse {
    /// Derived index of the round that was opened (always >= 2).
    pub round: u32,
    /// Human-readable name of the new round, e.g. `"Round 2"`.
    pub round_name: String,
    /// URL of the round comment: the round's identity.
    pub round_comment_url: String,
    /// The commit the round opened at (HEAD at open time).
    pub anchor: String,
    /// The branch the round was opened on, recorded in its round comment.
    pub branch: String,
    /// What the notification diff compared against: the previous approval, unless it
    /// was unreachable from `branch`.
    pub comparison_base: String,
    /// How the previous approval relates to `anchor`, when it was not an ancestor of
    /// it. `None` for the linear case, which is what `null` means on this field.
    pub divergence: Option<GapContinuityInfo>,
    /// Step 2: reopening the issue.
    pub reopened: StepOutcomeResponse,
    /// Step 3: refreshing the `## QC Round` block in the issue body.
    pub body_marker: StepOutcomeResponse,
    /// Step 4: the `# QC Notification` comment.
    pub notification: StepOutcomeResponse,
    /// Whether any of the steps above failed and wants a retry.
    pub needs_repair: bool,
    pub impacted_issues: ImpactedIssuesResponse,
}

impl From<&crate::StartRoundResult> for StartRoundResponse {
    fn from(result: &crate::StartRoundResult) -> Self {
        Self {
            round: result.round,
            round_name: format!("Round {}", result.round),
            round_comment_url: result.round_comment_url.clone(),
            anchor: result.anchor.to_string(),
            branch: result.branch.clone(),
            comparison_base: result.comparison_base.to_string(),
            divergence: GapContinuityInfo::divergence(&result.continuity),
            reopened: (&result.reopened).into(),
            body_marker: (&result.body_marker).into(),
            notification: (&result.notification).into(),
            needs_repair: result.needs_repair(),
            impacted_issues: (&result.impacted_issues).into(),
        }
    }
}

/// Which of the open round's follow-up steps are incomplete. Mirrors
/// [`crate::RepairPlan`], and is what `POST /api/issues/{n}/rounds/repair` would
/// act on.
#[derive(Debug, Clone, Serialize)]
pub struct RoundRepairStatus {
    /// Index of the open round these flags describe.
    pub round: u32,
    /// Name of that round, e.g. `"Round 2"`.
    pub round_name: String,
    /// The issue is closed while this round is open.
    pub reopen: bool,
    /// The `## QC Round` body marker is missing or disagrees with this round.
    pub body_marker: bool,
    /// This round carries no `# QC Notification`. Informational: **not** a defect,
    /// and therefore excluded from `needs_repair` — opening a round without
    /// notifying is a legitimate choice, and a repair only posts one on request.
    pub notification_missing: bool,
    /// Whether something is actually wrong, i.e. `reopen || body_marker`. The only
    /// field a client should use to decide whether to offer a repair.
    pub needs_repair: bool,
}

impl RoundRepairStatus {
    /// `None` unless `round` is open **and** was opened by a `# QC Round`
    /// comment: Initial QC is opened by creating the issue, so it has no follow-up
    /// steps of a start-round action to complete.
    pub fn derive(issue: &octocrab::models::issues::Issue, round: &crate::Round) -> Option<Self> {
        if !round.is_open() || !matches!(round.opened, crate::RoundOpen::NewRound { .. }) {
            return None;
        }
        let plan = crate::plan_repair(issue, round);
        Some(Self {
            round: round.index,
            round_name: round.name(),
            reopen: plan.reopen,
            body_marker: plan.body_marker,
            notification_missing: plan.notification_missing,
            needs_repair: plan.needs_repair(),
        })
    }
}

/// Response for repairing an issue's open round.
///
/// Mirrors [`StartRoundResponse`]'s per-step shape. A `failed` step is **not** an
/// error response: the repair is a best-effort completion of steps that are each
/// idempotent, so `needs_repair` simply says one still wants another attempt.
#[derive(Debug, Clone, Serialize)]
pub struct RepairRoundResponse {
    /// Index of the open round that was repaired.
    pub round: u32,
    /// Name of that round, e.g. `"Round 2"`.
    pub round_name: String,
    /// URL of the round's round comment. `null` when the comment came
    /// from the disk cache and so carries no identity — in which case the body
    /// marker is deliberately left alone rather than rewritten with a wrong URL.
    pub round_comment_url: Option<String>,
    /// Reopening the issue.
    pub reopened: StepOutcomeResponse,
    /// Refreshing the `## QC Round` block in the issue body. `skipped` with a
    /// `skipped_reason` when the round's comment URL is unknown, because no correct
    /// marker can be derived then — the CLI has always explained that skip, and this is
    /// the second step that can carry a reason.
    pub body_marker: StepOutcomeResponse,
    /// The `# QC Notification` comment. `skipped` unless the request asked for one
    /// *and* the round had none — and `skipped` with a `skipped_reason` when the round
    /// could not be placed, which is the one step placement blocks (the other two still
    /// run).
    pub notification: StepOutcomeResponse,
    /// Whether anything was actually written.
    pub repaired: bool,
    /// Whether a step that was attempted failed and still wants a retry.
    pub needs_repair: bool,
}

impl From<&crate::RepairRoundResult> for RepairRoundResponse {
    fn from(result: &crate::RepairRoundResult) -> Self {
        Self {
            round: result.round,
            round_name: result.round_name.clone(),
            round_comment_url: result.round_comment_url.clone(),
            reopened: (&result.reopened).into(),
            body_marker: StepOutcomeResponse::from(&result.body_marker)
                .skipped_because(result.body_marker_skip_reason()),
            notification: StepOutcomeResponse::from(&result.notification)
                .skipped_because(result.notification_skip_reason()),
            repaired: result.repaired(),
            needs_repair: result.needs_repair(),
        }
    }
}

/// Everything a "start new round" form needs, with no side effects.
#[derive(Debug, Clone, Serialize)]
pub struct RoundSeedResponse {
    /// Repo-relative path of the QC'd file, so a caller can diff it without
    /// inferring the path from the issue title.
    pub file: String,
    /// Index the next round would get.
    pub next_round: u32,
    /// Name the next round would get, e.g. `"Round 2"`.
    pub next_round_name: String,
    /// Seeded checklist markdown, boxes reset — the `default_round` entry of
    /// `checklist_options`. `None` when no round's checklist could be recovered.
    pub checklist_content: Option<String>,
    /// Name of the template the seed came from, when recorded.
    pub checklist_name: Option<String>,
    /// Every round's checklist, oldest first, so the form can offer a choice of
    /// which round to base the new one on. Rounds whose checklist could not be
    /// recovered are absent, so this may be shorter than the round list — and
    /// empty, in which case there is nothing to seed from.
    pub checklist_options: Vec<RoundChecklistOption>,
    /// `round` of the pre-selected entry of `checklist_options` (the most recent
    /// one). `None` when `checklist_options` is empty.
    pub default_round: Option<u32>,
    /// The anchor the round would open at (HEAD of the issue's branch). `None`
    /// when HEAD could not be resolved.
    pub anchor: Option<String>,
    /// The approval the new round would build on: the last round's closing commit.
    /// `None` when the last round is still open.
    pub previous_approval: Option<String>,
    /// The branch the round would open on: whatever is currently checked out, which
    /// need not be the branch the issue was created on. `None` when it could not be
    /// determined.
    pub branch: Option<String>,
    /// What the round's diff would actually compare against — `previous_approval`
    /// normally, or the merge-base when that approval is not an ancestor of `anchor`.
    pub comparison_base: Option<String>,
    /// Set only when the previous approval is unreachable from `branch`. `None` for
    /// the linear case, which is what `null` means on this field.
    pub divergence: Option<GapContinuityInfo>,
    /// Whether starting a round is currently legal.
    pub can_start: bool,
    /// Why not, when `can_start` is false.
    pub blocked_reason: Option<String>,
}

/// One selectable checklist source for a new round: an existing round, and the
/// checklist it was QC'd against with every box reset.
#[derive(Debug, Clone, Serialize)]
pub struct RoundChecklistOption {
    /// Index of the round this checklist came from.
    pub round: u32,
    /// That round's display name, e.g. `"Initial QC"` or `"Round 2"`.
    pub round_name: String,
    /// Template name that round recorded, when it recorded one.
    pub checklist_name: Option<String>,
    /// Checklist markdown, boxes reset.
    pub content: String,
}

impl From<&crate::ChecklistOption> for RoundChecklistOption {
    fn from(option: &crate::ChecklistOption) -> Self {
        Self {
            round: option.round,
            round_name: option.round_name.clone(),
            checklist_name: option.checklist_name.clone(),
            content: option.content.clone(),
        }
    }
}

/// Repository assignee.
#[derive(Debug, Serialize)]
pub struct Assignee {
    pub login: String,
    pub name: Option<String>,
}

/// Full checklist with content.
#[derive(Debug, Serialize, Deserialize)]
pub struct Checklist {
    pub name: String,
    pub content: String,
}

impl From<crate::Checklist> for Checklist {
    fn from(checklist: crate::Checklist) -> Self {
        Self {
            name: checklist.name,
            content: checklist.content,
        }
    }
}

/// Git repository configuration status.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigGitRepository {
    pub owner: String,
    pub repo: String,
    pub status: GitStatusEnum,
    /// Human readable summary of `status`, e.g. "Repository is behind by 3 commits".
    pub status_detail: String,
    /// Full shas of local-only commits (empty unless ahead/diverged).
    pub ahead_commits: Vec<String>,
    /// Full shas of remote-only commits (empty unless behind/diverged).
    pub behind_commits: Vec<String>,
    pub dirty_files: Vec<String>,
}

impl ConfigGitRepository {
    pub async fn new<G: GitProvider + Clone + Send + 'static>(
        git_info: &G,
    ) -> Result<Self, ApiError> {
        let owner = git_info.owner().to_string();
        let repo = git_info.repo().to_string();
        let git_info = git_info.clone();

        // Perform blocking git operations in a blocking task
        let (status, dirty_files) = tokio::task::spawn_blocking(move || {
            let status = get_git_status(&git_info)?;
            let api_status: GitStatus = status.state.into();
            let dirty_files = status
                .dirty
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>();

            Ok::<_, ApiError>((api_status, dirty_files))
        })
        .await
        .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))??;

        Ok(Self {
            owner,
            repo,
            status: status.status,
            status_detail: status.detail,
            ahead_commits: status.ahead_commits,
            behind_commits: status.behind_commits,
            dirty_files,
        })
    }
}

/// Configuration options.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigurationOptions {
    pub prepended_checklist_note: Option<String>,
    pub checklist_display_name: String,
    pub include_collaborators: bool,
    pub logo_path: String,
    pub logo_found: bool,
    pub checklist_directory: String,
    pub record_path: String,
    pub ui_repo_refresh_rate_seconds: u64,
    /// Resolved (config, env, default) flag for whether the UI may fast-forward the
    /// configuration repository.
    pub allow_config_update: bool,
}

/// Configuration status response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigurationStatusResponse {
    pub directory: String,
    pub exists: bool,
    pub git_repository: Option<ConfigGitRepository>,
    pub options: ConfigurationOptions,
    pub checklists: Vec<Checklist>,
    pub config_repo_env: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoInfoResponse {
    pub owner: String,
    pub repo: String,
    /// Name of the user's default remote (almost always `"origin"`, but
    /// configurable via `git clone --origin <name>` or rename).
    pub remote: String,
    pub branch: String,
    pub local_commit: String,
    pub remote_commit: String,
    pub git_status: GitStatusEnum,
    pub git_status_detail: String,
    pub dirty_files: Vec<String>,
    pub current_user: Option<String>,
}

/// Kind of a file tree entry.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeEntryKind {
    File,
    Directory,
}

/// A single entry in a file tree listing.
#[derive(Debug, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub kind: TreeEntryKind,
}

/// Response for a file tree listing at a given path.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileTreeResponse {
    pub path: String,
    pub entries: Vec<TreeEntry>,
}

/// Response for default file collaborators derived from git history.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileCollaboratorsResponse {
    pub path: String,
    pub author: Option<String>,
    pub collaborators: Vec<String>,
}

/// Response for archive generation.
#[derive(Debug, Serialize)]
pub struct ArchiveGenerateResponse {
    pub output_path: String,
}

/// Response for context PDF upload.
#[derive(Debug, Serialize)]
pub struct RecordUploadResponse {
    pub temp_path: String,
}

/// Response for record preview generation.
#[derive(Debug, Serialize)]
pub struct RecordPreviewResponse {
    pub key: String,
}

/// Extract the checklist name from the first h1 heading (e.g. "# Code Review").
fn parse_checklist_name(body: &str) -> Option<String> {
    body.lines()
        .find(|l| l.starts_with("# ") && !l.starts_with("## "))
        .map(|l| l[2..].trim().to_string())
}

/// Minimal branch parser — handles plain text and markdown links.
fn parse_branch_from_body_simple(body: &str) -> Option<String> {
    let pattern = "git branch: ";
    let start = body.find(pattern)?;
    let line = body[start + pattern.len()..].lines().next()?;
    // strip markdown/html links to just the link text
    if let (Some(a), Some(b)) = (line.find('['), line.find("](")) {
        return Some(line[a + 1..b].trim().to_string());
    }
    let plain = line.trim();
    if plain.is_empty() {
        None
    } else {
        Some(plain.to_string())
    }
}

/// Parse all entries from the "## Relevant Files" section.
fn parse_relevant_file_infos(body: &str) -> Vec<RelevantFileInfo> {
    use regex::Regex;
    use std::sync::LazyLock;
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap());
    static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*([^*]+)\*\*").unwrap());

    let rf_start = match body.find("## Relevant Files") {
        Some(p) => p,
        None => return vec![],
    };
    let section = &body[rf_start..];
    let end = section[17..]
        .find("\n## ")
        .map(|p| p + 17)
        .unwrap_or(section.len());
    let section = &section[..end];

    let mut result = Vec::new();

    for (sub, kind) in [
        ("### Previous QC", RelevantFileKind::BlockingQc),
        ("### Gating QC", RelevantFileKind::BlockingQc),
        ("### Relevant QC", RelevantFileKind::RelevantQc),
        ("### Relevant File", RelevantFileKind::File),
    ] {
        let sub_start = match section.find(sub) {
            Some(p) => p,
            None => continue,
        };
        let sub_section = &section[sub_start..];
        let sub_end = sub_section[sub.len()..]
            .find("\n### ")
            .map(|p| p + sub.len())
            .unwrap_or(sub_section.len());
        let sub_section = &sub_section[..sub_end];

        if kind == RelevantFileKind::File {
            for cap in BOLD.captures_iter(sub_section) {
                result.push(RelevantFileInfo {
                    file_name: cap[1].to_string(),
                    kind: RelevantFileKind::File,
                    issue_url: None,
                });
            }
        } else {
            for cap in LINK.captures_iter(sub_section) {
                result.push(RelevantFileInfo {
                    file_name: cap[1].to_string(),
                    kind: kind.clone(),
                    issue_url: Some(cap[2].to_string()),
                });
            }
        }
    }
    result
}

impl RepoInfoResponse {
    pub async fn new<G: GitProvider + Clone + Send + 'static>(
        git_info: &G,
    ) -> Result<Self, ApiError> {
        let owner = git_info.owner().to_string();
        let repo = git_info.repo().to_string();
        let remote = git_info.remote_name().to_string();

        // Async GitHub call — non-fatal, falls back to None
        let current_user = git_info.get_current_user().await.ok().flatten();

        let git_info = git_info.clone();

        // Perform blocking git operations in a blocking task
        let (branch, local_commit, remote_commit, git_status_enum, git_status_detail, dirty_files) =
            tokio::task::spawn_blocking(move || {
                let git_status = get_git_status(&git_info)?;
                let local_commit = git_info.commit()?;
                let branch = git_info.branch()?;
                let remote_commit = git_status.remote_commit.to_string();
                let api_git_status = GitStatus::from(git_status.state.clone());

                Ok::<_, ApiError>((
                    branch,
                    local_commit,
                    remote_commit,
                    api_git_status.status,
                    git_status.state.to_string(),
                    git_status
                        .dirty
                        .into_iter()
                        .map(|p| p.display().to_string())
                        .collect(),
                ))
            })
            .await
            .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))??;

        Ok(Self {
            owner,
            repo,
            remote,
            branch,
            local_commit,
            remote_commit,
            git_status: git_status_enum,
            git_status_detail,
            dirty_files,
            current_user,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use crate::{Gap, GapContinuity, Placement, Round, Segment, UnplaceableReason};

    /// A recognisable full sha made of one repeated hex digit.
    fn oid(digit: char) -> ObjectId {
        ObjectId::from_str(&std::iter::repeat_n(digit, 40).collect::<String>())
            .expect("40 hex digits is a sha")
    }

    fn commit(hash: ObjectId) -> crate::IssueCommit {
        crate::IssueCommit {
            hash,
            message: "a commit".to_string(),
            file_changed: true,
        }
    }

    fn round(index: u32, opened_at: ObjectId, commits: Vec<crate::IssueCommit>) -> Round {
        Round {
            index,
            opened_at,
            branch: "main".to_string(),
            opened: crate::RoundOpen::IssueCreated,
            checklist: crate::ChecklistSource::IssueBody,
            checklist_name: None,
            state: crate::RoundState::Open,
            events: Vec::new(),
            retractions: Vec::new(),
            extensions: Vec::new(),
            commits,
            placement: Placement::Placed,
        }
    }

    fn closed_at(mut round: Round, commit: ObjectId) -> Round {
        round.state = crate::RoundState::Closed {
            commit,
            by: "reviewer".to_string(),
            at: Utc::now(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        };
        round
    }

    fn notification(commit: ObjectId) -> crate::RoundEvent {
        crate::RoundEvent::Notification {
            commit,
            by: "author".to_string(),
            at: Utc::now(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn review(commit: ObjectId) -> crate::RoundEvent {
        crate::RoundEvent::Review {
            commit,
            by: "reviewer".to_string(),
            at: Utc::now(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn gap(commits: Vec<crate::IssueCommit>) -> Gap {
        Gap {
            branch: "main".to_string(),
            commits,
            continuity: GapContinuity::Linear,
            placement: Placement::Placed,
        }
    }

    /// A thread with nothing but the segments under test: everything the projections
    /// below read is a function of the segment list.
    fn thread(segments: Vec<Segment>) -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/test.rs"),
            open: true,
            milestone: "v1.0".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies: Vec::new(),
        }
    }

    /// Serialize the whole segment list, as the status response does.
    fn project(segments: &[Segment]) -> Vec<serde_json::Value> {
        (0..segments.len())
            .map(|position| {
                serde_json::to_value(SegmentInfo::project(segments, position))
                    .expect("a segment serializes")
            })
            .collect()
    }

    /// The `Placement` serde trap: internally tagging M5's newtype variant around a
    /// unit-only enum compiles *and* serializes, emitting the reason as a **key**
    /// (`{"kind":"unplaceable","branch_unavailable":null}`). Neither the compiler nor
    /// serialization catches it, so this asserts the emitted JSON.
    #[test]
    fn an_unplaceable_placement_emits_its_reason_as_a_value_not_a_key() {
        let mut initial = round(1, oid('a'), Vec::new());
        initial.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let segments = vec![Segment::Round(initial)];

        assert_eq!(
            project(&segments)[0]["placement"],
            serde_json::json!({ "kind": "unplaceable", "reason": "branch_unavailable" })
        );
    }

    /// `reason` is *absent* rather than null when placed, so a client narrows on `kind`
    /// alone.
    #[test]
    fn a_placed_placement_carries_no_reason_key_at_all() {
        let segments = vec![Segment::Round(round(1, oid('a'), Vec::new()))];

        assert_eq!(
            project(&segments)[0]["placement"],
            serde_json::json!({ "kind": "placed" })
        );
    }

    /// `kind` sits *alongside* the variant's own fields, which is what makes
    /// `segments.at(-1).kind === "round"` the test for "a round is open".
    #[test]
    fn a_segments_kind_is_a_sibling_of_its_own_fields() {
        let segments = vec![
            Segment::Round(closed_at(
                round(1, oid('a'), vec![commit(oid('a'))]),
                oid('a'),
            )),
            Segment::Gap(gap(Vec::new())),
        ];
        let projected = project(&segments);

        assert_eq!(projected[0]["kind"], "round");
        assert_eq!(projected[0]["index"], 1);
        assert_eq!(projected[1]["kind"], "gap");
        assert!(projected[1].get("index").is_none(), "gaps are positional");
    }

    /// The order is fixed and deduplicated, unlike the `HashSet` the field used to come
    /// from: the events below are declared review-first and notify the same commit
    /// twice.
    #[test]
    fn commit_statuses_are_emitted_in_a_fixed_order_without_duplicates() {
        let anchor = oid('a');
        let mut initial = closed_at(round(1, anchor, vec![commit(anchor)]), anchor);
        initial.events = vec![review(anchor), notification(anchor), notification(anchor)];
        let segments = vec![Segment::Round(initial), Segment::Gap(gap(Vec::new()))];

        assert_eq!(
            project(&segments)[0]["commits"][0]["statuses"],
            serde_json::json!(["initial", "notification", "approved", "reviewed"])
        );
    }

    /// D14: a boundary commit is projected per owning segment, so the same hash carries
    /// `approved` in the round it closed and nothing in the round it anchors. The
    /// asymmetry is correct — in round 2's frame that commit is not round 2's approval.
    #[test]
    fn a_shared_boundary_commit_is_approved_in_one_round_and_bare_in_the_next() {
        let (older, boundary, newer) = (oid('a'), oid('b'), oid('c'));
        let segments = vec![
            Segment::Round(closed_at(
                round(1, older, vec![commit(boundary), commit(older)]),
                boundary,
            )),
            Segment::Gap(gap(Vec::new())),
            Segment::Round(round(2, boundary, vec![commit(newer), commit(boundary)])),
        ];
        let projected = project(&segments);

        assert_eq!(projected[0]["commits"][0]["hash"], boundary.to_string());
        assert_eq!(
            projected[0]["commits"][0]["statuses"],
            serde_json::json!(["approved"])
        );
        assert_eq!(projected[2]["commits"][1]["hash"], boundary.to_string());
        assert_eq!(
            projected[2]["commits"][1]["statuses"],
            serde_json::json!([])
        );
    }

    /// D14's corollary: a gap has no events and no state, so its commits can only ever
    /// carry `initial` — and that belongs to a round's anchor.
    #[test]
    fn a_gaps_commits_carry_no_round_statuses() {
        let (anchor, drift) = (oid('a'), oid('c'));
        let segments = vec![
            Segment::Round(closed_at(round(1, anchor, vec![commit(anchor)]), anchor)),
            Segment::Gap(gap(vec![commit(drift)])),
        ];

        assert_eq!(
            project(&segments)[1]["commits"][0]["statuses"],
            serde_json::json!([])
        );
    }

    /// A5's bounds are read from the *immediate* neighbours: older at `pos - 1`, newer
    /// at `pos + 1`. A round's `pos - 2` rule would land on another gap.
    #[test]
    fn an_interior_gaps_bounds_come_from_its_two_neighbouring_rounds() {
        let (first, approval, drift, anchor) = (oid('a'), oid('b'), oid('c'), oid('d'));
        let segments = vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            Segment::Gap(gap(vec![commit(drift)])),
            Segment::Round(round(2, anchor, vec![commit(anchor)])),
        ];
        let projected = project(&segments);

        assert_eq!(projected[1]["lower_bound"], approval.to_string());
        assert_eq!(projected[1]["upper_bound"], anchor.to_string());
    }

    /// A trailing gap's newer bound is the branch tip, inclusive. With nothing since
    /// the approval the tip *is* that approval, so the two bounds coincide.
    #[test]
    fn an_empty_trailing_gap_has_both_bounds_on_the_approval() {
        let (first, approval) = (oid('a'), oid('b'));
        let segments = vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            Segment::Gap(gap(Vec::new())),
        ];
        let projected = project(&segments);

        assert_eq!(projected[1]["lower_bound"], approval.to_string());
        assert_eq!(projected[1]["upper_bound"], approval.to_string());
    }

    /// A gap that could not be placed **itself** has no range, so neither bound is a
    /// handle a consumer may draw on — however well the bounds resolve.
    ///
    /// This is the reachable shape: `build_gap` marks a gap whose bounding round is
    /// unplaceable as `NeighbourUnplaceable`, so a gap can never be `Placed` alongside an
    /// unplaceable neighbour. Reporting a real `lower_bound` on a segment U1 is graying
    /// hands it a detached handle to draw across an unknown range — W6's
    /// plausible-but-wrong, which is worse than nothing.
    #[test]
    fn a_self_unplaceable_gap_reports_neither_bound() {
        let (first, approval, anchor) = (oid('a'), oid('b'), oid('d'));
        let mut second = round(2, anchor, Vec::new());
        second.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let mut between = gap(Vec::new());
        between.placement = Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable);
        let segments = vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            Segment::Gap(between),
            Segment::Round(second),
        ];
        let projected = project(&segments);

        assert_eq!(
            projected[1]["lower_bound"],
            serde_json::Value::Null,
            "the approval resolves, but this gap's range is unknown"
        );
        assert_eq!(projected[1]["upper_bound"], serde_json::Value::Null);
        // The reason is still named, so U2 can gray it with an explanation (D4).
        assert_eq!(
            projected[1]["placement"],
            serde_json::json!({"kind": "unplaceable", "reason": "neighbour_unplaceable"})
        );
    }

    /// The neighbour filter, independently of the gap's own placement: an unplaceable
    /// round resolved no commits, so its anchor is not a bound anyone can be handed,
    /// while the approval on the other side still is.
    ///
    /// **Unreachable from the fold**, and hand-built for that reason: `build_gap` returns
    /// `NeighbourUnplaceable` before computing bounds whenever a bounding round is
    /// unplaceable, so a `Placed` gap beside one cannot arise — the assertion above
    /// covers every shape the fold emits. Kept so the per-neighbour rule keeps its
    /// meaning if the model ever places such a gap.
    #[test]
    fn a_placed_gap_still_loses_only_the_unplaceable_neighbours_bound() {
        let (first, approval, anchor) = (oid('a'), oid('b'), oid('d'));
        let mut second = round(2, anchor, Vec::new());
        second.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let segments = vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            Segment::Gap(gap(Vec::new())),
            Segment::Round(second),
        ];
        let projected = project(&segments);

        assert_eq!(projected[1]["lower_bound"], approval.to_string());
        assert_eq!(projected[1]["upper_bound"], serde_json::Value::Null);
    }

    /// An unplaceable round's anchor is the all-zero OID the fold leaves behind, which is
    /// not a commit anyone can address — so `opened_at` is null rather than a fake sha.
    #[test]
    fn an_unplaceable_round_reports_no_opened_at() {
        let mut initial = round(1, ObjectId::null(gix::hash::Kind::Sha1), Vec::new());
        initial.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let projected = project(&[Segment::Round(initial)]);

        assert_eq!(projected[0]["opened_at"], serde_json::Value::Null);
        assert!(
            !projected[0].to_string().contains("0000000000"),
            "no all-zero sha may reach the wire: {}",
            projected[0]
        );
    }

    /// An `Unrelated` trailing gap owns nothing and its tip shares no history with the
    /// approval, so there is no newer bound to report.
    ///
    /// **Unreachable from the fold today; this guards a future model change.** Per D15's
    /// addendum a trailing gap walks the previous round's branch, and a `Placed`/`Closed`
    /// round necessarily has its closing commit on that walk, so the lower bound always
    /// resolves and the continuity is always `Linear`. An approval whose commit has
    /// vanished degrades through `Unplaceable` instead. The state is hand-built here so
    /// the fallback keeps its meaning if the model ever emits it.
    #[test]
    fn an_unrelated_trailing_gap_reports_no_upper_bound() {
        let (first, approval) = (oid('a'), oid('b'));
        let mut trailing = gap(Vec::new());
        trailing.continuity = GapContinuity::Unrelated;
        let segments = vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            Segment::Gap(trailing),
        ];
        let projected = project(&segments);

        assert_eq!(
            projected[1]["continuity"],
            serde_json::json!({"kind": "unrelated"})
        );
        assert_eq!(projected[1]["lower_bound"], approval.to_string());
        assert_eq!(projected[1]["upper_bound"], serde_json::Value::Null);
    }

    /// `merge_base` is required and non-null on `diverged`, and absent on the other two
    /// variants — which is what makes the client's union ergonomic.
    #[test]
    fn gap_continuity_carries_a_merge_base_only_when_diverged() {
        let mut diverged = gap(Vec::new());
        diverged.continuity = GapContinuity::Diverged {
            merge_base: oid('e'),
        };
        let segments = vec![
            Segment::Round(closed_at(
                round(1, oid('a'), vec![commit(oid('a'))]),
                oid('a'),
            )),
            Segment::Gap(diverged),
        ];

        assert_eq!(
            project(&segments)[1]["continuity"],
            serde_json::json!({ "kind": "diverged", "merge_base": oid('e').to_string() })
        );
    }

    // ── D12: the two event-named commit fields ───────────────────────────────

    /// D12's whole point: *Reviewed* and *Last Posted* are different facts, so the two
    /// fields must be derived from the right event kind and must report the **newest**
    /// commit each names. The round below notifies the two oldest commits and reviews
    /// the two newest, so a swapped derivation and an oldest-instead-of-newest
    /// derivation both change the answer.
    #[test]
    fn the_active_rounds_reviewed_and_notified_commits_are_its_newest_of_each_kind() {
        let (newest, mid, oldest) = (oid('c'), oid('b'), oid('a'));
        let mut open = round(1, oldest, vec![commit(newest), commit(mid), commit(oldest)]);
        open.events = vec![
            notification(oldest),
            notification(mid),
            review(mid),
            review(newest),
        ];
        let status = QCStatus::from(&thread(vec![Segment::Round(open)]));

        assert_eq!(
            status.last_notified_commit.as_deref(),
            Some(mid.to_string().as_str())
        );
        assert_eq!(
            status.last_reviewed_commit.as_deref(),
            Some(newest.to_string().as_str())
        );
        assert_ne!(
            status.last_notified_commit, status.last_reviewed_commit,
            "the two fields must not be interchangeable — swapping them is the bug D12 exists for"
        );
    }

    /// Coverage is scoped to the round's own commits (S3), so an event naming a commit
    /// outside them reports nothing rather than leaking a foreign hash.
    #[test]
    fn an_event_naming_a_commit_the_round_does_not_own_reports_neither_field() {
        let (anchor, foreign) = (oid('a'), oid('f'));
        let mut open = round(1, anchor, vec![commit(anchor)]);
        open.events = vec![notification(foreign), review(foreign)];
        let status = QCStatus::from(&thread(vec![Segment::Round(open)]));

        assert_eq!(status.last_notified_commit, None);
        assert_eq!(status.last_reviewed_commit, None);
    }

    // ── A2/A3: the branch and the two approvals ──────────────────────────────

    /// A2, and the card-graying bug P3 exists to kill: `active_branch` is the **last**
    /// segment's branch, not the first's. A cross-branch round 2 is the only shape that
    /// tells them apart, which is why every earlier single-round assertion missed it.
    ///
    /// A3's asymmetry rides along: with a round open the standing approval is gone even
    /// though the thread's last approval is not.
    #[test]
    fn a_cross_branch_round_two_reports_its_own_branch_and_only_the_ungated_approval() {
        let (first, approval, anchor) = (oid('a'), oid('b'), oid('d'));
        let mut second = round(2, anchor, vec![commit(anchor)]);
        second.branch = "feature/x".to_string();
        let mut between = gap(Vec::new());
        between.branch = "feature/x".to_string();
        let thread = thread(vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            Segment::Gap(between),
            Segment::Round(second),
        ]);
        let issue = crate::test_utils::create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            "Quality check issue for src/test.rs",
            Some(1),
            "open",
        );
        let response = IssueStatusResponse::new(&issue, &thread, &[]);

        assert_eq!(response.active_branch, "feature/x");
        assert_eq!(
            thread.segments[0].branch(),
            "main",
            "the first segment's branch is the one the old field reported"
        );
        // A round is active, so nothing stands — but something was approved.
        assert_eq!(response.qc_status.standing_approval, None);
        assert_eq!(
            response.qc_status.last_approved_commit.as_deref(),
            Some(approval.to_string().as_str())
        );
        assert_eq!(
            response.qc_status.initial_commit.as_deref(),
            Some(first.to_string().as_str())
        );
    }

    /// An unplaceable Initial QC resolved no commits, so its anchor is not a sha anyone
    /// may address (S4/I5) and there is no status to report: `determine_status` yields
    /// `None`, which the wire reports as the dedicated `unknown` value rather than
    /// borrowing `in_progress`. The two are distinct facts — see
    /// [`crate::qc_status`]'s `a_placed_round_with_no_file_change_and_no_events_is_in_progress_not_unknown`,
    /// which pins the reachable `InProgress` state this must never collapse into.
    #[test]
    fn an_unplaceable_initial_qc_has_no_initial_commit_and_an_unknown_detail() {
        let mut initial = round(1, oid('a'), Vec::new());
        initial.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let thread = thread(vec![Segment::Round(initial)]);
        let issue = crate::test_utils::create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            "Quality check issue for src/test.rs",
            Some(1),
            "open",
        );
        let response = IssueStatusResponse::new(&issue, &thread, &[]);

        assert_eq!(response.qc_status.initial_commit, None);
        assert_eq!(response.qc_status.status_detail, "Unknown");
        assert_eq!(response.qc_status.status, QCStatusEnum::Unknown);
        // No commits to place means no base for the next notification either.
        assert_eq!(response.next_notification_from, None);
    }

    /// `changed_commit` names the trailing gap's newest **file-changing** commit, which
    /// is not the same commit as `latest_commit` whenever the newest drift never touched
    /// the file. **S1** picks the file-changing one, so a client rendering "changed at"
    /// from `latest_commit` would name a commit that never touched the file.
    #[test]
    fn changed_commit_names_the_file_changing_commit_not_the_newest() {
        let approval = oid('b');
        let initial = closed_at(round(1, oid('a'), vec![commit(oid('a'))]), approval);
        // Newest-first: the newest commit did not touch the file; the older one did.
        let untouched = crate::IssueCommit {
            file_changed: false,
            ..commit(oid('d'))
        };
        let touched = commit(oid('c'));
        let thread = thread(vec![
            Segment::Round(initial),
            Segment::Gap(gap(vec![untouched, touched])),
        ]);
        let issue = crate::test_utils::create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            "Quality check issue for src/test.rs",
            Some(1),
            "open",
        );
        let response = IssueStatusResponse::new(&issue, &thread, &[]);

        assert_eq!(
            response.qc_status.status,
            QCStatusEnum::ChangesAfterApproval
        );
        assert_eq!(
            response.qc_status.changed_commit.as_deref(),
            Some(oid('c').to_string().as_str()),
            "S1 selects the newest file-changing commit of the trailing gap"
        );
        assert_eq!(
            response.qc_status.latest_commit.as_deref(),
            Some(oid('d').to_string().as_str()),
            "latest_commit is the gap's newest commit, which changed nothing"
        );
        assert_ne!(
            response.qc_status.changed_commit, response.qc_status.latest_commit,
            "the whole point: these two are different commits here"
        );
    }

    /// `changed_commit` is null in every state but `changes_after_approval` — a trailing
    /// gap whose commits never touched the file is approved, not changed.
    #[test]
    fn changed_commit_is_null_when_nothing_changed_the_file() {
        let approval = oid('b');
        let initial = closed_at(round(1, oid('a'), vec![commit(oid('a'))]), approval);
        let untouched = crate::IssueCommit {
            file_changed: false,
            ..commit(oid('d'))
        };
        let thread = thread(vec![
            Segment::Round(initial),
            Segment::Gap(gap(vec![untouched])),
        ]);
        let issue = crate::test_utils::create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            "Quality check issue for src/test.rs",
            Some(1),
            "open",
        );
        let response = IssueStatusResponse::new(&issue, &thread, &[]);

        assert_eq!(response.qc_status.status, QCStatusEnum::Approved);
        assert_eq!(response.qc_status.changed_commit, None);
    }

    // ── W6: an unplaceable *neighbour* nulls one bound, never both ───────────

    /// An interior gap is bounded by position, not by "is there a placeable round
    /// after me": with its newer round unplaceable the newer bound is null, and the
    /// trailing rule — which would hand back the gap's own newest commit — must not
    /// apply (W6 forbids substituting a bound the record does not support).
    #[test]
    fn an_interior_gap_never_falls_through_to_the_trailing_bound_rule() {
        let (first, approval, drift, anchor) = (oid('a'), oid('b'), oid('c'), oid('d'));
        let mut second = round(2, anchor, Vec::new());
        second.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let segments = vec![
            Segment::Round(closed_at(
                round(1, first, vec![commit(approval), commit(first)]),
                approval,
            )),
            // Deliberately *not* itself unplaceable and deliberately non-empty: the
            // cascade that makes this shape unreachable today is not what the rule
            // rests on.
            Segment::Gap(gap(vec![commit(drift)])),
            Segment::Round(second),
        ];
        let projected = project(&segments);

        assert_eq!(projected[1]["lower_bound"], approval.to_string());
        assert_eq!(projected[1]["upper_bound"], serde_json::Value::Null);
    }

    /// On a round-start response `null` already means "no divergence", so the linear
    /// case has no encoding of its own there.
    #[test]
    fn linear_continuity_is_null_as_a_round_start_divergence() {
        assert!(GapContinuityInfo::divergence(&GapContinuity::Linear).is_none());
        assert_eq!(
            serde_json::to_value(GapContinuityInfo::divergence(&GapContinuity::Unrelated))
                .expect("serializes"),
            serde_json::json!({ "kind": "unrelated" })
        );
    }
}
