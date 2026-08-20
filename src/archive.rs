use std::{
    collections::HashMap,
    fmt,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use gix::ObjectId;
use serde::{Deserialize, Serialize};

use crate::{
    GapContinuity, GitFileOps, GitFileOpsError, IssueError, IssueThread, Placement, Round,
    RoundState, Segment, UnplaceableReason, utils::EnvProvider,
};

/// The QC facts about an archived file, flattened into [`ArchiveFile`].
///
/// Two independent facts, never one bool: where the archived bytes came from
/// (`round.approval`, permanent) and whether they were the newest QC state when the
/// archive was cut (`round.superseded`, perishable). The `approved: bool` this replaces
/// compressed both, and the two producers of this file set it from different
/// predicates.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveQC {
    pub milestone: String,
    /// Nested, not flattened: `round.round` and `round.approval.round` are two frames
    /// on the same file and must not read as siblings of `commit`.
    pub round: RoundProvenance,
}

/// Where the archived bytes came from, as a point-in-time snapshot. A reader never
/// recomputes it.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RoundProvenance {
    /// The round this selection addressed — not necessarily the newest round, which is
    /// what makes an older round addressable at all. Never becomes false.
    pub round: u32,
    /// `Some` ⇒ these bytes are some round's closing commit, with who closed it and
    /// when. `None` ⇒ the bytes were never approved.
    ///
    /// Always serialized, including as an explicit `null`: "never approved" is the
    /// load-bearing fact of the unapproved case, and a missing key would read as "this
    /// writer did not know".
    pub approval: Option<Approval>,
    /// `true` ⇒ at archive time these bytes were **not** the newest QC state of the
    /// file: a later round had closed, the latest round was open, or the file had
    /// changed since this approval.
    ///
    /// A glance-level warning, not evidence: every cause is recoverable from the thread
    /// plus [`ArchiveMetadata::created_at`], which is why this stays a plain bool and
    /// must not grow into a per-cause structure.
    pub superseded: bool,
}

/// An approval claim about one commit.
// `PartialEq`/`Eq` beyond the pinned derive list, so a test can compare a whole
// `ArchivePreview`. Neither affects the serde representation this type is pinned on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Approval {
    /// The round that closed on this commit. **May be less than
    /// [`RoundProvenance::round`]**: a round's anchor may be the previous round's
    /// closing commit, so selecting an open round 2 anchored at round 1's approval
    /// yields `round: 2, approval.round: 1` — read as *you were on round 2; the commit
    /// you took happens to be round 1's approval*. Nothing here asserts that
    /// `RoundProvenance::round` was approved.
    pub round: u32,
    #[serde(
        serialize_with = "display_as_string",
        deserialize_with = "parse_from_string"
    )]
    pub commit: ObjectId,
    pub by: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// Where an [`ArchiveTarget`] lands on a thread, and whether the archive can serve it.
///
/// **The one home of the selected-round gate.** Both surfaces call
/// [`selected_round`] rather than reimplementing the predicate: the API and the CLI had
/// already drifted to opposite answers about which segment to gate on — one archiving a
/// file the other refused, from the same repository — and a shared predicate is what
/// makes that unrepresentable rather than merely fixed. Rendering is the caller's: this
/// type carries the round, its name, and the reason when there is one, so each surface
/// can write its own callout without deriving the fact again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRound {
    /// 1-based index of the round the selection addressed.
    pub round: u32,
    /// The round's name as a reader knows it — `Initial QC`, `Round 2`.
    pub name: String,
    /// Whether that round's commits could be located at all.
    pub placement: Placement,
}

impl SelectedRound {
    /// Whether the archive can serve this selection.
    ///
    /// The gate is the **selected round**, not the active segment: an unplaceable
    /// *trailing gap* over a placed, closed round does not block, because that round's
    /// approval is a real, archivable sha and the only requirement is that every file in
    /// the request resolve to a commit. What the gap costs is currency, not
    /// archivability, and that is recorded in `superseded` instead (clause 4 of
    /// [`is_superseded`]).
    pub fn is_archivable(&self) -> bool {
        self.placement.is_placed()
    }

    /// Why the selection cannot be served, if it cannot.
    pub fn unplaceable_reason(&self) -> Option<UnplaceableReason> {
        match self.placement {
            Placement::Placed => None,
            Placement::Unplaceable(reason) => Some(reason),
        }
    }

    /// The refusal in the user's terms, as a clause that reads after the round it
    /// describes: *"Round 2 — its branch is unavailable locally"*.
    ///
    /// One wording for every surface: this delegates to
    /// [`UnplaceableReason::describe`], which the CLI's trust marker, the repair report
    /// and the API's skip reason all already share, so a user never sees the same
    /// degradation explained two ways.
    pub fn refusal(&self) -> Option<&'static str> {
        self.unplaceable_reason().map(UnplaceableReason::describe)
    }
}

/// Resolve `target` against `thread`: which round the selection addresses, and whether
/// the archive can serve it.
///
/// Callers gate on [`SelectedRound::is_archivable`] before building an [`ArchiveFile`],
/// and render their own callout from [`SelectedRound::refusal`]. `Err` only when the
/// target names no round on the thread, which is the same
/// [`ArchiveError::RoundSelection`] [`ArchiveFile::from_issue_thread`] returns for the
/// same input — one lookup, so a surface cannot validate the round differently from the
/// derivation that consumes it.
pub fn selected_round(
    thread: &IssueThread,
    target: ArchiveTarget,
) -> Result<SelectedRound, ArchiveError> {
    let (_, round) = resolve_target(thread, target)?;
    Ok(SelectedRound {
        round: round.index,
        name: round.name(),
        placement: round.placement,
    })
}

/// The round `target` addresses, with its position in `thread.segments`.
///
/// The single round lookup behind both [`selected_round`] and
/// [`ArchiveFile::from_issue_thread`]. The position is what lets the `superseded`
/// derivation ask about the segments *after* the selection without re-deriving where the
/// selection sits.
fn resolve_target(
    thread: &IssueThread,
    target: ArchiveTarget,
) -> Result<(usize, &Round), ArchiveError> {
    let rounds: Vec<(usize, &Round)> = thread
        .segments
        .iter()
        .enumerate()
        .filter_map(|(position, segment)| segment.as_round().map(|round| (position, round)))
        .collect();
    let n = rounds.len() as u32;
    let r = match target {
        ArchiveTarget::Latest => n,
        ArchiveTarget::Round(selected) => selected,
    };
    // Round indices are derived as the previous round's plus one starting at 1, so they
    // are exactly `1..=n` and a selection outside that range names no round.
    rounds
        .into_iter()
        .find(|(_, round)| round.index == r)
        .ok_or_else(|| ArchiveError::RoundSelection {
            file: thread.file.clone(),
            round: r,
            rounds: n,
        })
}

/// What archiving one round would produce, without producing it.
///
/// The **selection-time** projection of the same three rules the metadata records: **S1**'s
/// content rule, **I2**'s approval claim, and **S3**'s supersession clauses. It exists so
/// that no other surface re-derives them — the UI had grown a second implementation of all
/// three in TypeScript, and one rule in two languages is the drift this design keeps
/// removing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePreview {
    /// The commit archiving this round would use — **S1**'s three rows. Not necessarily
    /// the round's newest commit.
    pub commit: ObjectId,
    /// `Some` ⇒ those bytes are a round's closing commit. Per **I2** the round it names
    /// may be **older** than the round being previewed; nothing here asserts the previewed
    /// round was approved.
    pub approval: Option<Approval>,
    /// Empty ⇒ provably the newest QC state. Non-empty ⇒ every reason it is not, in clause
    /// order. Causes are **not** mutually exclusive.
    ///
    /// There is deliberately no `superseded: bool` beside this — `superseded` *is*
    /// `!superseding_causes.is_empty()`, and two fields for one fact is the defect this
    /// design exists to remove. A client that wants the bool folds the list.
    pub superseding_causes: Vec<SupersedingCause>,
}

/// One reason a round's archived bytes would not be provably the newest QC state — one
/// variant per **S3** clause, and the order they are reported in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupersedingCause {
    /// Clause 1: a round after the previewed one has closed.
    LaterApproval,
    /// Clause 2: the thread's latest round is open.
    RoundOpen,
    /// Clause 3: the previewed round is the latest, is closed, and its trailing gap holds
    /// a commit that changed the file.
    ChangedSince,
    /// Clause 4: currency cannot be determined — see [`superseding_causes`].
    Undeterminable,
}

/// What archiving `round` of `thread` would produce.
///
/// `None` when that round cannot be archived: it is unplaceable, so no commit exists that
/// the archive could honestly point at — the same refusal
/// [`selected_round`] reports, from the same code path, so a client never has to ask why
/// twice. `None` also when `thread` has no such round, which a caller projecting over the
/// thread's own rounds cannot hit.
///
/// Every field comes from the derivation [`ArchiveFile::from_issue_thread`] runs, not from
/// a parallel reimplementation of it: for one thread and round the two agree on the commit,
/// on the approval, and on `superseded` versus an empty cause list, by construction.
pub fn archive_preview(thread: &IssueThread, round: u32) -> Option<ArchivePreview> {
    archive_derivation(thread, ArchiveTarget::Round(round))
        .map(|(_, preview)| preview)
        .ok()
}

/// Which round an archive selection addresses.
///
/// The open/approved line is not a filter — it is a per-file round selection, and the
/// bytes follow from the round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveTarget {
    /// The latest round. The default: there is a reason a round is open, and the
    /// archive shows current reality first. For an approved-then-reopened file this is
    /// therefore *unapproved* content — intentional, and neither gated nor confirmed.
    Latest,
    /// An explicitly selected round, 1-based; `1` is Initial QC.
    Round(u32),
}

fn display_as_string<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: fmt::Display,
{
    serializer.serialize_str(&value.to_string())
}

fn parse_from_string<'de, D>(deserializer: D) -> Result<ObjectId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    ObjectId::from_hex(s.as_bytes()).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveFile {
    pub repository_file: PathBuf,
    pub archive_file: PathBuf,
    #[serde(
        serialize_with = "display_as_string",
        deserialize_with = "parse_from_string"
    )]
    pub commit: ObjectId,
    // archive file will only ever have a milestone AND approval status or neither
    #[serde(flatten)]
    pub qc: Option<ArchiveQC>,
}

impl ArchiveFile {
    pub fn file_content(&self, git_info: &impl GitFileOps) -> Result<Vec<u8>, GitFileOpsError> {
        git_info.file_bytes_at_commit(&self.repository_file, &self.commit)
    }

    /// Archive `issue_thread`'s file at the round `target` addresses.
    ///
    /// The content rule, given the selected round `r` on a thread with `n` rounds:
    ///
    /// | Case | Bytes archived | `approval` |
    /// |---|---|---|
    /// | `r < n` | `r`'s closing commit | `Some` |
    /// | `r == n`, closed | `r`'s closing commit, even with a non-empty trailing gap | `Some` |
    /// | `r == n`, open | `r`'s latest *actioned* commit | `None`, except per [`Approval::round`] |
    ///
    /// Row 3 is a deliberate change of archived bytes for a never-approved file: it used
    /// to take the active segment's newest commit, whether or not anyone ever put that
    /// commit up for review. It now takes the newest commit someone *acted on* — an
    /// anchor, a notification or a review — because an archive captures a repository at a
    /// reviewed period so the analysis can be reproduced, and drift nobody looked at was
    /// never part of any such period.
    ///
    /// Row 2 keeps the pre-round behaviour for the approved-with-drift case — the
    /// approval, not the drifted working tree — and merely labels it `superseded`.
    pub fn from_issue_thread(
        issue_thread: &IssueThread,
        flatten: bool,
        target: ArchiveTarget,
    ) -> Result<Self, ArchiveError> {
        // Every QC fact about the file comes from the one derivation, which the
        // status projection also calls: the metadata and the preview cannot disagree
        // about the commit, the approval or supersession, because there is nothing to
        // disagree with.
        let (r, preview) = archive_derivation(issue_thread, target)?;

        let round = RoundProvenance {
            round: r,
            approval: preview.approval,
            // The metadata keeps a plain bool and gains no cause breakdown: it is an
            // evidentiary artifact, every cause is recomputable from the thread plus
            // `created_at`, and a structure there would invite reading a snapshot as
            // current state. The causes are exposed at *selection* time instead, where a
            // user can still act on them. One evaluation, two shapes.
            superseded: !preview.superseding_causes.is_empty(),
        };
        let commit = preview.commit;

        let archive_file = if flatten {
            issue_thread
                .file
                .file_name()
                .map(PathBuf::from)
                .expect("File to have file name")
        } else {
            issue_thread
                .file
                .strip_prefix("/")
                .unwrap_or(&issue_thread.file)
                .to_path_buf()
        };

        Ok(Self {
            repository_file: issue_thread.file.clone(),
            archive_file,
            commit,
            qc: Some(ArchiveQC {
                milestone: issue_thread.milestone.to_string(),
                round,
            }),
        })
    }

    pub fn from_file(file: impl AsRef<Path>, commit: ObjectId, flatten: bool) -> Self {
        let file = file.as_ref();
        let archive_file = if flatten {
            file.file_name()
                .map(PathBuf::from)
                .expect("File to have file name")
        } else {
            file.strip_prefix("/").unwrap_or(file).to_path_buf()
        };
        Self {
            repository_file: file.to_path_buf(),
            archive_file,
            commit,
            qc: None,
        }
    }
}

/// The one derivation behind both the archive metadata and the status projection: which
/// commit archiving `target` would use, the approval claim for those bytes, and every
/// reason they are not provably the newest QC state.
///
/// Returns the selected round's index alongside the preview, because the metadata records
/// the round the *selection* addressed while the preview describes only its result.
///
/// The content rule, given the selected round `r` on a thread with `n` rounds:
///
/// | Case | Bytes archived | `approval` |
/// |---|---|---|
/// | `r < n` | `r`'s closing commit | `Some` |
/// | `r == n`, closed | `r`'s closing commit, even with a non-empty trailing gap | `Some` |
/// | `r == n`, open | `r`'s latest *actioned* commit | `None`, except per [`Approval::round`] |
///
/// Row 3 is a deliberate change of archived bytes for a never-approved file: it used to
/// take the active segment's newest commit, whether or not anyone ever put that commit up
/// for review. It now takes the newest commit someone *acted on* — an anchor, a
/// notification or a review — because an archive captures a repository at a reviewed period
/// so the analysis can be reproduced, and drift nobody looked at was never part of any such
/// period.
///
/// Row 2 keeps the pre-round behaviour for the approved-with-drift case — the approval, not
/// the drifted working tree — and merely labels it superseded.
fn archive_derivation(
    issue_thread: &IssueThread,
    target: ArchiveTarget,
) -> Result<(u32, ArchivePreview), ArchiveError> {
    let rounds: Vec<&Round> = issue_thread.rounds().collect();
    let n = rounds.len() as u32;
    let latest = *rounds
        .last()
        .expect("an IssueThread always has at least Initial QC");

    // The same lookup the surfaces' gate uses, so the round this derivation acts on is by
    // construction the round they validated.
    let (position, selected) = resolve_target(issue_thread, target)?;
    let r = selected.index;

    // The gate again, here rather than only in [`selected_round`]: a predicate a caller may
    // decline to consult is advisory, and an advisory gate is the same class of defect as a
    // version check that only one of two read paths performs. Both surfaces gate pre-flight
    // so they can render their own callout; this is the door, not a second opinion — a round
    // whose commits could not be located has no sha whose blob we can be confident of
    // extracting, including the identified-but-off-every-walk closing commit, and refusing
    // here yields a named error rather than a failure during tar construction.
    if let Placement::Unplaceable(reason) = selected.placement {
        return Err(ArchiveError::UnplaceableRound {
            file: issue_thread.file.clone(),
            round: r,
            reason,
        });
    }

    let commit = if r < n {
        // Row 1. Total: only the last segment may be an open round, and a retraction
        // reopens the *current* round, so every non-latest round is closed. If that ever
        // fails, the fold's invariant broke first.
        //
        // Reported as a named error rather than a panic, in every profile: the fold is fed
        // by network data, so a fold bug must return a diagnosable failure instead of taking
        // an axum handler task down. This is not the forbidden defensive fallback — a
        // fallback would hide the bug behind a plausible commit; this archives nothing and
        // names the violation.
        //
        // Deliberately *no* `debug_assert!` beside it: the fold already checks this
        // invariant, and a debug-only tripwire here would mean the error arm never executes
        // in any build the test suite or CI runs, leaving one behaviour untested and another
        // untestable.
        match selected.closing_commit() {
            Some(closing) => *closing,
            None => {
                return Err(ArchiveError::NonLatestRoundOpen {
                    file: issue_thread.file.clone(),
                    round: r,
                    rounds: n,
                });
            }
        }
    } else if let Some(closing) = selected.closing_commit() {
        // Row 2 — the trailing gap is deliberately ignored; see the cause list.
        *closing
    } else {
        // Row 3.
        match selected.latest_actioned_commit() {
            Some(actioned) => actioned.hash,
            // Never approved and nothing actioned could be placed: there is nothing this
            // archive could honestly point at. An unplaceable round is turned away above,
            // so this is the last line of defence, not the intended path.
            None => return Err(ArchiveError::CommitDetermination(issue_thread.file.clone())),
        }
    };

    Ok((
        r,
        ArchivePreview {
            commit,
            approval: approval_at(&rounds, selected, &commit),
            superseding_causes: superseding_causes(
                issue_thread,
                &rounds,
                selected,
                latest,
                position,
                r,
                n,
            ),
        },
    ))
}

/// The approval claim for `commit`, if any round closed on it.
///
/// `approval.is_some()` ⟺ the archived commit is some round's closing commit — and the
/// round named is the one that *closed* there, which may be older than the round the
/// selection addressed. A round's anchor may be the previous round's closing commit, so
/// an open round selected under row 3 can still archive approved bytes; the check is
/// therefore on the commit, never hardcoded from the row.
///
/// The selected round is preferred when it closed on the commit too, so rows 1 and 2
/// always name themselves rather than an older round that happened to close there.
fn approval_at(rounds: &[&Round], selected: &Round, commit: &ObjectId) -> Option<Approval> {
    let closed_here = |round: &Round| round.closing_commit() == Some(commit);
    let round = if closed_here(selected) {
        selected
    } else {
        rounds
            .iter()
            .copied()
            .rev()
            .find(|round| closed_here(round))?
    };
    match &round.state {
        RoundState::Closed { commit, by, at, .. } => Some(Approval {
            round: round.index,
            commit: *commit,
            by: by.clone(),
            at: *at,
        }),
        // `closed_here` only matches a closed round.
        RoundState::Open => None,
    }
}

/// Every reason it is **not provable that nothing newer exists** than the archived bytes,
/// in clause order. Empty ⇒ these were the latest round's approval with no file changes
/// since, as of archive time.
///
/// 1. [`SupersedingCause::LaterApproval`] — a round later than the selected one has closed;
/// 2. [`SupersedingCause::RoundOpen`] — the latest round is open;
/// 3. [`SupersedingCause::ChangedSince`] — the selected round is the latest, is closed, and
///    the gap trailing it holds a commit that changed the file;
/// 4. [`SupersedingCause::Undeterminable`] — currency cannot be determined at all; see
///    [`currency_undeterminable`].
///
/// **One evaluation, two shapes.** The metadata's `superseded` bool is this list folded with
/// `!is_empty()`, and the status projection carries the list itself. Nothing computes the
/// bool independently, so the two artifacts cannot disagree about one thread and round.
///
/// The framing is deliberately negative. The bool is a glance-level "go check the thread",
/// never evidence, and a reader's action is identical under all four clauses — so the
/// alternative, a positive claim of currency derived from segments we could not read, would
/// put a fact we do not have into an audit artifact.
///
/// Causes are not mutually exclusive: clauses 1 and 2 co-occur routinely.
fn superseding_causes(
    issue_thread: &IssueThread,
    rounds: &[&Round],
    selected: &Round,
    latest: &Round,
    position: usize,
    r: u32,
    n: u32,
) -> Vec<SupersedingCause> {
    let mut causes = Vec::new();

    if rounds
        .iter()
        .any(|round| round.index > r && round.closing_commit().is_some())
    {
        causes.push(SupersedingCause::LaterApproval);
    }

    if latest.is_open() {
        causes.push(SupersedingCause::RoundOpen);
    }

    // The selected round is the latest and closed, so the last segment is the gap trailing
    // it: a closed round is never last.
    if r == n
        && selected.closing_commit().is_some()
        && issue_thread
            .active_segment()
            .as_gap()
            .is_some_and(|gap| gap.commits.iter().any(|commit| commit.file_changed))
    {
        causes.push(SupersedingCause::ChangedSince);
    }

    if currency_undeterminable(issue_thread, position) {
        causes.push(SupersedingCause::Undeterminable);
    }

    causes
}

/// Whether any segment after the selected round has a commit set we cannot trust to be
/// complete, so clauses 1–3 were evaluated on partial information.
///
/// This is what makes the gate's new leniency honest: a file whose trailing gap could not
/// be placed is now archivable at its round's approval, and that file is exactly the one
/// clause 3 silently reads as "no file-changing commit" — because an unreadable segment
/// owns nothing, not because nothing happened.
///
/// **Two ways a segment after the selection can own nothing, and the distinction is the
/// whole clause:**
///
/// - it is `Unplaceable` — we could not read it;
/// - it is a `Placed` **`Unrelated`** gap — we read it and no range between its bounds is
///   meaningful, because its ends share no history, so it owns nothing *by construction*.
///
/// The rule, exactly: **`Unplaceable`, or a `Placed` gap owning nothing *because its
/// continuity is `Unrelated`*.** It is a property of the commit set **and the reason the
/// set is empty**, and it must not be simplified to `matches!(placement, Unplaceable(_))`:
/// an `Unrelated` gap is `Placed` yet owns nothing, so a placement-only check waves it
/// through and the file is recorded `superseded: false` — the identical false claim, by a
/// different route.
///
/// **An empty `Linear` gap proves currency; an empty `Unrelated` gap proves nothing.**
/// That asymmetry is the whole reason the emptiness alone is not the test. Empty gaps are
/// legal and expected, and a `Linear` gap owning no commits is the *fact* that nothing
/// landed since the approval — complete information. Since a closed latest round always
/// trails into a gap, treating any empty gap as undeterminable would make
/// `superseded: false` unreachable for every closed-round selection, which is the one
/// state clause 3 exists to distinguish. Pinned by
/// `an_ordinary_empty_trailing_gap_still_proves_currency`.
///
/// **Neither half is fold-reachable in isolation today, and that is a property of the
/// fold's shape, not a sign this clause is dead.** A trailing gap is walked on the older
/// round's own branch, so an unplaceable trailing gap implies an unplaceable round; a
/// trailing gap is always `Linear`, so `Unrelated` gaps are interior; and any segment
/// after a *non-latest* selection already trips clause 1 or 2. Its isolating tests
/// therefore build their segment lists by hand and say so. Do not remove the clause for
/// want of a live case — what is dormant is the shape, not the rule.
fn currency_undeterminable(issue_thread: &IssueThread, position: usize) -> bool {
    issue_thread
        .segments
        .iter()
        .skip(position + 1)
        .any(|segment| match segment {
            _ if !segment.is_placed() => true,
            Segment::Gap(gap) => {
                gap.commits.is_empty() && matches!(gap.continuity, GapContinuity::Unrelated)
            }
            // A placed round always owns at least its own anchor.
            Segment::Round(_) => false,
        })
}

/// Version of the metadata *structure* this build writes and knows how to read.
///
/// **It versions the JSON shape, never the tool.** It is not the crate version, not a
/// semver string, and it is never derived from `CARGO_PKG_VERSION`: a ghqc release that
/// changes no metadata shape does not change this number.
///
/// Version **0** is the pre-round shape, which carried no version field at all and a
/// `qc.approved: bool` in place of [`RoundProvenance`]. Version **1** is the current
/// shape.
///
/// **Increment by one whenever any field of [`ArchiveMetadata`], [`ArchiveFile`],
/// [`ArchiveQC`], [`RoundProvenance`] or [`Approval`] is added, removed, renamed, or
/// changes meaning or serde representation — additive-only changes included.** A reader
/// must never have to guess which additive generation of a shape it is holding.
pub const METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(try_from = "RawArchiveMetadata")]
pub struct ArchiveMetadata {
    /// Version of the metadata structure — see [`METADATA_VERSION`] for the increment
    /// rule. Serialized first so `head`ing the file shows it.
    ///
    /// **Private, like every other field here, and deliberately.** Set only by
    /// [`ArchiveMetadata::new`] from [`METADATA_VERSION`], or carried over by the
    /// version-checked conversion from [`RawArchiveMetadata`] — which admits exactly the
    /// one version this build can read. A settable version would let a holder emit a
    /// tarball this same build would then refuse to read, which is a worse failure than
    /// the unreadable *foreign* document the read-side check exists to reject: the
    /// unreadable artifact would be one we produced. If a caller ever needs to observe
    /// the version, that is a getter, not a public field.
    metadata_version: u32,
    creator: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    files: Vec<ArchiveFile>,
}

/// The document exactly as it sits on disk, before its structure version has been
/// checked.
///
/// `ArchiveMetadata`'s `Deserialize` goes through here via `serde(try_from)`, so **every**
/// JSON → [`ArchiveMetadata`] path is version-checked — including a direct
/// `serde_json::from_str::<ArchiveMetadata>`, which would otherwise sidestep a check that
/// lived only in a helper constructor. The unchecked read is unrepresentable rather than
/// merely discouraged.
#[derive(serde::Deserialize)]
struct RawArchiveMetadata {
    /// Absent ⇒ 0: the pre-round shape carried no version field. 0 is then *refused*
    /// like any other version this build cannot interpret — see
    /// [`TryFrom<RawArchiveMetadata>`].
    #[serde(default)]
    metadata_version: u32,
    creator: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    files: Vec<ArchiveFile>,
}

impl TryFrom<RawArchiveMetadata> for ArchiveMetadata {
    type Error = ArchiveError;

    /// Accept exactly the structure versions this build can interpret — today
    /// `{`[`METADATA_VERSION`]`}` — and refuse every other value by name.
    ///
    /// **Version 0 is refused, and that is the point rather than a side effect.** A
    /// version-0 document carries `{milestone, approved}` where this build expects
    /// `{milestone, round}`, and `ArchiveQC` is a *flattened `Option`* on
    /// [`ArchiveFile`]: a v0 file therefore does not fail to parse, it parses with
    /// `qc: None`, reporting every QC'd file in a pre-round archive as a manually added
    /// one with no QC history at all. That is a false negative on approval inside an
    /// audit artifact — one document carrying two disagreeing accounts of whether QC
    /// happened. Refusing the old shape is the opposite of supporting it.
    fn try_from(raw: RawArchiveMetadata) -> Result<Self, Self::Error> {
        if raw.metadata_version != METADATA_VERSION {
            return Err(ArchiveError::UnsupportedMetadataVersion {
                found: raw.metadata_version,
                known: METADATA_VERSION,
            });
        }
        Ok(Self {
            metadata_version: raw.metadata_version,
            creator: raw.creator,
            created_at: raw.created_at,
            files: raw.files,
        })
    }
}

impl ArchiveMetadata {
    pub fn new(files: Vec<ArchiveFile>, env: &impl EnvProvider) -> Result<Self, ArchiveError> {
        // Check for duplicate archive paths and collect ALL conflicts
        let mut path_to_sources = HashMap::new();

        for file in &files {
            let archive_path = &file.archive_file;
            let source_path = &file.repository_file;
            path_to_sources
                .entry(archive_path.clone())
                .or_insert_with(Vec::new)
                .push(source_path.clone());
        }

        // Find all conflicts (archive paths with multiple sources)
        let conflicts: Vec<_> = path_to_sources
            .into_iter()
            .filter(|(_, sources)| sources.len() > 1)
            .collect();

        if !conflicts.is_empty() {
            // Create well-formatted error message showing all conflicts
            let conflict_descriptions: Vec<String> = conflicts
                .into_iter()
                .map(|(archive_path, sources)| {
                    let sources_str = sources
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" + ");
                    format!("{} -> {}", sources_str, archive_path.display())
                })
                .collect();

            let error_message =
                format!("Conflicts detected:\n{}", conflict_descriptions.join("\n"));

            return Err(ArchiveError::FileConflict(error_message));
        }

        let creator = env.var("USER").ok();
        if creator.is_none() {
            log::warn!("Failed to determine creator using environment variable USER");
        }
        Ok(Self {
            metadata_version: METADATA_VERSION,
            creator,
            created_at: chrono::Utc::now(),
            files,
        })
    }

    /// Read a `ghqc_archive_metadata.json` document.
    ///
    /// A convenience over `serde_json::from_str`, not a privileged door: the structure
    /// version is checked in deserialization itself (see [`RawArchiveMetadata`]), so this
    /// function and the direct `serde_json` route refuse exactly the same documents. Only
    /// the versions this build can interpret are accepted — every other value, **version
    /// 0 included**, is refused rather than read best-effort, because a partial read of an
    /// audit artifact is worse than a refusal.
    ///
    /// It parses the raw shape and converts explicitly, rather than deserializing
    /// `Self` in one step, for one reason: serde stringifies a `try_from` failure into a
    /// `serde_json::Error`, so the direct route can only report the refusal as text. This
    /// route keeps [`ArchiveError::UnsupportedMetadataVersion`] matchable by callers.
    pub fn from_json(json: &str) -> Result<Self, ArchiveError> {
        Self::try_from(serde_json::from_str::<RawArchiveMetadata>(json)?)
    }
}

pub fn archive(
    archive_metadata: ArchiveMetadata,
    git_info: &impl GitFileOps,
    path: impl AsRef<Path>,
) -> Result<(), ArchiveError> {
    let path = path.as_ref();
    log::debug!(
        "Writing {} files to archive at {}",
        archive_metadata.files.len(),
        path.display()
    );
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(encoder);

    log::trace!("Writing metadata file to archive at ghqc_archive_metadata.json");
    let metadata = serde_json::to_string_pretty(&archive_metadata)?;
    write_content(&mut tar, "ghqc_archive_metadata.json", metadata.as_bytes())?;

    for archive_file in archive_metadata.files {
        log::trace!(
            "Writing {} at {} to archive at {}",
            archive_file.repository_file.display(),
            archive_file.commit.to_string(),
            archive_file.archive_file.display()
        );
        let content = archive_file.file_content(git_info)?;
        write_content(&mut tar, &archive_file.archive_file, &content)?;
    }

    tar.finish()?;
    log::debug!(
        "Successfully created compressed archive at {}",
        path.display()
    );

    Ok(())
}

fn write_content(
    tar: &mut tar::Builder<GzEncoder<File>>,
    path: impl AsRef<Path>,
    content: &[u8],
) -> io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_path(path)?;
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();

    tar.append(&header, content)
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("Failed to analyze issue due to: {0}")]
    IssueError(#[from] IssueError),
    #[error("Failed to get file content at commit due to: {0}")]
    GitFileOpsError(#[from] GitFileOpsError),
    #[error("Cannot create archive: multiple files have the same archive name '{0}'")]
    FileConflict(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Failed to determine commit for {0}")]
    CommitDetermination(PathBuf),
    #[error("Round {round} does not exist for {file}: it has {rounds} round(s)")]
    RoundSelection {
        file: PathBuf,
        round: u32,
        rounds: u32,
    },
    /// The selected round's commits could not be located, so no commit exists that the
    /// archive could honestly point at for it. Distinct from
    /// [`Self::CommitDetermination`], which is "nothing could be placed" — a different
    /// fact — and from [`Self::RoundSelection`], which is "that round does not exist".
    ///
    /// The wording comes from [`UnplaceableReason::describe`], the one string the CLI's
    /// callout, the API's 400 and this error all render, so a user never sees the same
    /// degradation explained two ways.
    #[error("Round {round} for {file} cannot be archived: {}", reason.describe())]
    UnplaceableRound {
        file: PathBuf,
        round: u32,
        reason: UnplaceableReason,
    },
    #[error(
        "Round {round} of {rounds} for {file} is not the latest round and is not closed: \
         the segment fold's invariant that only the latest round may be open was violated"
    )]
    NonLatestRoundOpen {
        file: PathBuf,
        round: u32,
        rounds: u32,
    },
    #[error(
        "Archive metadata is version {found}, but this build can only read version \
         {known}: refusing to read it rather than misreporting a shape it does not \
         understand"
    )]
    UnsupportedMetadataVersion { found: u32, known: u32 },
    #[error("Failed to serialize metadata: {0}")]
    Serde(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GitComment, IssueCommit, IssueThread,
        git::MockGitFileOps,
        round::{BranchWalks, fold_rounds_from_comments, resolve_segments, round_branches},
        utils::MockEnvProvider,
    };
    use chrono::Utc;
    use flate2::read::GzDecoder;
    use gix::ObjectId;
    use std::collections::HashMap;
    use std::str::FromStr;
    use tar::Archive;
    use tempfile::TempDir;

    fn create_test_object_id(suffix: &str) -> ObjectId {
        // Create a valid 40-character hex string for SHA-1
        let hex_str = format!("{:0<40}", format!("deadbeef{}", suffix));
        ObjectId::from_hex(hex_str.as_bytes()).unwrap()
    }

    // ── Thread fixtures, built through the fold ──────────────────────────────
    //
    // Threads are folded from comment text and pre-canned walks, exactly as
    // `src/round.rs`'s probes do it, rather than assembled segment by segment: the
    // selection rules lean on the fold's invariants (only the last round may be open,
    // strict round/gap alternation), so a hand-built thread could assert behaviour on a
    // shape the fold never produces.

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";
    const D: &str = "ddddddd000000000000000000000000000000004";
    const E: &str = "eeeeeee000000000000000000000000000000005";
    const F: &str = "fffffff000000000000000000000000000000006";

    /// The issue body's branch, i.e. Initial QC's.
    const MAIN: &str = "main";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    fn comment(body: &str) -> GitComment {
        GitComment {
            body: body.to_string(),
            author_login: "reviewer".to_string(),
            created_at: Utc::now(),
            id: None,
            html_url: None,
            html: None,
        }
    }

    fn notification(sha: &str) -> GitComment {
        comment(&format!(
            "# QC Notification\n\n@reviewer\n\n## Metadata\ncurrent commit: {sha}\n"
        ))
    }

    fn approval(sha: &str) -> GitComment {
        comment(&format!(
            "# QC Approval\n\n## Metadata\napproved qc commit: {sha}\n"
        ))
    }

    fn new_round(round: u32, round_commit: &str, previous: &str) -> GitComment {
        comment(&format!(
            "# QC Round\n\n## Metadata\nround: {round}\ninitial qc round commit: {round_commit}\nprevious approved commit: {previous}\ngit branch: {MAIN}\n\n# Checklist\n- [ ] item\n"
        ))
    }

    /// A round comment that moved the round onto another branch.
    fn new_round_on(round: u32, round_commit: &str, previous: &str, branch: &str) -> GitComment {
        comment(&format!(
            "# QC Round\n\n## Metadata\nround: {round}\ninitial qc round commit: {round_commit}\nprevious approved commit: {previous}\ngit branch: {branch}\n\n# Checklist\n- [ ] item\n"
        ))
    }

    /// A branch walk. `shas` is oldest-first here for readability and reversed into the
    /// newest-first order every walk uses. `touched` names the commits that changed the
    /// file; everything else is drift the file did not take part in.
    fn walk(shas: &[&str], touched: &[&str]) -> Vec<IssueCommit> {
        shas.iter()
            .rev()
            .map(|sha| IssueCommit {
                hash: oid(sha),
                message: format!("commit {sha}"),
                file_changed: touched.contains(sha),
            })
            .collect()
    }

    fn no_merge_base(_: &ObjectId, _: &ObjectId) -> Option<ObjectId> {
        None
    }

    /// A single-branch thread: `shas` oldest-first, every commit touching the file.
    fn thread(shas: &[&str], initial: &str, comments: &[GitComment]) -> IssueThread {
        thread_touching(shas, shas, initial, comments)
    }

    /// As [`thread`], but only `touched` changed the file.
    fn thread_touching(
        shas: &[&str],
        touched: &[&str],
        initial: &str,
        comments: &[GitComment],
    ) -> IssueThread {
        let walks = BranchWalks::from([(MAIN.to_string(), Ok(walk(shas, touched)))]);
        let (raw, raw_anomalies) = fold_rounds_from_comments(initial, None, comments);
        let branches = round_branches(&raw, MAIN);
        let (segments, anomalies) =
            resolve_segments(raw, &branches, &walks, &no_merge_base, raw_anomalies);
        IssueThread {
            file: PathBuf::from("src/test.rs"),
            open: false,
            milestone: "v1.0".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies,
        }
    }

    /// A branch a later round moved to.
    const FEATURE: &str = "feature/x";

    /// A thread whose second round was QC'd on another branch, so the gap between the two
    /// is walked between ends that share no history.
    fn thread_on_two_branches(
        main: &[&str],
        feature: &[&str],
        initial: &str,
        comments: &[GitComment],
    ) -> IssueThread {
        let walks = BranchWalks::from([
            (MAIN.to_string(), Ok(walk(main, main))),
            (FEATURE.to_string(), Ok(walk(feature, feature))),
        ]);
        let (raw, raw_anomalies) = fold_rounds_from_comments(initial, None, comments);
        let branches = round_branches(&raw, MAIN);
        let (segments, anomalies) =
            resolve_segments(raw, &branches, &walks, &no_merge_base, raw_anomalies);
        IssueThread {
            file: PathBuf::from("src/test.rs"),
            open: false,
            milestone: "v1.0".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies,
        }
    }

    /// Initial QC approved at `B`, with the empty gap that follows it.
    fn create_test_issue_thread() -> IssueThread {
        thread(&[A, B], A, &[notification(B), approval(B)])
    }

    /// A hand-built provenance for the metadata fixtures below, which exercise
    /// `ArchiveMetadata` rather than the derivation.
    fn provenance(
        round: u32,
        approval: Option<(u32, ObjectId)>,
        superseded: bool,
    ) -> RoundProvenance {
        RoundProvenance {
            round,
            approval: approval.map(|(round, commit)| Approval {
                round,
                commit,
                by: "reviewer".to_string(),
                at: Utc::now(),
            }),
            superseded,
        }
    }

    /// The `qc` of a mode-1 archive file.
    fn qc_of(archive_file: &ArchiveFile) -> &ArchiveQC {
        archive_file.qc.as_ref().expect("a mode-1 file has qc")
    }

    fn setup_mock_env_with_user() -> MockEnvProvider {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USER"))
            .returning(|_| Ok("test_user".to_string()));
        mock_env
    }

    fn setup_mock_env_no_user() -> MockEnvProvider {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USER"))
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
    }

    #[test]
    fn test_archive_metadata_new_success() {
        let mock_env = setup_mock_env_with_user();

        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("src/main.rs"),
                commit: create_test_object_id("123"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    round: provenance(1, Some((1, create_test_object_id("123"))), false),
                }),
            },
            ArchiveFile {
                repository_file: PathBuf::from("src/lib.rs"),
                archive_file: PathBuf::from("src/lib.rs"),
                commit: create_test_object_id("456"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    round: provenance(2, None, true),
                }),
            },
        ];

        let result = ArchiveMetadata::new(files.clone(), &mock_env);
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert_eq!(metadata.creator, Some("test_user".to_string()));
        assert_eq!(metadata.files.len(), 2);
        assert_eq!(
            metadata.files[0].repository_file,
            PathBuf::from("src/main.rs")
        );
        assert_eq!(
            metadata.files[1].repository_file,
            PathBuf::from("src/lib.rs")
        );
    }

    #[test]
    fn test_archive_metadata_new_no_user() {
        let mock_env = setup_mock_env_no_user();

        let files = vec![ArchiveFile {
            repository_file: PathBuf::from("src/main.rs"),
            archive_file: PathBuf::from("main.rs"),
            commit: create_test_object_id("123"),
            qc: None,
        }];

        let result = ArchiveMetadata::new(files, &mock_env);
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert_eq!(metadata.creator, None);
        assert_eq!(metadata.files.len(), 1);
    }

    #[test]
    fn test_archive_metadata_new_duplicate_paths_error() {
        let mock_env = setup_mock_env_with_user();

        // Create files that will conflict in the archive (same archive_file path)
        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("main.rs"), // Flattened path
                commit: create_test_object_id("123"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("tests/main.rs"),
                archive_file: PathBuf::from("main.rs"), // Same flattened path!
                commit: create_test_object_id("456"),
                qc: None,
            },
        ];

        let result = ArchiveMetadata::new(files, &mock_env);
        assert!(result.is_err());

        match result.unwrap_err() {
            ArchiveError::FileConflict(msg) => {
                assert!(msg.contains("Conflicts detected"));
                assert!(msg.contains("src/main.rs + tests/main.rs -> main.rs"));
            }
            _ => panic!("Expected FileConflict error"),
        }
    }

    #[test]
    fn test_archive_metadata_new_multiple_conflicts() {
        let mock_env = setup_mock_env_with_user();

        let files = vec![
            // First conflict: main.rs
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("main.rs"),
                commit: create_test_object_id("123"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("tests/main.rs"),
                archive_file: PathBuf::from("main.rs"),
                commit: create_test_object_id("456"),
                qc: None,
            },
            // Second conflict: config.rs
            ArchiveFile {
                repository_file: PathBuf::from("src/config.rs"),
                archive_file: PathBuf::from("config.rs"),
                commit: create_test_object_id("789"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("lib/config.rs"),
                archive_file: PathBuf::from("config.rs"),
                commit: create_test_object_id("abc"),
                qc: None,
            },
        ];

        let result = ArchiveMetadata::new(files, &mock_env);
        assert!(result.is_err());

        match result.unwrap_err() {
            ArchiveError::FileConflict(msg) => {
                assert!(msg.contains("Conflicts detected"));
                // Should contain both conflicts
                assert!(msg.contains("main.rs"));
                assert!(msg.contains("config.rs"));
            }
            _ => panic!("Expected FileConflict error"),
        }
    }

    #[test]
    fn test_archive_file_from_issue_thread_approved() {
        let issue_thread = create_test_issue_thread();

        let result = ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.repository_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.archive_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.commit, oid(B)); // The approval that closed Initial QC

        let qc = qc_of(&archive_file);
        assert_eq!(qc.milestone, "v1.0");
        assert_eq!(qc.round.round, 1);
        assert_eq!(qc.round.approval.as_ref().map(|a| a.round), Some(1));
    }

    #[test]
    fn test_archive_file_from_issue_thread_flattened() {
        let issue_thread = create_test_issue_thread();

        let result = ArchiveFile::from_issue_thread(&issue_thread, true, ArchiveTarget::Latest);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.repository_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.archive_file, PathBuf::from("test.rs")); // Flattened
        assert_eq!(archive_file.commit, oid(B));
    }

    #[test]
    fn test_archive_file_from_file() {
        let file_path = PathBuf::from("src/example.rs");
        let commit = create_test_object_id("123");

        let archive_file = ArchiveFile::from_file(&file_path, commit.clone(), false);

        assert_eq!(archive_file.repository_file, file_path);
        assert_eq!(archive_file.archive_file, PathBuf::from("src/example.rs"));
        assert_eq!(archive_file.commit, commit);
        assert!(archive_file.qc.is_none());
    }

    #[test]
    fn test_archive_file_from_file_flattened() {
        let file_path = PathBuf::from("src/example.rs");
        let commit = create_test_object_id("123");

        let archive_file = ArchiveFile::from_file(&file_path, commit.clone(), true);

        assert_eq!(archive_file.repository_file, file_path);
        assert_eq!(archive_file.archive_file, PathBuf::from("example.rs")); // Flattened
        assert_eq!(archive_file.commit, commit);
    }

    #[test]
    fn test_archive_file_content() {
        let mut mock_git = MockGitFileOps::new();
        let file_content = b"fn main() { println!(\"Hello\"); }";
        let commit = create_test_object_id("123");

        mock_git
            .expect_file_bytes_at_commit()
            .with(
                mockall::predicate::eq(PathBuf::from("src/main.rs")),
                mockall::predicate::eq(commit.clone()),
            )
            .returning(move |_, _| Ok(file_content.to_vec()));

        let archive_file = ArchiveFile {
            repository_file: PathBuf::from("src/main.rs"),
            archive_file: PathBuf::from("main.rs"),
            commit: commit.clone(),
            qc: None,
        };

        let result = archive_file.file_content(&mock_git);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), file_content.to_vec());
    }

    #[test]
    fn test_archive_creates_valid_tar_gz() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test_archive.tar.gz");

        let mut mock_git = MockGitFileOps::new();
        let file1_content = b"content of file1";
        let file2_content = b"content of file2";

        mock_git
            .expect_file_bytes_at_commit()
            .with(
                mockall::predicate::eq(PathBuf::from("src/file1.rs")),
                mockall::predicate::eq(create_test_object_id("123")),
            )
            .returning(move |_, _| Ok(file1_content.to_vec()));

        mock_git
            .expect_file_bytes_at_commit()
            .with(
                mockall::predicate::eq(PathBuf::from("src/file2.rs")),
                mockall::predicate::eq(create_test_object_id("456")),
            )
            .returning(move |_, _| Ok(file2_content.to_vec()));

        let mock_env = setup_mock_env_with_user();

        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/file1.rs"),
                archive_file: PathBuf::from("file1.rs"),
                commit: create_test_object_id("123"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    round: provenance(1, Some((1, create_test_object_id("123"))), false),
                }),
            },
            ArchiveFile {
                repository_file: PathBuf::from("src/file2.rs"),
                archive_file: PathBuf::from("file2.rs"),
                commit: create_test_object_id("456"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    round: provenance(2, None, true),
                }),
            },
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let result = archive(metadata, &mock_git, &archive_path);

        assert!(result.is_ok());
        assert!(archive_path.exists());

        // Verify the archive can be read and contains expected files
        let file = std::fs::File::open(&archive_path).unwrap();
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().to_string();
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
            entries.insert(path, contents);
        }

        // Should contain metadata file + 2 source files
        assert_eq!(entries.len(), 3);
        assert!(entries.contains_key("ghqc_archive_metadata.json"));
        assert!(entries.contains_key("file1.rs"));
        assert!(entries.contains_key("file2.rs"));

        // Verify file contents
        assert_eq!(entries["file1.rs"], file1_content);
        assert_eq!(entries["file2.rs"], file2_content);

        // Verify metadata file contains valid JSON
        let metadata_content =
            String::from_utf8(entries["ghqc_archive_metadata.json"].clone()).unwrap();
        let parsed_metadata = ArchiveMetadata::from_json(&metadata_content).unwrap();
        assert_eq!(parsed_metadata.creator, Some("test_user".to_string()));
        assert_eq!(parsed_metadata.files.len(), 2);
    }

    #[test]
    fn test_archive_creates_directory_structure() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir
            .path()
            .join("nested")
            .join("directory")
            .join("archive.tar.gz");

        let mut mock_git = MockGitFileOps::new();
        let file_content = b"test content";

        mock_git
            .expect_file_bytes_at_commit()
            .returning(move |_, _| Ok(file_content.to_vec()));

        let mock_env = setup_mock_env_with_user();

        let files = vec![ArchiveFile {
            repository_file: PathBuf::from("src/test.rs"),
            archive_file: PathBuf::from("test.rs"),
            commit: create_test_object_id("123"),
            qc: None,
        }];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let result = archive(metadata, &mock_git, &nested_path);

        assert!(result.is_ok());
        assert!(nested_path.exists());
        assert!(nested_path.parent().unwrap().is_dir());
    }

    #[test]
    fn test_archive_preserves_directory_structure() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("structured_archive.tar.gz");

        let mut mock_git = MockGitFileOps::new();
        let file_content = b"content";

        mock_git
            .expect_file_bytes_at_commit()
            .returning(move |_, _| Ok(file_content.to_vec()));

        let mock_env = setup_mock_env_with_user();

        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("src/main.rs"), // Keep directory structure
                commit: create_test_object_id("123"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("tests/integration.rs"),
                archive_file: PathBuf::from("tests/integration.rs"), // Keep directory structure
                commit: create_test_object_id("456"),
                qc: None,
            },
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let result = archive(metadata, &mock_git, &archive_path);

        assert!(result.is_ok());

        // Verify directory structure is preserved in archive
        let file = std::fs::File::open(&archive_path).unwrap();
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        let paths: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"tests/integration.rs".to_string()));
        assert!(paths.contains(&"ghqc_archive_metadata.json".to_string()));
    }

    // ── Round selection: the content rule, `superseded`, and the approval claim ───

    /// Two closed rounds, empty trailing gap. Selecting the older one is the only way
    /// to archive the standing approval of a superseded round.
    fn two_closed_rounds(shas: &[&str], touched: &[&str]) -> IssueThread {
        thread_touching(
            shas,
            touched,
            A,
            &[approval(B), new_round(2, C, B), approval(D)],
        )
    }

    /// Row 1: a non-latest round archives *its* closing commit, not the newest one.
    #[test]
    fn an_older_round_archives_its_own_approval() {
        let issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(1)).unwrap();

        assert_eq!(archive_file.commit, oid(B));
        let round = &qc_of(&archive_file).round;
        assert_eq!(round.round, 1);
        let approval = round.approval.as_ref().expect("row 1 is always approved");
        assert_eq!(approval.round, 1);
        assert_eq!(approval.commit, oid(B));
        assert_eq!(approval.by, "reviewer");
        // S3 clause 1: round 2 has closed since.
        assert!(round.superseded);
    }

    /// Row 2: the latest round is closed, so its approval is archived — and with an
    /// empty trailing gap nothing has superseded it.
    #[test]
    fn the_latest_closed_round_archives_its_approval_and_is_not_superseded() {
        let issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert_eq!(archive_file.commit, oid(D));
        let round = &qc_of(&archive_file).round;
        assert_eq!(round.round, 2);
        assert_eq!(round.approval.as_ref().map(|a| a.round), Some(2));
        assert!(!round.superseded);
    }

    /// Row 2 again, with drift on top: the bytes are still the approval, and the drift
    /// is *labelled* rather than archived (S3 clause 3).
    #[test]
    fn drift_after_the_latest_approval_is_labelled_not_archived() {
        let issue_thread = two_closed_rounds(&[A, B, C, D, E], &[A, B, C, D, E]);
        assert_eq!(issue_thread.segments.len(), 4, "R1, gap, R2, trailing gap");

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert_eq!(archive_file.commit, oid(D), "the approval, not the drift");
        let round = &qc_of(&archive_file).round;
        assert_eq!(round.approval.as_ref().map(|a| a.commit), Some(oid(D)));
        assert!(round.superseded);
    }

    /// S3 clause 3 is about the *file*: a trailing gap that never touched it leaves the
    /// approval current.
    #[test]
    fn a_trailing_gap_that_never_touched_the_file_is_not_superseding() {
        let issue_thread = two_closed_rounds(&[A, B, C, D, E], &[A, B, C, D]);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert_eq!(archive_file.commit, oid(D));
        assert!(!qc_of(&archive_file).round.superseded);
    }

    /// Row 3: an open round archives its newest *actioned* commit — the newest commit
    /// someone put up for review — not the branch tip. This is the behaviour change:
    /// `F` is what the pre-round archive would have taken.
    #[test]
    fn an_open_round_archives_its_latest_actioned_commit_not_its_newest() {
        let issue_thread = thread(&[A, B, C, D, F], A, &[notification(C), notification(D)]);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert_eq!(issue_thread.latest_commit().map(|c| c.hash), Some(oid(F)));
        assert_eq!(archive_file.commit, oid(D), "the newest notified commit");
        let round = &qc_of(&archive_file).round;
        assert_eq!(round.round, 1);
        assert!(round.approval.is_none(), "nothing here was ever approved");
        // S3 clause 2: the latest round is open.
        assert!(round.superseded);
    }

    /// S2/D9: an approved-then-reopened file defaults to the *open* round's unapproved
    /// bytes. The standing approval is one explicit retarget away, and used to be the
    /// default.
    #[test]
    fn a_reopened_file_defaults_to_unapproved_bytes() {
        let issue_thread = thread(&[A, B, C], A, &[approval(B), new_round(2, C, B)]);
        assert_eq!(issue_thread.last_approved_commit(), Some(&oid(B)));

        let default =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();
        assert_eq!(
            default.commit,
            oid(C),
            "round 2's anchor, not R1's approval"
        );
        assert_ne!(
            default.commit,
            oid(B),
            "the pre-round default took the newest approval, ever"
        );
        let round = &qc_of(&default).round;
        assert_eq!(round.round, 2);
        assert!(round.approval.is_none());
        assert!(round.superseded);

        let retargeted =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(1)).unwrap();
        assert_eq!(retargeted.commit, oid(B));
        assert_eq!(
            retargeted
                .qc
                .as_ref()
                .and_then(|qc| qc.round.approval.as_ref())
                .map(|a| a.round),
            Some(1)
        );
    }

    /// The tie-break when **two** rounds closed on the same commit: round 1 closes at
    /// `B`, round 2 opens at `B` and closes at `B` again. Selecting round 1 must name
    /// round 1 — the selected round is preferred over the newest round that closed
    /// there, so rows 1 and 2 always name themselves. "Always the newest" would report
    /// round 2 here, and "always the oldest" would report round 1 for a commit round 2
    /// closed on when round 2 is the selection.
    #[test]
    fn two_rounds_closing_on_one_commit_name_the_selected_round() {
        let issue_thread = thread(&[A, B], A, &[approval(B), new_round(2, B, B), approval(B)]);
        let closing: Vec<Option<&ObjectId>> = issue_thread
            .rounds()
            .map(|round| round.closing_commit())
            .collect();
        assert_eq!(
            closing,
            vec![Some(&oid(B)), Some(&oid(B))],
            "the fixture must have both rounds closing on the same commit"
        );

        let older =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(1)).unwrap();
        assert_eq!(older.commit, oid(B));
        let approval = qc_of(&older)
            .round
            .approval
            .as_ref()
            .expect("row 1 is always approved");
        assert_eq!(approval.round, 1, "the selected round, not the newer one");

        // And the latest round selects itself on the very same commit.
        let latest =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();
        assert_eq!(latest.commit, oid(B));
        assert_eq!(
            qc_of(&latest).round.approval.as_ref().map(|a| a.round),
            Some(2)
        );
    }

    /// I2: round 2 opened without HEAD having moved, so its anchor *is* round 1's
    /// approval. Selecting it archives approved bytes under a different frame — two
    /// numbers, two frames, and neither asserts the other.
    #[test]
    fn an_open_round_anchored_at_the_previous_approval_names_that_older_round() {
        let issue_thread = thread(&[A, B], A, &[approval(B), new_round(2, B, B)]);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert_eq!(archive_file.commit, oid(B));
        let round = &qc_of(&archive_file).round;
        assert_eq!(round.round, 2, "the frame the selection was made in");
        let approval = round.approval.as_ref().expect("the bytes were approved");
        assert_eq!(approval.round, 1, "the round that closed on this commit");
        assert_eq!(approval.commit, archive_file.commit);
        assert!(round.superseded, "the latest round is open");
    }

    /// The invariant the metadata's readers may rely on: an unapproved entry is always
    /// flagged, because `approval: None` only happens for an open latest round, which
    /// S3 clause 2 always supersedes. The converse does not hold.
    #[test]
    fn an_unapproved_entry_is_always_superseded() {
        let threads = [
            two_closed_rounds(&[A, B, C, D], &[A, B, C, D]),
            two_closed_rounds(&[A, B, C, D, E], &[A, B, C, D, E]),
            thread(&[A, B, C, D, F], A, &[notification(C), notification(D)]),
            thread(&[A, B, C], A, &[approval(B), new_round(2, C, B)]),
            thread(&[A, B], A, &[approval(B), new_round(2, B, B)]),
            create_test_issue_thread(),
            thread_with_an_unplaceable_trailing_gap(),
            thread_with_an_unrelated_trailing_gap(),
            thread_on_two_branches(
                &[A, B],
                &[D, E, F],
                A,
                &[approval(B), new_round_on(2, E, B, FEATURE)],
            ),
        ];

        for issue_thread in &threads {
            let rounds = issue_thread.rounds().count() as u32;
            let targets = (1..=rounds)
                .map(ArchiveTarget::Round)
                .chain(std::iter::once(ArchiveTarget::Latest));
            for target in targets {
                let archive_file =
                    ArchiveFile::from_issue_thread(issue_thread, false, target).unwrap();
                let round = &qc_of(&archive_file).round;
                assert!(
                    round.approval.is_some() || round.superseded,
                    "{target:?} on {:?} produced an unapproved entry that is not flagged",
                    issue_thread.segments.len()
                );
            }
        }
    }

    /// A round that does not exist is a rejection, not a silently adjusted selection.
    #[test]
    fn selecting_a_round_the_thread_does_not_have_is_an_error() {
        let issue_thread = create_test_issue_thread();

        for target in [ArchiveTarget::Round(0), ArchiveTarget::Round(2)] {
            match ArchiveFile::from_issue_thread(&issue_thread, false, target) {
                Err(ArchiveError::RoundSelection { round, rounds, .. }) => {
                    assert_eq!(rounds, 1);
                    assert!(round == 0 || round == 2);
                }
                other => panic!("expected a RoundSelection error, got {other:?}"),
            }
        }
    }

    /// A manually added file carries no QC claim at all: the flattened `None` emits
    /// neither `milestone` nor `round`, and reads back as `None`.
    #[test]
    fn a_manually_added_file_emits_no_qc_keys_and_round_trips() {
        let mock_env = setup_mock_env_with_user();
        let files = vec![ArchiveFile::from_file(
            PathBuf::from("scripts/helpers.R"),
            create_test_object_id("123"),
            false,
        )];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let json = serde_json::to_string_pretty(&metadata).unwrap();

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let file = &value["files"][0];
        // `serde_json::Value` sorts its keys; the point is *which* keys exist.
        let keys: Vec<&String> = file.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["archive_file", "commit", "repository_file"]);
        assert!(!json.contains("approved"));

        let parsed = ArchiveMetadata::from_json(&json).unwrap();
        assert!(parsed.files[0].qc.is_none());
    }

    /// The mode-1 shape on the wire: `round` nested, `approval` present as an explicit
    /// null when the bytes were never approved.
    #[test]
    fn an_unapproved_mode_one_file_emits_a_nested_null_approval() {
        let mock_env = setup_mock_env_with_user();
        let issue_thread = thread(&[A, B, C], A, &[approval(B), new_round(2, C, B)]);
        let files = vec![
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap(),
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let json = serde_json::to_string(&metadata).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["files"][0]["milestone"], "v1.0");
        assert_eq!(
            value["files"][0]["round"],
            serde_json::json!({ "round": 2, "approval": null, "superseded": true })
        );

        let parsed = ArchiveMetadata::from_json(&json).unwrap();
        assert!(
            parsed.files[0]
                .qc
                .as_ref()
                .unwrap()
                .round
                .approval
                .is_none()
        );
    }

    /// An approval on the wire: the commit is the full 40-char hex the rest of the file
    /// uses, and `approval.round` is its own key inside `round`.
    #[test]
    fn an_approval_serializes_its_commit_as_full_hex_inside_round() {
        let mock_env = setup_mock_env_with_user();
        let issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);
        let files = vec![
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(1)).unwrap(),
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let json = serde_json::to_string(&metadata).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        let round = &value["files"][0]["round"];
        assert_eq!(round["round"], 1);
        assert_eq!(round["approval"]["round"], 1);
        assert_eq!(round["approval"]["commit"], B);
        assert_eq!(round["approval"]["by"], "reviewer");
        assert_eq!(round["superseded"], true);
        assert!(round["approval"]["at"].is_string());

        let parsed = ArchiveMetadata::from_json(&json).unwrap();
        let approval = parsed.files[0]
            .qc
            .as_ref()
            .unwrap()
            .round
            .approval
            .as_ref()
            .unwrap();
        assert_eq!(approval.commit, oid(B));
    }

    // ── Metadata structure version ───────────────────────────────────────────

    /// Serialized first, so `head`ing the file shows which shape it is.
    #[test]
    fn the_metadata_declares_its_structure_version_first() {
        let mock_env = setup_mock_env_with_user();
        let files = vec![ArchiveFile::from_file(
            PathBuf::from("scripts/helpers.R"),
            create_test_object_id("123"),
            false,
        )];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        assert_eq!(metadata.metadata_version, METADATA_VERSION);

        let json = serde_json::to_string_pretty(&metadata).unwrap();
        let first_key = json.lines().nth(1).expect("an object with fields");
        assert_eq!(first_key.trim(), "\"metadata_version\": 1,");
    }

    /// The pre-round shape carried no version field, so an absent one *is* version 0 —
    /// and version 0 is refused. It does not fail to parse on its own: `ArchiveQC` is a
    /// flattened `Option`, so a v0 file's `{milestone, approved}` reads back as
    /// `qc: None` and every QC'd file in it looks manually added. The refusal is what
    /// stops that false negative, so the observation and the refusal are asserted
    /// together.
    #[test]
    fn metadata_with_no_version_field_is_refused_as_version_zero() {
        let json = r#"{
            "creator": "wes",
            "created_at": "2026-08-19T18:36:39.248217Z",
            "files": [
                {
                    "repository_file": "scripts/analysis.R",
                    "archive_file": "scripts/analysis.R",
                    "commit": "aaaaaaa000000000000000000000000000000001",
                    "milestone": "Milestone 3",
                    "approved": true
                }
            ]
        }"#;

        // Absent ⇒ 0, and the raw shape parses happily — which is the hazard.
        let raw: RawArchiveMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(raw.metadata_version, 0);
        assert!(
            raw.files[0].qc.is_none(),
            "a v0 file's QC silently vanishes into the flattened Option"
        );

        match ArchiveMetadata::from_json(json) {
            Err(ArchiveError::UnsupportedMetadataVersion { found, known }) => {
                assert_eq!((found, known), (0, METADATA_VERSION));
            }
            other => panic!("expected version 0 to be refused, got {other:?}"),
        }
    }

    /// The refusal lives in deserialization, not in a helper: the direct
    /// `serde_json::from_str` route — the one a future caller reaches for — refuses the
    /// same documents `from_json` does.
    #[test]
    fn the_direct_deserialization_route_is_version_checked_too() {
        let too_new = format!(
            r#"{{ "metadata_version": {}, "creator": "wes",
                  "created_at": "2026-08-19T18:36:39.248217Z", "files": [] }}"#,
            METADATA_VERSION + 1
        );
        let unversioned = r#"{ "creator": "wes",
                               "created_at": "2026-08-19T18:36:39.248217Z", "files": [] }"#;

        for json in [too_new.as_str(), unversioned] {
            let error = serde_json::from_str::<ArchiveMetadata>(json)
                .expect_err("the direct route must refuse it too");
            assert!(
                error.to_string().contains("can only read version"),
                "unexpected error: {error}"
            );
        }
    }

    /// A shape from the future is refused, not read best-effort: an archive is an audit
    /// artifact, and silently dropping a field this build cannot see is worse than
    /// declining to read the file.
    #[test]
    fn metadata_from_a_newer_structure_version_is_refused() {
        let json = format!(
            r#"{{
                "metadata_version": {},
                "creator": "wes",
                "created_at": "2026-08-19T18:36:39.248217Z",
                "files": []
            }}"#,
            METADATA_VERSION + 1
        );

        match ArchiveMetadata::from_json(&json) {
            Err(ArchiveError::UnsupportedMetadataVersion { found, known }) => {
                assert_eq!(found, METADATA_VERSION + 1);
                assert_eq!(known, METADATA_VERSION);
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// The version this build writes is readable by this build — the round-trip the
    /// refusal must not catch.
    #[test]
    fn the_version_this_build_writes_is_accepted() {
        let mock_env = setup_mock_env_with_user();
        let issue_thread = create_test_issue_thread();
        let files = vec![
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap(),
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let json = serde_json::to_string(&metadata).unwrap();

        let parsed = ArchiveMetadata::from_json(&json).unwrap();
        assert_eq!(parsed.metadata_version, METADATA_VERSION);
        assert_eq!(parsed.files.len(), 1);
    }

    // ── S1 row 1's unreachable case ──────────────────────────────────────────

    /// A thread the fold cannot produce: a non-latest round left open, which **I1**
    /// forbids. Assembled by hand precisely because no comment sequence yields it.
    fn thread_with_an_open_older_round() -> IssueThread {
        let mut issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);
        match &mut issue_thread.segments[0] {
            crate::Segment::Round(round) => round.state = crate::RoundState::Open,
            crate::Segment::Gap(_) => panic!("segments[0] is Initial QC"),
        }
        issue_thread
    }

    /// Every profile reports the violation by name and archives nothing — no plausible
    /// commit is substituted, and no handler task dies. Runs in the default debug
    /// profile, which is the only one CI and lefthook build.
    #[test]
    fn an_open_older_round_is_a_named_error_not_a_panic() {
        let issue_thread = thread_with_an_open_older_round();

        match ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(1)) {
            Err(ArchiveError::NonLatestRoundOpen { round, rounds, .. }) => {
                assert_eq!((round, rounds), (1, 2));
            }
            other => panic!("expected NonLatestRoundOpen, got {other:?}"),
        }
    }

    // ── The selected-round gate, and clause 4 ────────────────────────────────

    /// The fold does not currently build an unplaceable *trailing* gap — a trailing gap
    /// walks the branch its own placed round was found on — so the shape is assembled
    /// here. It is the shape the gate question was escalated about: a real, archivable
    /// approval sitting behind a segment we could not read. `mutate` receives the
    /// trailing gap.
    fn thread_with_a_trailing_gap(mutate: impl FnOnce(&mut crate::Gap)) -> IssueThread {
        let mut issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);
        assert_eq!(issue_thread.segments.len(), 4, "R1, gap, R2, trailing gap");
        match &mut issue_thread.segments[3] {
            Segment::Gap(gap) => mutate(gap),
            Segment::Round(_) => panic!("the last segment of a closed round is its gap"),
        }
        issue_thread
    }

    fn thread_with_an_unplaceable_trailing_gap() -> IssueThread {
        thread_with_a_trailing_gap(|gap| {
            gap.placement = Placement::Unplaceable(UnplaceableReason::MergeBaseUnreachable);
            gap.commits.clear();
        })
    }

    /// A gap the fold *placed* which nonetheless owns nothing, because its ends share no
    /// history and no range between them is meaningful.
    fn thread_with_an_unrelated_trailing_gap() -> IssueThread {
        thread_with_a_trailing_gap(|gap| {
            gap.continuity = GapContinuity::Unrelated;
            gap.commits.clear();
        })
    }

    /// §18.1: the gate is the *selected round*. An unplaceable trailing gap no longer
    /// blocks a placed, closed round — that round's approval is a real sha, and what the
    /// gap costs is currency, not archivability.
    #[test]
    fn an_unplaceable_trailing_gap_does_not_block_the_rounds_approval() {
        let issue_thread = thread_with_an_unplaceable_trailing_gap();
        assert!(
            !issue_thread.active_segment().is_placed(),
            "the fixture must have an unreadable active segment"
        );

        let selection = selected_round(&issue_thread, ArchiveTarget::Latest).unwrap();
        assert_eq!(selection.round, 2);
        assert_eq!(selection.name, "Round 2");
        assert!(selection.is_archivable());
        assert_eq!(selection.refusal(), None);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();
        assert_eq!(archive_file.commit, oid(D), "round 2's approval");
    }

    /// The other half of the gate: the selected round itself owns no locatable commits,
    /// so there is no commit the archive could honestly point at.
    #[test]
    fn an_unplaceable_selected_round_is_refused_with_its_reason() {
        let missing = "fffffff000000000000000000000000000000009";
        let issue_thread = thread(&[A, B], missing, &[]);

        let selection = selected_round(&issue_thread, ArchiveTarget::Latest).unwrap();
        assert_eq!(selection.round, 1);
        assert_eq!(selection.name, "Initial QC");
        assert!(!selection.is_archivable());
        assert_eq!(
            selection.unplaceable_reason(),
            Some(UnplaceableReason::AnchorUnreachable)
        );
        // The one shared wording, not a copy of it: the CLI's trust marker, the repair
        // report and the API's skip reason all render this same string.
        assert_eq!(
            selection.refusal(),
            Some(UnplaceableReason::AnchorUnreachable.describe())
        );
        assert_eq!(
            selection.refusal(),
            Some("its commits are not on that branch")
        );
    }

    /// A target naming no round is the same error the derivation returns for it, so a
    /// surface cannot validate the round differently from what consumes it.
    #[test]
    fn the_gate_and_the_derivation_reject_the_same_out_of_range_round() {
        let issue_thread = create_test_issue_thread();

        let gate = selected_round(&issue_thread, ArchiveTarget::Round(7)).unwrap_err();
        let derivation =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(7))
                .unwrap_err();
        assert!(matches!(
            gate,
            ArchiveError::RoundSelection {
                round: 7,
                rounds: 1,
                ..
            }
        ));
        assert_eq!(gate.to_string(), derivation.to_string());
    }

    /// Clause 4, the unreadable half: the trailing gap could not be placed, so clause 3
    /// read "no file-changing commit" from a segment it could not read at all. Nothing
    /// else fires here — no later round has closed and the latest round is closed — so
    /// this asserts clause 4 alone.
    #[test]
    fn an_unplaceable_segment_after_the_selection_is_not_evidence_of_currency() {
        let issue_thread = thread_with_an_unplaceable_trailing_gap();

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        let round = &qc_of(&archive_file).round;
        assert_eq!(round.approval.as_ref().map(|a| a.commit), Some(oid(D)));
        assert!(
            round.superseded,
            "currency cannot be determined from a segment we could not read"
        );
    }

    /// Clause 4, the `Unrelated` half — the case a `placement`-only check waves through.
    /// This gap is **`Placed`** and owns nothing, so its empty commit list is not the
    /// fact that nothing landed; it is the absence of any meaningful range between its
    /// ends. Same false claim as the unplaceable case, different route.
    #[test]
    fn an_unrelated_gap_after_the_selection_is_not_evidence_of_currency() {
        let issue_thread = thread_with_an_unrelated_trailing_gap();
        let trailing = issue_thread.active_segment().as_gap().unwrap();
        assert!(
            trailing.is_placed() && trailing.commits.is_empty(),
            "the fixture must be placed *and* own nothing — that is the whole case"
        );

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert_eq!(archive_file.commit, oid(D));
        assert!(
            qc_of(&archive_file).round.superseded,
            "an Unrelated gap proves nothing about currency"
        );
    }

    /// An ordinary empty gap is **not** clause 4: its emptiness is complete information —
    /// nothing landed since the approval — which is the one state clause 3 exists to
    /// distinguish. Pinned so clause 4 cannot be widened into "any placed gap owning no
    /// commits", which would make `superseded: false` unreachable.
    #[test]
    fn an_ordinary_empty_trailing_gap_still_proves_currency() {
        let issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);
        let trailing = issue_thread.active_segment().as_gap().unwrap();
        assert!(trailing.is_placed() && trailing.commits.is_empty());
        assert_eq!(trailing.continuity, GapContinuity::Linear);

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest).unwrap();

        assert!(!qc_of(&archive_file).round.superseded);
    }

    /// The fold-reachable shape of an `Unrelated` gap: round 2 was QC'd on a branch that
    /// shares no history with Initial QC's. Selecting round 1 archives its approval and
    /// flags it — here clauses 2 and 4 both fire, which is the realistic case; the
    /// isolating tests above are what pin clause 4 on its own.
    #[test]
    fn a_round_behind_a_folded_unrelated_gap_is_archivable_and_flagged() {
        let issue_thread = thread_on_two_branches(
            &[A, B],
            &[D, E, F],
            A,
            &[approval(B), new_round_on(2, E, B, FEATURE)],
        );
        let gap = issue_thread.segments[1]
            .as_gap()
            .expect("a gap sits between");
        assert_eq!(gap.continuity, GapContinuity::Unrelated);
        assert!(gap.is_placed() && gap.commits.is_empty());

        let selection = selected_round(&issue_thread, ArchiveTarget::Round(1)).unwrap();
        assert!(selection.is_archivable());

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Round(1)).unwrap();
        assert_eq!(archive_file.commit, oid(B));
        assert!(qc_of(&archive_file).round.superseded);
    }

    // ── The refusal is the door, not a second opinion ────────────────────────

    /// The direct route: no call to [`selected_round`] anywhere in this test. A caller
    /// that skips the gate is refused by the derivation itself, with the same reason the
    /// gate would have given.
    #[test]
    fn the_derivation_refuses_an_unplaceable_round_without_consulting_the_gate() {
        let missing = "fffffff000000000000000000000000000000009";
        let issue_thread = thread(&[A, B], missing, &[]);

        let error = ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest)
            .expect_err("an unplaceable round has no commit to archive");

        match &error {
            ArchiveError::UnplaceableRound {
                file,
                round,
                reason,
            } => {
                assert_eq!(file, &PathBuf::from("src/test.rs"));
                assert_eq!(*round, 1);
                assert_eq!(*reason, UnplaceableReason::AnchorUnreachable);
            }
            other => panic!("expected UnplaceableRound, got {other:?}"),
        }
        // One wording for every surface.
        assert!(
            error
                .to_string()
                .contains(UnplaceableReason::AnchorUnreachable.describe()),
            "the message must carry the shared reason: {error}"
        );
    }

    /// The case the gate's predicate is keyed on `Placement` for: round 2 declares a
    /// branch its anchor is not on, so it is `Unplaceable` — yet it *closed*, at a sha
    /// that resolves on some other walk. The sha is identified but sits on no branch this
    /// round was walked against, so it is not one whose blob we can be confident of
    /// extracting: the derivation refuses instead of archiving it.
    #[test]
    fn an_unplaceable_round_closed_at_an_identified_sha_is_still_refused() {
        let issue_thread = thread_on_two_branches(
            &[A, B, C],
            &[A, D],
            A,
            &[approval(B), new_round_on(2, C, B, FEATURE), approval(D)],
        );

        let round_two = issue_thread.segments[2].as_round().expect("round 2");
        assert!(!round_two.is_placed(), "round 2 could not be placed");
        assert_eq!(
            round_two.closing_commit(),
            Some(&oid(D)),
            "and yet it closed at a real, identified sha"
        );

        let selection = selected_round(&issue_thread, ArchiveTarget::Latest).unwrap();
        assert!(!selection.is_archivable());
        assert_eq!(
            selection.refusal(),
            Some(UnplaceableReason::AnchorUnreachable.describe())
        );

        let error = ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest)
            .expect_err("the identified sha is not archivable");
        assert!(
            matches!(
                error,
                ArchiveError::UnplaceableRound {
                    round: 2,
                    reason: UnplaceableReason::AnchorUnreachable,
                    ..
                }
            ),
            "expected UnplaceableRound, got {error:?}"
        );
    }

    /// The other side of the same coin, restated against the new refusal: what is gated is
    /// the **selected round**, so a placed, closed round behind an unreadable *trailing
    /// gap* is still archived. The refusal must not have widened into the active-segment
    /// gate it replaced.
    #[test]
    fn the_refusal_does_not_widen_back_onto_the_active_segment() {
        let issue_thread = thread_with_an_unplaceable_trailing_gap();
        assert!(!issue_thread.active_segment().is_placed());

        let archive_file =
            ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest)
                .expect("the selected round is placed and closed");
        assert_eq!(archive_file.commit, oid(D));
        assert!(qc_of(&archive_file).round.superseded);
    }

    // ── The selection-time projection ────────────────────────────────────────

    /// The causes archiving `round` would record, for a thread that can be archived there.
    fn causes(issue_thread: &IssueThread, round: u32) -> Vec<SupersedingCause> {
        archive_preview(issue_thread, round)
            .expect("this round is archivable")
            .superseding_causes
    }

    /// Three rounds: 1 and 2 closed, 3 open — the shape that makes two clauses fire at
    /// once.
    fn three_rounds_latest_open() -> IssueThread {
        thread(
            &[A, B, C, D, E],
            A,
            &[
                approval(B),
                new_round(2, C, B),
                approval(D),
                new_round(3, E, D),
            ],
        )
    }

    /// Clause 1 alone: a later round has closed, the latest round is closed, nothing has
    /// landed since, every segment readable.
    #[test]
    fn clause_one_alone_is_a_later_approval() {
        let issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);

        assert_eq!(
            causes(&issue_thread, 1),
            vec![SupersedingCause::LaterApproval]
        );
    }

    /// Clause 2 alone: one round, open. Nothing has ever closed, so no later approval, and
    /// there is no segment after it.
    #[test]
    fn clause_two_alone_is_an_open_latest_round() {
        let issue_thread = thread(&[A, B, C, D, F], A, &[notification(C), notification(D)]);

        assert_eq!(causes(&issue_thread, 1), vec![SupersedingCause::RoundOpen]);
    }

    /// Clause 3 alone: the latest round is closed and its trailing gap changed the file.
    #[test]
    fn clause_three_alone_is_a_file_change_after_the_approval() {
        let issue_thread = two_closed_rounds(&[A, B, C, D, E], &[A, B, C, D, E]);

        assert_eq!(
            causes(&issue_thread, 2),
            vec![SupersedingCause::ChangedSince]
        );
    }

    /// Clause 4 alone: the trailing gap could not be read, so clauses 1–3 were evaluated on
    /// partial information.
    #[test]
    fn clause_four_alone_is_an_unreadable_segment_after_the_round() {
        let issue_thread = thread_with_an_unplaceable_trailing_gap();

        assert_eq!(
            causes(&issue_thread, 2),
            vec![SupersedingCause::Undeterminable]
        );
    }

    /// Clause 4 by the other route: a gap the fold **placed** which owns nothing because
    /// its ends share no history. This is the case a `placement`-only shortcut misses.
    #[test]
    fn clause_four_fires_for_an_unrelated_gap_that_was_placed() {
        let issue_thread = thread_with_an_unrelated_trailing_gap();
        let trailing = issue_thread.active_segment().as_gap().unwrap();
        assert!(trailing.is_placed() && trailing.commits.is_empty());

        assert_eq!(
            causes(&issue_thread, 2),
            vec![SupersedingCause::Undeterminable]
        );
    }

    /// And **not** for an ordinary empty gap: a `Linear` gap owning no commits is the fact
    /// that nothing landed, which is complete information. Empty causes is the positive
    /// claim of currency, and it must stay reachable.
    #[test]
    fn an_ordinary_empty_trailing_gap_yields_no_causes_at_all() {
        let issue_thread = two_closed_rounds(&[A, B, C, D], &[A, B, C, D]);
        let trailing = issue_thread.active_segment().as_gap().unwrap();
        assert_eq!(trailing.continuity, GapContinuity::Linear);
        assert!(trailing.is_placed() && trailing.commits.is_empty());

        assert!(causes(&issue_thread, 2).is_empty());
    }

    /// Causes are not mutually exclusive, and they are reported in clause order.
    #[test]
    fn a_later_approval_and_an_open_latest_round_co_occur() {
        let issue_thread = three_rounds_latest_open();
        assert_eq!(issue_thread.rounds().count(), 3);

        assert_eq!(
            causes(&issue_thread, 1),
            vec![SupersedingCause::LaterApproval, SupersedingCause::RoundOpen]
        );
    }

    /// The projection and the metadata are one derivation in two shapes. Asserted per round
    /// across every fixture: same commit, same approval — including the **I2** case where
    /// `approval.round` is older than the previewed round — and the bool is exactly the
    /// emptiness of the cause list.
    #[test]
    fn the_preview_and_the_metadata_agree_on_every_round() {
        let threads = [
            two_closed_rounds(&[A, B, C, D], &[A, B, C, D]),
            two_closed_rounds(&[A, B, C, D, E], &[A, B, C, D, E]),
            two_closed_rounds(&[A, B, C, D, E], &[A, B, C, D]),
            three_rounds_latest_open(),
            thread(&[A, B, C, D, F], A, &[notification(C), notification(D)]),
            thread(&[A, B, C], A, &[approval(B), new_round(2, C, B)]),
            // I2: round 2 open, anchored at round 1's approval.
            thread(&[A, B], A, &[approval(B), new_round(2, B, B)]),
            create_test_issue_thread(),
            thread_with_an_unplaceable_trailing_gap(),
            thread_with_an_unrelated_trailing_gap(),
        ];

        let mut saw_older_approval = false;
        for issue_thread in &threads {
            for round in 1..=issue_thread.rounds().count() as u32 {
                let preview = archive_preview(issue_thread, round)
                    .expect("every round of these fixtures is placed");
                let archive_file = ArchiveFile::from_issue_thread(
                    issue_thread,
                    false,
                    ArchiveTarget::Round(round),
                )
                .expect("and archivable");
                let recorded = &qc_of(&archive_file).round;

                assert_eq!(preview.commit, archive_file.commit, "round {round}");
                assert_eq!(preview.approval, recorded.approval, "round {round}");
                assert_eq!(
                    recorded.superseded,
                    !preview.superseding_causes.is_empty(),
                    "the bool is the emptiness of the cause list, round {round}"
                );
                assert_eq!(recorded.round, round);

                if preview
                    .approval
                    .as_ref()
                    .is_some_and(|approval| approval.round < round)
                {
                    saw_older_approval = true;
                }
            }
        }
        assert!(
            saw_older_approval,
            "the fixtures must exercise I2's older-round approval, or this test proves less \
             than it claims"
        );
    }

    /// A round that cannot be archived previews nothing — the same refusal the gate gives,
    /// so a client never has to ask why twice. The reason is already on the round's
    /// placement.
    #[test]
    fn an_unplaceable_round_previews_nothing() {
        let missing = "fffffff000000000000000000000000000000009";
        let unplaceable_initial = thread(&[A, B], missing, &[]);
        assert!(
            !selected_round(&unplaceable_initial, ArchiveTarget::Latest)
                .unwrap()
                .is_archivable()
        );
        assert_eq!(archive_preview(&unplaceable_initial, 1), None);

        // The §20.2 shape: unplaceable, yet closed at a real identified sha.
        let identified_but_off_walk = thread_on_two_branches(
            &[A, B, C],
            &[A, D],
            A,
            &[approval(B), new_round_on(2, C, B, FEATURE), approval(D)],
        );
        assert_eq!(archive_preview(&identified_but_off_walk, 2), None);
        // Its round 1 is placed and closed, so it previews normally.
        assert!(archive_preview(&identified_but_off_walk, 1).is_some());
    }

    /// A round the thread does not have previews nothing either — a caller projecting over
    /// the thread's own rounds cannot hit it, and inventing a preview for it would be a
    /// claim about a round that does not exist.
    #[test]
    fn a_round_the_thread_does_not_have_previews_nothing() {
        let issue_thread = create_test_issue_thread();

        assert_eq!(archive_preview(&issue_thread, 0), None);
        assert_eq!(archive_preview(&issue_thread, 2), None);
        assert!(archive_preview(&issue_thread, 1).is_some());
    }

    /// **No construction path can produce a foreign version.** The field is private and
    /// set in exactly two places — [`ArchiveMetadata::new`], and the version-checked
    /// conversion that admits only [`METADATA_VERSION`] — so the set of versions this
    /// build can hold, and therefore emit, is `{METADATA_VERSION}`. Without this the write
    /// side would be open while the read side is sealed, and we could emit a tarball we
    /// would refuse to read.
    #[test]
    fn no_construction_path_yields_a_version_this_build_would_refuse() {
        let mock_env = setup_mock_env_with_user();
        let issue_thread = create_test_issue_thread();

        // Path 1: the constructor.
        let constructed = ArchiveMetadata::new(
            vec![
                ArchiveFile::from_issue_thread(&issue_thread, false, ArchiveTarget::Latest)
                    .unwrap(),
            ],
            &mock_env,
        )
        .unwrap();

        // Path 2 and 3: the library's reader, and the direct serde route.
        let json = serde_json::to_string(&constructed).unwrap();
        let read = ArchiveMetadata::from_json(&json).unwrap();
        let deserialized: ArchiveMetadata = serde_json::from_str(&json).unwrap();

        for metadata in [&constructed, &read, &deserialized] {
            assert_eq!(metadata.metadata_version, METADATA_VERSION);
            // What lands in the tarball, not just what is in memory.
            let emitted: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(metadata).unwrap()).unwrap();
            assert_eq!(emitted["metadata_version"], METADATA_VERSION);
            // And what we emit, we can read back.
            assert!(
                ArchiveMetadata::from_json(&serde_json::to_string(metadata).unwrap()).is_ok(),
                "this build must never emit a document it would refuse to read"
            );
        }
    }
}
