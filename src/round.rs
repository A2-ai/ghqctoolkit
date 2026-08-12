//! QC Rounds: a derived view of an issue's comment thread as a sequence of rounds.
//!
//! A QC issue is a sequence of *rounds*. Round 1 is "Initial QC": it opens at the
//! `initial qc commit:` recorded in the issue body and closes when an approval is
//! posted. A later round is opened explicitly by a `# QC New Round` comment, which
//! carries its own anchor commit (`round commit:`) and its own checklist.
//!
//! The fold trusts written metadata only for values it cannot derive from the
//! comment log. The anchor is such a value, so it is authoritative; the round
//! number and the approval a round builds on are both derivable, so they are always
//! derived. A `# QC New Round` comment that cannot open a new round — because the
//! current round is still open, or because its written `round:` disagrees with the
//! derived number — instead *extends* the current round, so its author-written
//! checklist is never discarded.
//!
//! Metadata keys are only read inside a comment's `## Metadata` section, and
//! markers are only recognised at the start of a line.
//!
//! Everything in this module is **derived and additive**. Nothing here changes how
//! [`crate::IssueThread::latest_commit`], [`crate::IssueThread::approved_commit`] or
//! [`crate::QCStatus`] behave.
//!
//! The fold is deliberately split into two stages so the interesting logic is
//! testable without a git repository:
//!
//! 1. [`fold_rounds_from_comments`] is a pure function over `&[GitComment]` that
//!    works on SHA strings exactly as they appear in the comments.
//! 2. [`resolve_rounds`] resolves those strings against the already-built
//!    `Vec<IssueCommit>` using the same short-SHA prefix matching as
//!    `IssueThread::from_issue_comments`.

use chrono::{DateTime, Utc};
use gix::ObjectId;

use crate::git::GitComment;
use crate::issue::IssueCommit;

/// Comment marker opening a new QC round.
pub(crate) const NEW_ROUND_MARKER: &str = "# QC New Round";
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
/// Metadata key holding the 1-based round number of a `# QC New Round` comment.
pub(crate) const ROUND_KEY: &str = "round: ";
/// Metadata key holding the anchor commit of a `# QC New Round` comment.
pub(crate) const ROUND_COMMIT_KEY: &str = "round commit: ";
/// Metadata key holding the approval the new round builds on.
pub(crate) const PREVIOUS_APPROVED_COMMIT_KEY: &str = "previous approved commit: ";
/// Metadata key holding a free-text note on a `# QC New Round` comment.
pub(crate) const NOTE_KEY: &str = "note: ";

/// How a round came into being.
#[derive(Debug, Clone, PartialEq)]
pub enum RoundOpen {
    /// Round 1 ("Initial QC"), opened when the issue was created.
    IssueCreated,
    /// Round N > 1, opened by a `# QC New Round` comment.
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
    },
}

/// A `# QC New Round` comment that could not open a new round and therefore
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

/// Why a `# QC New Round` comment extended the current round instead of opening
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
    /// Round N > 1: the checklist is in the `# QC New Round` comment.
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
    /// Indices are therefore unique and monotonic, and `index - 1` is the round's
    /// position in [`crate::IssueThread::rounds`].
    pub index: u32,
    /// The commit the file was at when this round opened. Taken from the comment's
    /// `round commit:` (the one value the comment log cannot derive), and never
    /// moved by a later extension.
    pub opened_at: ObjectId,
    /// The approval this round builds on: always **derived** as the previous
    /// round's closing commit. `None` for Initial QC.
    pub previous_approval: Option<ObjectId>,
    pub opened: RoundOpen,
    pub checklist: ChecklistSource,
    pub state: RoundState,
    pub events: Vec<RoundEvent>,
    pub retractions: Vec<Retraction>,
    /// `# QC New Round` comments that extended this round rather than opening a
    /// new one, oldest first.
    pub extensions: Vec<Extension>,
}

impl Round {
    /// Human-readable name of the round.
    pub fn name(&self) -> String {
        if self.index == 1 {
            "Initial QC".to_string()
        } else {
            format!("Round {}", self.index)
        }
    }

    /// The commit that closed this round, if it is currently closed.
    pub fn closing_commit(&self) -> Option<&ObjectId> {
        match &self.state {
            RoundState::Closed { commit, .. } => Some(commit),
            RoundState::Open => None,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, RoundState::Open)
    }
}

/// A problem found while folding rounds. The fold never fails; it accumulates
/// anomalies instead so callers can surface them without losing the derived state.
/// Anomalies are diagnostics, so they identify comments by index only — no
/// comment id or URL is carried here.
#[derive(Debug, Clone, PartialEq)]
pub enum RoundAnomaly {
    /// A `# QC New Round` comment could not open a new round, so it extended the
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

/// [`Round`] before any SHA has been resolved to an [`ObjectId`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawRound<'a> {
    pub index: u32,
    pub opened_at: &'a str,
    pub previous_approval: Option<&'a str>,
    pub opened: RawRoundOpen<'a>,
    pub checklist: ChecklistSource,
    pub state: RawRoundState<'a>,
    pub events: Vec<RawRoundEvent<'a>>,
    pub retractions: Vec<RawRetraction<'a>>,
    pub extensions: Vec<RawExtension<'a>>,
}

/// Heading that opens the metadata block every QC comment renders.
const METADATA_HEADING: &str = "## Metadata";

/// The slice of `body` that belongs to its `## Metadata` section: everything after
/// a line equal to `## Metadata` up to the next line beginning with `## ` (or the
/// end of the body). Empty if the body has no metadata section.
///
/// Metadata keys are only ever looked up inside this slice, so text that merely
/// *looks* like metadata elsewhere in the comment — a `current commit: ` line
/// inside an inlined `## File Difference` diff, for instance — is never parsed.
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
                if line.starts_with("## ") {
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

/// Whether any line of `body` begins with `marker`, ignoring leading whitespace.
///
/// Anchoring at the start of a line means a quoted `> # QC New Round` does not
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
/// A thread with no `# QC New Round` markers always folds to exactly one round.
/// There is deliberately no legacy reinterpretation: un-approvals in such a thread
/// are plain retractions on round 1.
pub(crate) fn fold_rounds_from_comments<'a>(
    initial_commit_sha: &'a str,
    comments: &'a [GitComment],
) -> (Vec<RawRound<'a>>, Vec<RawAnomaly>) {
    let mut anomalies: Vec<RawAnomaly> = Vec::new();
    let mut rounds: Vec<RawRound<'a>> = vec![RawRound {
        index: 1,
        opened_at: initial_commit_sha,
        previous_approval: None,
        opened: RawRoundOpen::IssueCreated,
        checklist: ChecklistSource::IssueBody,
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
        if has_marker(body, NEW_ROUND_MARKER) {
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
                        "comment {comment_index}: '{NEW_ROUND_MARKER}' extends round {} ({reason:?})",
                        cur.index
                    );
                    cur.state = RawRoundState::Open;
                    cur.checklist = ChecklistSource::Comment {
                        comment_index,
                        comment_id,
                        comment_url: comment_url.map(|url| url.to_string()),
                    };
                    cur.extensions.push(RawExtension {
                        comment_index,
                        comment_id,
                        comment_url,
                        by: author,
                        at,
                        at_commit: metadata_commit(metadata, ROUND_COMMIT_KEY),
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
                        metadata_commit(metadata, ROUND_COMMIT_KEY).unwrap_or(derived_base);

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
                        },
                        checklist: ChecklistSource::Comment {
                            comment_index,
                            comment_id,
                            comment_url: comment_url.map(|url| url.to_string()),
                        },
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

// ── Stage 2: resolve SHAs against the issue's commits ───────────────────────

/// Resolve a comment SHA against the issue's commits using the same matching
/// semantics as `IssueThread::from_issue_comments`: exact match, or a prefix match
/// for abbreviated SHAs of at least 7 characters.
fn resolve_sha(sha: &str, commits: &[IssueCommit]) -> Option<ObjectId> {
    commits
        .iter()
        .find(|commit| {
            let full = commit.hash.to_string();
            full == sha || (sha.len() >= 7 && full.starts_with(sha))
        })
        .map(|commit| commit.hash)
}

/// Position of a resolved commit in `commits` (newest-first), or `usize::MAX` if it
/// is somehow absent, so an unknown commit sorts as the oldest possible.
fn commit_position(commits: &[IssueCommit], hash: &ObjectId) -> usize {
    commits
        .iter()
        .position(|commit| commit.hash == *hash)
        .unwrap_or(usize::MAX)
}

/// Resolve raw rounds against the issue's resolved commit list.
///
/// `commits` is ordered newest-first, matching [`crate::IssueThread::commits`].
///
/// Unresolvable SHAs never drop a round. Instead:
/// * an unresolvable **anchor** emits [`RoundAnomaly::AnchorUnreachable`] and falls
///   back to the oldest known commit, which is the widest possible membership and
///   therefore the least likely to silently hide commits from a reviewer;
/// * an unresolvable **closing commit** emits `AnchorUnreachable` and leaves the
///   round `Open`, since `Closed` cannot be represented without an `ObjectId`;
/// * unresolvable **event** and **retraction** commits emit `AnchorUnreachable` and
///   are dropped.
///
/// If `commits` is empty nothing can be resolved and no rounds are emitted.
pub(crate) fn resolve_rounds(
    raw_rounds: Vec<RawRound<'_>>,
    mut anomalies: Vec<RawAnomaly>,
    commits: &[IssueCommit],
) -> (Vec<Round>, Vec<RoundAnomaly>) {
    if commits.is_empty() {
        return (Vec::new(), anomalies);
    }
    // Oldest commit: `commits` is newest-first.
    let oldest = commits[commits.len() - 1].hash;

    let mut rounds = Vec::with_capacity(raw_rounds.len());
    for raw in raw_rounds {
        let anchor_comment_index = match &raw.opened {
            RawRoundOpen::NewRound { comment_index, .. } => *comment_index,
            RawRoundOpen::IssueCreated => 0,
        };

        let opened_at = match resolve_sha(raw.opened_at, commits) {
            Some(id) => id,
            None => {
                log::debug!(
                    "round {}: anchor {} unresolvable; falling back to oldest commit",
                    raw.index,
                    raw.opened_at
                );
                anomalies.push(RoundAnomaly::AnchorUnreachable {
                    comment_index: anchor_comment_index,
                    sha: raw.opened_at.to_string(),
                });
                oldest
            }
        };

        let previous_approval = raw.previous_approval.and_then(|sha| {
            let resolved = resolve_sha(sha, commits);
            if resolved.is_none() {
                anomalies.push(RoundAnomaly::AnchorUnreachable {
                    comment_index: anchor_comment_index,
                    sha: sha.to_string(),
                });
            }
            resolved
        });

        // Sanity guard: the anchor is authoritative, but an anchor older than the
        // round's base (a larger index in the newest-first `commits`) would make
        // this round's membership overlap the previous round's. Fall back to the
        // base so every commit still belongs to at most one round.
        let opened_at = match previous_approval {
            Some(base)
                if commit_position(commits, &opened_at) > commit_position(commits, &base) =>
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

        let state = match raw.state {
            RawRoundState::Open => RoundState::Open,
            RawRoundState::Closed {
                commit,
                by,
                at,
                comment_index,
                comment_id,
                comment_url,
            } => match resolve_sha(commit, commits) {
                Some(id) => RoundState::Closed {
                    commit: id,
                    by: by.to_string(),
                    at,
                    comment_index,
                    comment_id,
                    comment_url: comment_url.map(|url| url.to_string()),
                },
                None => {
                    anomalies.push(RoundAnomaly::AnchorUnreachable {
                        comment_index,
                        sha: commit.to_string(),
                    });
                    RoundState::Open
                }
            },
        };

        let mut events = Vec::with_capacity(raw.events.len());
        for event in raw.events {
            let (commit, by, at, comment_index, comment_id, comment_url, is_notification) =
                match event {
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
            match resolve_sha(commit, commits) {
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
            match resolve_sha(retraction.retracted_commit, commits) {
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
            } => RoundOpen::NewRound {
                comment_index,
                comment_id,
                comment_url: comment_url.map(|url| url.to_string()),
                author: author.to_string(),
                at,
                note: note.map(|n| n.to_string()),
            },
        };

        // An extension's `at_commit` is informational only, so an unresolvable one
        // is simply dropped rather than reported.
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
                    .and_then(|sha| resolve_sha(sha, commits)),
                note: extension.note.map(|note| note.to_string()),
            })
            .collect();

        rounds.push(Round {
            index: raw.index,
            opened_at,
            previous_approval,
            opened,
            checklist: raw.checklist,
            state,
            events,
            retractions,
            extensions,
        });
    }

    (rounds, anomalies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::{CommitStatus, IssueThread};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";
    const D: &str = "ddddddd000000000000000000000000000000004";
    const E: &str = "eeeeeee000000000000000000000000000000005";

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

    /// `# QC New Round` with an arbitrary (possibly non-numeric) `round:` value.
    fn new_round_raw(round: &str, round_commit: &str, previous: &str) -> GitComment {
        comment(&format!(
            "# QC New Round\n\n## Metadata\nround: {round}\nround commit: {round_commit}\nprevious approved commit: {previous}\nnote: second pass\n\n# Checklist\n- [ ] item\n"
        ))
    }

    /// Build an `IssueThread` directly: rounds fold over comments, so no git is needed.
    /// `shas` is oldest-first here for readability and reversed into newest-first.
    fn thread(shas: &[&str], initial: &str, comments: &[GitComment]) -> IssueThread {
        let commits: Vec<IssueCommit> = shas
            .iter()
            .rev()
            .map(|sha| IssueCommit {
                hash: ObjectId::from_str(sha).unwrap(),
                message: format!("commit {sha}"),
                statuses: if *sha == initial {
                    HashSet::from([CommitStatus::Initial])
                } else {
                    HashSet::new()
                },
                file_changed: true,
            })
            .collect();
        let (raw, raw_anomalies) = fold_rounds_from_comments(initial, comments);
        let (rounds, round_anomalies) = resolve_rounds(raw, raw_anomalies, &commits);
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            branch: "main".to_string(),
            open: true,
            commits,
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            rounds,
            round_anomalies,
        }
    }

    // ── Stage 1 fold rules ───────────────────────────────────────────────────

    #[test]
    fn legacy_notifications_only_folds_to_one_open_round() {
        let comments = vec![notification(B), notification(C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

        assert_eq!(rounds.len(), 1);
        assert!(anomalies.is_empty());
        assert_eq!(rounds[0].state, RawRoundState::Open);
        assert_eq!(rounds[0].retractions.len(), 1);
        assert_eq!(rounds[0].retractions[0].retracted_commit, B);
        assert_eq!(rounds[0].retractions[0].comment_index, 2);

        let thread = thread(&[A, B], A, &comments);
        assert_eq!(thread.rounds.len(), 1);
        assert_eq!(thread.latest_standing_approval(), None);
        assert!(thread.open_round().is_some());
    }

    #[test]
    fn second_unapproval_is_idempotent_and_flagged() {
        let comments = vec![approval(B), unapproval(), unapproval()];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        assert_eq!(thread.rounds.len(), 1);
        assert_eq!(thread.latest_standing_approval(), None);
        assert_eq!(thread.rounds[0].extensions.len(), 1);
        assert_eq!(
            thread.rounds[0].extensions[0].at_commit,
            Some(ObjectId::from_str(D).unwrap())
        );
    }

    #[test]
    fn new_round_base_is_derived_and_written_mismatch_is_warn_only() {
        // Matching `round:`, but the comment wrote the wrong previous approval.
        let comments = vec![approval(B), new_round(2, D, C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let body = format!("# QC New Round\n\n## Metadata\nround commit: {D}\n");
        let comments = vec![approval(B), comment(&body)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");
        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[1].index, 2);
        assert_eq!(rounds[1].previous_approval, Some(B));
        assert_eq!(rounds[1].opened_at, D);
    }

    #[test]
    fn event_after_close_is_recorded_and_flagged() {
        let comments = vec![approval(B), notification(C)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, _) = fold_rounds_from_comments(A, &comments);
        assert_eq!(rounds[0].events.len(), 2);
        assert!(matches!(
            rounds[0].events[1],
            RawRoundEvent::Review { commit, .. } if commit == B
        ));
    }

    // ── Stage 2 resolution ───────────────────────────────────────────────────

    #[test]
    fn short_shas_resolve_in_stage_two() {
        let short = &B[..7];
        let comments = vec![notification(short), approval(short)];
        let thread = thread(&[A, B], A, &comments);

        assert_eq!(thread.rounds.len(), 1);
        assert!(thread.round_anomalies.is_empty());
        let expected = ObjectId::from_str(B).unwrap();
        assert_eq!(thread.latest_standing_approval(), Some(&expected));
        assert_eq!(thread.latest_notified_commit(), Some(&expected));
    }

    #[test]
    fn unresolvable_anchor_falls_back_to_oldest_commit() {
        let missing = "fffffff000000000000000000000000000000009";
        let comments: Vec<GitComment> = vec![];
        let thread = thread(&[A, B], missing, &comments);

        assert_eq!(thread.rounds.len(), 1);
        assert_eq!(thread.rounds[0].opened_at, ObjectId::from_str(A).unwrap());
        assert_eq!(
            thread.round_anomalies,
            vec![RoundAnomaly::AnchorUnreachable {
                comment_index: 0,
                sha: missing.to_string(),
            }]
        );
    }

    // ── Derived accessors ────────────────────────────────────────────────────

    #[test]
    fn next_notification_from_falls_back_to_initial_commit() {
        let comments: Vec<GitComment> = vec![];
        let thread = thread(&[A, B], A, &comments);
        assert_eq!(
            thread.next_notification_from(),
            ObjectId::from_str(A).unwrap()
        );
    }

    #[test]
    fn next_notification_from_uses_notified_commit() {
        let comments = vec![notification(B)];
        let thread = thread(&[A, B, C], A, &comments);
        assert_eq!(
            thread.next_notification_from(),
            ObjectId::from_str(B).unwrap()
        );
    }

    #[test]
    fn next_notification_from_prefers_newer_review() {
        let comments = vec![notification(B), review(C)];
        let thread = thread(&[A, B, C, D], A, &comments);
        assert_eq!(
            thread.next_notification_from(),
            ObjectId::from_str(C).unwrap()
        );
        assert_eq!(
            thread.latest_reviewed_commit(),
            Some(&ObjectId::from_str(C).unwrap())
        );
    }

    #[test]
    fn next_notification_from_open_round_two_without_notification_uses_previous_approval() {
        // A = initial, B = approved (closes round 1), C = draft gap, D = round 2 anchor,
        // E = newer work. Round 2 has no notification yet, so the previous approval (B)
        // must win: the round anchor D is deliberately NOT a candidate.
        let comments = vec![notification(B), approval(B), new_round(2, D, B)];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        assert_eq!(thread.rounds.len(), 2);
        assert!(thread.rounds[1].is_open());
        assert_eq!(
            thread.next_notification_from(),
            ObjectId::from_str(B).unwrap()
        );
    }

    #[test]
    fn membership_and_draft_gap_split_commits_between_rounds() {
        let comments = vec![notification(B), approval(B), new_round(2, D, B)];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        // Round 1: after A (exclusive) through its closing commit B.
        let first: Vec<String> = thread
            .round_membership(0)
            .iter()
            .map(|c| c.hash.to_string())
            .collect();
        assert_eq!(first, vec![B.to_string()]);

        // Round 2 is open: after D (exclusive) through the newest commit E.
        let second: Vec<String> = thread
            .round_membership(1)
            .iter()
            .map(|c| c.hash.to_string())
            .collect();
        assert_eq!(second, vec![E.to_string()]);

        // C sits strictly between round 1's approval (B) and round 2's anchor (D).
        let gap: Vec<String> = thread
            .draft_gap(1)
            .iter()
            .map(|c| c.hash.to_string())
            .collect();
        assert_eq!(gap, vec![C.to_string()]);

        // Initial QC has no previous approval, so it can never have a draft gap.
        assert!(thread.rounds[0].previous_approval.is_none());
        assert!(thread.draft_gap(0).is_empty());
        // Out-of-range positions are empty rather than panicking.
        assert!(thread.round_membership(2).is_empty());
        assert!(thread.draft_gap(2).is_empty());
    }

    #[test]
    fn latest_standing_approval_ignores_retracted_and_earlier_rounds() {
        let comments = vec![approval(B), new_round(2, C, B), approval(E), unapproval()];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        assert_eq!(thread.rounds.len(), 2);
        // Round 2 was reopened, so the only standing approval is round 1's.
        assert_eq!(
            thread.latest_standing_approval(),
            Some(&ObjectId::from_str(B).unwrap())
        );
        assert_eq!(thread.open_round().map(|r| r.index), Some(2));
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
        let (rounds, _) = fold_rounds_from_comments(A, &comments);

        let indices: Vec<u32> = rounds.iter().map(|r| r.index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
        assert!(indices.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(rounds[1].extensions.len(), 1);
    }

    #[test]
    fn extending_a_closed_round_does_not_record_a_retraction() {
        let comments = vec![approval(B), new_round(9, D, B)];
        let (rounds, _) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

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
        let (rounds, _) = fold_rounds_from_comments(A, &comments);
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

    // ── Marker matching ──────────────────────────────────────────────────────

    #[test]
    fn quoted_marker_does_not_trigger_and_split_header_does() {
        let quoted =
            format!("Replying:\n\n> # QC New Round\n\n## Metadata\nround: 2\nround commit: {C}\n");
        assert!(!has_marker(&quoted, NEW_ROUND_MARKER));
        let comments = vec![approval(B), comment(&quoted)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);
        assert_eq!(rounds.len(), 1, "a quoted marker must not open a round");
        assert!(anomalies.is_empty());

        let split = format!("# QC Notification (2/3)\n\n## Metadata\ncurrent commit: {B}\n");
        assert!(has_marker(&split, NOTIFICATION_MARKER));
        let split_comments = vec![comment(&split)];
        let (rounds, _) = fold_rounds_from_comments(A, &split_comments);
        assert_eq!(rounds[0].events.len(), 1);
    }

    // ── Anchor sanity guard ──────────────────────────────────────────────────

    #[test]
    fn anchor_older_than_base_falls_back_and_memberships_do_not_overlap() {
        // Round 1 closes at D; the new round writes an anchor (B) older than D.
        let comments = vec![approval(D), new_round(2, B, D)];
        let thread = thread(&[A, B, C, D, E], A, &comments);

        assert_eq!(thread.rounds.len(), 2);
        assert!(
            thread
                .round_anomalies
                .contains(&RoundAnomaly::AnchorOlderThanBase {
                    comment_index: 1,
                    anchor: B.to_string(),
                    base: D.to_string(),
                })
        );
        assert_eq!(thread.rounds[1].opened_at, ObjectId::from_str(D).unwrap());

        let first: Vec<String> = thread
            .round_membership(0)
            .iter()
            .map(|c| c.hash.to_string())
            .collect();
        let second: Vec<String> = thread
            .round_membership(1)
            .iter()
            .map(|c| c.hash.to_string())
            .collect();
        assert_eq!(first, vec![D.to_string(), C.to_string(), B.to_string()]);
        assert_eq!(second, vec![E.to_string()]);
        assert!(
            first.iter().all(|sha| !second.contains(sha)),
            "memberships must not overlap: {first:?} vs {second:?}"
        );
    }

    #[test]
    fn short_previous_approved_commit_is_not_a_base_mismatch() {
        let body = format!(
            "# QC New Round\n\n## Metadata\nround: 2\nround commit: {}\nprevious approved commit: {}\n",
            &C[..7],
            &B[..7]
        );
        let comments = vec![approval(B), comment(&body)];
        let (rounds, anomalies) = fold_rounds_from_comments(A, &comments);

        assert_eq!(rounds.len(), 2);
        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");

        // The short SHAs still resolve to the full commits in stage 2.
        let thread = thread(&[A, B, C], A, &comments);
        assert!(thread.round_anomalies.is_empty());
        assert_eq!(
            thread.rounds[1].previous_approval,
            Some(ObjectId::from_str(B).unwrap())
        );
        assert_eq!(thread.rounds[1].opened_at, ObjectId::from_str(C).unwrap());
    }

    #[test]
    fn reapproval_after_retraction_closes_at_the_newer_commit() {
        let comments = vec![approval(B), unapproval(), approval(D)];
        let thread = thread(&[A, B, C, D], A, &comments);

        assert_eq!(thread.rounds.len(), 1);
        assert_eq!(thread.rounds[0].retractions.len(), 1);
        assert_eq!(
            thread.rounds[0].retractions[0].retracted_commit,
            ObjectId::from_str(B).unwrap()
        );
        assert!(!thread.rounds[0].is_open());
        assert_eq!(thread.open_round(), None);
        assert_eq!(
            thread.latest_standing_approval(),
            Some(&ObjectId::from_str(D).unwrap())
        );
    }

    #[test]
    fn notification_after_approval_wins_the_flat_max() {
        let comments = vec![approval(B), notification(D)];
        let thread = thread(&[A, B, C, D], A, &comments);

        assert_eq!(
            thread.round_anomalies,
            vec![RoundAnomaly::EventAfterClose { comment_index: 1 }]
        );
        assert_eq!(
            thread.latest_standing_approval(),
            Some(&ObjectId::from_str(B).unwrap())
        );
        assert_eq!(
            thread.latest_notified_commit(),
            Some(&ObjectId::from_str(D).unwrap())
        );
        assert_eq!(
            thread.next_notification_from(),
            ObjectId::from_str(D).unwrap()
        );
    }

    #[test]
    fn single_commit_thread_has_an_empty_open_round() {
        let comments: Vec<GitComment> = vec![];
        let thread = thread(&[A], A, &comments);

        assert_eq!(thread.rounds.len(), 1);
        assert!(thread.open_round().is_some());
        assert!(thread.round_membership(0).is_empty());
        assert!(thread.draft_gap(0).is_empty());
        assert_eq!(thread.latest_standing_approval(), None);
        assert_eq!(thread.latest_notified_commit(), None);
        assert_eq!(thread.latest_reviewed_commit(), None);
        assert_eq!(
            thread.next_notification_from(),
            ObjectId::from_str(A).unwrap()
        );
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
                    "# QC New Round\n\n## Metadata\nround: 2\nround commit: {C}\nprevious approved commit: {B}\nnote: second pass\n"
                ),
                15,
                &format!("{URL}15"),
            ),
            // Round 2 is open, so this one extends it rather than opening round 3.
            identified(
                &format!("# QC New Round\n\n## Metadata\nround: 3\nround commit: {D}\n"),
                16,
                &format!("{URL}16"),
            ),
        ];
        let thread = thread(&[A, B, C, D, E], A, &comments);
        assert_eq!(thread.rounds.len(), 2);

        let first = &thread.rounds[0];
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

        let second = &thread.rounds[1];
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
            new_round(2, C, B),
            notification(D),
            approval(D),
            unapproval(),
        ];
        let (raw, raw_anomalies) = fold_rounds_from_comments(A, &comments);
        assert!(
            raw_anomalies.is_empty(),
            "unexpected anomalies: {raw_anomalies:?}"
        );
        assert_eq!(raw.len(), 2);

        let thread = resolve_rounds(
            raw,
            raw_anomalies,
            &thread(&[A, B, C, D], A, &comments).commits,
        );
        let (rounds, anomalies) = thread;
        assert!(anomalies.is_empty());
        assert_eq!(rounds.len(), 2);

        assert!(matches!(
            &rounds[0].state,
            RoundState::Closed {
                comment_id: None,
                comment_url: None,
                ..
            }
        ));
        assert!(matches!(
            &rounds[0].events[0],
            RoundEvent::Notification {
                comment_id: None,
                comment_url: None,
                ..
            }
        ));
        assert!(matches!(
            &rounds[1].opened,
            RoundOpen::NewRound {
                comment_id: None,
                comment_url: None,
                ..
            }
        ));
        assert_eq!(
            rounds[1].checklist,
            ChecklistSource::Comment {
                comment_index: 2,
                comment_id: None,
                comment_url: None,
            }
        );
        assert_eq!(rounds[1].retractions[0].comment_id, None);
        assert_eq!(rounds[1].retractions[0].comment_url, None);
    }

    #[test]
    fn no_commits_yields_no_rounds_and_no_panics() {
        let (raw, raw_anomalies) = fold_rounds_from_comments(A, &[]);
        let (rounds, anomalies) = resolve_rounds(raw, raw_anomalies, &[]);
        assert!(rounds.is_empty());
        assert!(anomalies.is_empty());
    }
}
