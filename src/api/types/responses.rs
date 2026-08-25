//! API response types.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use gix::ObjectId;
use octocrab::models::IssueState;
use serde::{Deserialize, Serialize};

use crate::{
    FileRenameEvent, GitHubApiError, GitProvider, IssueThread, ReviewStashResult, api::ApiError,
    create::CreateResult, get_git_status, parse_blocking_qcs, parse_file_history,
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
    /// Round 1's declared branch, parsed from the body — **retained** (D50; D48's
    /// removal is reversed) because the cheap list paths return bare issues with no
    /// round data and must not fold a thread per issue just to render a branch.
    ///
    /// Its validity condition:
    /// - **No `## QC Rounds` marker in the body ⇒ a single round ⇒ this IS the current
    ///   branch.** The body is complete and authoritative for that issue's branch.
    /// - **Marker present ⇒ rounds exist ⇒ this is round 1's and may be stale.** A
    ///   consumer needing the current branch must read `rounds[last].branch` (D9/M7),
    ///   which requires the comment fetch.
    ///
    /// Inside a rounds-bearing `IssueStatusResponse` this duplicates
    /// `rounds[0].branch`, so it is a **stated exception to D24's letter** (D50),
    /// justified because the same `Issue` type is reused by list and create flows that
    /// never fetch `rounds[]`. Consumers of a rounds-bearing response MUST read
    /// `rounds[last].branch`.
    pub branch: Option<String>,
    /// D51: the `## QC Rounds` marker's **presence**, so a cheap path that never
    /// fetches comments can tell whether `branch` above is still current. Presence
    /// only — the section's text is never authoritative, and this says nothing about
    /// how many rounds there are.
    pub has_qc_rounds_marker: bool,
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
            has_qc_rounds_marker: issue
                .body
                .as_deref()
                .map(crate::issue::has_qc_rounds_marker)
                .unwrap_or(false),
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

/// QC status information — a **verdict, not a commit carrier** (D35).
///
/// `approved_commit`, `initial_commit` and `latest_commit` are gone: all three were
/// round-scoped and duplicated `rounds[last].state`, `rounds[0].start_commit` and
/// `rounds[last].archive_commit`. The `ChangesAfterApproval` hash, which
/// `QCStatusEnum` drops, is served as `drift.newest_file_change`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QCStatus {
    pub status: QCStatusEnum,
    pub status_detail: String,
}

impl TryFrom<&IssueThread> for QCStatus {
    type Error = crate::IssueError;

    /// D55: fallible, because `determine_status` refuses rather than guessing when the
    /// latest round has no resolvable commit. The error carries the branch to fetch and
    /// reaches the client as `IssueStatusError { kind: branch_not_local, branch }` —
    /// `QCStatusEnum` gains no variant for it (S5).
    fn try_from(issue: &IssueThread) -> Result<Self, Self::Error> {
        let status = crate::QCStatus::determine_status(issue)?;
        Ok(Self {
            status_detail: status.to_string(),
            status: status.into(),
        })
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

/// Commit information for an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueCommit {
    pub hash: String,
    pub message: String,
    pub statuses: Vec<CommitStatusEnum>,
    pub file_changed: bool,
}

impl From<&crate::IssueCommit> for IssueCommit {
    fn from(commit: &crate::IssueCommit) -> Self {
        Self {
            hash: commit.hash.to_string(),
            message: commit.message.to_string(),
            statuses: commit.statuses.iter().map(CommitStatusEnum::from).collect(),
            file_changed: commit.file_changed,
        }
    }
}

impl IssueCommit {
    /// D33 removed `CommitStatus::Approved` from storage; the wire keeps it and it is
    /// re-injected here from the round's `state`, where it cannot desync (D28.2).
    fn with_approval(commit: &crate::IssueCommit, approved: Option<&gix::ObjectId>) -> Self {
        let mut wire = Self::from(commit);
        if approved == Some(&commit.hash) {
            wire.statuses.push(CommitStatusEnum::Approved);
        }
        wire
    }
}

/// Commit status enum values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatusEnum {
    Initial,
    Notification,
    Approved,
    Reviewed,
}

impl From<&crate::CommitStatus> for CommitStatusEnum {
    fn from(status: &crate::CommitStatus) -> Self {
        match status {
            crate::CommitStatus::Initial => CommitStatusEnum::Initial,
            crate::CommitStatus::Notification => CommitStatusEnum::Notification,
            crate::CommitStatus::Reviewed => CommitStatusEnum::Reviewed,
        }
    }
}

/// Checklist completion summary.
#[derive(Debug, Clone, Serialize)]
pub struct ChecklistSummary {
    pub completed: u32,
    pub total: u32,
    pub percentage: f32,
}

impl From<crate::ChecklistSummary> for ChecklistSummary {
    /// Derived server-side on purpose (D28.2): the alternative is the frontend
    /// parsing markdown checkboxes.
    fn from(summary: crate::ChecklistSummary) -> Self {
        Self {
            completed: summary.completed as u32,
            total: summary.total as u32,
            // Fraction, not percent — unchanged from the pre-rounds wire shape.
            percentage: if summary.total == 0 {
                0.0
            } else {
                summary.completed as f32 / summary.total as f32
            },
        }
    }
}

/// A gap's wire shape. Both positions — a round's `preceding_gap` and the thread's
/// `drift` — share this one shape (D30); the field name carries the meaning.
#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    /// newest-first; `[]` is normal and meaningful (the D8 overlap case).
    pub commits: Vec<IssueCommit>,
    /// The anchoring approval is not in this branch's ancestry (D22/D31).
    pub divergent: bool,
    /// D35: the hash S1 reports in `ChangesAfterApproval`. The **only** legal source
    /// for it — a client-side rescan of `commits` violates U7.
    pub newest_file_change: Option<String>,
}

impl From<&crate::Gap> for Gap {
    fn from(gap: &crate::Gap) -> Self {
        Self {
            commits: gap.commits.iter().map(IssueCommit::from).collect(),
            divergent: gap.divergent,
            newest_file_change: gap.newest_file_change().map(|c| c.to_string()),
        }
    }
}

/// A round's state on the wire (D36): a **tagged union**, so `{kind:'open'}`
/// carrying an approval is inexpressible — mirroring M3's guarantee rather than
/// undoing it. This is the sole encoding of approvedness, and it carries
/// `comment_id` for the approval-comment deep-link (O2/U8).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoundStateWire {
    Open,
    Approved {
        commit: String,
        /// D44: `null` when the fold could not determine the declaring comment's id.
        /// The UI omits U8's deep-link rather than linking to comment `0`.
        comment_id: Option<u64>,
    },
    Superseded,
}

impl From<crate::DerivedState<'_>> for RoundStateWire {
    fn from(state: crate::DerivedState<'_>) -> Self {
        match state {
            crate::DerivedState::Open => RoundStateWire::Open,
            crate::DerivedState::Approved(approval) => RoundStateWire::Approved {
                commit: approval.commit.to_string(),
                comment_id: approval.comment_id,
            },
            crate::DerivedState::Superseded => RoundStateWire::Superseded,
        }
    }
}

/// Whether the round could be placed on its branch (D53). `Unplaceable` needs no
/// branch of its own on the wire: `RoundInfo.branch` already carries it, and D24 forbids
/// a second copy that could disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundPlacementWire {
    Placed,
    /// The declared start commit could not be resolved on `branch` — usually because it
    /// is not fetched locally. The round still exists and keeps its declared index.
    Unplaceable,
}

impl From<&crate::issue::RoundPlacement> for RoundPlacementWire {
    fn from(placement: &crate::issue::RoundPlacement) -> Self {
        match placement {
            crate::issue::RoundPlacement::Placed => RoundPlacementWire::Placed,
            crate::issue::RoundPlacement::Unplaceable { .. } => RoundPlacementWire::Unplaceable,
        }
    }
}

/// One QC round (A4). The status card reads `drift`; the round switcher reads
/// `rounds[]`.
#[derive(Debug, Clone, Serialize)]
pub struct RoundInfo {
    /// The **declared** round number (D53.2) — never a re-indexed position, so it always
    /// matches the `# QC Round N` comment.
    pub index: u32,
    pub branch: String,
    /// D56: `true` when the round comment declared no `git branch:` and inherited this
    /// one. Surfaced wherever the round is viewed — a silent inherited branch can
    /// mis-scope both the round's and its gap's walk (D7/D9).
    pub branch_inherited: bool,
    /// D53: `unplaceable` ⇒ `commits` and `preceding_gap` are empty and
    /// `archive_commit` is `null`; the remedy is to fetch `branch`.
    pub placement: RoundPlacementWire,
    pub start_commit: String,
    /// The sole encoding of approvedness (D36).
    pub state: RoundStateWire,
    /// "" when the round has no checklist.
    pub checklist_name: String,
    /// Excludes its `# ` heading line (D37).
    pub checklist_content: String,
    pub checklist_summary: ChecklistSummary,
    pub commits: Vec<IssueCommit>,
    /// The one `Gap` shape, not inlined fields (D30).
    pub preceding_gap: Gap,
    /// `Round::latest_commit().hash` (M8) — this **names** that function and never
    /// restates its priority rule; the two copies had already drifted once.
    ///
    /// D54: `null` in exactly that function's two `None` cases — the round is
    /// unplaceable, or it is approved and its approval is no longer among the commits it
    /// owns. A substitute here would let an archive claim approved content over content
    /// that was never approved (D55), so there is none.
    pub archive_commit: Option<String>,
    /// Committed changes only (R11). Conservatively `true` when any gap in the thread
    /// is divergent, because segment order then no longer implies commit order
    /// (D39.3) — the safe direction for an audit artefact.
    pub subsequent_file_changes: bool,
}

impl RoundInfo {
    fn new(thread: &IssueThread, position: usize) -> Self {
        let round = &thread.rounds[position];
        let summary = round.checklist.summary();
        let archive_commit = round.latest_commit().map(|commit| commit.hash);

        // D39.3: a divergent gap's no-`stop_at` walk can hold commits older than the
        // round before it, so W3's newest→oldest ordering is unsound.
        let divergent_anywhere = thread.drift.divergent
            || thread
                .rounds
                .iter()
                .any(|round| round.preceding_gap.divergent);

        Self {
            index: round.index,
            branch: round.branch.clone(),
            branch_inherited: round.branch_inherited,
            placement: (&round.placement).into(),
            start_commit: round.start_commit.to_string(),
            state: thread.round_state(position).into(),
            checklist_name: round.checklist.name.clone(),
            checklist_content: round.checklist.content.clone(),
            checklist_summary: summary.into(),
            commits: round
                .commits
                .iter()
                // D28.2: `CommitStatus::Approved` is re-injected here from `state`,
                // the one source it can be derived from.
                .map(|commit| IssueCommit::with_approval(commit, round.approved_commit()))
                .collect(),
            preceding_gap: (&round.preceding_gap).into(),
            archive_commit: archive_commit.map(|commit| commit.to_string()),
            // D54/D39.3: with no resolved commit there is nothing to measure "after",
            // so the answer is conservatively `true` — for an audit surface the safe
            // direction is to claim changes may exist, never the reverse.
            subsequent_file_changes: match archive_commit {
                Some(archive_commit) => {
                    divergent_anywhere || !thread.file_commits_after(&archive_commit).is_empty()
                }
                None => true,
            },
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

/// Which kind of segment a `SegmentRef` points at (M2).
///
/// `Drift` is a distinct kind even though it is the same `Gap` type as a preceding gap:
/// D30 — what a trailing gap *is* comes from its position, and the client picks the
/// pinned tail block (D80/D81) by kind, not by index arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Round,
    Gap,
    Drift,
}

/// One row of the History dropdown (M2): a **pointer** into `rounds`/`drift`, never a
/// copy of their commits.
///
/// The order of `IssueStatusResponse::history` is W6's, projected from
/// `IssueThread::segments()`. It carries two positional suppression rules — round 1's
/// preceding gap is skipped, and `drift` is emitted only when the latest round is closed
/// — and rebuilding those client-side is the derivation D30/U7 pushes to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SegmentRef {
    pub kind: SegmentKind,
    /// The round this segment belongs to: itself for a round, the round it precedes for
    /// a gap, the latest round for drift. A **declared** index (D53.2), so it is never a
    /// position in `history` or in `rounds`.
    pub round_index: u32,
}

impl From<&crate::issue::Segment<'_>> for SegmentRef {
    fn from(segment: &crate::issue::Segment<'_>) -> Self {
        Self {
            kind: match segment {
                crate::issue::Segment::Round(_) => SegmentKind::Round,
                crate::issue::Segment::Gap { .. } => SegmentKind::Gap,
                crate::issue::Segment::Drift { .. } => SegmentKind::Drift,
            },
            round_index: segment.round().index,
        }
    }
}

/// Full issue status response.
///
/// D24/A2/A3: no top-level field may duplicate a round-scoped value. `commits`,
/// `branch` and `checklist_summary` are gone — consumers read
/// `rounds[rounds.length - 1]` (list indexing, not derivation) and `drift`. What
/// stays at top level is exactly what is *not* per-round.
#[derive(Debug, Clone, Serialize)]
pub struct IssueStatusResponse {
    pub issue: Issue,
    pub qc_status: QCStatus,
    pub dirty: bool,
    /// Non-empty (I1); `rounds[0]` comes from the issue body.
    pub rounds: Vec<RoundInfo>,
    /// Always present; empty when the latest round is unapproved (D17). The frontend
    /// checks `rounds[rounds.length - 1].state.kind` to know whether it is
    /// meaningful — the same dispatch the backend uses (S0).
    pub drift: Gap,
    /// W6's segment order (M2) — the History dropdown's rows. Always non-empty: I1
    /// guarantees a round, and a lone open round projects to exactly `[Round 1]`.
    pub history: Vec<SegmentRef>,
    pub blocking_qc_status: BlockingQCStatus,
}

impl IssueStatusResponse {
    /// D55: fallible. When the latest round has no resolvable commit the endpoint owes
    /// the client an `IssueStatusError { kind: branch_not_local, branch }` for that
    /// issue, not a `QCStatus` it had to invent.
    pub fn new(
        issue: &octocrab::models::issues::Issue,
        issue_thread: &IssueThread,
        dirty_files: &[PathBuf],
    ) -> Result<Self, crate::IssueError> {
        Ok(Self {
            dirty: dirty_files.contains(&PathBuf::from(&issue.title)),
            issue: issue.clone().into(),
            qc_status: issue_thread.try_into()?,
            rounds: (0..issue_thread.rounds.len())
                .map(|position| RoundInfo::new(issue_thread, position))
                .collect(),
            drift: (&issue_thread.drift).into(),
            history: issue_thread
                .segments()
                .iter()
                .map(SegmentRef::from)
                .collect(),
            blocking_qc_status: BlockingQCStatus::default(),
        })
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

/// Response for starting a new QC round (A5).
#[derive(Debug, Serialize)]
pub struct CreateRoundResponse {
    /// 1-based index of the round just created — always `rounds.len()` after the fold
    /// re-reads the thread (D29/I2).
    pub round_index: u32,
    pub comment_url: String,
    /// D45: whether the D6 re-open succeeded. Still non-fatal — the round comment is
    /// already posted and is what the fold reads — but never silent: a failed re-open
    /// leaves the issue closed with an unapproved latest round, which S3 reads as
    /// `ApprovalRequired`. A round that was just created reporting "approval required"
    /// must be visible, not logged.
    pub reopened: bool,
    /// D45: the optional D5 notification's outcome. A bare `notification_url: null`
    /// conflated "not requested" with "requested but failed"; the client can now offer
    /// to post it manually.
    pub notification: NotificationOutcome,
}

/// The outcome of the optional `# QC Notification` comment (D5/D45).
///
/// A tagged union for the same reason `RoundStateWire` is one (D36): one encoding, and
/// "requested but failed" is not spelled the same way as "not requested".
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationOutcome {
    /// The caller did not ask for a notification.
    NotRequested,
    Posted {
        url: String,
    },
    /// Requested, attempted, and failed. Non-fatal: the round comment is already on the
    /// issue (D5), so the round exists either way.
    Failed {
        error: String,
    },
}

/// Response for issue unapproval.
#[derive(Debug, Serialize)]
pub struct UnapprovalResponse {
    pub unapproval_url: String,
    pub opened: bool,
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

/// One file the archive omits, echoed back to the client (D62).
#[derive(Debug, Serialize)]
pub struct SkippedFileResponse {
    pub repository_file: PathBuf,
    pub round: u32,
    pub branch: String,
    pub reason: String,
}

/// Response for archive generation.
#[derive(Debug, Serialize)]
pub struct ArchiveGenerateResponse {
    pub output_path: String,
    /// D62: what the archive recorded as skipped, so the client can confirm its
    /// declaration was written into the manifest.
    pub skipped: Vec<SkippedFileResponse>,
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
    use crate::{Approval, Round, RoundChecklist, RoundState, issue::RoundPlacement};
    use std::collections::HashSet;
    use std::str::FromStr;

    fn oid(n: u64) -> ObjectId {
        ObjectId::from_str(&format!("{:040x}", n)).unwrap()
    }

    fn commit(n: u64, file_changed: bool, statuses: &[crate::CommitStatus]) -> crate::IssueCommit {
        crate::IssueCommit {
            hash: oid(n),
            message: format!("c{n}"),
            statuses: statuses.iter().cloned().collect::<HashSet<_>>(),
            file_changed,
        }
    }

    fn round(index: u32, start: u64, commits: Vec<crate::IssueCommit>, state: RoundState) -> Round {
        Round {
            index,
            branch: "main".to_string(),
            branch_inherited: false,
            placement: RoundPlacement::Placed,
            start_commit: oid(start),
            preceding_gap: crate::Gap::default(),
            checklist: RoundChecklist {
                name: format!("Round {index} checklist"),
                content: "- [x] done\n- [ ] todo\n".to_string(),
            },
            commits,
            state,
        }
    }

    fn issue() -> octocrab::models::issues::Issue {
        crate::test_utils::create_test_issue(
            "owner",
            "repo",
            7,
            "src/main.rs",
            "## Metadata\n* initial qc commit: {}\n* git branch: main\n",
            Some(1),
            "open",
        )
    }

    fn response(thread: &IssueThread) -> serde_json::Value {
        serde_json::to_value(IssueStatusResponse::new(&issue(), thread, &[]).unwrap()).unwrap()
    }

    /// Three rounds, one per `RoundStateWire` variant: approved, superseded (D21 —
    /// unapproved with a later round), open (unapproved and last).
    fn three_round_thread() -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1.0".to_string(),
            open: true,
            blocking_qcs: Vec::new(),
            rounds: vec![
                round(
                    1,
                    1,
                    vec![
                        commit(2, true, &[]),
                        commit(1, false, &[crate::CommitStatus::Initial]),
                    ],
                    RoundState::Approved(Approval {
                        commit: oid(2),
                        comment_id: Some(4242),
                    }),
                ),
                round(
                    2,
                    3,
                    vec![commit(3, true, &[crate::CommitStatus::Initial])],
                    RoundState::Unapproved,
                ),
                round(
                    3,
                    4,
                    vec![commit(4, true, &[crate::CommitStatus::Initial])],
                    RoundState::Unapproved,
                ),
            ],
            // I14: the latest round is unapproved, so there is no anchor and no drift.
            drift: crate::Gap::default(),
        }
    }

    // ── M2: the `history` projection ────────────────────────────────────────────

    fn history(value: &serde_json::Value) -> Vec<(String, u64)> {
        value["history"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["kind"].as_str().unwrap().to_string(),
                    entry["round_index"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    /// W6 order, on the wire. Round 1's preceding gap is suppressed (W6.1) and each gap
    /// names the round it **precedes**, so the client never infers ownership from a
    /// position.
    #[test]
    fn test_history_projects_w6_order_with_gaps_naming_the_round_they_precede() {
        let value = response(&three_round_thread());

        assert_eq!(
            history(&value),
            vec![
                ("round".to_string(), 1),
                ("gap".to_string(), 2),
                ("round".to_string(), 2),
                ("gap".to_string(), 3),
                ("round".to_string(), 3),
            ],
            "R1 (G R)* with no leading gap and no trailing drift"
        );
    }

    /// W6.2: `drift` is emitted only when the latest round is closed. An open latest
    /// round has no trailing segment, so the tail block (D81) is the round itself.
    #[test]
    fn test_history_omits_drift_while_the_latest_round_is_open() {
        let value = response(&three_round_thread());

        assert!(
            !history(&value).iter().any(|(kind, _)| kind == "drift"),
            "an open latest round has no trailing segment: {:?}",
            history(&value)
        );
    }

    /// The other half of W6.2: once the latest round is approved, drift is the tail and
    /// it names the latest round (D81).
    #[test]
    fn test_history_appends_drift_once_the_latest_round_is_approved() {
        let mut thread = three_round_thread();
        thread.rounds[2].state = RoundState::Approved(Approval {
            commit: oid(4),
            comment_id: Some(9),
        });

        let value = response(&thread);
        assert_eq!(
            history(&value).last(),
            Some(&("drift".to_string(), 3)),
            "drift is the tail and belongs to the latest round: {:?}",
            history(&value)
        );
    }

    /// D53.2: `round_index` is the **declared** index. With a hole in `rounds` a gap
    /// still names the round it precedes, so no index may be read off a position — in
    /// `history` or in `rounds`.
    #[test]
    fn test_history_uses_declared_indices_when_rounds_have_a_hole() {
        let mut thread = three_round_thread();
        // Round 2's declaration was malformed and dropped without renumbering, so
        // `rounds` is [1, 3] and position 1 holds round *3*.
        thread.rounds.remove(1);

        let value = response(&thread);
        assert_eq!(
            history(&value),
            vec![
                ("round".to_string(), 1),
                ("gap".to_string(), 3),
                ("round".to_string(), 3),
            ],
            "a positional index would name the gap 2 — a round that does not exist"
        );
    }

    /// I1 guarantees a round, so `history` is never empty: the single-round case
    /// projects to exactly one row, which is also D71's default selection.
    #[test]
    fn test_history_of_a_single_open_round_is_one_row() {
        let mut thread = three_round_thread();
        thread.rounds.truncate(1);
        thread.rounds[0].state = RoundState::Unapproved;

        let value = response(&thread);
        assert_eq!(history(&value), vec![("round".to_string(), 1)]);
    }

    /// D53/D54 on the wire: an unplaceable round is **present**, with its declared
    /// index, its `placement`, and `archive_commit: null` — the UI has everything it
    /// needs to say "fetch `<branch>`" and nothing it could mistake for a commit.
    /// D56: `branch_inherited` travels with it.
    #[test]
    fn test_an_unplaceable_round_serializes_with_a_null_archive_commit() {
        let mut thread = three_round_thread();
        let second = &mut thread.rounds[1];
        second.commits.clear();
        second.branch = "feature".to_string();
        second.branch_inherited = true;
        second.placement = crate::issue::RoundPlacement::Unplaceable {
            branch: "feature".to_string(),
        };

        let value = response(&thread);
        let rounds = value["rounds"].as_array().unwrap();
        assert_eq!(rounds.len(), 3);
        assert_eq!(rounds[1]["index"], 2);
        assert_eq!(rounds[1]["placement"], "unplaceable");
        assert_eq!(rounds[1]["branch"], "feature");
        assert_eq!(rounds[1]["branch_inherited"], true);
        assert_eq!(rounds[1]["archive_commit"], serde_json::Value::Null);
        assert!(rounds[1]["commits"].as_array().unwrap().is_empty());
        // D39.3's direction: with no resolved commit, "changes after it" is claimed,
        // never denied.
        assert_eq!(rounds[1]["subsequent_file_changes"], true);

        // The placed rounds are untouched and still carry a hash.
        assert_eq!(rounds[0]["placement"], "placed");
        assert_eq!(rounds[0]["branch_inherited"], false);
        assert!(rounds[0]["archive_commit"].is_string());
    }

    /// D55: the status endpoint owes the client an error for an issue whose latest round
    /// has no resolvable commit — not a `QCStatus` it had to invent. `IssueError::
    /// LocalBranchNotFound` is what `classify_issue_error` turns into
    /// `{ kind: branch_not_local, branch }`.
    #[test]
    fn test_status_response_refuses_an_unplaceable_latest_round() {
        let mut thread = three_round_thread();
        let latest = thread.rounds.last_mut().unwrap();
        latest.commits.clear();
        latest.branch = "feature".to_string();
        latest.placement = crate::issue::RoundPlacement::Unplaceable {
            branch: "feature".to_string(),
        };

        match IssueStatusResponse::new(&issue(), &thread, &[]) {
            Err(crate::IssueError::LocalBranchNotFound(branch)) => {
                assert_eq!(branch, "feature");
            }
            other => panic!("expected LocalBranchNotFound, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn test_round_state_serializes_as_a_tagged_union() {
        let value = response(&three_round_thread());
        let rounds = value["rounds"].as_array().unwrap();
        assert_eq!(rounds.len(), 3);

        // D36: the approval travels inside the tag, so `{kind:'open'}` carrying an
        // approval is inexpressible on the wire.
        assert_eq!(
            rounds[0]["state"],
            serde_json::json!({
                "kind": "approved",
                "commit": oid(2).to_string(),
                "comment_id": 4242,
            })
        );
        assert_eq!(
            rounds[1]["state"],
            serde_json::json!({"kind": "superseded"})
        );
        assert_eq!(rounds[2]["state"], serde_json::json!({"kind": "open"}));

        // No second encoding of approvedness alongside the union.
        for round in rounds {
            assert!(round.get("approved_commit").is_none());
            assert!(round["state"].get("kind").is_some());
        }
    }

    #[test]
    fn test_round_info_embeds_the_one_gap_shape_and_names_latest_commit() {
        let thread = three_round_thread();
        let value = response(&thread);
        let rounds = value["rounds"].as_array().unwrap();

        // D30: `preceding_gap` is the `Gap` shape, not inlined `preceding_gap_*`.
        for (position, round) in rounds.iter().enumerate() {
            let gap = &round["preceding_gap"];
            assert!(gap["commits"].is_array());
            assert_eq!(gap["divergent"], serde_json::json!(false));
            assert!(gap.as_object().unwrap().contains_key("newest_file_change"));
            assert!(round.get("preceding_gap_commits").is_none());
            assert!(round.get("preceding_gap_divergent").is_none());

            // M8: `archive_commit` **names** `Round::latest_commit()`.
            assert_eq!(
                round["archive_commit"],
                serde_json::json!(
                    thread.rounds[position]
                        .latest_commit()
                        .unwrap()
                        .hash
                        .to_string()
                )
            );
        }

        // D28.2: `approved` is re-injected at serialization from `state`.
        let statuses = rounds[0]["commits"][0]["statuses"].as_array().unwrap();
        assert!(statuses.contains(&serde_json::json!("approved")));
        // …and only on the approved commit.
        assert!(
            !rounds[0]["commits"][1]["statuses"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("approved"))
        );
    }

    #[test]
    fn test_drift_carries_the_changes_after_approval_hash() {
        let mut thread = three_round_thread();
        // Latest round approved, with post-approval commits: the S1 shape.
        thread.rounds.pop();
        thread.rounds.pop();
        thread.drift = crate::Gap {
            commits: vec![
                commit(9, false, &[]),
                commit(8, true, &[]),
                commit(7, true, &[]),
            ],
            divergent: false,
        };

        let value = response(&thread);

        // D35/U7: the only legal source for the `ChangesAfterApproval` hash — the
        // newest file-changing drift commit, not a client-side rescan.
        assert_eq!(
            value["drift"]["newest_file_change"],
            serde_json::json!(oid(8).to_string())
        );
        assert_eq!(value["drift"]["commits"].as_array().unwrap().len(), 3);
        assert_eq!(value["drift"]["divergent"], serde_json::json!(false));
        assert_eq!(
            value["qc_status"]["status"],
            serde_json::json!("changes_after_approval")
        );

        // A round whose commits are followed by file changes says so (R11).
        assert_eq!(
            value["rounds"][0]["subsequent_file_changes"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn test_empty_drift_reports_a_null_newest_file_change() {
        let value = response(&three_round_thread());
        assert_eq!(
            value["drift"]["newest_file_change"],
            serde_json::Value::Null
        );
        assert!(value["drift"]["commits"].as_array().unwrap().is_empty());
    }

    /// D39.3: under divergence, segment order no longer implies commit order, so W3's
    /// newest→oldest walk is unsound and the audit artefact must claim "changes may
    /// exist". Both threads below have an **empty** `file_commits_after(archive_commit)`,
    /// so the flag can only come from the divergence disjunct.
    #[test]
    fn test_a_divergent_gap_forces_subsequent_file_changes_true() {
        // (a) a divergent `preceding_gap` on round 2.
        let mut thread = three_round_thread();
        thread.rounds.pop();
        thread.rounds[1].preceding_gap = crate::Gap {
            // Reached by the no-`stop_at` walk (D22); none of it touches the file, so
            // W3 alone would report "no subsequent changes" for every round.
            commits: vec![commit(5, false, &[])],
            divergent: true,
        };
        thread.rounds[1].commits = vec![commit(3, false, &[crate::CommitStatus::Initial])];

        for position in 0..thread.rounds.len() {
            let archive_commit = thread.rounds[position].latest_commit().unwrap().hash;
            assert!(
                thread.file_commits_after(&archive_commit).is_empty(),
                "round {position} must have no W3-visible changes for this test to bite"
            );
        }

        let value = response(&thread);
        let rounds = value["rounds"].as_array().unwrap();
        assert_eq!(rounds.len(), 2);
        for round in rounds {
            assert_eq!(
                round["subsequent_file_changes"],
                serde_json::json!(true),
                "a divergent gap anywhere in the thread is conservatively `true`"
            );
        }

        // (b) a divergent `drift` — the D31 force-push shape.
        let mut thread = three_round_thread();
        thread.rounds.truncate(1);
        thread.drift = crate::Gap {
            commits: vec![commit(9, false, &[])],
            divergent: true,
        };
        let archive_commit = thread.rounds[0].latest_commit().unwrap().hash;
        assert!(thread.file_commits_after(&archive_commit).is_empty());

        let value = response(&thread);
        assert_eq!(
            value["rounds"][0]["subsequent_file_changes"],
            serde_json::json!(true)
        );
        assert_eq!(value["drift"]["divergent"], serde_json::json!(true));
    }

    /// M8: `archive_commit` **names** `Round::latest_commit()` and never restates its
    /// priority rule — the two copies drifted once already. The only shape that tells
    /// the two apart: an unapproved round whose newest commit carries no status, sitting
    /// on a `Reviewed` one. `latest_commit()` skips the unlabeled tip.
    #[test]
    fn test_archive_commit_skips_an_unlabeled_newest_commit() {
        let mut thread = three_round_thread();
        thread.rounds.truncate(1);
        thread.rounds[0].state = RoundState::Unapproved;
        thread.rounds[0].commits = vec![
            commit(6, true, &[]),
            commit(5, false, &[crate::CommitStatus::Reviewed]),
            commit(4, false, &[crate::CommitStatus::Initial]),
        ];
        thread.rounds[0].start_commit = oid(4);
        // I14: unapproved latest round ⇒ no anchor, no drift.
        thread.drift = crate::Gap::default();

        let value = response(&thread);
        let round = &value["rounds"][0];

        assert_eq!(
            round["archive_commit"],
            serde_json::json!(oid(5).to_string()),
            "archive_commit must be the newest status-bearing commit"
        );
        assert_ne!(
            round["archive_commit"],
            serde_json::json!(oid(6).to_string()),
            "not `commits.first()` — that is the unlabeled tip"
        );
        // …and the flag is computed relative to that hash, so the unlabeled tip's file
        // change is subsequent to it (R11).
        assert_eq!(round["subsequent_file_changes"], serde_json::json!(true));
    }

    /// D44: an approval whose declaring comment id is unknown serializes as `null`, not
    /// as `0`. `0` is a valid-looking comment id, so the UI would render U8's deep-link
    /// pointing at a comment that is not the approval; `null` tells it to omit the link.
    #[test]
    fn test_an_unknown_approval_comment_id_serializes_as_null() {
        let mut thread = three_round_thread();
        thread.rounds[0].state = RoundState::Approved(Approval {
            commit: oid(2),
            comment_id: None,
        });

        let value = serde_json::to_value(IssueStatusResponse::new(&issue(), &thread, &[]).unwrap())
            .unwrap();

        assert_eq!(
            value["rounds"][0]["state"],
            serde_json::json!({
                "kind": "approved",
                "commit": oid(2).to_string(),
                "comment_id": serde_json::Value::Null,
            })
        );
        // Not a sentinel that a client cannot tell from a real id.
        assert_ne!(
            value["rounds"][0]["state"]["comment_id"],
            serde_json::json!(0)
        );
    }

    /// D50: `issue.branch` is retained for the rounds-less cheap paths, and D51's marker
    /// bit is what tells a consumer whether it is still current.
    #[test]
    fn test_issue_carries_the_branch_and_the_marker_bit() {
        let plain = Issue::from(issue());
        assert_eq!(plain.branch.as_deref(), Some("main"));
        assert!(
            !plain.has_qc_rounds_marker,
            "no marker ⇒ a single round ⇒ `branch` IS current (D50)"
        );

        let mut rounds_bearing = issue();
        rounds_bearing.body = Some(crate::issue::splice_qc_rounds(
            rounds_bearing.body.as_deref().unwrap_or_default(),
            &crate::issue::qc_rounds_section(2),
        ));
        let marked = Issue::from(rounds_bearing);
        assert_eq!(
            marked.branch.as_deref(),
            Some("main"),
            "still round 1's branch — and now possibly stale (D50)"
        );
        assert!(
            marked.has_qc_rounds_marker,
            "the marker's presence is the fetch hint (D51)"
        );
    }

    #[test]
    fn test_status_response_drops_the_round_scoped_duplicates() {
        let value = response(&three_round_thread());

        // A2/A3/D24: no top-level copy that can disagree with `rounds[last]`.
        let top = value.as_object().unwrap();
        for removed in ["commits", "branch", "checklist_summary"] {
            assert!(top.get(removed).is_none(), "{removed} should be gone");
        }
        assert!(value["issue"].get("checklist_name").is_none());

        // D35: `qc_status` is a verdict, not a commit carrier.
        let qc_status = value["qc_status"].as_object().unwrap();
        assert_eq!(
            qc_status.keys().cloned().collect::<Vec<_>>(),
            vec!["status".to_string(), "status_detail".to_string()]
        );
    }
}
