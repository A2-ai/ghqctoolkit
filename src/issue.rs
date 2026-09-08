use std::{collections::HashSet, fmt, path::PathBuf, str::FromStr, sync::LazyLock};

use gix::ObjectId;
use octocrab::models::{IssueState, issues::Issue};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
    cache::{DiskCache, get_issue_comments},
    git::{
        GitComment, GitCommit, GitCommitOps, GitFileOpsError, GitHubApiError, GitHubReader,
        find_or_cache_file_changes, get_commits_robust,
    },
    qc_status::{ChecklistSummary, analyze_checklist_in_text},
};

static MARKDOWN_LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());

/// Regex to extract file name and issue number from markdown links to issues
/// Pattern: [file_name](url/issues/123) - captures link text and issue number
/// Works with any host (github.com, GHE, etc.)
static BLOCKING_QC_LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\/issues\/(\d+)[^)]*\)").unwrap());

pub(crate) static HTML_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+[^>]*href\s*=\s*["']([^"']+)["'][^>]*>([^<]*)</a>"#).unwrap()
});

/// Statuses a commit can carry, parsed from the issue's comment log.
///
/// D33: `Approved` is **not** a stored status — approvedness lives in exactly one
/// place, `RoundState::Approved(_)`, and is re-injected at serialization. A stored
/// copy passed every invariant while corrupting `Round::latest_commit()` and the
/// archive commit. `Initial` **is** deliberately retained (R26): S4 reuses the
/// current algorithm verbatim and that algorithm treats an `Initial`-only commit as
/// status-bearing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CommitStatus {
    Initial,
    Notification,
    Reviewed,
}

impl fmt::Display for CommitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let self_str = match self {
            Self::Initial => "initial",
            Self::Notification => "notification",
            Self::Reviewed => "reviewed",
        };
        write!(f, "{self_str}")
    }
}

/// Relationship type for blocking QC issues
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum BlockingRelationship {
    /// A QC that was done previously on this file or a closely related one
    PreviousQC,
    /// A QC which the issue of interest is developed based on
    GatingQC,
    /// Relationship could not be determined (issue not found in child's body)
    Unknown,
}

impl fmt::Display for BlockingRelationship {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let self_str = match self {
            Self::PreviousQC => "previous QC",
            Self::GatingQC => "gating QC",
            Self::Unknown => "unknown relationship",
        };
        write!(f, "{self_str}")
    }
}

/// A blocking QC issue parsed from the issue body
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockingQC {
    /// The issue number of the blocking QC
    pub issue_number: u64,
    /// The file name associated with the blocking QC (link text)
    pub file_name: PathBuf,
    /// The relationship type (GatingQC or PreviousQC)
    pub relationship: BlockingRelationship,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IssueCommit {
    pub hash: ObjectId,
    pub message: String,
    pub statuses: HashSet<CommitStatus>,
    pub file_changed: bool,
}

/// An approval of a round, parsed from an `approved qc commit:` comment (M3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub commit: ObjectId,
    /// O2: the id of the comment that declared the approval, for U8's deep-link.
    ///
    /// D44: `None` when the comment carried no id (a cache written before
    /// `GitComment.id` existed, or an API response without one). Never `0` — a
    /// sentinel there is indistinguishable from a real comment id, so the deep-link
    /// is simply omitted instead of pointing somewhere wrong.
    pub comment_id: Option<u64>,
}

/// Stored round state. Two variants, because "superseded" is a positional fact (D27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundState {
    /// Open if this is the last round, superseded (D21) if a later round exists.
    Unapproved,
    Approved(Approval),
}

/// Derived three-state view of a round, from [`IssueThread::round_state`] (D27).
/// Never stored: `Superseded`-as-the-last-round and `Open`-as-a-non-last-round are
/// inexpressible rather than merely forbidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivedState<'a> {
    /// `Unapproved` + last round.
    Open,
    Approved(&'a Approval),
    /// `Unapproved` + a later round exists (D21).
    Superseded,
}

/// Frozen commits between one anchor and the next.
///
/// Branch is the owning round's branch (D9) — never stored, never looked up forward.
/// Round 1's gap is always empty: it has no predecessor. `segments()` suppresses it
/// by position, never by emptiness (W6).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gap {
    /// newest-first; empty is normal. **Bounds depend on position (D34):** a
    /// `preceding_gap` is exclusive at BOTH ends, `(approval_{n-1} .. start_n)`;
    /// `drift` is exclusive-lower / INCLUSIVE-upper, `(approval_last .. tip]`. S1
    /// depends on the tip being included — the newest post-approval change is very
    /// often HEAD itself.
    pub commits: Vec<IssueCommit>,
    /// The anchoring approval is not in this branch's ancestry (D22, D31).
    pub divergent: bool,
}

impl Gap {
    /// The newest file-changing commit in the gap, if any. For `drift` this is the
    /// hash S1 reports in `ChangesAfterApproval` (W2/D35).
    pub fn newest_file_change(&self) -> Option<&ObjectId> {
        self.commits
            .iter()
            .find(|commit| commit.file_changed)
            .map(|commit| &commit.hash)
    }
}

/// A round's checklist, split at its `# {name}` heading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoundChecklist {
    /// The H1 heading text. **Not** derivable — `content` excludes its heading (D37).
    /// "" when the body/comment carries no checklist H1.
    pub name: String,
    /// Everything AFTER the `# {name}` line, matching `configuration::Checklist`,
    /// whose `Display` emits `# {name}` then a blank line then `{content}`. Including
    /// the heading here would emit it twice and make F4's "second H1" rule latch onto
    /// the wrong heading in the following round (D37).
    pub content: String,
}

/// Trim whole blank lines off both ends of a checklist body.
///
/// A checklist section runs from its `# ` heading to the end of the body or comment, so
/// the content almost always opens with the blank line after the heading and closes with
/// whatever trailing newlines the comment had. Those are artefacts of where the section
/// was cut, not content.
///
/// Trims **lines**, never characters: interior blank lines separate `## ` subsections
/// (R5), and leading whitespace on the first real line is a nested checklist item.
fn trim_blank_lines(content: &str) -> &str {
    let mut start = 0usize;
    while let Some(len) = content[start..].find('\n').map(|i| i + 1) {
        if !content[start..start + len].trim().is_empty() {
            break;
        }
        start += len;
    }

    let mut end = content.len();
    loop {
        let head = &content[start..end];
        let last = head.rfind('\n').map(|i| i + 1);
        match last {
            // A blank final line: drop it *and* the newline that introduced it.
            Some(at) if head[at..].trim().is_empty() => end = start + at - 1,
            // No newline left, so `head` is the only line.
            None if head.trim().is_empty() => {
                end = start;
                break;
            }
            _ => break,
        }
    }

    &content[start..end]
}

impl RoundChecklist {
    /// Split a checklist section (`# {name}\n{rest}`) per D37.
    fn from_section(section: &str) -> Self {
        match section.split_once('\n') {
            Some((heading, content)) => Self {
                name: heading.trim_start_matches('#').trim().to_string(),
                content: trim_blank_lines(content).to_string(),
            },
            None => Self {
                name: section.trim_start_matches('#').trim().to_string(),
                content: String::new(),
            },
        }
    }

    /// Derived, never stored (M5) — a stored copy can desync from the content it
    /// describes. A round with no checklist is observable as `summary().total == 0`
    /// (D26); `analyze_checklist_in_text` counts checkboxes, so dropping the heading
    /// line per D37 does not change the count.
    pub fn summary(&self) -> ChecklistSummary {
        analyze_checklist_in_text(&self.content)
    }
}

/// Whether a round's declared start commit could be placed on its branch (D53).
///
/// A round the fold cannot place is **never dropped and never causes re-indexing**:
/// most commit resolution is local, so a branch the user has not fetched is a *local*
/// gap, not evidence that the round does not exist. The round is in the comment log, so
/// it is real, and the user's remedy is to fetch the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundPlacement {
    Placed,
    /// The declared start commit could not be resolved on `branch` — usually because
    /// the branch is not fetched locally. The round still exists and keeps its declared
    /// index. The branch is carried so every surface can name what to fetch, matching
    /// the existing `IssueStatusErrorKind::branch_not_local` idiom (D53.5).
    Unplaceable {
        branch: String,
    },
}

/// One QC review cycle. Rounds are 1-indexed; round 1 is the issue itself (D1/D2).
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    /// The **declared** round number: round 1 for the issue body, and the comment
    /// ordinal for every `# QC Round N` comment (F3). A knowing exception to D26
    /// (D29): a `&Round` detached from the `Vec` must be self-identifying.
    ///
    /// D53.2: this always matches the declared number, because an unplaceable round is
    /// retained rather than dropped-and-re-indexed. A `# QC Round 3` comment can no
    /// longer become `Round { index: 2 }`, so `Round.index` and `ArchiveQC.round` can
    /// no longer disagree with the GitHub comment log.
    pub index: u32,
    /// Each round owns its own branch (D7); round 1's comes from the issue body.
    pub branch: String,
    pub start_commit: ObjectId,
    /// D56: `true` when the round comment carried no `git branch:` key and inherited
    /// the previous round's branch. The inheritance is the sensible default and is
    /// kept, but branch is load-bearing for both this round's and its gap's commit walk
    /// (D7/D9), so a silent inherited branch can mis-scope two walks — it is surfaced
    /// wherever this round is the one being viewed.
    pub branch_inherited: bool,
    /// D53: `Unplaceable` when the declared start commit could not be resolved on
    /// `branch`. `commits` is then empty, as is any gap bounded by this round.
    pub placement: RoundPlacement,
    /// Default-empty for `index == 1` (D26/I13), and for a gap bounded by an
    /// unplaceable round (D53.3).
    pub preceding_gap: Gap,
    /// An empty checklist is `summary().total == 0`, not an `Option` (D26).
    pub checklist: RoundChecklist,
    /// newest-first, bounded by this round only, per D8. Empty for an `Unplaceable`
    /// round (D53.3).
    pub commits: Vec<IssueCommit>,
    pub state: RoundState,
}

impl Round {
    pub fn approved_commit(&self) -> Option<&ObjectId> {
        match &self.state {
            RoundState::Approved(approval) => Some(&approval.commit),
            RoundState::Unapproved => None,
        }
    }

    /// The branch to fetch when this round could not be placed (D53.5).
    pub fn unplaceable_branch(&self) -> Option<&str> {
        match &self.placement {
            RoundPlacement::Placed => None,
            RoundPlacement::Unplaceable { branch } => Some(branch),
        }
    }

    /// THE single definition of a round's representative commit (M8). The approval
    /// comes from `state` (D33), then the newest status-bearing commit. `archive_commit`
    /// names this function rather than restating its priority order (A4).
    ///
    /// **D54: `None` in exactly two cases.**
    /// 1. The round is `Unplaceable` — it owns no commits at all.
    /// 2. The round is `Approved` but `approval.commit` is **not** in `commits` — the
    ///    force-push/rebase case. There is deliberately **no fallthrough** here: the
    ///    old code searched `commits`, failed, and fell through to the newest
    ///    status-bearing commit, so a divergent thread's `archive_commit` became a
    ///    *post-approval* commit while `ArchiveQC.approved` stayed `true`. The archive
    ///    then claimed "this content was approved" over content that was not — exactly
    ///    the audit lie D15 and D28.3 exist to prevent.
    ///
    /// The fallthrough is **retained for `Unapproved` placed rounds**, which is the
    /// legitimate case it was written for.
    pub fn latest_commit(&self) -> Option<&IssueCommit> {
        if let RoundPlacement::Unplaceable { .. } = self.placement {
            // D54.1
            return None;
        }

        if let Some(approved) = self.approved_commit() {
            // D54.2: report, never substitute.
            return self.commits.iter().find(|c| c.hash == *approved);
        }

        self.commits
            .iter()
            .find(|commit| !commit.statuses.is_empty())
            .or_else(|| self.commits.first())
    }

    /// Why [`Round::latest_commit`] is `None`, as a reportable error — `None` when the
    /// commit resolves. **THE single place the two D54 cases are turned into an error**,
    /// so every surface reports the same thing.
    ///
    /// D60: the cases get *different messages* — `LocalBranchNotFound` for an
    /// unplaceable round (D54.1, the branch really is missing) and `ApprovalNotOnBranch`
    /// for an approval rewritten off a branch the user already has (D54.2) — while
    /// remaining **one** case for clients (`branch_not_local`, D60).
    pub fn unresolved_commit_error(&self) -> Option<IssueError> {
        if self.latest_commit().is_some() {
            return None;
        }
        match &self.placement {
            // D54.1
            RoundPlacement::Unplaceable { branch } => {
                Some(IssueError::LocalBranchNotFound(branch.clone()))
            }
            // D54.2
            RoundPlacement::Placed => match self.approved_commit() {
                Some(commit) => Some(IssueError::ApprovalNotOnBranch {
                    commit: *commit,
                    branch: self.branch.clone(),
                }),
                // A placed round owns at least its start commit (I4), so an unapproved
                // one always has a representative commit and never reaches here. Report
                // the branch rather than assert: a defect in the fold must not panic a
                // status read.
                None => Some(IssueError::LocalBranchNotFound(self.branch.clone())),
            },
        }
    }

    /// `matches!(state, Approved(_))` — the one spelling S0 dispatches on.
    pub fn is_closed(&self) -> bool {
        matches!(self.state, RoundState::Approved(_))
    }

    pub fn file_commits(&self) -> Vec<&ObjectId> {
        self.commits
            .iter()
            .filter(|commit| commit.file_changed)
            .map(|commit| &commit.hash)
            .collect()
    }
}

/// The derived alternating view of a thread: `R1 (G R)* S?` (W6). Used by the record
/// and by the invariant checks — not by status, archive, or the API.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment<'a> {
    Round(&'a Round),
    /// A round's preceding gap, with the round it precedes.
    Gap {
        gap: &'a Gap,
        round: &'a Round,
    },
    /// The trailing gap — same type, labelled by position (D30) — with the latest
    /// round.
    Drift {
        gap: &'a Gap,
        round: &'a Round,
    },
}

impl<'a> Segment<'a> {
    /// The segment's commits, newest-first.
    pub fn commits(&self) -> &'a [IssueCommit] {
        match self {
            Segment::Round(round) => &round.commits,
            Segment::Gap { gap, .. } | Segment::Drift { gap, .. } => &gap.commits,
        }
    }

    /// The round this segment belongs to: itself for a round, the round it precedes for
    /// a gap, the latest round for drift.
    ///
    /// M2/D9: a gap's owning round is part of its identity, not merely its position —
    /// a gap's branch *is* its owning round's branch — so the wire projection can name
    /// it without re-deriving W6's ordering.
    pub fn round(&self) -> &'a Round {
        match self {
            Segment::Round(round) | Segment::Gap { round, .. } | Segment::Drift { round, .. } => {
                round
            }
        }
    }

    /// Whether this segment's history does not continue the previous segment's
    /// (D22/D31). A round is never itself divergent — its *preceding gap* is.
    pub fn divergent(&self) -> bool {
        match self {
            Segment::Round(_) => false,
            Segment::Gap { gap, .. } | Segment::Drift { gap, .. } => gap.divergent,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IssueThread {
    pub file: PathBuf,
    pub milestone: String,
    pub(crate) open: bool,
    /// Blocking QC issues parsed from issue body
    /// Includes both Gating QC and Previous QC sections
    pub blocking_qcs: Vec<BlockingQC>,
    /// Non-empty (I1); `rounds[0]` comes from the issue body (D2).
    pub rounds: Vec<Round>,
    /// The commits after the latest approval that no round has closed yet. Empty when
    /// `latest_round()` is `Unapproved` (I14) — consumers dispatch on `RoundState`,
    /// never on emptiness (D17/S0).
    pub drift: Gap,
}

impl IssueThread {
    /// Create IssueThread from issue and pre-fetched comments (F1–F11).
    pub fn from_issue_comments(
        issue: &Issue,
        comments: &[GitComment],
        git_info: &impl GitCommitOps,
        disk_cache: Option<&DiskCache>,
    ) -> Result<Self, IssueError> {
        let file = PathBuf::from(&issue.title);
        let issue_is_open = matches!(issue.state, IssueState::Open);
        let milestone = if let Some(m) = &issue.milestone {
            m.title.to_string()
        } else {
            return Err(IssueError::MilestoneNotFound);
        };
        let body = issue.body.as_deref().unwrap_or_default();

        // ── F2: round 1 is declared by the issue body, exactly as today (D2) ─────
        let mut declarations = vec![RoundDeclaration {
            index: 1,
            partition: 0,
            branch: parse_branch_from_body(body).ok_or(IssueError::BranchNotFound)?,
            branch_inherited: false,
            start: parse_commit_from_pattern(body, "initial qc commit: ")
                .ok_or(IssueError::InitialCommitNotFound)?,
            checklist: find_checklist_start(body)
                .map(|offset| RoundChecklist::from_section(&body[offset..]))
                .unwrap_or_default(),
        }];

        // ── F3: partition the comments at `# QC Round` H1s ──────────────────────
        // Comment order is authoritative; a `round: N` that disagrees with its
        // ordinal is a warning, never an error.
        let boundaries: Vec<usize> = comments
            .iter()
            .enumerate()
            .filter(|(_, comment)| is_round_comment(&comment.body))
            .map(|(index, _)| index)
            .collect();

        let mut partitions: Vec<&[GitComment]> = Vec::new();
        let mut cursor = 0usize;
        for boundary in &boundaries {
            partitions.push(&comments[cursor..*boundary]);
            cursor = *boundary;
        }
        partitions.push(&comments[cursor..]);

        // ── F4: parse each round comment with the body's key set (D20) ──────────
        for (offset, boundary) in boundaries.iter().enumerate() {
            let comment = &comments[*boundary];
            let index = offset as u32 + 2;

            if let Some(declared) = parse_round_number(&comment.body) {
                if declared != index {
                    log::warn!(
                        "Round comment declares `round: {declared}` but is the {index}th round \
                         by comment order; comment order wins (F3)"
                    );
                }
            }

            let start = match parse_commit_from_pattern(&comment.body, "initial qc commit: ") {
                Some(start) => start,
                None => {
                    // The one comment shape that cannot become an `Unplaceable` round
                    // (D53): it declares no start commit at all, so `Round.start_commit`
                    // has no honest value and a sentinel there would lie (D44). It is
                    // dropped — but **nothing is re-indexed** (D53.1), so the rounds
                    // that follow keep their declared numbers and `Round.index` still
                    // never disagrees with the comment log.
                    log::warn!(
                        "Round {index} comment carries no `initial qc commit:`; the round cannot \
                         be declared and is dropped. The rounds after it keep their declared \
                         numbers, so round indices will skip {index}"
                    );
                    continue;
                }
            };

            // D56: a round comment missing `git branch:` inherits the previous round's
            // branch — the sensible default, kept. But branch scopes both this round's
            // and its gap's walk (D7/D9), so the inheritance is recorded and surfaced
            // wherever this round is viewed, not merely logged here.
            let declared_branch = parse_branch_from_body(&comment.body);
            let branch_inherited = declared_branch.is_none();
            let branch = declared_branch.unwrap_or_else(|| {
                let fallback = declarations
                    .last()
                    .map(|d: &RoundDeclaration| d.branch.clone())
                    .unwrap_or_default();
                log::warn!(
                    "Round {index} comment carries no `git branch:`; falling back to '{fallback}'"
                );
                fallback
            });

            declarations.push(RoundDeclaration {
                index,
                partition: offset + 1,
                branch,
                branch_inherited,
                start,
                checklist: round_comment_checklist(&comment.body),
            });
        }

        // ── F5: parse the markers, partition-locally ────────────────────────────
        // Approval and unapproval are scoped to their partition, so an unapproval
        // can never reach into another round and no stale flag can survive (D33).
        let parsed: Vec<ParsedPartition> = partitions
            .iter()
            .map(|partition| parse_partition(partition))
            .collect();
        for (index, partition) in parsed.iter().enumerate() {
            if !declarations.iter().any(|d| d.partition == index) {
                log::warn!(
                    "Dropping the statuses of an unplaceable round comment ({} referenced commits)",
                    partition.statuses.len()
                );
            }
        }

        // ── F7: one walk per distinct branch, stopping at the oldest boundary ───
        //
        // D53: a branch that is not fetched locally is a *local* gap, not a reason to
        // refuse the whole issue, so `bounded` records it instead of failing — every
        // round declared on it becomes `Unplaceable`. Round 1 is the exception below:
        // its branch comes from the body, and a thread with no placeable round 1 has no
        // starting point at all.
        let mut walker = BranchWalks::new();
        for declaration in &declarations {
            walker.bounded(
                &declaration.branch,
                oldest_full_start(&declarations, &declaration.branch),
                git_info,
                disk_cache,
            )?;
        }
        if walker.is_missing(&declarations[0].branch) {
            return Err(IssueError::LocalBranchNotFound(
                declarations[0].branch.clone(),
            ));
        }

        // Resolve every declared start commit against its own branch's walk.
        let mut rounds: Vec<PlacedRound> = Vec::new();
        for declaration in &declarations {
            let walk = walker.get(&declaration.branch);
            let start = match resolve_declared(declaration.start, walk) {
                Some(start) => start,
                None => {
                    if declaration.index == 1 {
                        return Err(IssueError::CommitNotParseable(
                            declaration.start.to_string(),
                        ));
                    }
                    // An abbreviated sha absent from the walk resolves to no
                    // `ObjectId` at all, so — like a comment with no
                    // `initial qc commit:` — it cannot be represented as
                    // `Unplaceable` (`Round.start_commit` would have no honest value).
                    // Dropped, and again with **no re-indexing** (D53.1).
                    log::warn!(
                        "Round {} start commit '{}' cannot be resolved to a commit id; the round \
                         is dropped and the round indices after it will skip {}",
                        declaration.index,
                        declaration.start,
                        declaration.index
                    );
                    continue;
                }
            };
            // D53: the declared start commit is known but is not on its branch —
            // usually because the branch is not fetched. The round is **kept**, with its
            // declared index, and carries the branch to fetch.
            let start_index = match position_of(walk, &start) {
                Some(index) => Some(index),
                None => {
                    if declaration.index == 1 {
                        // Matches the pre-rounds failure: the QC's starting point is
                        // not on the branch we can see.
                        return Err(IssueError::CommitNotFound(file));
                    }
                    log::warn!(
                        "Round {} start commit {} is not on branch '{}'; the round cannot be \
                         placed — fetch '{}' to resolve it",
                        declaration.index,
                        start,
                        declaration.branch,
                        declaration.branch
                    );
                    None
                }
            };

            let statuses = parsed
                .get(declaration.partition)
                .cloned()
                .unwrap_or_default();
            let state = match statuses.approval {
                Some((sha, comment_id)) => match resolve_declared(sha, walk) {
                    Some(commit) => RoundState::Approved(Approval { commit, comment_id }),
                    None => {
                        log::warn!(
                            "Round {} approval commit '{sha}' is not parseable; treating the \
                             round as unapproved",
                            declaration.index
                        );
                        RoundState::Unapproved
                    }
                },
                None => RoundState::Unapproved,
            };

            rounds.push(PlacedRound {
                index: declaration.index,
                branch: declaration.branch.clone(),
                branch_inherited: declaration.branch_inherited,
                start,
                start_index,
                checklist: declaration.checklist.clone(),
                state,
                statuses: statuses.statuses,
            });
        }

        // ── D22/D31: divergence needs a second, unbounded walk of that branch ───
        let last = rounds.len().saturating_sub(1);
        let mut divergent_gaps: Vec<bool> = vec![false; rounds.len()];
        let mut divergent_drift = false;
        for position in 0..rounds.len() {
            if position > 0 {
                // D53.3: a gap bounded by an unplaceable round is empty, so there is
                // nothing to call divergent — and no reason to pay for an unbounded
                // walk of a branch whose anchor we could not find anyway.
                if rounds[position].start_index.is_none() || !rounds[position - 1].is_placed() {
                    continue;
                }
                let previous_approval = rounds[position - 1].state.clone();
                if let RoundState::Approved(approval) = previous_approval {
                    let walk = walker.get(&rounds[position].branch);
                    let anchor = position_of(walk, &approval.commit);
                    // Not an ancestor of this round's start ⇒ divergent (I6). The
                    // D8 overlap case `approval_{n-1} == start_n` is not divergence.
                    divergent_gaps[position] = match anchor {
                        Some(index) => index < rounds[position].start_index.unwrap_or(index),
                        None => true,
                    };
                }
            }
        }
        if let Some(round) = rounds.last() {
            // D53.3 again: an unplaceable latest round bounds an empty drift.
            if let (RoundState::Approved(approval), true) = (&round.state, round.is_placed()) {
                let walk = walker.get(&round.branch);
                divergent_drift = position_of(walk, &approval.commit).is_none();
            }
        }
        for (position, divergent) in divergent_gaps.iter().enumerate() {
            if *divergent {
                walker.full(&rounds[position].branch, git_info, disk_cache)?;
            }
        }
        if divergent_drift {
            walker.full(&rounds[last].branch, git_info, disk_cache)?;
        }

        // ── F8: one file-change lookup per (branch, path), old paths included ───
        let old_paths: Vec<PathBuf> = parse_file_history(body)
            .into_iter()
            .map(|event| PathBuf::from(event.old_path))
            .collect();
        let file_changes = walker.file_changes(git_info, &file, &old_paths, disk_cache)?;

        // ── F9: assign commits by the D8 tables ─────────────────────────────────
        let mut built: Vec<Round> = Vec::new();
        for position in 0..rounds.len() {
            let round = &rounds[position];

            // D53: an unplaceable round owns no commits and bounds an empty gap. It is
            // still a round: it keeps its declared index, its checklist, its state and
            // the branch to fetch.
            let start_index = match round.start_index {
                Some(index) => index,
                None => {
                    built.push(Round {
                        index: round.index,
                        branch: round.branch.clone(),
                        branch_inherited: round.branch_inherited,
                        placement: RoundPlacement::Unplaceable {
                            branch: round.branch.clone(),
                        },
                        start_commit: round.start,
                        preceding_gap: Gap::default(),
                        checklist: round.checklist.clone(),
                        commits: Vec::new(),
                        state: round.state.clone(),
                    });
                    continue;
                }
            };
            let walk = walker.get(&round.branch);
            let changed = &file_changes[&round.branch];

            let newest = match &round.state {
                // `[start_n ..= approval_n]`
                RoundState::Approved(approval) => match position_of(walk, &approval.commit) {
                    Some(index) if index <= start_index => index,
                    _ => {
                        // The approval was rewritten off its branch (D31): the upper
                        // bound is undefined, so fall back to the branch tip.
                        log::warn!(
                            "Round {} approval {} is not in branch '{}' at or after its start; \
                             bounding the round at the branch tip",
                            round.index,
                            approval.commit,
                            round.branch
                        );
                        0
                    }
                },
                RoundState::Unapproved if position == last => 0, // `[start_n .. tip]`
                RoundState::Unapproved => {
                    // Superseded: `[start_n .. start_{n+1})`, or to the tip when
                    // `start_{n+1}` is not on this branch (D38).
                    let next = &rounds[position + 1];
                    match position_of(walk, &next.start) {
                        Some(index) if index < start_index => index + 1,
                        Some(_) => {
                            // Degenerate: `start_{n+1}` is at or older than
                            // `start_n`, so `[start_n .. start_{n+1})` is empty and
                            // the round collapses to its start commit. D32 is
                            // log-and-flag, not silently cope (D43).
                            log::warn!(
                                "Round {} is superseded by a round starting at {}, which is not \
                                 newer than its own start {} on branch '{}'; the round is bounded \
                                 at its start commit",
                                round.index,
                                next.start,
                                round.start,
                                round.branch
                            );
                            start_index
                        }
                        None => {
                            // D38: `start_{n+1}` is not on `branch_n`, which D7
                            // permits, so the round owns `[start_n .. tip(branch_n)]`.
                            log::warn!(
                                "Round {}'s successor starts at {}, which is not on branch '{}'; \
                                 bounding the superseded round at the branch tip (D38)",
                                round.index,
                                next.start,
                                round.branch
                            );
                            0
                        }
                    }
                }
            };
            let newest = newest.min(start_index);

            let mut commits = materialize(&walk[newest..=start_index], changed, &round.statuses);
            // I4: exactly one Initial per round, and it is the start commit (D13).
            if let Some(start) = commits.last_mut() {
                start.statuses.insert(CommitStatus::Initial);
            }

            // `R[n].preceding_gap` = `(approval_{n-1} .. start_n)`, exclusive both
            // ends (D8/D34), on this round's branch (D9).
            let preceding_gap = if position == 0 {
                Gap::default()
            } else if divergent_gaps[position] {
                // D22: a divergent gap is walked with no `stop_at` — the full
                // duration of its owning round's branch.
                let full = walker.get_full(&round.branch);
                let from = position_of(full, &round.start).map(|index| index + 1);
                Gap {
                    commits: from
                        .map(|from| materialize(&full[from..], changed, &round.statuses))
                        .unwrap_or_default(),
                    divergent: true,
                }
            } else if !rounds[position - 1].is_placed() {
                // D53.3: a gap bounded by an unplaceable neighbour is empty — the
                // anchor it would be measured from could not be placed.
                Gap::default()
            } else {
                match rounds[position - 1].state.clone() {
                    // D21: the malformed round swallowed everything up to this
                    // round's start, so there is nothing left for the gap.
                    RoundState::Unapproved => Gap::default(),
                    RoundState::Approved(approval) => {
                        let anchor = position_of(walk, &approval.commit);
                        let commits = match anchor {
                            Some(index) if index > start_index + 1 => {
                                materialize(&walk[start_index + 1..index], changed, &round.statuses)
                            }
                            _ => Vec::new(),
                        };
                        Gap {
                            commits,
                            divergent: false,
                        }
                    }
                }
            };

            built.push(Round {
                index: round.index,
                branch: round.branch.clone(),
                branch_inherited: round.branch_inherited,
                placement: RoundPlacement::Placed,
                start_commit: round.start,
                preceding_gap,
                checklist: round.checklist.clone(),
                commits,
                state: round.state.clone(),
            });
        }

        if built.is_empty() {
            return Err(IssueError::CommitNotFound(file));
        }

        // `thread.drift` = `(approval_last .. tip]` — exclusive lower, INCLUSIVE
        // upper (D34). Empty when the latest round is unapproved (I14).
        let latest = &rounds[last];
        let drift = match &latest.state {
            RoundState::Unapproved => Gap::default(),
            // D53.3: an unplaceable latest round bounds an empty drift. Status refuses
            // rather than reading it (D55), so this empty drift can never be mistaken
            // for "nothing changed since the approval".
            _ if !latest.is_placed() => Gap::default(),
            RoundState::Approved(approval) => {
                let changed = &file_changes[&latest.branch];
                if divergent_drift {
                    // D31: the approval was force-pushed or rebased off its branch,
                    // so the drift can span the whole branch. Reporting it empty
                    // (and therefore `Approved`) is the worst available failure.
                    let full = walker.get_full(&latest.branch);
                    Gap {
                        commits: materialize(full, changed, &latest.statuses),
                        divergent: true,
                    }
                } else {
                    let walk = walker.get(&latest.branch);
                    let anchor = position_of(walk, &approval.commit).unwrap_or(0);
                    Gap {
                        commits: materialize(&walk[..anchor], changed, &latest.statuses),
                        divergent: false,
                    }
                }
            }
        };

        // F10/D14: a status referencing a commit its own round does not own is
        // dropped and logged, never applied to another round.
        for (position, placed) in rounds.iter().enumerate() {
            let round = &built[position];
            for sha in placed.statuses.keys() {
                let owned = round
                    .commits
                    .iter()
                    .chain(round.preceding_gap.commits.iter())
                    .chain(if position == last {
                        drift.commits.iter()
                    } else {
                        [].iter()
                    })
                    .any(|commit| commit_matches(&commit.hash, sha));
                if !owned {
                    log::warn!(
                        "Round {} references commit '{sha}', which it does not own; the status                          is dropped",
                        round.index
                    );
                }
            }
        }

        let thread = IssueThread {
            file,
            milestone,
            open: issue_is_open,
            blocking_qcs: parse_blocking_qcs(body),
            rounds: built,
            drift,
        };

        // ── F11: assert I1–I14; log and flag, never fail (D32) ──────────────────
        for violation in thread.invariant_violations() {
            log::warn!("QC round invariant violated: {violation}");
        }

        Ok(thread)
    }

    // TODO: order the notification commits based on commit timeline
    pub async fn from_issue(
        issue: &Issue,
        disk_cache: Option<&DiskCache>,
        git_info: &(impl GitHubReader + GitCommitOps),
    ) -> Result<Self, IssueError> {
        let comments = get_issue_comments(issue, disk_cache, git_info).await?;
        Self::from_issue_comments(issue, &comments, git_info, disk_cache)
    }

    /// W1. The round every status rule and the status card address (D10/D16).
    pub fn latest_round(&self) -> &Round {
        self.rounds.last().expect("I1: a thread has >= 1 round")
    }

    /// A round by its 1-based index (D29).
    pub fn round(&self, index: u32) -> Option<&Round> {
        self.rounds.iter().find(|round| round.index == index)
    }

    /// The branch the user's checkout must be on (W4) — the latest round's (D7).
    pub fn branch(&self) -> &str {
        &self.latest_round().branch
    }

    /// The number a newly created round must declare.
    ///
    /// D53.2 made `index` the **declared** round number rather than a position, so
    /// `rounds.len() + 1` is no longer the answer: a round comment the fold had to drop
    /// leaves a hole, and reusing its number would put two `# QC Round N` comments in the
    /// log. One past the highest index is always free, because comment order only ever
    /// increases (F3).
    pub fn next_round_index(&self) -> u32 {
        self.latest_round().index + 1
    }

    /// Round 1's start commit — the QC's original starting point (D2).
    pub fn initial_commit(&self) -> &ObjectId {
        &self.rounds[0].start_commit
    }

    /// The latest round's representative commit. Delegates to [`Round::latest_commit`],
    /// which is the single definition (M8), and is `None` in the same two cases (D54).
    pub fn latest_commit(&self) -> Option<&IssueCommit> {
        self.latest_round().latest_commit()
    }

    /// Every file-changing commit in the thread, newest-first.
    ///
    /// Walks `segments()` newest→oldest; each segment's own commits are already
    /// newest-first (D8/M4).
    pub fn file_commits(&self) -> Vec<&ObjectId> {
        self.segments()
            .into_iter()
            .rev()
            .flat_map(|segment| segment.commits().iter())
            .filter(|commit| commit.file_changed)
            .map(|commit| &commit.hash)
            .collect()
    }

    /// The file-changing commits newer than `commit`, walking `segments()`
    /// newest→oldest (W3).
    pub fn file_commits_after(&self, commit: &ObjectId) -> Vec<&ObjectId> {
        let mut newer = Vec::new();
        for segment in self.segments().into_iter().rev() {
            for candidate in segment.commits() {
                if candidate.hash == *commit {
                    return newer;
                }
                if candidate.file_changed {
                    newer.push(&candidate.hash);
                }
            }
        }
        newer
    }

    /// The sum over every round's checklist. The record wants this — reading the body
    /// alone would silently miss rounds 2..n (R8/R13).
    pub fn checklist_summary_all_rounds(&self) -> ChecklistSummary {
        let summaries: Vec<ChecklistSummary> = self
            .rounds
            .iter()
            .map(|round| round.checklist.summary())
            .collect();
        ChecklistSummary::sum(summaries.iter())
    }

    /// D27: derive open-vs-superseded from position. The fold stores neither.
    pub fn round_state(&self, index: usize) -> DerivedState<'_> {
        match &self.rounds[index].state {
            RoundState::Approved(approval) => DerivedState::Approved(approval),
            RoundState::Unapproved if index + 1 == self.rounds.len() => DerivedState::Open,
            RoundState::Unapproved => DerivedState::Superseded,
        }
    }

    /// The derived alternating view, `R1 (G R)* S?` (W6). Both suppression rules are
    /// **positional**, never emptiness rules: an empty `G2` is meaningful (it is the
    /// D8 overlap case) and must still appear in the record.
    pub fn segments(&self) -> Vec<Segment<'_>> {
        let mut segments = Vec::new();
        for (position, round) in self.rounds.iter().enumerate() {
            // W6.1: round 1 has no predecessor, so emitting its gap would prepend a
            // `G` before `R1`.
            if position > 0 {
                segments.push(Segment::Gap {
                    gap: &round.preceding_gap,
                    round,
                });
            }
            segments.push(Segment::Round(round));
        }
        // W6.2: R1's "no trailing segment after an open round", enforced here rather
        // than in storage (D17).
        if let Some(latest) = self.rounds.last().filter(|round| round.is_closed()) {
            segments.push(Segment::Drift {
                gap: &self.drift,
                round: latest,
            });
        }
        segments
    }

    /// I1–I14. Every one is **log-and-flag, never fatal** (D32): these run against an
    /// append-only comment log and a git history users can rewrite, so a fatal
    /// assertion would turn someone else's force-push into a tool that refuses to
    /// read a real issue. I6's violation is *represented* rather than reported — it
    /// sets `preceding_gap.divergent` (D22).
    pub(crate) fn invariant_violations(&self) -> Vec<String> {
        let mut violations = Vec::new();

        // I1
        if self.rounds.is_empty() {
            violations.push("I1: a thread must have at least one round".to_string());
            return violations;
        }

        for (position, round) in self.rounds.iter().enumerate() {
            // I2, restated for D53: `index` is the **declared** round number, and a
            // declaration the fold could not even parse a start commit for is dropped
            // without re-indexing, which leaves a hole rather than a lie. So the
            // checkable form is monotonic-and-never-below-position, not equality: an
            // index *smaller* than `position + 1` would mean rounds were renumbered,
            // the audit divergence D53.2 exists to prevent.
            if round.index < position as u32 + 1 {
                violations.push(format!(
                    "I2: round at position {position} carries index {}, which is below its \
                     position — rounds must never be re-indexed",
                    round.index
                ));
            }
            if position > 0 && round.index <= self.rounds[position - 1].index {
                violations.push(format!(
                    "I2: round index {} does not increase on the preceding round's {}",
                    round.index,
                    self.rounds[position - 1].index
                ));
            }

            // I4, scoped to `Placed` rounds (D53.4): an `Unplaceable` round owns no
            // commits, so it has no `Initial` commit to assert.
            if round.placement == RoundPlacement::Placed {
                let initials: Vec<&IssueCommit> = round
                    .commits
                    .iter()
                    .filter(|commit| commit.statuses.contains(&CommitStatus::Initial))
                    .collect();
                if initials.len() != 1 {
                    violations.push(format!(
                        "I4: round {} carries {} Initial commits, expected exactly one",
                        round.index,
                        initials.len()
                    ));
                } else if initials[0].hash != round.start_commit {
                    violations.push(format!(
                        "I4: round {}'s Initial commit {} is not its start commit {}",
                        round.index, initials[0].hash, round.start_commit
                    ));
                }
            }

            // I5, scoped to `!drift.divergent` (D39): a divergent drift means the
            // approval was rewritten off its branch and cannot appear in a walk of it.
            // Scoped to `Placed` for the same reason as I4 (D53.4): an unplaceable
            // round owns no commits, so its declared approval cannot be among them.
            if let RoundState::Approved(approval) = &round.state {
                if round.placement == RoundPlacement::Placed
                    && !self.drift.divergent
                    && round.commits.first().map(|c| c.hash) != Some(approval.commit)
                {
                    violations.push(format!(
                        "I5: round {}'s approval {} is not the newest commit it owns",
                        round.index, approval.commit
                    ));
                }
            }

            // I8: a preceding gap is disjoint from both bounding commits.
            if position > 0 {
                let gap = &round.preceding_gap;
                // The `start_n` half applies unconditionally (D58).
                if gap.commits.iter().any(|c| c.hash == round.start_commit) {
                    violations.push(format!(
                        "I8: round {}'s preceding gap contains its start commit",
                        round.index
                    ));
                }
                // D58: the `approval_{n-1}` half does **not** apply to a divergent gap.
                // A divergent gap is walked with no `stop_at` (D22), so it sweeps the
                // whole branch and will legitimately contain the previous approval when
                // that approval is older than `start_n`. D39 scoped I5 and I7 for
                // divergence but omitted I8; this carve-out is that omission, recorded.
                if let Some(previous) = self.rounds[position - 1].approved_commit() {
                    if !gap.divergent && gap.commits.iter().any(|c| c.hash == *previous) {
                        violations.push(format!(
                            "I8: round {}'s preceding gap contains the previous approval {previous}",
                            round.index
                        ));
                    }
                }
            }

            // I9, demoted by D40 to a logged expectation: a round comment is an
            // ordinary GitHub comment anyone may edit.
            if position > 0 && round.checklist.summary().total == 0 {
                violations.push(format!(
                    "I9: round {} carries an empty checklist",
                    round.index
                ));
            }

            // I12
            if matches!(self.round_state(position), DerivedState::Superseded)
                && !self.rounds[position + 1].preceding_gap.commits.is_empty()
            {
                violations.push(format!(
                    "I12: round {} is superseded but round {} has a non-empty preceding gap",
                    round.index,
                    self.rounds[position + 1].index
                ));
            }
        }

        // I8, drift half (D41): `drift` is `(approval_last .. tip]`, so it is
        // exclusive of `approval_last`. The *inclusive* half — that the tip is in
        // there — is not assertable here: M1 stores no branch tip and should not
        // start, so it is an F9 fold requirement guarded by direct tests on the slice.
        if let RoundState::Approved(approval) = &self.latest_round().state {
            if self
                .drift
                .commits
                .iter()
                .any(|commit| commit.hash == approval.commit)
            {
                violations.push(format!(
                    "I8: the drift contains the latest approval {}",
                    approval.commit
                ));
            }
        }

        // I13: round 1 has no predecessor to diverge from.
        let first = &self.rounds[0].preceding_gap;
        if !first.commits.is_empty() || first.divergent {
            violations.push("I13: round 1 carries a non-empty preceding gap".to_string());
        }

        // I7: no commit hash in two segments. Two exceptions: `approval_n ==
        // start_{n+1}` (D8), and any commit inside a divergent gap, whose no-`stop_at`
        // walk legitimately reaches commits older segments own (D39).
        let overlaps: HashSet<ObjectId> = self
            .rounds
            .windows(2)
            .filter_map(|pair| pair[0].approved_commit().copied())
            .collect();
        let mut seen: HashSet<ObjectId> = HashSet::new();
        for segment in self.segments() {
            let divergent = segment.divergent();
            for commit in segment.commits() {
                if divergent || overlaps.contains(&commit.hash) {
                    continue;
                }
                if !seen.insert(commit.hash) {
                    violations.push(format!(
                        "I7: commit {} appears in two segments",
                        commit.hash
                    ));
                }
            }
        }

        // I14: with no approval there is no anchor to diverge from. The converse does
        // not hold, which is why consumers dispatch on `RoundState` (D17).
        if !self.latest_round().is_closed()
            && (!self.drift.commits.is_empty() || self.drift.divergent)
        {
            violations
                .push("I14: the latest round is unapproved but the drift is non-empty".to_string());
        }

        violations
    }
}

/// A round as declared, before its commits are known.
struct RoundDeclaration<'a> {
    /// The declared round number: 1 for the body, the comment ordinal otherwise (F3).
    /// **Never re-assigned** — a dropped declaration leaves a hole rather than shifting
    /// the rounds after it (D53.1/D53.2).
    index: u32,
    /// Which comment partition declares this round (F3). Carried so that dropping an
    /// unplaceable declaration cannot misalign the partitions that follow it.
    partition: usize,
    branch: String,
    /// D56: the branch was inherited from the previous round, not declared.
    branch_inherited: bool,
    /// The declared sha, possibly abbreviated. Declared-vs-observed is not
    /// duplication (D28.1) — the comparison is the validation.
    start: &'a str,
    checklist: RoundChecklist,
}

/// A round resolved against its branch, before its commits are materialized.
struct PlacedRound<'a> {
    index: u32,
    branch: String,
    branch_inherited: bool,
    start: ObjectId,
    /// The start commit's position in its branch's walk, or `None` when the commit
    /// could not be found there — the `RoundPlacement::Unplaceable` case (D53).
    start_index: Option<usize>,
    checklist: RoundChecklist,
    state: RoundState,
    statuses: CommitStatusMap<'a>,
}

impl PlacedRound<'_> {
    fn is_placed(&self) -> bool {
        self.start_index.is_some()
    }
}

type CommitStatusMap<'a> = std::collections::HashMap<&'a str, HashSet<CommitStatus>>;

/// The markers of one comment partition (F5).
#[derive(Debug, Clone, Default)]
struct ParsedPartition<'a> {
    statuses: CommitStatusMap<'a>,
    /// The surviving approval sha and the id of the comment that declared it, if any:
    /// an unapproval in this partition clears both. The comment id feeds
    /// `Approval::comment_id`, which U8 deep-links (D36); it is itself optional
    /// (D44), so "no approval" and "approval whose comment id is unknown" stay
    /// distinct.
    approval: Option<(&'a str, Option<u64>)>,
}

/// True when a comment declares a new round (D3). Detection keys off the
/// `# QC Round` H1, never off a metadata key (D20).
fn is_round_comment(body: &str) -> bool {
    body.lines().any(|line| line.starts_with("# QC Round"))
}

/// `round: {N}` — the one metadata key a round comment adds over the body (D20).
fn parse_round_number(body: &str) -> Option<u32> {
    parse_commit_from_pattern(body, "round: ")?.parse().ok()
}

/// F4: a round comment's checklist section runs from the **second** H1 (the first is
/// `# QC Round N`) to the end of the comment. Later H1s stay inside `content` as
/// subsections (R5).
fn round_comment_checklist(body: &str) -> RoundChecklist {
    let first = match find_checklist_start(body) {
        Some(offset) => offset,
        None => return RoundChecklist::default(),
    };
    let after_first = first
        + body[first..]
            .find('\n')
            .map(|n| n + 1)
            .unwrap_or(body.len() - first);
    match find_checklist_start(&body[after_first..]) {
        Some(offset) => RoundChecklist::from_section(&body[after_first + offset..]),
        None => RoundChecklist::default(),
    }
}

/// Parse one comment partition's markers (F5). Approval and unapproval are
/// **partition-local**: this is the core change to the old global scan, and it is why
/// an unapproval can no longer leave a stale approval behind (D33).
fn parse_partition<'a>(comments: &'a [GitComment]) -> ParsedPartition<'a> {
    let mut parsed = ParsedPartition::default();

    for comment in comments {
        // Notification: "current commit: {hash}"
        if let Some(commit) = parse_commit_from_pattern(&comment.body, "current commit: ") {
            parsed
                .statuses
                .entry(commit)
                .or_default()
                .insert(CommitStatus::Notification);
        }

        // Review: "comparing commit: {hash}" inside a "# QC Review" comment
        if comment.body.contains("# QC Review") {
            if let Some(commit) = parse_commit_from_pattern(&comment.body, "comparing commit: ") {
                parsed
                    .statuses
                    .entry(commit)
                    .or_default()
                    .insert(CommitStatus::Reviewed);
            }
        }

        // Approval: "approved qc commit: {hash}" → RoundState, not a commit flag (D33)
        if let Some(commit) = parse_commit_from_pattern(&comment.body, "approved qc commit: ") {
            parsed.approval = Some((commit, comment.id));
        }

        // Unapproval revokes this partition's approval (D6/D11).
        if comment.body.contains("# QC Un-Approval") {
            parsed.approval = None;
        }
    }

    parsed
}

/// The oldest boundary commit expected on `branch`: the start commit of the earliest
/// round declared on it, when it is a full sha (F7). An abbreviated sha cannot be a
/// `stop_at`, so the walk runs unbounded — as it did before rounds existed.
fn oldest_full_start(declarations: &[RoundDeclaration<'_>], branch: &str) -> Option<ObjectId> {
    declarations
        .iter()
        .find(|declaration| declaration.branch == branch)
        .and_then(|declaration| ObjectId::from_str(declaration.start).ok())
}

/// Resolve a declared sha (possibly abbreviated) against a branch walk. A sha that
/// parses but is absent from the walk still resolves — that is exactly the divergent
/// case D22/D31 must be able to represent.
fn resolve_declared(sha: &str, walk: &[GitCommit]) -> Option<ObjectId> {
    walk.iter()
        .find(|commit| commit_matches(&commit.commit, sha))
        .map(|commit| commit.commit)
        .or_else(|| ObjectId::from_str(sha).ok())
}

/// Exact match, or a short-sha prefix of at least 7 characters.
fn commit_matches(commit: &ObjectId, sha: &str) -> bool {
    let full = commit.to_string();
    full == sha || (sha.len() >= 7 && full.starts_with(sha))
}

fn position_of(walk: &[GitCommit], commit: &ObjectId) -> Option<usize> {
    walk.iter().position(|walked| walked.commit == *commit)
}

/// Turn a slice of walked commits into `IssueCommit`s, attaching the statuses of the
/// round's own partition (F10/D14). A status referencing a commit this round does not
/// own is dropped — see `log_dropped_statuses`.
fn materialize(
    walked: &[GitCommit],
    file_changes: &HashSet<String>,
    statuses: &CommitStatusMap<'_>,
) -> Vec<IssueCommit> {
    walked
        .iter()
        .map(|commit| {
            let full = commit.commit.to_string();
            let mut merged = HashSet::new();
            for (sha, commit_statuses) in statuses {
                if commit_matches(&commit.commit, sha) {
                    merged.extend(commit_statuses.iter().cloned());
                }
            }
            IssueCommit {
                hash: commit.commit,
                message: commit.message.clone(),
                statuses: merged,
                file_changed: file_changes.contains(&full),
            }
        })
        .collect()
}

/// The per-branch walks a fold needs: one bounded walk per branch (F7), plus an
/// unbounded one for any branch carrying a divergent gap (D22).
struct BranchWalks {
    bounded: std::collections::HashMap<String, Vec<GitCommit>>,
    full: std::collections::HashMap<String, Vec<GitCommit>>,
    /// Branches that could not be walked because they are not fetched locally. D53: a
    /// missing branch is a *local* gap, so its rounds become `Unplaceable` rather than
    /// failing the whole fold — the walk is recorded as empty and named here.
    missing: HashSet<String>,
}

impl BranchWalks {
    fn new() -> Self {
        Self {
            bounded: std::collections::HashMap::new(),
            full: std::collections::HashMap::new(),
            missing: HashSet::new(),
        }
    }

    /// D53: the branch is not fetched locally, so nothing declared on it can be placed.
    fn is_missing(&self, branch: &str) -> bool {
        self.missing.contains(branch)
    }

    fn bounded(
        &mut self,
        branch: &str,
        stop_at: Option<ObjectId>,
        git_info: &impl GitCommitOps,
        disk_cache: Option<&DiskCache>,
    ) -> Result<(), IssueError> {
        if self.bounded.contains_key(branch) {
            return Ok(());
        }
        let walk = match get_commits_robust(
            git_info,
            &Some(branch.to_string()),
            stop_at.as_ref(),
            stop_at,
            disk_cache,
        ) {
            Ok(walk) => walk,
            Err(GitFileOpsError::LocalBranchNotFound(name)) => {
                // D53: record the gap instead of failing. Every round declared on this
                // branch becomes `Unplaceable { branch }` and the user is told to fetch
                // it; a round the tool cannot see is not a round that does not exist.
                log::warn!(
                    "Branch '{name}' is not checked out locally; the rounds declared on it                      cannot be placed — fetch it to resolve them"
                );
                self.missing.insert(branch.to_string());
                Vec::new()
            }
            Err(other) => return Err(other.into()),
        };
        log::debug!("Walked {} commits on branch '{branch}'", walk.len());
        self.bounded.insert(branch.to_string(), walk);
        Ok(())
    }

    fn full(
        &mut self,
        branch: &str,
        git_info: &impl GitCommitOps,
        disk_cache: Option<&DiskCache>,
    ) -> Result<(), IssueError> {
        if self.full.contains_key(branch) {
            return Ok(());
        }
        let walk = get_commits_robust(git_info, &Some(branch.to_string()), None, None, disk_cache)?;
        log::debug!(
            "Walked {} commits on branch '{branch}' with no stop_at (divergent segment)",
            walk.len()
        );
        self.full.insert(branch.to_string(), walk);
        Ok(())
    }

    fn get(&self, branch: &str) -> &[GitCommit] {
        self.bounded
            .get(branch)
            .map(|w| w.as_slice())
            .unwrap_or(&[])
    }

    fn get_full(&self, branch: &str) -> &[GitCommit] {
        self.full
            .get(branch)
            .map(|w| w.as_slice())
            .unwrap_or_else(|| self.get(branch))
    }

    /// F8: one `find_or_cache_file_changes` per (branch, path), including the old
    /// paths recorded in `## File History` so commits against the old filename are
    /// still flagged as file-changing.
    fn file_changes(
        &self,
        git_info: &impl GitCommitOps,
        file: &std::path::Path,
        old_paths: &[PathBuf],
        disk_cache: Option<&DiskCache>,
    ) -> Result<std::collections::HashMap<String, HashSet<String>>, IssueError> {
        let mut per_branch = std::collections::HashMap::new();
        for branch in self.bounded.keys() {
            let hashes: Vec<String> = self
                .get(branch)
                .iter()
                .chain(self.full.get(branch).into_iter().flatten())
                .map(|commit| commit.commit.to_string())
                .collect();

            let mut touching = find_or_cache_file_changes(
                &hashes,
                git_info,
                Some(branch.clone()),
                file,
                disk_cache,
            )?;
            for old_path in old_paths {
                touching.extend(
                    find_or_cache_file_changes(
                        &hashes,
                        git_info,
                        Some(branch.clone()),
                        old_path,
                        disk_cache,
                    )
                    .map_err(IssueError::GitFileOpsError)?,
                );
            }
            per_branch.insert(branch.clone(), touching);
        }
        Ok(per_branch)
    }
}

/// Parse a metadata value out of a body, keyed on `pattern`.
///
/// Supports both full and short SHAs with minimum 7 character length.
///
/// The pattern must **begin a metadata line** (D95). Every real marker is written
/// that way — `metadata.join("\n* ")` yields `* approved qc commit: {sha}` — so
/// anchoring costs nothing and buys two things a bare `find` cannot:
///
/// * A round's checklist is user-editable text living inside a comment this scan
///   walks, so a checklist item reading `- [ ] approved qc commit: matches build`
///   must not register an approval. After the optional bullet is stripped such a
///   line still begins `[ ] `, so it cannot match. Prose mentioning the phrase
///   mid-sentence cannot match either.
/// * It closes D20's `new qc initial qc commit:` collision at the source, instead
///   of relying on H1 detection to keep that body out of reach of this function.
///
/// Anchoring on the line — rather than on the comment's `# QC ...` heading — is
/// deliberate: `body_splitter::inject_part_label` gives part 2+ of an oversized
/// comment a `_2/3_` prefix instead of the heading, so a heading gate could drop a
/// real approval. A metadata bullet keeps its own line through any split.
fn parse_commit_from_pattern<'a>(body: &'a str, pattern: &str) -> Option<&'a str> {
    body.lines().find_map(|line| {
        let line = line.trim_start();
        // One list bullet may precede the key. A checklist item still begins `[ ]`
        // once its `- ` is stripped, so it can never match a pattern.
        let rest = line
            .strip_prefix("* ")
            .or_else(|| line.strip_prefix("- "))
            .or_else(|| line.strip_prefix("+ "))
            .unwrap_or(line);
        rest.strip_prefix(pattern)?.split_whitespace().next()
    })
}

/// Parse branch name from issue body
/// Only looks for the "git branch: <branch-name>" pattern
/// Branch name can be plain text, markdown link text, or HTML link text
pub fn parse_branch_from_body(body: &str) -> Option<String> {
    let pattern = "git branch: ";
    let start = body.find(pattern)?;
    let branch_start = start + pattern.len();
    let remaining = &body[branch_start..];
    let line = remaining.lines().next()?;

    // Check if the branch name is a markdown link [name](url)
    if let Some(md_captures) = MARKDOWN_LINK_REGEX.captures(line) {
        if let Some(link_text) = md_captures.get(1) {
            let branch_name = link_text.as_str().trim();
            if !branch_name.is_empty() {
                return Some(branch_name.to_string());
            }
        }
    }

    // Check if the branch name is an HTML link <a href="url">text</a>
    if let Some(html_captures) = HTML_LINK_REGEX.captures(line) {
        if let Some(link_text) = html_captures.get(2) {
            let branch_name = link_text.as_str().trim();
            if !branch_name.is_empty() {
                return Some(branch_name.to_string());
            }
        }
    }

    // Fall back to plain text branch name
    let branch_name = line.trim();
    if !branch_name.is_empty() {
        Some(branch_name.to_string())
    } else {
        None
    }
}

/// Parse blocking QC issues from issue body
///
/// Looks for the "## Relevant Files" section and extracts:
/// - `### Gating QC` subsection → `BlockingRelationship::GatingQC`
/// - `### Previous QC` subsection → `BlockingRelationship::PreviousQC`
///
/// Extracts file name (link text) and issue number from markdown links.
pub fn parse_blocking_qcs(body: &str) -> Vec<BlockingQC> {
    let mut blocking_qcs = Vec::new();

    // Find the start of "## Relevant Files" section
    let relevant_files_start = match body.find("## Relevant Files") {
        Some(pos) => pos,
        None => return blocking_qcs,
    };

    let relevant_section = &body[relevant_files_start..];

    // Find the end of the relevant files section (next level 2 header or end of body)
    let section_end = relevant_section[17..] // Skip "## Relevant Files"
        .find("\n## ")
        .map(|pos| pos + 17)
        .unwrap_or(relevant_section.len());

    let relevant_section = &relevant_section[..section_end];

    // Parse Gating QC section
    if let Some(gating_start) = relevant_section.find("### Gating QC") {
        let gating_section = &relevant_section[gating_start..];
        let gating_end = gating_section[13..] // Skip "### Gating QC"
            .find("\n### ")
            .map(|pos| pos + 13)
            .unwrap_or(gating_section.len());
        let gating_section = &gating_section[..gating_end];

        for capture in BLOCKING_QC_LINK_REGEX.captures_iter(gating_section) {
            if let (Some(file_name), Some(issue_number)) = (capture.get(1), capture.get(2)) {
                if let Ok(issue_num) = issue_number.as_str().parse::<u64>() {
                    blocking_qcs.push(BlockingQC {
                        issue_number: issue_num,
                        file_name: PathBuf::from(file_name.as_str()),
                        relationship: BlockingRelationship::GatingQC,
                    });
                }
            }
        }
    }

    // Parse Previous QC section
    if let Some(previous_start) = relevant_section.find("### Previous QC") {
        let previous_section = &relevant_section[previous_start..];
        let previous_end = previous_section[15..] // Skip "### Previous QC"
            .find("\n### ")
            .map(|pos| pos + 15)
            .unwrap_or(previous_section.len());
        let previous_section = &previous_section[..previous_end];

        for capture in BLOCKING_QC_LINK_REGEX.captures_iter(previous_section) {
            if let (Some(file_name), Some(issue_number)) = (capture.get(1), capture.get(2)) {
                if let Ok(issue_num) = issue_number.as_str().parse::<u64>() {
                    blocking_qcs.push(BlockingQC {
                        issue_number: issue_num,
                        file_name: PathBuf::from(file_name.as_str()),
                        relationship: BlockingRelationship::PreviousQC,
                    });
                }
            }
        }
    }

    blocking_qcs
}

/// Determine the relationship type from a child's body by finding where the parent issue appears
///
/// The relationship type is stored in the child's body - the child lists its blockers
/// under "### Gating QC" or "### Previous QC" sections.
pub fn determine_relationship_from_body(
    body: &str,
    parent_issue_number: u64,
) -> BlockingRelationship {
    let blocking_qcs = parse_blocking_qcs(body);

    for qc in blocking_qcs {
        if qc.issue_number == parent_issue_number {
            return qc.relationship;
        }
    }

    // Parent not found in child's body - indicates data inconsistency
    BlockingRelationship::Unknown
}

/// A single file rename event stored in the "## File History" section of an issue body.
///
/// Format: `* \`old_path\` → \`new_path\` (commit: abc1234)`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileRenameEvent {
    pub old_path: String,
    pub new_path: String,
    pub commit: String,
}

/// Parse file rename events from the "## File History" section of an issue body.
///
/// Each line has the form: `* \`old_path\` → \`new_path\` (commit: abc1234)`
pub fn parse_file_history(body: &str) -> Vec<FileRenameEvent> {
    let section_start = match body.find("## File History") {
        Some(pos) => pos,
        None => return vec![],
    };

    let section = &body[section_start..];
    let section_end = section["## File History".len()..]
        .find("\n## ")
        .map(|pos| pos + "## File History".len())
        .unwrap_or(section.len());
    let section = &section[..section_end];

    let mut events = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with("* `") {
            continue;
        }
        // Line: * `old_path` → `new_path` (commit: abc1234)
        let rest = &line[3..]; // skip "* `"
        let old_end = match rest.find('`') {
            Some(i) => i,
            None => continue,
        };
        let old_path = rest[..old_end].to_string();

        let after_old = &rest[old_end + 1..]; // after closing `
        // find " → `"
        let arrow = " \u{2192} `";
        let new_start = match after_old.find(arrow) {
            Some(i) => i + arrow.len(),
            None => continue,
        };
        let after_arrow = &after_old[new_start..];
        let new_end = match after_arrow.find('`') {
            Some(i) => i,
            None => continue,
        };
        let new_path = after_arrow[..new_end].to_string();

        // find "(commit: ...)"
        let commit_prefix = "(commit: ";
        let commit = match after_arrow[new_end + 1..].find(commit_prefix) {
            Some(i) => {
                let commit_start = i + commit_prefix.len();
                let rest = &after_arrow[new_end + 1 + commit_start..];
                match rest.find(')') {
                    Some(end) => rest[..end].to_string(),
                    None => continue,
                }
            }
            None => continue,
        };

        events.push(FileRenameEvent {
            old_path,
            new_path,
            commit,
        });
    }

    events
}

/// Insert (or replace) the `## File History` section in the issue body.
///
/// If the section already exists it is replaced in-place.
/// Otherwise it is inserted immediately before the first `# ` checklist heading,
/// or appended at the end if no such heading exists.
pub fn splice_file_history(body: &str, history_section: &str) -> String {
    let history_trimmed = history_section.trim_end();

    if let Some(start) = body.find("## File History") {
        let before = body[..start].trim_end();
        let after_header = &body[start + "## File History".len()..];
        let after = match after_header.find("\n## ") {
            Some(p) => &after_header[p + 1..],
            None => "",
        };
        return if after.is_empty() {
            format!("{}\n{}", before, history_trimmed)
        } else {
            format!("{}\n{}\n\n{}", before, history_trimmed, after)
        };
    }

    if let Some(checklist_pos) = find_checklist_start(body) {
        let before = body[..checklist_pos].trim_end();
        let rest = &body[checklist_pos..];
        format!("{}\n\n{}\n\n{}", before, history_trimmed, rest)
    } else {
        format!("{}\n\n{}", body.trim_end(), history_trimmed)
    }
}

/// The `## QC Rounds` marker section (D49/D51).
///
/// Two jobs, and only two:
/// 1. **Prose for a human reader on GitHub.** A reader who opens the issue and does not
///    know to scroll would otherwise read round 1's checklist as the current state.
/// 2. **Its presence — never its content — is a fetch hint (D51).** A cheap path that
///    returns bare issues may read presence/absence to decide whether a comment fetch is
///    needed, and hence whether `Issue.branch` is still current (D50).
///
/// D3 stands: rounds are declared by `# QC Round N` comments and round 1 by the body's
/// metadata. The section's **content is never authoritative** — round count, round
/// metadata and round checklists come only from those comments — and the fold MUST NOT
/// consult it for any purpose (D49.1/D51). The fold already fetches comments
/// unconditionally, so it has no reason to.
///
/// D49.3: prose, with no metadata key. A key here would risk a `find()` collision with
/// the round comment's `round: {N}` (D20), and nothing parses this text anyway.
pub fn qc_rounds_section(round_count: u32) -> String {
    let plural = if round_count == 1 { "round" } else { "rounds" };
    let mut section = String::from("## QC Rounds\n");
    section.push_str(&format!(
        "This QC has been through {round_count} QC {plural}. "
    ));
    section.push_str(
        "The checklist in this issue body belongs to the first round only; every later \
         round — its start commit, its branch and its own checklist — is declared in a \
         `QC Round` comment below. Scroll down to the newest one to see the current state \
         of this QC.\n",
    );
    // Blank line so GitHub renders this as its own paragraph.
    section.push_str("\nThis note is a convenience for readers; the comments are the record.\n");
    section
}

/// Insert (or replace) the `## QC Rounds` section in the issue body (D49/D51).
///
/// The caller writes this **before** posting the round comment and treats a failure as
/// fatal (D51): a cheap path may skip the comment fetch when the marker is absent, so a
/// missing marker on a multi-round issue makes a consumer trust a stale `Issue.branch`.
/// Over-fetching (marker written, comment post failed) is the safe direction.
///
/// Same slot and same idiom as [`splice_file_history`]: an **H2**, spliced immediately
/// before the checklist H1. It must never be an H1 — [`find_checklist_start`] returns the
/// first H1, so an H1 pointer would be parsed as round 1's checklist and break the fold
/// (D49.2).
///
/// Unlike `splice_file_history` the existing section ends at the next heading of **any**
/// level, not only at the next `## `: the following element is normally the checklist H1,
/// which must survive the splice untouched.
pub fn splice_qc_rounds(body: &str, section: &str) -> String {
    let section_trimmed = section.trim_end();

    if let Some(start) = find_line_start(body, "## QC Rounds") {
        let before = body[..start].trim_end();
        let after = next_heading(&body[start..])
            .map(|offset| &body[start + offset..])
            .unwrap_or("");
        return if after.is_empty() {
            format!("{before}\n{section_trimmed}")
        } else {
            format!("{before}\n{section_trimmed}\n\n{after}")
        };
    }

    if let Some(checklist_pos) = find_checklist_start(body) {
        let before = body[..checklist_pos].trim_end();
        let rest = &body[checklist_pos..];
        format!("{before}\n\n{section_trimmed}\n\n{rest}")
    } else {
        format!("{}\n\n{section_trimmed}", body.trim_end())
    }
}

/// True when the body carries a `## QC Rounds` marker (D51).
///
/// **Presence only.** This is the one machine-readable fact the section carries: rounds
/// exist, so `Issue.branch` is round 1's and may be stale (D50). Never read the section's
/// text for a round count — that comes from the comments.
pub fn has_qc_rounds_marker(body: &str) -> bool {
    find_line_start(body, "## QC Rounds").is_some()
}

/// Byte offset of the first line equal to `heading` (after trimming trailing space).
fn find_line_start(body: &str, heading: &str) -> Option<usize> {
    let mut pos = 0usize;
    for line in body.lines() {
        if line.trim_end() == heading {
            return Some(pos);
        }
        pos += line.len() + 1;
    }
    None
}

/// Byte offset of the next markdown heading of any level, skipping the line at offset 0.
fn next_heading(section: &str) -> Option<usize> {
    let mut pos = 0usize;
    for (index, line) in section.lines().enumerate() {
        if index > 0 && line.starts_with('#') {
            return Some(pos);
        }
        pos += line.len() + 1;
    }
    None
}

/// Find the byte offset of the first `# ` heading that is NOT `## `.
pub fn find_checklist_start(body: &str) -> Option<usize> {
    let mut pos = 0usize;
    for line in body.lines() {
        if line.starts_with("# ") && !line.starts_with("## ") {
            return Some(pos);
        }
        pos += line.len() + 1;
    }
    None
}

/// Generate the markdown for a "## File History" section.
pub fn file_history_section(events: &[FileRenameEvent]) -> String {
    let mut section = String::from("## File History\n");
    for event in events {
        section.push_str(&format!(
            "* `{}` \u{2192} `{}` (commit: {})\n",
            event.old_path, event.new_path, event.commit
        ));
    }
    section
}

#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    #[error(transparent)]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Branch '{0}' is not checked out locally")]
    LocalBranchNotFound(String),
    /// D60: the other `None` case of `Round::latest_commit()` (D54.2). The branch *is*
    /// local here — the approval was force-pushed or rebased off it — so
    /// `LocalBranchNotFound`'s "fetch this branch" text would tell the user to fetch a
    /// branch they already have and read as a broken tool. It still classifies as
    /// `branch_not_local` on the wire (see [`IssueError::branch_not_local`]); only the
    /// human-readable message differs.
    #[error(
        "Approval commit {commit} is no longer reachable on branch '{branch}' — its \
         history was likely rewritten"
    )]
    ApprovalNotOnBranch { commit: ObjectId, branch: String },
    #[error(transparent)]
    GitFileOpsError(GitFileOpsError),
    #[error("Initial commit not found in issue body")]
    InitialCommitNotFound,
    #[error("Branch not found in issue body")]
    BranchNotFound,
    #[error("Milestone not found for issue")]
    MilestoneNotFound,
    #[error("Commit string '{0}' could not be parsed to a valid ObjectId")]
    CommitNotParseable(String),
    #[error("No commits found for file: {0}")]
    CommitNotFound(PathBuf),
}

impl IssueError {
    /// The branch to report to a client, when this error is one of the two D54 cases.
    ///
    /// **D60: the two cases stay one case for clients.** `LocalBranchNotFound` and
    /// `ApprovalNotOnBranch` carry different messages but the same wire classification
    /// (`IssueStatusErrorKind::branch_not_local` plus the branch), so the existing wire
    /// contract and the UI affordance built on it keep working unchanged.
    pub fn branch_not_local(&self) -> Option<&str> {
        match self {
            IssueError::LocalBranchNotFound(branch) => Some(branch),
            IssueError::ApprovalNotOnBranch { branch, .. } => Some(branch),
            _ => None,
        }
    }
}

impl From<GitFileOpsError> for IssueError {
    fn from(e: GitFileOpsError) -> Self {
        match e {
            GitFileOpsError::LocalBranchNotFound(name) => IssueError::LocalBranchNotFound(name),
            other => IssueError::GitFileOpsError(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{
        GitComment, GitCommit, GitCommitOps, GitFileOps, GitFileOpsError, GitHubReader,
    };
    use octocrab::models::issues::Issue;
    use std::path::PathBuf;
    use std::str::FromStr;

    fn load_issue(file_name: &str) -> Issue {
        let path = format!("src/tests/issue_threads/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    fn load_comments(file_name: &str) -> Vec<serde_json::Value> {
        let path = format!("src/tests/issue_threads/comments/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read comments file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse comments file {}: {}", path, e))
    }

    /// The fixture commits, **newest-first** — the order `get_commits_robust` returns
    /// and the order the fold and `Round.commits` use (D8). Declared oldest-first for
    /// readability, then reversed.
    fn create_test_commits() -> Vec<(ObjectId, String)> {
        let mut commits = vec![
            (
                ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                "Initial commit".to_string(),
            ),
            (
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                "Second commit".to_string(),
            ),
            (
                ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                "Third commit".to_string(),
            ),
            (
                ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap(),
                "Fourth commit".to_string(),
            ),
            (
                ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap(),
                "Fifth commit".to_string(),
            ),
            (
                ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap(),
                "Sixth commit".to_string(),
            ),
            (
                ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap(),
                "Seventh commit".to_string(),
            ),
            // Additional commits for test_from_issue_open_with_approval_and_notification
            (
                ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap(),
                "Initial test commit".to_string(),
            ),
            (
                ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap(),
                "Second test commit".to_string(),
            ),
            (
                ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap(),
                "Third test commit".to_string(),
            ),
        ];
        commits.reverse();
        commits
    }

    /// Convert the JSON comment fixtures into `GitComment`s.
    fn git_comments(comments: Vec<serde_json::Value>) -> Vec<GitComment> {
        comments
            .into_iter()
            .map(|comment| GitComment {
                id: comment["id"].as_u64(),
                body: comment["body"].as_str().unwrap().to_string(),
                author_login: comment["user"]["login"]
                    .as_str()
                    .unwrap_or("test-user")
                    .to_string(),
                created_at: chrono::Utc::now(),
                html: None,
            })
            .collect()
    }

    // Simple mock for IssueThread tests
    struct SimpleMockGitInfo {
        commits: Vec<(ObjectId, String)>,
        /// Per-branch walks, for the multi-branch rounds D7 allows.
        branch_commits: std::collections::HashMap<String, Vec<(ObjectId, String)>>,
        /// Branches the local clone has never fetched — `commits()` fails for these
        /// exactly as `find_commits` does, which is what D53's `Unplaceable` represents.
        missing_branches: HashSet<String>,
        comments: Vec<GitComment>,
    }

    impl SimpleMockGitInfo {
        fn new() -> Self {
            Self {
                commits: Vec::new(),
                branch_commits: std::collections::HashMap::new(),
                missing_branches: HashSet::new(),
                comments: Vec::new(),
            }
        }

        /// D53: the branch exists on the remote and in the comment log, but not here.
        fn with_missing_branch(mut self, branch: &str) -> Self {
            self.missing_branches.insert(branch.to_string());
            self
        }

        fn with_branch_commits(mut self, branch: &str, commits: Vec<(ObjectId, String)>) -> Self {
            self.branch_commits.insert(branch.to_string(), commits);
            self
        }

        fn walk_for(&self, branch: &Option<String>) -> &[(ObjectId, String)] {
            branch
                .as_ref()
                .and_then(|b| self.branch_commits.get(b))
                .map(|c| c.as_slice())
                .unwrap_or(&self.commits)
        }

        fn with_commits(mut self, commits: Vec<(ObjectId, String)>) -> Self {
            self.commits = commits;
            self
        }

        fn with_comments(mut self, comments: Vec<GitComment>) -> Self {
            self.comments = comments;
            self
        }
    }

    impl GitCommitOps for SimpleMockGitInfo {
        fn commits(
            &self,
            branch: &Option<String>,
            _stop_at: Option<ObjectId>,
        ) -> Result<Vec<GitCommit>, GitFileOpsError> {
            if let Some(name) = branch {
                if self.missing_branches.contains(name) {
                    return Err(GitFileOpsError::LocalBranchNotFound(name.clone()));
                }
            }
            Ok(self
                .walk_for(branch)
                .iter()
                .map(|(commit, message)| GitCommit {
                    commit: *commit,
                    message: message.clone(),
                })
                .collect())
        }

        fn branch_tip(&self, _branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
            Err(GitFileOpsError::LocalBranchNotFound("mock".to_string()))
        }

        fn file_touching_commits(
            &self,
            branch: Option<String>,
            _file: &std::path::Path,
        ) -> Result<std::collections::HashSet<String>, GitFileOpsError> {
            // Return all commit hashes as "touching" since tests use a single file
            Ok(self
                .walk_for(&branch)
                .iter()
                .map(|(id, _)| id.to_string())
                .collect())
        }

        fn get_branches_containing_commit(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<String>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn find_merged_into_branch(
            &self,
            _target_commit: &ObjectId,
        ) -> Result<Option<String>, GitFileOpsError> {
            Ok(None)
        }
    }

    impl GitFileOps for SimpleMockGitInfo {
        fn authors(
            &self,
            _file: &std::path::Path,
        ) -> Result<Vec<crate::git::GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &std::path::Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    impl GitHubReader for SimpleMockGitInfo {
        async fn get_milestones(
            &self,
        ) -> Result<Vec<octocrab::models::Milestone>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issues(
            &self,
            _milestone: Option<u64>,
        ) -> Result<Vec<Issue>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issue(&self, _issue_number: u64) -> Result<Issue, crate::git::GitHubApiError> {
            Err(crate::git::GitHubApiError::NoApi)
        }

        async fn get_assignees(&self) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_user_details(
            &self,
            _username: &str,
        ) -> Result<crate::RepoUser, crate::git::GitHubApiError> {
            Ok(crate::RepoUser {
                login: _username.to_string(),
                name: None,
            })
        }

        async fn get_labels(&self) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issue_comments(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<GitComment>, crate::git::GitHubApiError> {
            Ok(self.comments.clone())
        }

        async fn get_issue_events(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<serde_json::Value>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_blocked_issues(
            &self,
            _issue_number: u64,
        ) -> Result<Vec<Issue>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_current_user(&self) -> Result<Option<String>, crate::git::GitHubApiError> {
            Ok(None)
        }
    }

    /// The zero-migration case (D2): an existing issue with no round comment is a
    /// valid single-round issue.
    #[tokio::test]
    async fn test_from_issue_open_with_notifications() {
        // Comment sequence:
        // 1. Initial commit: abc123def456789012345678901234567890abcd (from issue body)
        // 2. Notification: current commit: def456789abc012345678901234567890123abcd
        // 3. Notification: current commit: 123abcd (short SHA)
        // No approval commits in this test

        let issue = load_issue("open_issue_with_notifications.json");
        let comments = load_comments("open_issue_notifications.json");

        let git_info = SimpleMockGitInfo::new()
            .with_commits(create_test_commits())
            .with_comments(git_comments(comments));

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        // One round, from the body alone
        assert_eq!(result.rounds.len(), 1);
        assert!(matches!(result.round_state(0), DerivedState::Open));
        assert_eq!(
            *result.initial_commit(),
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap()
        );
        // I14: an open round has no drift, so status can never read one.
        assert!(result.drift.commits.is_empty());
        assert!(!result.drift.divergent);

        // Verify notification commits (both full and short SHAs should be parsed),
        // newest-first within the round
        let notification_commits: Vec<&ObjectId> = result
            .latest_round()
            .commits
            .iter()
            .filter(|c| c.statuses.contains(&CommitStatus::Notification))
            .map(|c| &c.hash)
            .collect();
        assert_eq!(notification_commits.len(), 2);
        assert_eq!(
            *notification_commits[0],
            ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap() // 123abcd matches this commit
        );
        assert_eq!(
            *notification_commits[1],
            ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap()
        );

        // Open issue should have no approval
        assert_eq!(result.latest_round().approved_commit(), None);
        assert_eq!(result.file, PathBuf::from("src/main.rs"));
        assert_eq!(result.branch(), "feature/new-feature");
        assert!(result.invariant_violations().is_empty());
    }

    #[tokio::test]
    async fn test_from_issue_closed_with_approval() {
        // Comment sequence:
        // 1. Initial commit: def456abc789012345678901234567890123abcd (from issue body)
        // 2. Notification: current commit: 456def789abc012345678901234567890123cdef
        // 3. Approval: approved qc commit: 456def789abc012345678901234567890123cdef
        // No unapproval - approval remains valid

        let issue = load_issue("closed_approved_issue.json");
        let comments = load_comments("closed_approved_comments.json");

        let git_info = SimpleMockGitInfo::new()
            .with_commits(create_test_commits())
            .with_comments(git_comments(comments));

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        let start = ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap();
        let approval = ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap();

        assert_eq!(*result.initial_commit(), start);

        // D8: the round owns `[start ..= approval]`, and the approval is its newest
        // commit (I5).
        let round = result.latest_round();
        assert_eq!(round.approved_commit(), Some(&approval));
        assert_eq!(round.commits.first().map(|c| c.hash), Some(approval));
        assert_eq!(round.commits.last().map(|c| c.hash), Some(start));
        assert_eq!(round.commits.len(), 2);

        // The approval commit also carries the notification status; approvedness is
        // NOT a commit status any more (D33).
        assert!(
            round.commits[0]
                .statuses
                .contains(&CommitStatus::Notification)
        );

        // D34: the drift is `(approval .. tip]` — exclusive lower, inclusive upper.
        assert!(!result.drift.commits.is_empty());
        assert!(!result.drift.commits.iter().any(|c| c.hash == approval));

        assert_eq!(result.file, PathBuf::from("src/lib.rs"));
        assert_eq!(result.branch(), "bugfix/memory-leak");
        assert!(result.invariant_violations().is_empty());
    }

    #[tokio::test]
    async fn test_from_issue_with_unapproval() {
        // Comment sequence:
        // 1. Initial commit: 789abc12def345678901234567890123456789ef (from issue body)
        // 2. Notification: current commit: 890cdef123abc456789012345678901234567890
        // 3. Approval: approved qc commit: 890cdef123abc456789012345678901234567890
        // 4. Notification: current commit: abc1234 (short SHA)
        // 5. Unapproval: # QC Un-Approval (invalidates the approval from step 3)

        let issue = load_issue("unapproved_issue.json");
        let comments = load_comments("unapproved_comments.json");

        let git_info = SimpleMockGitInfo::new()
            .with_commits(create_test_commits())
            .with_comments(git_comments(comments));

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        assert_eq!(
            *result.initial_commit(),
            ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap()
        );

        // D11/F5: the unapproval reopened the round, so `drift`'s commits folded back
        // into the round and no stale approval survives (D33).
        assert_eq!(result.latest_round().approved_commit(), None);
        assert!(matches!(result.round_state(0), DerivedState::Open));
        assert!(result.drift.commits.is_empty());

        // "890cdef..." was notification → approved → unapproved (notification remains)
        // "abc1234" was notification
        let notification_commits: Vec<&ObjectId> = result
            .latest_round()
            .commits
            .iter()
            .filter(|c| c.statuses.contains(&CommitStatus::Notification))
            .map(|c| &c.hash)
            .collect();
        assert_eq!(notification_commits.len(), 2);
        assert_eq!(
            *notification_commits[0],
            ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap()
        );
        assert_eq!(
            *notification_commits[1],
            ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap()
        );

        assert_eq!(result.file, PathBuf::from("src/utils.rs"));
        assert_eq!(result.branch(), "feature/utils-refactor");
        assert!(result.invariant_violations().is_empty());
    }

    #[tokio::test]
    async fn test_from_issue_open_with_approval_and_notification() {
        // Comment sequence:
        // 1. Initial commit: 111def456789012345678901234567890123abcd (from issue body)
        // 2. Notification: current commit: 222abc123456789012345678901234567890def
        // 3. Approval: approved qc commit: 222abc123456789012345678901234567890def
        // 4. Notification: current commit: 333cdef78 (short SHA)
        // Issue is open but approval remains valid (no unapproval)

        let issue = load_issue("open_issue_with_approval_and_notification.json");
        let comments = load_comments("open_issue_approval_and_notification.json");

        // newest-first
        let test_commits = vec![
            (
                ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap(),
                "Third".to_string(),
            ),
            (
                ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap(),
                "Second".to_string(),
            ),
            (
                ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap(),
                "Initial".to_string(),
            ),
        ];

        let git_info = SimpleMockGitInfo::new()
            .with_commits(test_commits)
            .with_comments(git_comments(comments));

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        let start = ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap();
        let approval = ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap();
        let after = ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap();

        assert_eq!(*result.initial_commit(), start);

        // The round owns `[start ..= approval]`; the notification posted after the
        // approval lands in the drift, which is what S1 reads.
        let round = result.latest_round();
        assert_eq!(round.approved_commit(), Some(&approval));
        assert_eq!(
            round.commits.iter().map(|c| c.hash).collect::<Vec<_>>(),
            vec![approval, start]
        );
        assert!(
            round.commits[0]
                .statuses
                .contains(&CommitStatus::Notification)
        );

        assert_eq!(
            result
                .drift
                .commits
                .iter()
                .map(|c| c.hash)
                .collect::<Vec<_>>(),
            vec![after]
        );
        assert!(
            result.drift.commits[0]
                .statuses
                .contains(&CommitStatus::Notification)
        );
        assert!(!result.drift.divergent);

        assert_eq!(result.file, PathBuf::from("src/test.rs"));
        assert_eq!(result.branch(), "feature/test-branch");
        assert_eq!(result.open, true);
        assert!(result.invariant_violations().is_empty());
    }

    // ── Rounds: the fold, the segment view, and the invariant flags ─────────────

    fn oid(n: u64) -> ObjectId {
        ObjectId::from_str(&format!("{:040x}", n)).unwrap()
    }

    /// A branch walk, newest-first: `walk(&[5, 4, 3, 2, 1])` is tip c5 … root c1.
    fn walk(numbers: &[u64]) -> Vec<(ObjectId, String)> {
        numbers
            .iter()
            .map(|n| (oid(*n), format!("commit {n}")))
            .collect()
    }

    fn rounds_issue(body: &str, state: &str) -> Issue {
        crate::test_utils::create_test_issue(
            "a2-ai",
            "ghqc",
            1,
            "src/model.R",
            body,
            Some(1),
            state,
        )
    }

    const ROUND_1_BODY: &str = "\
## Metadata
initial qc commit: 0000000000000000000000000000000000000001
git branch: main

# Checklist One
- [x] first
- [ ] second
";

    /// A `# QC Round 2` comment (D3/D20): the body's exact metadata keys plus
    /// `round:`, then the round's checklist and nothing else (D4).
    fn round_2_comment(start: u64, branch: &str, checklist: &str) -> GitComment {
        comment(&format!(
            "# QC Round 2\n\n## Metadata\nround: 2\ninitial qc commit: {}\ngit branch: {}\n\n{}",
            oid(start),
            branch,
            checklist
        ))
    }

    #[tokio::test]
    async fn test_fold_multi_round_thread() {
        // main: c5(tip) c4 c3 c2 c1 ; round 1 = [c1..=c2] approved, round 2 = [c4..tip]
        let comments = vec![
            comment(&format!("current commit: {}", oid(2))),
            comment(&format!("approved qc commit: {}", oid(2))),
            round_2_comment(4, "main", "# Checklist Two\n- [ ] x\n- [ ] y\n"),
            comment(&format!("current commit: {}", oid(5))),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[5, 4, 3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        assert_eq!(thread.rounds.len(), 2);
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());

        // D8: `Approved` owns `[start_1 ..= approval_1]`
        let first = thread.round(1).unwrap();
        assert_eq!(first.start_commit, oid(1));
        assert_eq!(first.approved_commit(), Some(&oid(2)));
        assert_eq!(
            first.commits.iter().map(|c| c.hash).collect::<Vec<_>>(),
            vec![oid(2), oid(1)]
        );
        assert!(matches!(thread.round_state(0), DerivedState::Approved(_)));

        // D8: the open last round owns `[start_2 .. tip]`
        let second = thread.round(2).unwrap();
        assert_eq!(second.index, 2);
        assert_eq!(second.start_commit, oid(4));
        assert_eq!(
            second.commits.iter().map(|c| c.hash).collect::<Vec<_>>(),
            vec![oid(5), oid(4)]
        );
        assert!(matches!(thread.round_state(1), DerivedState::Open));

        // D8/D34: `R2.preceding_gap` = `(approval_1 .. start_2)`, exclusive both ends
        assert_eq!(
            second
                .preceding_gap
                .commits
                .iter()
                .map(|c| c.hash)
                .collect::<Vec<_>>(),
            vec![oid(3)]
        );
        assert!(!second.preceding_gap.divergent);

        // Each round carries its own checklist, heading excluded from `content` (D37)
        assert_eq!(first.checklist.name, "Checklist One");
        assert!(!first.checklist.content.contains("# Checklist One"));
        assert_eq!(second.checklist.name, "Checklist Two");
        assert!(!second.checklist.content.contains("# Checklist Two"));
        // R8: the record sums every round
        let all = thread.checklist_summary_all_rounds();
        assert_eq!((all.completed, all.total), (1, 4));

        // W6: `R1 G2 R2`, with no trailing segment after an open round
        let segments = thread.segments();
        assert_eq!(segments.len(), 3);
        assert!(matches!(segments[0], Segment::Round(r) if r.index == 1));
        assert!(matches!(segments[1], Segment::Gap { round, .. } if round.index == 2));
        assert!(matches!(segments[2], Segment::Round(r) if r.index == 2));

        // D10: status reads the latest round, so the walk no longer grows with the
        // QC's whole life (S5).
        assert_eq!(thread.branch(), "main");
        assert_eq!(*thread.initial_commit(), oid(1));
    }

    /// D12/W6: starting a round from plain `Approved` produces `start_2 == approval_1`
    /// — the one permitted overlap (D8). The empty gap is meaningful and is suppressed
    /// by position only.
    #[tokio::test]
    async fn test_fold_round_started_at_the_approval_leaves_an_empty_gap() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(3))),
            round_2_comment(3, "main", "# Checklist Two\n- [ ] x\n"),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[4, 3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        let second = thread.round(2).unwrap();
        assert!(second.preceding_gap.commits.is_empty());
        assert!(!second.preceding_gap.divergent);
        assert_eq!(second.start_commit, oid(3));
        assert_eq!(thread.round(1).unwrap().approved_commit(), Some(&oid(3)));
        // I7 exempts exactly this overlap.
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());
        // Still `R1 G2 R2`: suppression is positional, never by emptiness (W6).
        assert_eq!(thread.segments().len(), 3);
    }

    /// D17: "no drift" and "empty drift" are distinct states, and the distinction is
    /// carried by `RoundState` — never by the drift being empty.
    #[tokio::test]
    async fn test_empty_drift_and_open_round_are_distinguished_by_state() {
        // Approved at the tip: the drift is empty *and* meaningful.
        let approved_at_tip = SimpleMockGitInfo::new()
            .with_commits(walk(&[2, 1]))
            .with_comments(vec![comment(&format!("approved qc commit: {}", oid(2)))]);
        let approved = IssueThread::from_issue(
            &rounds_issue(ROUND_1_BODY, "closed"),
            None,
            &approved_at_tip,
        )
        .await
        .unwrap();

        // Never approved: the drift is empty and means nothing (I14).
        let never_approved = SimpleMockGitInfo::new()
            .with_commits(walk(&[2, 1]))
            .with_comments(vec![comment(&format!("current commit: {}", oid(2)))]);
        let open =
            IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &never_approved)
                .await
                .unwrap();

        // Emptiness cannot tell them apart …
        assert!(approved.drift.commits.is_empty());
        assert!(open.drift.commits.is_empty());
        // … `RoundState` can (S0).
        assert!(approved.latest_round().is_closed());
        assert!(!open.latest_round().is_closed());

        // W6.2: the trailing segment is emitted only after a closed round.
        assert!(matches!(
            approved.segments().last(),
            Some(Segment::Drift { .. })
        ));
        assert!(matches!(open.segments().last(), Some(Segment::Round(_))));

        assert_eq!(approved.invariant_violations(), Vec::<String>::new());
        assert_eq!(open.invariant_violations(), Vec::<String>::new());
    }

    /// D22/D39: round 1 on `main` (c1…c5, approved c5) with round 2 on `feat`, forked
    /// at c3. `approval_1` is not in `feat`'s ancestry, so the gap is divergent and is
    /// walked with no `stop_at` — which legitimately reaches commits round 1 owns.
    /// I7's exception exists for exactly this shape, which is the *common* one.
    #[tokio::test]
    async fn test_fold_divergent_preceding_gap() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(5))),
            round_2_comment(11, "feat", "# Checklist Two\n- [ ] x\n"),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_branch_commits("main", walk(&[5, 4, 3, 2, 1]))
            .with_branch_commits("feat", walk(&[12, 11, 3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        assert_eq!(thread.rounds.len(), 2);
        // D7: each round owns its own branch; W4 asks the checkout for the latest.
        assert_eq!(thread.round(1).unwrap().branch, "main");
        assert_eq!(thread.branch(), "feat");

        let gap = &thread.round(2).unwrap().preceding_gap;
        assert!(gap.divergent);
        // The no-`stop_at` walk spans the full duration of the owning round's branch.
        assert_eq!(
            gap.commits.iter().map(|c| c.hash).collect::<Vec<_>>(),
            vec![oid(3), oid(2), oid(1)]
        );
        // Those commits are also owned by round 1 — I7 exempts a divergent gap, so
        // this must NOT be reported as a violation (D39.2).
        assert!(
            thread
                .round(1)
                .unwrap()
                .commits
                .iter()
                .any(|c| c.hash == oid(3))
        );
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());
    }

    /// D31: a `drift` whose approval was rewritten off its branch is divergent. Status
    /// stays `ChangesAfterApproval` and fails honestly; reporting `Approved` would be
    /// the worst available failure.
    #[tokio::test]
    async fn test_fold_divergent_drift() {
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[3, 2, 1]))
            // approval 99 is nowhere in the branch
            .with_comments(vec![comment(&format!("approved qc commit: {}", oid(99)))]);

        let thread =
            IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "closed"), None, &git_info)
                .await
                .unwrap();

        assert!(thread.drift.divergent);
        assert!(!thread.drift.commits.is_empty());
        assert!(thread.latest_round().is_closed());
        // I5 is scoped to `!drift.divergent` (D39.1): the approval cannot appear in a
        // walk of the branch it was rewritten off, so this is not a violation.
        assert!(
            !thread
                .invariant_violations()
                .iter()
                .any(|v| v.starts_with("I5"))
        );
    }

    /// D21/D39.4: an unapproval before a later round comment leaves the earlier round
    /// superseded — a real, reachable state that does not imply malformation.
    #[tokio::test]
    async fn test_fold_superseded_round() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            comment("# QC Un-Approval\nthe approval was wrong"),
            round_2_comment(4, "main", "# Checklist Two\n- [ ] x\n"),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[5, 4, 3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        // F6: the fold stores only `Unapproved`; superseded is positional (D27).
        assert_eq!(thread.rounds[0].state, RoundState::Unapproved);
        assert!(matches!(thread.round_state(0), DerivedState::Superseded));
        assert!(matches!(thread.round_state(1), DerivedState::Open));

        // D21: the malformed round swallows every commit up to `start_2`, keeping its
        // statuses attachable (O4); `R2.preceding_gap` is empty, not absent (D26).
        assert_eq!(
            thread.rounds[0]
                .commits
                .iter()
                .map(|c| c.hash)
                .collect::<Vec<_>>(),
            vec![oid(3), oid(2), oid(1)]
        );
        assert!(thread.rounds[1].preceding_gap.commits.is_empty());
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());
    }

    /// D8/D38: D7 lets round 2 live on another branch, so `[start_1 .. start_2)` can
    /// name an upper bound that is not on `branch_1` at all. The superseded round then
    /// owns `[start_1 .. tip(branch_1)]` — never just its start commit.
    #[tokio::test]
    async fn test_fold_superseded_round_bounded_at_tip_when_successor_is_off_branch() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            comment("# QC Un-Approval\nthe approval was wrong"),
            // Round 2 starts at c12, which exists only on `feat`.
            round_2_comment(12, "feat", "# Checklist Two\n- [ ] x\n"),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_branch_commits("main", walk(&[5, 4, 3, 2, 1]))
            .with_branch_commits("feat", walk(&[13, 12, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        assert!(matches!(thread.round_state(0), DerivedState::Superseded));
        assert_eq!(thread.rounds[0].branch, "main");
        assert_eq!(thread.rounds[1].branch, "feat");

        // D38: bounded at tip(main) = c5, not at its own start commit.
        let owned: Vec<ObjectId> = thread.rounds[0].commits.iter().map(|c| c.hash).collect();
        assert_eq!(
            owned.first(),
            Some(&oid(5)),
            "D38: a superseded round whose successor is off-branch must be bounded at \
             tip(branch_n), got {owned:?}"
        );
        assert_eq!(owned, vec![oid(5), oid(4), oid(3), oid(2), oid(1)]);
        assert!(thread.rounds[1].preceding_gap.commits.is_empty());
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());
    }

    /// F9/D34/D41: `drift` is `(approval_last .. tip]` — exclusive of the approval,
    /// INCLUSIVE of the branch tip. The inclusive half is not assertable after the
    /// fold (M1 stores no tip), so the slice bounds are pinned here, directly and by
    /// name, rather than left to incidental field assertions.
    #[tokio::test]
    async fn test_drift_slice_excludes_the_approval_and_includes_the_tip() {
        // main: c5(tip) c4 c3 c2 c1 ; approved at c3 ⇒ drift = (c3 .. c5] = [c5, c4].
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[5, 4, 3, 2, 1]))
            .with_comments(vec![comment(&format!("approved qc commit: {}", oid(3)))]);

        let thread =
            IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "closed"), None, &git_info)
                .await
                .unwrap();

        let drift: Vec<ObjectId> = thread.drift.commits.iter().map(|c| c.hash).collect();
        assert!(!thread.drift.divergent);

        // INCLUSIVE upper bound: S1 reads the newest post-approval change, which is
        // very often HEAD itself. An exclusive tip would silently report `Approved`.
        assert_eq!(
            drift.first(),
            Some(&oid(5)),
            "drift must INCLUDE the branch tip (F9/D34), got {drift:?}"
        );
        // EXCLUSIVE lower bound: the approval belongs to the round, not the drift.
        assert!(
            !drift.contains(&oid(3)),
            "drift must EXCLUDE approval_last (I8/D34), got {drift:?}"
        );
        assert_eq!(drift, vec![oid(5), oid(4)]);
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());
    }

    /// I8's drift half (D41): a drift containing `approval_last` is flagged. The
    /// preceding-gap half was implemented; this one was missing entirely.
    #[test]
    fn test_i8_flags_a_drift_containing_the_approval() {
        let commit = |hash| IssueCommit {
            hash,
            message: "c".to_string(),
            statuses: HashSet::new(),
            file_changed: true,
        };
        let mut start = commit(oid(1));
        start.statuses.insert(CommitStatus::Initial);

        let broken = IssueThread {
            file: PathBuf::from("src/model.R"),
            milestone: "v1.0".to_string(),
            open: false,
            blocking_qcs: vec![],
            rounds: vec![Round {
                index: 1,
                branch: "main".to_string(),
                branch_inherited: false,
                placement: RoundPlacement::Placed,
                start_commit: oid(1),
                preceding_gap: Gap::default(),
                checklist: RoundChecklist::default(),
                commits: vec![commit(oid(2)), start],
                state: RoundState::Approved(Approval {
                    commit: oid(2),
                    comment_id: None,
                }),
            }],
            // `(approval_last .. tip]` is exclusive at the bottom, so c2 must not be
            // in here.
            drift: Gap {
                commits: vec![commit(oid(3)), commit(oid(2))],
                divergent: false,
            },
        };

        let violations = broken.invariant_violations();
        assert!(
            violations
                .iter()
                .any(|v| v.starts_with("I8") && v.contains("drift")),
            "expected an I8 drift flag, got {violations:?}"
        );
    }

    /// D32/D40: an emptied round checklist is logged and flagged, never fatal — the
    /// fold still returns an `IssueThread`.
    #[tokio::test]
    async fn test_invariant_violation_is_flagged_not_fatal() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            round_2_comment(4, "main", "# Emptied By A Human\n\nno checkboxes left\n"),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[5, 4, 3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        assert_eq!(thread.rounds.len(), 2);
        assert_eq!(thread.round(2).unwrap().checklist.summary().total, 0);
        let violations = thread.invariant_violations();
        assert!(
            violations.iter().any(|v| v.starts_with("I9")),
            "expected an I9 flag, got {violations:?}"
        );
    }

    /// The invariant checks report every violation of a hand-built thread rather than
    /// panicking on any of them (D32).
    #[test]
    fn test_invariant_violations_reported_not_asserted() {
        let broken = IssueThread {
            file: PathBuf::from("src/model.R"),
            milestone: "v1.0".to_string(),
            open: true,
            blocking_qcs: vec![],
            rounds: vec![Round {
                index: 0, // I2: below its own position, i.e. rounds were re-indexed
                branch: "main".to_string(),
                branch_inherited: false,
                placement: RoundPlacement::Placed,
                start_commit: oid(1),
                preceding_gap: Gap {
                    commits: vec![IssueCommit {
                        hash: oid(9),
                        message: "stray".to_string(),
                        statuses: HashSet::new(),
                        file_changed: false,
                    }],
                    divergent: true,
                }, // I13
                checklist: RoundChecklist::default(),
                commits: vec![IssueCommit {
                    hash: oid(1),
                    message: "start".to_string(),
                    statuses: HashSet::new(), // I4: no Initial
                    file_changed: true,
                }],
                state: RoundState::Unapproved,
            }],
            drift: Gap {
                commits: vec![IssueCommit {
                    hash: oid(2),
                    message: "drifted".to_string(),
                    statuses: HashSet::new(),
                    file_changed: true,
                }],
                divergent: false,
            }, // I14
        };

        let violations = broken.invariant_violations();
        for id in ["I2", "I4", "I13", "I14"] {
            assert!(
                violations.iter().any(|v| v.starts_with(id)),
                "expected a {id} flag, got {violations:?}"
            );
        }
    }

    /// D53.1/D53.2: a round comment with **no `initial qc commit:`** is the one shape
    /// that cannot become an `Unplaceable` round — `Round.start_commit` would have no
    /// honest value — so it is dropped. But **nothing is re-indexed**: the `# QC Round 3`
    /// comment stays `Round { index: 3 }`. Before D53 it became `index: 2`, so
    /// `Round.index` and `ArchiveQC.round` disagreed with the GitHub comment log.
    #[tokio::test]
    async fn test_a_dropped_round_comment_never_re_indexes_the_rounds() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            comment("# QC Round 2\n\n## Metadata\nround: 2\ngit branch: main\n\n# Two\n- [ ] x\n"),
            comment(&format!(
                "# QC Round 3\n\n## Metadata\nround: 3\ninitial qc commit: {}\ngit branch: main\n\n# Three\n- [ ] y\n",
                oid(4)
            )),
            comment(&format!("current commit: {}", oid(5))),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[5, 4, 3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        // Two rounds survive; the survivor keeps its DECLARED number.
        assert_eq!(thread.rounds.len(), 2);
        assert_eq!(thread.rounds[1].index, 3);
        assert_eq!(thread.round(3).map(|r| r.index), Some(3));
        assert_eq!(thread.rounds[1].start_commit, oid(4));
        assert_eq!(thread.rounds[1].checklist.name, "Three");
        // A new round must not reuse the dropped comment's number.
        assert_eq!(thread.next_round_index(), 4);
        // The surviving round still gets ITS partition's statuses, not the skipped
        // comment's.
        assert!(
            thread.rounds[1]
                .commits
                .iter()
                .any(|c| c.hash == oid(5) && c.statuses.contains(&CommitStatus::Notification))
        );
        assert_eq!(thread.invariant_violations(), Vec::<String>::new());
    }

    /// D53: a round declared on a branch the clone has never fetched is **kept**, with
    /// its declared index and the branch to fetch. It owns no commits, it bounds an
    /// empty gap, and it does not shift the rounds around it.
    #[tokio::test]
    async fn test_fold_keeps_a_round_whose_branch_is_not_fetched() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            round_2_comment(9, "feature", "# Two\n- [ ] x\n"),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[3, 2, 1]))
            .with_missing_branch("feature")
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        // Never dropped (D53.1), and its index is the declared one (D53.2).
        assert_eq!(thread.rounds.len(), 2);
        let second = thread.round(2).unwrap();
        assert_eq!(second.index, 2);
        assert_eq!(
            second.placement,
            RoundPlacement::Unplaceable {
                branch: "feature".to_string()
            }
        );
        assert_eq!(second.unplaceable_branch(), Some("feature"));
        // D53.3: no commits, and the gap it bounds is empty.
        assert!(second.commits.is_empty());
        assert!(second.preceding_gap.commits.is_empty());
        assert!(!second.preceding_gap.divergent);
        assert!(thread.drift.commits.is_empty());
        // D54.1: no representative commit at all.
        assert_eq!(second.latest_commit(), None);
        // The declared start commit is still reported — it is what the comment says.
        assert_eq!(second.start_commit, oid(9));
        // D53.4: I4 (and I5) are scoped to `Placed`, so an unplaceable round does not
        // manufacture invariant violations out of legal data.
        let violations = thread.invariant_violations();
        assert!(
            !violations.iter().any(|v| v.starts_with("I4")),
            "I4 must be scoped to placed rounds, got {violations:?}"
        );
        assert!(
            !violations.iter().any(|v| v.starts_with("I2")),
            "an unplaceable round must not disturb the indices, got {violations:?}"
        );
    }

    /// Round 1's branch comes from the issue body, and a thread with no placeable round
    /// 1 has no starting point at all — that stays the pre-rounds failure, reported as
    /// `branch_not_local` with the branch to fetch.
    #[tokio::test]
    async fn test_round_one_on_an_unfetched_branch_is_still_an_error() {
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[3, 2, 1]))
            .with_missing_branch("main");

        match IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info).await {
            Err(IssueError::LocalBranchNotFound(branch)) => assert_eq!(branch, "main"),
            other => panic!("expected LocalBranchNotFound, got {other:?}"),
        }
    }

    /// D54.2: an approved round whose approval is **not** among the commits it owns has
    /// no representative commit. The pre-D54 code fell through to the newest
    /// status-bearing commit, so `archive_commit` became a *post-approval* commit while
    /// `approved` stayed `true` — the archive then claimed approved content over content
    /// that was never approved.
    #[test]
    fn test_latest_commit_is_none_when_the_approval_is_not_in_the_commits() {
        let start = IssueCommit {
            hash: oid(1),
            message: "start".to_string(),
            statuses: HashSet::from([CommitStatus::Initial]),
            file_changed: true,
        };
        let after = IssueCommit {
            hash: oid(2),
            message: "post-approval work".to_string(),
            statuses: HashSet::from([CommitStatus::Notification]),
            file_changed: true,
        };
        let round = Round {
            index: 1,
            branch: "main".to_string(),
            branch_inherited: false,
            placement: RoundPlacement::Placed,
            start_commit: oid(1),
            preceding_gap: Gap::default(),
            checklist: RoundChecklist::default(),
            commits: vec![after, start],
            // Force-pushed off the branch: declared, but not among the commits owned.
            state: RoundState::Approved(Approval {
                commit: oid(7),
                comment_id: None,
            }),
        };

        assert_eq!(round.latest_commit(), None);
    }

    /// The fallthrough D54 keeps: an `Unapproved` **placed** round still resolves to its
    /// newest status-bearing commit. That is the case it was written for.
    #[test]
    fn test_latest_commit_still_falls_through_for_an_unapproved_round() {
        let round = Round {
            index: 1,
            branch: "main".to_string(),
            branch_inherited: false,
            placement: RoundPlacement::Placed,
            start_commit: oid(1),
            preceding_gap: Gap::default(),
            checklist: RoundChecklist::default(),
            commits: vec![
                IssueCommit {
                    hash: oid(3),
                    message: "unnotified".to_string(),
                    statuses: HashSet::new(),
                    file_changed: true,
                },
                IssueCommit {
                    hash: oid(2),
                    message: "notified".to_string(),
                    statuses: HashSet::from([CommitStatus::Notification]),
                    file_changed: true,
                },
                IssueCommit {
                    hash: oid(1),
                    message: "start".to_string(),
                    statuses: HashSet::from([CommitStatus::Initial]),
                    file_changed: true,
                },
            ],
            state: RoundState::Unapproved,
        };

        assert_eq!(round.latest_commit().map(|c| c.hash), Some(oid(2)));
    }

    /// D56: the fold records that a round comment carrying no `git branch:` inherited
    /// its branch. The inheritance is kept — the fact is what must not be silent.
    #[tokio::test]
    async fn test_fold_records_an_inherited_branch() {
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            comment(&format!(
                "# QC Round 2\n\n## Metadata\nround: 2\ninitial qc commit: {}\n\n# Two\n- [ ] x\n",
                oid(3)
            )),
        ];
        let git_info = SimpleMockGitInfo::new()
            .with_commits(walk(&[3, 2, 1]))
            .with_comments(comments);

        let thread = IssueThread::from_issue(&rounds_issue(ROUND_1_BODY, "open"), None, &git_info)
            .await
            .unwrap();

        let second = thread.round(2).unwrap();
        assert_eq!(second.branch, "main");
        assert!(second.branch_inherited);
        // Round 1 declares its branch in the body, so it never inherits.
        assert!(!thread.round(1).unwrap().branch_inherited);
    }

    #[test]
    fn test_round_comment_checklist_excludes_its_heading() {
        // F4: the section runs from the SECOND H1 to the end of the comment; later H1s
        // stay inside `content` as subsections (R5). D37: `content` excludes the `# `
        // line, so `POST /rounds` cannot emit it twice.
        let body = "# QC Round 2\n\n## Metadata\nround: 2\n\n# My Checklist\n- [ ] one\n\n# Sub\n- [x] two\n";
        let checklist = round_comment_checklist(body);
        assert_eq!(checklist.name, "My Checklist");
        assert!(checklist.content.starts_with("- [ ] one"));
        assert!(checklist.content.contains("# Sub"));
        assert_eq!(checklist.summary().total, 2);
    }

    #[test]
    fn test_round_comment_detection_keys_off_the_h1() {
        // D20: detection keys off the `# QC Round` H1, never off a metadata key.
        assert!(is_round_comment("# QC Round 3\n\nround: 3\n"));
        assert!(!is_round_comment("## QC Round 3\n"));
        assert!(!is_round_comment("new qc initial qc commit: abc1234\n"));
        assert_eq!(parse_round_number("round: 4\n"), Some(4));
    }

    #[test]
    fn test_parse_commit_from_pattern_full_sha() {
        let body = "approved qc commit: abc123def456789012345678901234567890abcd";

        let result = parse_commit_from_pattern(body, "approved qc commit: ");
        assert_eq!(result, Some("abc123def456789012345678901234567890abcd"));
    }

    #[test]
    fn test_parse_commit_from_pattern_short_sha() {
        let body = "current commit: abc123d";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("abc123d"));
    }

    #[test]
    fn test_parse_commit_from_pattern_minimum_length() {
        let body = "current commit: abc123";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("abc123"));
    }

    #[test]
    fn test_parse_commit_from_pattern_no_match() {
        let body = "current commit: nonexistent123";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("nonexistent123"));
    }

    #[test]
    fn test_parse_commit_from_pattern_not_found() {
        let body = "some other content";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, None);
    }

    /// D95: a round's checklist is user-editable text inside a comment the partition
    /// scan walks, so a checklist item that spells a marker verbatim must not be read
    /// as one. Rejected on **position**, not content — the sha here is well-formed.
    #[test]
    fn test_a_checklist_item_is_not_a_status_marker() {
        for pattern in [
            "approved qc commit: ",
            "current commit: ",
            "comparing commit: ",
            "initial qc commit: ",
        ] {
            let body = format!(
                "# Second Pass\n\n- [ ] {pattern}abc123def456789012345678901234567890abcd\n- [x] done\n"
            );
            assert_eq!(
                parse_commit_from_pattern(&body, pattern),
                None,
                "checklist item was read as a `{pattern}` marker"
            );
        }
    }

    /// The same guard covers prose: a marker has to *begin* its line.
    #[test]
    fn test_prose_mentioning_a_marker_is_not_a_marker() {
        let body = "I checked that the approved qc commit: abc123def456789012345678901234567890abcd looks right";

        assert_eq!(
            parse_commit_from_pattern(body, "approved qc commit: "),
            None
        );
    }

    /// Anchoring must not cost a real marker: these are the exact shapes the writers
    /// emit — a `metadata.join("\n* ")` bullet, and the bare key some bodies use.
    #[test]
    fn test_the_emitted_metadata_forms_still_parse() {
        let bulleted = format!(
            "# QC Approved\n\n## Metadata\n* approved qc commit: {}\n* [file](url)",
            "abc123def456789012345678901234567890abcd"
        );
        assert_eq!(
            parse_commit_from_pattern(&bulleted, "approved qc commit: "),
            Some("abc123def456789012345678901234567890abcd")
        );

        let bare = format!(
            "## Metadata\nround: 2\ninitial qc commit: {}",
            "abc123def456789012345678901234567890abcd"
        );
        assert_eq!(
            parse_commit_from_pattern(&bare, "initial qc commit: "),
            Some("abc123def456789012345678901234567890abcd")
        );
    }

    /// D20's collision, now closed at the source rather than by relying on H1
    /// detection to keep `# Previous QC` bodies away from this function.
    #[test]
    fn test_new_qc_initial_qc_commit_is_not_an_initial_qc_commit() {
        let body = format!(
            "# Previous QC\n\n## Metadata\n* new qc initial qc commit: {}",
            "abc123def456789012345678901234567890abcd"
        );

        assert_eq!(
            parse_commit_from_pattern(&body, "initial qc commit: "),
            None
        );
    }

    /// The end-to-end claim: a round declaration whose checklist spells an approval
    /// leaves the partition unapproved. Without the anchor this partition approves.
    #[test]
    fn test_parse_partition_ignores_an_approval_spelled_in_a_checklist() {
        let comments = vec![comment(
            "# QC Round 2\n\n## Metadata\n* round: 2\n* initial qc commit: abc123def456789012345678901234567890abcd\n* git branch: main\n\n# Second Pass\n\n- [ ] approved qc commit: abc123def456789012345678901234567890abcd\n",
        )];

        let parsed = parse_partition(&comments);

        assert_eq!(parsed.approval, None);
        assert!(parsed.statuses.is_empty());
    }

    fn comment(body: &str) -> GitComment {
        GitComment {
            id: None,
            body: body.to_string(),
            author_login: "test-user".to_string(),
            created_at: chrono::Utc::now(),
            html: None,
        }
    }

    #[test]
    fn test_parse_partition_with_approval() {
        let comments = vec![
            comment("current commit: abc123def456789012345678901234567890abcd"),
            comment("approved qc commit: def456789abc012345678901234567890123abcd"),
        ];

        let parsed = parse_partition(&comments);

        // The notification is a commit status; the approval is not (D33) — it is the
        // partition's terminal state (F6).
        assert_eq!(parsed.statuses.len(), 1);
        assert!(
            parsed.statuses["abc123def456789012345678901234567890abcd"]
                .contains(&CommitStatus::Notification)
        );
        assert_eq!(
            parsed.approval,
            Some(("def456789abc012345678901234567890123abcd", None))
        );
    }

    /// D36/U8: the approval carries the id of the comment that declared it. Without this
    /// the deep-link on the approved-commit row points at comment 0.
    #[test]
    fn test_parse_partition_carries_the_approval_comment_id() {
        let mut approval = comment("approved qc commit: abc123def456789012345678901234567890abcd");
        approval.id = Some(918_273);
        let comments = vec![comment("current commit: aaa"), approval];

        let parsed = parse_partition(&comments);

        assert_eq!(
            parsed.approval,
            Some(("abc123def456789012345678901234567890abcd", Some(918_273))),
            "the approval must carry its declaring comment's id (D36/U8)"
        );
    }

    /// An unapproval clears the id along with the sha (D11) — a stale id would deep-link
    /// to a revoked approval.
    #[test]
    fn test_parse_partition_unapproval_clears_the_comment_id() {
        let mut approval = comment("approved qc commit: abc123def456789012345678901234567890abcd");
        approval.id = Some(918_273);
        let comments = vec![approval, comment("# QC Un-Approval\n\nreason")];

        assert_eq!(parse_partition(&comments).approval, None);
    }

    #[test]
    fn test_parse_partition_notifications_only() {
        let comments = vec![
            comment("current commit: abc123def456789012345678901234567890abcd"),
            comment("current commit: def456789abc012345678901234567890123abcd"),
        ];

        let parsed = parse_partition(&comments);

        assert_eq!(parsed.statuses.len(), 2);
        assert!(
            parsed.statuses["abc123def456789012345678901234567890abcd"]
                .contains(&CommitStatus::Notification)
        );
        assert!(
            parsed.statuses["def456789abc012345678901234567890123abcd"]
                .contains(&CommitStatus::Notification)
        );
        assert_eq!(parsed.approval, None);
    }

    #[test]
    fn test_parse_partition_with_unapproval() {
        let comments = vec![
            comment("current commit: abc123def456789012345678901234567890abcd"),
            comment("approved qc commit: def456789abc012345678901234567890123abcd"),
            comment("# QC Un-Approval\nWithdrawing approval"),
        ];

        let parsed = parse_partition(&comments);

        // Because approvedness lives in exactly one place, an unapproval cannot leave
        // a stale flag behind (D33/F5).
        assert_eq!(parsed.approval, None);
        assert_eq!(parsed.statuses.len(), 1);
        assert!(
            parsed.statuses["abc123def456789012345678901234567890abcd"]
                .contains(&CommitStatus::Notification)
        );
    }

    #[test]
    fn test_parse_partition_with_review() {
        let comments = vec![
            comment("current commit: abc123def456789012345678901234567890abcd"),
            comment(
                "# QC Review\n@user\n\n## Metadata\ncomparing commit: def456789abc012345678901234567890123abcd\n[file at commit](url)",
            ),
        ];

        let parsed = parse_partition(&comments);

        assert_eq!(parsed.statuses.len(), 2);
        assert!(
            parsed.statuses["abc123def456789012345678901234567890abcd"]
                .contains(&CommitStatus::Notification)
        );
        assert!(
            parsed.statuses["def456789abc012345678901234567890123abcd"]
                .contains(&CommitStatus::Reviewed)
        );
    }

    #[test]
    fn test_parse_partition_notification_then_review() {
        let comments = vec![
            comment("current commit: abc123def456789012345678901234567890abcd"),
            comment(
                "# QC Review\n@user\n\n## Metadata\ncomparing commit: abc123def456789012345678901234567890abcd\n[file at commit](url)",
            ),
        ];

        let parsed = parse_partition(&comments);

        // Same commit, both statuses accumulate.
        assert_eq!(parsed.statuses.len(), 1);
        let statuses = &parsed.statuses["abc123def456789012345678901234567890abcd"];
        assert!(statuses.contains(&CommitStatus::Notification));
        assert!(statuses.contains(&CommitStatus::Reviewed));
    }

    #[test]
    fn test_parse_partition_review_then_approval() {
        let comments = vec![
            comment(
                "# QC Review\n@user\n\n## Metadata\ncomparing commit: abc123def456789012345678901234567890abcd\n[file at commit](url)",
            ),
            comment("approved qc commit: abc123def456789012345678901234567890abcd"),
        ];

        let parsed = parse_partition(&comments);

        assert_eq!(
            parsed.approval,
            Some(("abc123def456789012345678901234567890abcd", None))
        );
        assert!(
            parsed.statuses["abc123def456789012345678901234567890abcd"]
                .contains(&CommitStatus::Reviewed)
        );
    }

    #[test]
    fn test_parse_partition_multiple_reviews_same_commit() {
        let comments = vec![
            comment(
                "# QC Review\n@reviewer1\n\n## Metadata\ncomparing commit: abc123def456789012345678901234567890abcd\n[file at commit](url)",
            ),
            comment(
                "# QC Review\n@reviewer2\n\n## Metadata\ncomparing commit: abc123def456789012345678901234567890abcd\n[file at commit](url)",
            ),
        ];

        let parsed = parse_partition(&comments);

        assert_eq!(parsed.statuses.len(), 1);
        let statuses = &parsed.statuses["abc123def456789012345678901234567890abcd"];
        assert!(statuses.contains(&CommitStatus::Reviewed));
        assert!(!statuses.contains(&CommitStatus::Notification));
    }

    #[test]
    fn test_parse_branch_from_body_basic() {
        let body = "## Metadata\ninitial qc commit: abc123\ngit branch: feature/new-feature\nauthor: John Doe";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_with_extra_whitespace() {
        let body = "git branch:   main  \nother content";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_complex_branch_name() {
        let body = "git branch: feature/JIRA-123_fix-memory-leak\n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/JIRA-123_fix-memory-leak".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_not_found() {
        let body = "## Metadata\ninitial qc commit: abc123\nauthor: John Doe";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_empty_branch() {
        let body = "git branch: \n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_only_spaces() {
        let body = "git branch:    \n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link() {
        let body = "git branch: [feature/new-feature](https://github.com/owner/repo/tree/feature/new-feature)";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link_main() {
        let body = "git branch: [main](https://github.com/owner/repo) branch.";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link_complex_name() {
        let body = "git branch: [bugfix/JIRA-123_memory-leak](https://github.com/owner/repo/tree/bugfix/JIRA-123_memory-leak)";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("bugfix/JIRA-123_memory-leak".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link_http_ignored() {
        let body = "Check [https://example.com](https://example.com) for details.";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None); // Should ignore HTTP URLs
    }

    #[test]
    fn test_parse_branch_from_body_prefers_git_branch_pattern() {
        let body =
            "git branch: main\n\nSee also [develop](https://github.com/owner/repo/tree/develop)";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string())); // Should prefer git branch pattern
    }

    #[test]
    fn test_parse_branch_from_body_git_branch_markdown_link() {
        let body = "git branch: [main](https://github.com/A2-ai/ghqc_status_project2/tree/main)\nauthor: test";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_git_branch_html_link() {
        let body = r#"git branch: <a href="https://github.com/A2-ai/ghqc_status_project2/tree/main" target="_blank">main</a>
author: test"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_html_link_in_content() {
        let body = r#"git branch: <a href="https://github.com/owner/repo/tree/feature/new-feature">feature/new-feature</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_html_link_extract_from_url() {
        let body = r#"git branch: <a href="https://github.com/A2-ai/repo/tree/bugfix/memory-leak" target="_blank">file contents</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("file contents".to_string())); // Should extract link text, not URL
    }

    #[test]
    fn test_parse_branch_from_body_complex_example() {
        let body = r#"## Metadata

* initial qc commit: a7075606219a40c7536af8cd1b5f0b761965826c
* git branch: [main](https://github.com/A2-ai/ghqc_status_project2/tree/a7075606219a40c7536af8cd1b5f0b761965826c)
* author: jenna-a2ai <jenna@a2-ai.com>
* <a href="https://github.com/A2-ai/ghqc_status_project2/blob/a70756/dvs.yaml" target="_blank">file contents at initial qc commit</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_html_link_with_spaces_ignored() {
        let body = r#"<a href="https://docs.com">Code Review Process</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, None); // Should ignore links with spaces in text
    }

    // Tests for parse_blocking_qcs

    #[test]
    fn test_parse_blocking_qcs_from_body() {
        let body = r#"## Metadata

* initial qc commit: abc123
* git branch: main
* author: test

## Relevant Files

### Previous QC
- [previous.R](https://github.com/owner/repo/issues/123) - Previous version of this file
- [old_analysis.R](https://github.com/owner/repo/issues/124)

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/200) - Upstream dependency

### Relevant QC
- [related.R](https://github.com/owner/repo/issues/300)

# Code Review Checklist
- [ ] Check 1
- [ ] Check 2"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 3);

        // Check Previous QC entries
        let previous_qcs: Vec<_> = result
            .iter()
            .filter(|qc| qc.relationship == BlockingRelationship::PreviousQC)
            .collect();
        assert_eq!(previous_qcs.len(), 2);
        assert!(previous_qcs.iter().any(|qc| qc.issue_number == 123));
        assert!(previous_qcs.iter().any(|qc| qc.issue_number == 124));

        // Check Gating QC entry
        let gating_qcs: Vec<_> = result
            .iter()
            .filter(|qc| qc.relationship == BlockingRelationship::GatingQC)
            .collect();
        assert_eq!(gating_qcs.len(), 1);
        assert_eq!(gating_qcs[0].issue_number, 200);
        assert_eq!(gating_qcs[0].file_name, PathBuf::from("upstream.R"));
    }

    #[test]
    fn test_parse_blocking_qcs_empty() {
        let body = "## Metadata\n\n* commit: abc123\n\n# Checklist\n- [ ] Item";
        let result = parse_blocking_qcs(body);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_blocking_qcs_no_relevant_files_section() {
        let body = "## Metadata\n\nSome content without relevant files section";
        let result = parse_blocking_qcs(body);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_blocking_qcs_partial() {
        // Only Gating QC section present, no Previous QC
        let body = r#"## Relevant Files

### Gating QC
- [gating.R](https://github.com/owner/repo/issues/50)

### Relevant QC
- [other.R](https://github.com/owner/repo/issues/60)"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].issue_number, 50);
        assert_eq!(result[0].relationship, BlockingRelationship::GatingQC);
    }

    #[test]
    fn test_parse_blocking_qcs_ghe_url() {
        // Test with GitHub Enterprise URLs
        let body = r#"## Relevant Files

### Gating QC
- [script.R](https://ghe.company.com/org/repo/issues/42) - Enterprise issue"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].issue_number, 42);
        assert_eq!(result[0].file_name, PathBuf::from("script.R"));
    }

    #[test]
    fn test_parse_blocking_qcs_multiple_urls() {
        let body = r#"## Relevant Files

### Gating QC
- [file1.R](https://github.com/owner/repo/issues/1)
- [file2.R](https://github.com/owner/repo/issues/2)
- [file3.R](https://github.com/owner/repo/issues/3)"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 3);
        let issue_numbers: Vec<u64> = result.iter().map(|qc| qc.issue_number).collect();
        assert!(issue_numbers.contains(&1));
        assert!(issue_numbers.contains(&2));
        assert!(issue_numbers.contains(&3));
    }

    #[test]
    fn test_parse_blocking_qcs_extracts_file_name() {
        let body = r#"## Relevant Files

### Previous QC
- [path/to/complex-file_name.R](https://github.com/owner/repo/issues/99)"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].file_name,
            PathBuf::from("path/to/complex-file_name.R")
        );
    }

    // Tests for determine_relationship_from_body

    #[test]
    fn test_determine_relationship_from_child_body_gating() {
        let body = r#"## Relevant Files

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/50)

### Previous QC
- [old.R](https://github.com/owner/repo/issues/60)"#;

        let result = determine_relationship_from_body(body, 50);
        assert_eq!(result, BlockingRelationship::GatingQC);
    }

    #[test]
    fn test_determine_relationship_from_child_body_previous() {
        let body = r#"## Relevant Files

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/50)

### Previous QC
- [old.R](https://github.com/owner/repo/issues/60)"#;

        let result = determine_relationship_from_body(body, 60);
        assert_eq!(result, BlockingRelationship::PreviousQC);
    }

    #[test]
    fn test_determine_relationship_from_child_body_not_found() {
        let body = r#"## Relevant Files

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/50)"#;

        let result = determine_relationship_from_body(body, 999);
        assert_eq!(result, BlockingRelationship::Unknown);
    }

    #[test]
    fn test_determine_relationship_no_relevant_files() {
        let body = "## Metadata\n\nNo relevant files section";
        let result = determine_relationship_from_body(body, 123);
        assert_eq!(result, BlockingRelationship::Unknown);
    }

    // ── parse_file_history ────────────────────────────────────────────────────

    #[test]
    fn test_parse_file_history_no_section() {
        let body = "## Metadata\nsome content\n";
        let events = parse_file_history(body);
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_file_history_single_event() {
        let body = "## File History\n* `old/path.R` → `new/path.R` (commit: abc1234)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].old_path, "old/path.R");
        assert_eq!(events[0].new_path, "new/path.R");
        assert_eq!(events[0].commit, "abc1234");
    }

    #[test]
    fn test_parse_file_history_multiple_events() {
        let body = "## File History\n\
            * `a.R` → `b.R` (commit: 111aaaa)\n\
            * `b.R` → `c.R` (commit: 222bbbb)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].old_path, "a.R");
        assert_eq!(events[0].new_path, "b.R");
        assert_eq!(events[0].commit, "111aaaa");
        assert_eq!(events[1].old_path, "b.R");
        assert_eq!(events[1].new_path, "c.R");
        assert_eq!(events[1].commit, "222bbbb");
    }

    #[test]
    fn test_parse_file_history_malformed_lines_skipped() {
        let body = "## File History\n\
            * `good.R` → `better.R` (commit: abc0001)\n\
            * malformed line without backticks\n\
            * `missing_arrow.R` something wrong\n\
            * `also_good.R` → `also_better.R` (commit: abc0002)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].old_path, "good.R");
        assert_eq!(events[1].old_path, "also_good.R");
    }

    #[test]
    fn test_parse_file_history_terminates_at_next_section() {
        let body = "## File History\n\
            * `old.R` → `new.R` (commit: abc1234)\n\
            ## Other Section\n\
            * `should_not.R` → `be_parsed.R` (commit: 000000)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].old_path, "old.R");
    }

    // ── file_history_section ──────────────────────────────────────────────────

    #[test]
    fn test_file_history_section_empty_slice() {
        let section = file_history_section(&[]);
        assert_eq!(section, "## File History\n");
    }

    #[test]
    fn test_file_history_section_single_event() {
        let events = vec![FileRenameEvent {
            old_path: "src/old.R".to_string(),
            new_path: "src/new.R".to_string(),
            commit: "deadbeef".to_string(),
        }];
        let section = file_history_section(&events);
        assert_eq!(
            section,
            "## File History\n* `src/old.R` → `src/new.R` (commit: deadbeef)\n"
        );
    }

    #[test]
    fn test_file_history_section_multiple_events() {
        let events = vec![
            FileRenameEvent {
                old_path: "a.R".to_string(),
                new_path: "b.R".to_string(),
                commit: "aaa1111".to_string(),
            },
            FileRenameEvent {
                old_path: "b.R".to_string(),
                new_path: "c.R".to_string(),
                commit: "bbb2222".to_string(),
            },
        ];
        let section = file_history_section(&events);
        assert!(section.starts_with("## File History\n"));
        assert!(section.contains("* `a.R` → `b.R` (commit: aaa1111)\n"));
        assert!(section.contains("* `b.R` → `c.R` (commit: bbb2222)\n"));
    }

    // ── find_checklist_start ──────────────────────────────────────────────────

    #[test]
    fn test_find_checklist_start_finds_h1() {
        let body = "## Metadata\nsome text\n# Checklist\n- item\n";
        let pos = find_checklist_start(body);
        assert!(pos.is_some());
        let offset = pos.unwrap();
        assert!(body[offset..].starts_with("# Checklist"));
    }

    #[test]
    fn test_find_checklist_start_only_h2_returns_none() {
        let body = "## Metadata\n## File History\n## Another\n";
        let pos = find_checklist_start(body);
        assert!(pos.is_none());
    }

    #[test]
    fn test_find_checklist_start_empty_body() {
        let pos = find_checklist_start("");
        assert!(pos.is_none());
    }

    // ── checklist content is trimmed of blank lines ───────────────────────────

    /// The section is cut at its heading and runs to the end of the body, so the blank
    /// line after the heading and the body's trailing newlines are artefacts of the cut.
    #[test]
    fn test_checklist_content_drops_leading_and_trailing_blank_lines() {
        let checklist =
            RoundChecklist::from_section("# My list\n\n\n- [ ] one\n- [ ] two\n\n   \n\n");
        assert_eq!(checklist.name, "My list");
        assert_eq!(checklist.content, "- [ ] one\n- [ ] two");
    }

    /// Lines, not characters: an interior blank line separates `## ` subsections (R5),
    /// and leading whitespace on the first real line is a nested item.
    #[test]
    fn test_checklist_content_keeps_interior_blanks_and_indentation() {
        let checklist =
            RoundChecklist::from_section("# L\n\n  - [ ] nested\n\n## Part two\n\n- [ ] b\n");
        assert_eq!(
            checklist.content,
            "  - [ ] nested\n\n## Part two\n\n- [ ] b"
        );
    }

    /// An all-blank section is empty content, not a string of newlines — `summary()`
    /// counts checkboxes either way, but the editable content the UI seeds from it
    /// should not open with phantom lines.
    #[test]
    fn test_checklist_content_all_blank_is_empty() {
        assert_eq!(RoundChecklist::from_section("# L\n\n \n\n").content, "");
        assert_eq!(RoundChecklist::from_section("# L\n").content, "");
        assert_eq!(RoundChecklist::from_section("# L").content, "");
    }

    // ── splice_file_history ───────────────────────────────────────────────────

    #[test]
    fn test_splice_file_history_no_section_with_checklist() {
        let body = "## Metadata\nsome text\n\n# Checklist\n- [ ] item\n";
        let history = "## File History\n* `old.R` → `new.R` (commit: abc)\n";
        let result = splice_file_history(body, history);
        // History must appear before the checklist
        let history_pos = result.find("## File History").expect("history missing");
        let checklist_pos = result.find("# Checklist").expect("checklist missing");
        assert!(history_pos < checklist_pos);
    }

    #[test]
    fn test_splice_file_history_no_section_no_checklist() {
        let body = "## Metadata\nsome text\n";
        let history = "## File History\n* `old.R` → `new.R` (commit: abc)\n";
        let result = splice_file_history(body, history);
        // History appended at end
        assert!(result.contains("## File History"));
        assert!(result.ends_with("## File History\n* `old.R` → `new.R` (commit: abc)"));
    }

    // ── D49/D51: the `## QC Rounds` marker ─────────────────────────────────────

    /// D49.2: an **H2**, immediately before the checklist H1. An H1 here would be
    /// returned by `find_checklist_start` and parsed as round 1's checklist.
    #[test]
    fn test_qc_rounds_section_is_an_h2_spliced_before_the_checklist_h1() {
        let body = "## Metadata\ninitial qc commit: abc\ngit branch: main\n\n# Checklist One\n- [ ] item\n";
        let spliced = splice_qc_rounds(body, &qc_rounds_section(2));

        let marker = spliced.find("## QC Rounds").expect("the marker is present");
        let checklist = find_checklist_start(&spliced).expect("the checklist H1 survives");
        assert!(
            marker < checklist,
            "the marker precedes the checklist H1: {spliced}"
        );

        // Still an H2: the first H1 is still the checklist, so the fold's round-1
        // checklist is unchanged.
        assert_eq!(spliced[checklist..].lines().next(), Some("# Checklist One"));
        assert!(
            spliced
                .lines()
                .any(|line| line.trim_end() == "## QC Rounds"),
            "the heading is its own line: {spliced}"
        );

        // D49.3: prose, not a metadata key — nothing here can collide with the round
        // comment's `round: {N}` (D20) or with the body's own keys.
        let section = qc_rounds_section(2);
        assert!(!section.contains("round: "));
        assert!(!section.contains("initial qc commit: "));
        assert!(!section.contains("git branch: "));
    }

    /// A second round rewrites the section in place — one marker, never a stack of
    /// them — and leaves both neighbours intact.
    #[test]
    fn test_splice_qc_rounds_replaces_in_place() {
        let body = "## Metadata\ninitial qc commit: abc\n\n# Checklist One\n- [ ] item\n";
        let once = splice_qc_rounds(body, &qc_rounds_section(2));
        let twice = splice_qc_rounds(&once, &qc_rounds_section(3));

        assert_eq!(twice.matches("## QC Rounds").count(), 1);
        assert!(twice.contains("3 QC rounds"));
        assert!(!twice.contains("2 QC rounds"));
        assert!(twice.contains("## Metadata"));
        assert!(twice.contains("# Checklist One\n- [ ] item\n"));
        assert_eq!(find_checklist_start(&twice).is_some(), true);
    }

    /// It coexists with `## File History`, which occupies the same slot.
    #[test]
    fn test_splice_qc_rounds_coexists_with_file_history() {
        let body = "## Metadata\ninitial qc commit: abc\n\n## File History\n* `a` → `b` (commit: 111)\n\n# Checklist One\n- [ ] item\n";
        let spliced = splice_qc_rounds(body, &qc_rounds_section(2));

        assert_eq!(spliced.matches("## File History").count(), 1);
        assert_eq!(spliced.matches("## QC Rounds").count(), 1);
        assert_eq!(parse_file_history(&spliced).len(), 1);
        assert_eq!(
            find_checklist_start(&spliced).map(|offset| spliced[offset..].lines().next()),
            Some(Some("# Checklist One"))
        );
    }

    /// D51: presence is the one machine-readable fact — and it is *presence*, not the
    /// count in the prose.
    #[test]
    fn test_has_qc_rounds_marker_reads_presence_only() {
        assert!(!has_qc_rounds_marker(ROUND_1_BODY));
        assert!(has_qc_rounds_marker(&splice_qc_rounds(
            ROUND_1_BODY,
            &qc_rounds_section(2)
        )));
        // A `## QC Rounds` mention inside prose is not the heading.
        assert!(!has_qc_rounds_marker(
            "see the ## QC Rounds section below\n"
        ));
    }

    /// **D49.5 / D51 — the safeguard.** The marker is inert: the fold's output must be
    /// identical with the section absent, present, and **deliberately wrong**. This is
    /// what stops "it's only a hint" from drifting into authority.
    #[tokio::test]
    async fn test_the_qc_rounds_marker_is_inert_to_the_fold() {
        // Two real rounds, declared the only way rounds are declared: comments (D3).
        let comments = vec![
            comment(&format!("approved qc commit: {}", oid(2))),
            round_2_comment(4, "main", "# Checklist Two\n- [ ] x\n"),
            comment(&format!("current commit: {}", oid(5))),
        ];

        let fold = |body: &str| {
            let comments = comments.clone();
            let body = body.to_string();
            async move {
                let git_info = SimpleMockGitInfo::new()
                    .with_commits(walk(&[5, 4, 3, 2, 1]))
                    .with_comments(comments);
                IssueThread::from_issue(&rounds_issue(&body, "open"), None, &git_info)
                    .await
                    .unwrap()
            }
        };

        let absent = fold(ROUND_1_BODY).await;

        let present = fold(&splice_qc_rounds(ROUND_1_BODY, &qc_rounds_section(2))).await;

        // Deliberately wrong: claims five rounds when two exist, and repeats the
        // round-comment vocabulary as prose.
        let wrong = fold(&splice_qc_rounds(
            ROUND_1_BODY,
            "## QC Rounds\nThis QC has been through 5 QC rounds. QC Round 5 is the current one.\n",
        ))
        .await;

        assert_eq!(
            absent.rounds.len(),
            2,
            "the comments are the authority (D3)"
        );
        assert_eq!(
            present, absent,
            "a present marker must not change the fold's output (D49.5)"
        );
        assert_eq!(
            wrong, absent,
            "a wrong marker must not change the fold's output either (D49.5)"
        );
    }

    #[test]
    fn test_splice_file_history_replaces_existing_section() {
        let body = "## Metadata\nsome text\n\n## File History\n* `old.R` → `mid.R` (commit: 111)\n\n# Checklist\n- [ ] item\n";
        let new_history = "## File History\n* `old.R` → `mid.R` (commit: 111)\n* `mid.R` → `new.R` (commit: 222)\n";
        let result = splice_file_history(body, new_history);
        // Only one File History section
        assert_eq!(result.matches("## File History").count(), 1);
        assert!(result.contains("commit: 222"));
    }

    #[test]
    fn test_splice_file_history_existing_section_at_end() {
        let body = "## Metadata\nsome text\n\n## File History\n* `old.R` → `new.R` (commit: abc)\n";
        let new_history = "## File History\n* `old.R` → `new.R` (commit: abc)\n* `new.R` → `newest.R` (commit: def)\n";
        let result = splice_file_history(body, new_history);
        assert_eq!(result.matches("## File History").count(), 1);
        assert!(result.contains("commit: def"));
    }
}
