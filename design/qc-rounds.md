# QC Rounds — Spec v1

**This document is the implementation authority.** Implementers follow it and do not
redesign. Where an implementation detail is genuinely undetermined, it is listed under
*Still open* — resolve it by asking, not by inventing. Later numbered sections supersede
earlier ones and say so explicitly; nothing above is silently edited.

---

## §0 Diagnosis

Why this spec exists. Do not "improve" the model back into these bugs.

**§0.1** `IssueThread` is flat: one `branch`, one `Vec<IssueCommit>` spanning the entire QC
life, and `parse_commits_from_comments` tracks a **single global** `approved_commit`. There
is nowhere to record "this QC has been through three review cycles."

**§0.2** `initial_commit()` asserts *exactly one* commit carries `CommitStatus::Initial`,
sourced from the body's `initial qc commit:`. There is structurally no second starting point.

**§0.3** `branch` is parsed once from the body (`parse_branch_from_body`). If cycle 2 happens
on a different branch, the body must be **edited** — mutating the audit surface — or the
commit walk silently misses commits. `get_commits_robust` walks one branch with
`stop_at = initial_commit`, so walk length grows monotonically with the QC's whole life.

**§0.4** The checklist is `# {Name}` in the **issue body**, found by `find_checklist_start`
(first H1). `analyze_issue_checklists` reads the body only. A second cycle has nowhere to put
a checklist, so users either uncheck round-1 items (destroying the round-1 record) or check
nothing.

**§0.5** `QCStatus::determine_status` scans **all** commits: `commits[..approved_index]` for
post-approval file changes, and "newest status commit covers newest file-change" over the
whole list. Every cycle lengthens the list, and `CommitSlider` renders one mark per commit —
unusable past ~20.

**§0.6** `# QC Un-Approval` is the only re-open mechanism. It means "the approval was wrong,"
but is used to mean "the file changed and needs more QC." An auditor cannot distinguish the
two from the comment log.

**§0.7** `ArchiveFile::from_issue_thread` records `{milestone, approved: bool}` and picks
`approved_commit()` or `latest_commit()`. It cannot express "I archived the round-2 approval,
and the file has changed since."

---

## §1 D — Decisions

**D1.** A QC issue is a sequence of **Rounds** alternating with **Gaps**. Rounds are
1-indexed. Round 1 is the issue itself — no new artifact.

**D2.** Round 1's metadata (start commit, branch, checklist) stays in the **issue body**,
exactly as today. **Zero migration**: every existing issue is a valid single-round issue.

**D3.** Rounds 2..n are declared by an append-only **`# QC Round N` comment**. Never by
editing the body, never by labels.

**D4.** The round comment carries metadata + the round's checklist and **nothing else**. It
never carries a diff, so `body_splitter` can never split a checklist across comment parts.

**D5.** The optional "notify the difference" is a **separate, ordinary `# QC Notification`
comment** posted immediately after the round comment, with `previous commit` = the prior
round's approval commit and `current commit` = the new round's start commit.

**D6.** Starting a round **re-opens** the issue. `# QC Un-Approval` is untouched and keeps its
exact current meaning. The two markers are the audit bifurcation: unapproval revokes,
a round continues.

**D7.** Each round owns its own **branch**, declared in its round comment (round 1's in the
body). Rounds may be on different branches.

**D8.** Commit ownership.

| Round state | The round owns |
|---|---|
| `Approved` | `[start_n ..= approval_n]` |
| `Superseded` | `[start_n .. start_{n+1})`; or `[start_n .. tip(branch_n)]` when `start_{n+1}` is not on `branch_n` (D38) |
| `Open` (last round only) | `[start_n .. tip]` |

| Gap kind | Owns |
|---|---|
| `R[n].preceding_gap` (n>1) | `(approval_{n-1} .. start_n)`, exclusive both ends |
| `thread.drift` | `(approval_last .. tip]` |

The **only** permitted overlap is `approval_n == start_{n+1}`.

**D9.** A gap's branch is **its owning round's branch**. `preceding_gap` belongs to the round
it precedes; `drift` belongs to the thread and uses the latest round's branch. Branch is
therefore never stored on a gap and never looked up forward.

**D10.** Status is derived from the **latest round plus `drift` only**. No status rule may
read an earlier round or any `preceding_gap`.

**D11.** Unapproval reopens the **latest** round: state goes closed → `Open`, and `drift`'s
commits fold into `commits`. An unapproval comment while the latest round is already open is
a no-op.

**D12.** A new round may be started **only** when `QCStatus::is_approved()` — `Approved` or
`ChangesAfterApproval`. Starting from plain `Approved` is legal and produces
`start_{n+1} == approval_n` (the D8 overlap case).

**D13.** `CommitStatus::Initial` is reused for a round start. Uniqueness weakens from *one per
thread* to **one per round**.

**D14.** Comments are attributed to rounds by **position** relative to round-comment
boundaries. A status referencing a commit not owned by that round is dropped and logged,
never applied to another round.

**D15.** Archive selection is **per round**. Metadata records the round index, whether that
commit was approved, and whether file-changing commits exist after it. It does **not** record
a total round count — that would be a claim invalidated by any later round.

**D16.** The commit slider is round-scoped. The status card always shows the latest round;
earlier rounds are reachable in `IssueDetailModal`.

**D17.** *(restated — the original clause was wrong; see R19.)* `drift` is **not** optional.
"No drift" (the latest round is `Open`) and "empty drift" (approved, nothing committed since)
are indeed distinct states — but the distinction is carried by `RoundState`, and storing it a
second time in an `Option` could only desync. Every consumer dispatches on `RoundState` before
reading drift (S1–S4). The rule that no trailing segment follows an open round (R1) is
enforced in the derived view, `segments()` (W6), not in storage.

**D18.** Rounds own their **preceding** gap; the thread owns the single **drift**. The flat
alternating sequence `R1 G2 R2 G3 R3 S` is a derived *view* (`segments()`), not the
representation. Rationale: six of seven consumers (status, archive, status API, UI slider, UI
archive select, CLI `--base-round`) address a round; only the record addresses the sequence,
and it does so read-only and in order.

**D19.** *(deleted — superseded by D9. Gap branches need no fold-time resolution.)*

**D20.** The round comment reuses the **exact same metadata keys as the issue body**:
`initial qc commit: {sha}`, `git branch: {branch}`,
`[file contents at initial qc commit]({url})`, plus one addition, `round: {N}`.
`initial qc commit` means the same thing in both places, so it is the same key parsed by the
same `parse_commit_from_pattern`. The substring collision with `relevant_files.rs`'s
`new qc initial qc commit:` is harmless because **round detection keys off the `# QC Round`
H1, never off a metadata key**, and metadata always precedes the checklist within a round
comment, so `find` hits the metadata occurrence first. The round comment carries **no**
`author:`/`collaborators:` — those are issue-level (D23 rationale, and see R14).

**D21.** *(cause list widened by D39.)* A superseded round is a **real, reachable state**. Two
causes, neither necessarily meaning malformed: an approval comment **deleted** on GitHub, or an
approval **revoked by `# QC Un-Approval`** before a later round comment — and D6 gives
unapproval a legitimate meaning, so "superseded ⇒ malformed" is an invalid inference in UI copy
and CLI messaging. The boundary is the **start of `R[n+1]`**: the malformed `R[n]`
swallows every commit up to but excluding `start_{n+1}`, and `R[n+1].preceding_gap` is
`Gap { commits: [], divergent: false }` — not `Option` (D26). This keeps `R[n]`'s notification and review
statuses inside `R[n]` where D14 can attach them; the alternative (round = `[start_n]` alone)
would discard them.

**D22.** When `approval_{n-1}` is **not** an ancestor of `start_n`, `R[n].preceding_gap` is
`divergent`. A divergent gap is walked with **no `stop_at`** — the full duration of its
owning round's branch, unlike every other commit determination in the codebase. This is an
allowable-but-undesired state, surfaced in both UI and CLI as "no cohesive history." Diffs
still work, which is what matters.
*(revised by D31.)* **Either gap position can be divergent**, from two distinct causes: a
`preceding_gap` when the *user* chose a start commit not descended from the prior approval,
and `drift` when a *force-push or rebase* removed `approval_last` from its branch. Same fact —
the approval anchor is not in this branch's ancestry — same field, same badge.

**D23.** The new round's **branch and start commit both come from the current checkout**,
read-only, exactly like `issue create`. There is **no commit picker** anywhere in the round
flow. If HEAD violates I6 the round is still created, flagged divergent per D22.

**D24.** *(generalized by F2; phrasing softened by D35.)* **No top-level
`IssueStatusResponse` field may duplicate a *round-scoped* value** — one available in `rounds[]`
or in `drift`. Deliberate server-side projections *within* a round object are governed by
D28.2, not by this clause. Removed: `commits`, `branch`, `checklist_summary`, and
`issue.checklist_name`. The frontend reads `rounds[rounds.length - 1]` — list indexing, not
derivation. Retained at top level only because they are *not* per-round: `qc_status`, `dirty`,
`drift`, `blocking_qc_status`. A top-level copy that can disagree with `rounds[last]` is a bug
class worth the UI churn to delete.

**D25.** *(superseded by D31.)* Originally: an unreachable `approval_last` gets only a logged
warning and no flag. This was wrong — see O3 and D31. An unreachable approval sets
`drift.divergent`.

**D30.** Do not encode in a **type** what **position** already determines. The trailing
segment was originally its own type, `Drift`, justified on five behavioural splits (R17). Four
of them — moving vs frozen upper bound, live vs frozen, sole status input, at-most-one-ever —
are properties of *where the value is stored*, not of the value. The fifth, can-it-diverge,
dissolved under D31. So `Drift` was deleted: `IssueThread.drift` is a `Gap`, and the field name
carries the meaning the type no longer needs to. Confirmation that these were always one
thing: starting a round now *moves* `thread.drift` into `rounds[n+1].preceding_gap` and splits
off what the new round claims (R17's partition) — a move and a split, with no conversion
between types. The **view** still labels the two positions, `Segment::Gap` vs
`Segment::Drift`, both borrowing `&Gap`: deriving labels from position is exactly a view's job.
This is the type-level form of D26/D27.

**D31.** *(O3)* `drift` can be **divergent**, contradicting D22 as originally written. If
`approval_last` was removed from its branch by force-push or rebase, `rev-list
approval_last..tip` falls back to merge-base semantics and `drift` can span the whole branch —
so R18's claim that the polled status path is always bounded was false. Setting `divergent`
makes it fail honestly: status stays `ChangesAfterApproval`, which may well be true, while the
UI badges "approval commit not in branch history" so the hash is not read as meaningful. An
approval **absent from the repo entirely** is a different failure and already handled by
`IssueError::CommitNotFound`. Rejected: treating an orphaned drift as empty and reporting
`Approved` — silently claiming no post-approval changes when the file may have changed is the
worst available failure. No payload cap: D22 already accepted unbounded walks for divergence,
and this case is rarer.

**D27.** *(F1)* `RoundState` stores two variants, `Unapproved | Approved(Approval)`. Whether
an unapproved round is *open* or *superseded* is derived from position: open if it is the last
round, superseded (D21) if a later round exists. Exposed as `DerivedState` via
`IssueThread::round_state(i)`. This deletes I3 and the second half of I12.

**D28.** *(F3)* **D26 governs stored model fields only.** It does **not** apply to:
1. **Declared vs observed pairs.** `Round.start_commit` is parsed from metadata;
   `Round.commits` comes from a git walk. `start_commit == commits.last().hash` is not
   duplication — it is two independent sources, and I4/I5 asserting agreement is the
   **validation** that catches a shallow clone, a force-push, or a non-local branch. Never
   collapse a declared value into its observed counterpart.
2. **The wire.** `RoundInfo.archive_commit`, `checklist_summary`, `subsequent_file_changes`,
   and `ChecklistSummary.percentage` are derived **server-side on purpose**. D24 and U7 want
   that redundancy; the alternative is the frontend parsing markdown checkboxes and
   re-deriving status. D26 must not be used to strip wire projections.
3. **Serialization boundaries.** `ArchiveQC` stores `approved`, `round`, and
   `subsequent_file_changes` even though a live thread could derive all three, because it is a
   detached audit snapshot with no thread present. D15 marks the correct line: freeze
   *observations*, never *claims that go stale* — which is why total round count is refused.

**D29.** *(F4)* `Round.index` is a **knowing exception** to D26: it is `position + 1` (I2). It
is kept because a `&Round` detached from the `Vec` — handed to the archive, the API
serializer, or CLI display — must be self-identifying. Mitigation: it is assigned only by the
fold, from enumeration position, and there is no public constructor that accepts it.

**D26.** No field may be `Option` when its emptiness is already determined by another field.
Concretely: `Round.preceding_gap`, `Round.checklist`, and `IssueThread.drift` are all
non-optional, defaulting to empty. Their "absent" cases are derivable — from `index == 1`,
from `summary().total == 0`, and from `RoundState::Open` respectively — so an `Option` would
duplicate a fact that can then contradict its source. Emptiness is the representation;
meaning comes from the field that determines it. This is the same reasoning that deleted I11:
a stored copy of a derivable fact is a bug waiting for a desync.

---

## §2 M — Types

**M1.**
```rust
pub struct IssueThread {
    pub file: PathBuf,
    pub milestone: String,
    pub(crate) open: bool,              // issue open/closed, unchanged
    pub blocking_qcs: Vec<BlockingQC>,  // issue-level, body-parsed, unchanged
    pub rounds: Vec<Round>,             // non-empty; rounds[0] from the issue body
    pub drift: Gap,                     // empty when latest_round() is Unapproved (D17)
}
```
`branch` and `commits` are **removed** from `IssueThread`.

**M2.**
```rust
pub struct Round {
    pub index: u32,                     // 1-based
    pub branch: String,
    pub start_commit: ObjectId,
    pub preceding_gap: Gap,             // default-empty for index == 1 (see D26)
    pub checklist: RoundChecklist,      // empty checklist ⇒ summary().total == 0
    pub commits: Vec<IssueCommit>,      // newest-first, per D8
    pub state: RoundState,
}
```

**M3.** *(revised by F1 — see D27.)*
```rust
/// Stored state. Two variants, because "superseded" is a positional fact (D27).
pub enum RoundState {
    Unapproved,            // open if this is the last round, superseded if not
    Approved(Approval),
}

pub struct Approval {
    pub commit: ObjectId,
    pub comment_id: u64,   // O2: read by nothing yet; kept for a deep-link to the comment
}

/// Derived three-state view, from `IssueThread::round_state(i)`. Never stored.
pub enum DerivedState<'a> {
    Open,                       // Unapproved + last round
    Approved(&'a Approval),
    Superseded,                 // Unapproved + a later round exists (D21)
}
```
`Superseded`-as-the-last-round and `Open`-as-a-non-last-round are not merely forbidden by
invariant — they are inexpressible.

**M4.**
```rust
/// Frozen commits between the previous round's approval and this round's start.
/// Branch is the owning round's branch (D9) — not stored.
/// Round 1's is always empty: it has no predecessor. `segments()` suppresses it by
/// position, never by emptiness (W6).
pub struct Gap {
    /// newest-first; empty is normal. **Bounds depend on position (D34):** a `preceding_gap`
    /// is exclusive at BOTH ends, `(approval_{n-1} .. start_n)`; `drift` is exclusive-lower /
    /// INCLUSIVE-upper, `(approval_last .. tip]`. S1 depends on the tip being included — the
    /// newest post-approval change is very often HEAD itself.
    pub commits: Vec<IssueCommit>,
    pub divergent: bool,             // anchor not in this branch's ancestry (D22, D31)
}

/// There is no separate type for the trailing segment (D30). `IssueThread.drift` is a `Gap`:
/// the commits after the latest approval that no round has closed yet. Its branch is the
/// latest round's (D9); it is empty when the latest round is `Unapproved`, and consumers
/// dispatch on `RoundState`, never on emptiness (S0). It is `divergent` when `approval_last`
/// is not reachable from the branch tip — force-push or rebase (D22).
```

**M5.**
```rust
pub struct RoundChecklist {
    /// The H1 heading text. **Not** derivable — `content` excludes its heading (D37).
    pub name: String,      // "" when the body/comment carries no checklist H1
    /// Everything AFTER the `# {name}` line, matching `configuration::Checklist`, whose
    /// `Display` emits `# {name}` then a blank line then `{content}`. Including the heading
    /// here would emit it twice through A5 and make F4's "second H1" rule latch onto the
    /// wrong heading in the following round.
    pub content: String,   // "" likewise
}

impl RoundChecklist {
    /// Derived, never stored — a stored copy can desync from the content it describes.
    fn summary(&self) -> ChecklistSummary;   // analyze_checklist_in_text(&self.content)
}
```
A round with no checklist is observable as `summary().total == 0` (D26). There is no
`Option` for it: no consumer branches on presence, and `ChecklistSummary::completion_percentage`
already returns 100.0 at `total == 0`.

**M6.** *(revised by D33.)* `IssueCommit` is unchanged in shape.
`CommitStatus::Approved` is **removed from storage** — it is a zero-cost function of
`RoundState::Approved(a).commit`, and a stale copy passes every invariant while corrupting
`M8::latest_commit()` and `archive_commit`. It is re-injected at serialization from `state`
(permitted by D28.2; it cannot desync, because there is one source). `CommitStatus::Initial` is
**retained**: S4 reuses the current algorithm verbatim, and that algorithm treats an
`Initial`-only commit as status-bearing, so removing the flag would flip
`AwaitingReview`/`ChangeRequested` into `ChangesToComment`. Stored variants: `Initial`,
`Notification`, `Reviewed`.

**M7.** Accessors on `IssueThread`. Consumers use these and never index `rounds` directly.
```rust
fn latest_round(&self) -> &Round;
fn round(&self, index: u32) -> Option<&Round>;
fn branch(&self) -> &str;               // latest_round().branch
fn initial_commit(&self) -> &ObjectId;  // rounds[0].start_commit
fn file_commits_after(&self, commit: &ObjectId) -> Vec<&ObjectId>;
fn checklist_summary_all_rounds(&self) -> ChecklistSummary;  // record only (R8, R13)
fn round_state(&self, i: usize) -> DerivedState<'_>;         // D27: derives Open/Superseded
fn segments(&self) -> Vec<Segment<'_>>;
```

**M8.** On `Round`:
```rust
fn approved_commit(&self) -> Option<&ObjectId>;
fn latest_commit(&self) -> &IssueCommit;   // D33: approval from `state`, then newest
                                           // Notification|Reviewed|Initial. THE single
                                           // definition — A4 names it, never restates it.
fn is_closed(&self) -> bool;               // matches!(state, Approved(_))
fn file_commits(&self) -> Vec<&ObjectId>;
```

**M9.**
```rust
pub enum Segment<'a> {
    Round(&'a Round),
    Gap(&'a Gap),      // a round's preceding gap
    Drift(&'a Gap),    // the trailing gap — same type, labelled by position
}
```
Used by the record (P7) and by I7/I8 tests. Not by status, archive, or the API.

**M10.**
```rust
pub struct ArchiveQC {
    pub milestone: String,
    pub approved: bool,
    pub round: u32,
    pub subsequent_file_changes: bool,
}
```

**M11.** There is no `RoundRecord` type. Per R13 the record needs no round-specific type.

---

## §3 I — Invariants

**Disposition (D32):** every invariant below is **log-and-flag, never fatal** — F11 is
authoritative. An invariant that fails must not prevent the fold from returning an
`IssueThread`. Rationale: these assertions run against an append-only GitHub comment log and a
git history users can rewrite; a fatal assertion turns someone else's force-push into a tool
that refuses to read a real issue. I6 additionally has a *defined* recovery (set `divergent`),
and I5/I7 inherit the same treatment under D39.

**I1.** `rounds.len() >= 1`.
**I2.** `rounds[i].index == i as u32 + 1`.
**I3.** *(deleted — inexpressible under D27, not merely forbidden.)*
**I4.** `rounds[i].commits` contains exactly one `CommitStatus::Initial` commit, and it is
`start_commit`.
**I5.** *(scoped by D39.)* For `Approved(approval)` **and `!drift.divergent`**:
`approval.commit ∈ commits` and is the newest element. When `drift.divergent` the approval was
rewritten off its branch (D31) and cannot appear in a walk of it — expected, so log and flag
(D32), never fail.
**I6.** `start_n` is `approval_{n-1}` or a descendant. Violation ⇒
`preceding_gap.divergent = true` (D22), **not** an error.
**I7.** *(scoped by D39.)* No commit hash appears in two segments, with two exceptions:
`approval_n == start_{n+1}` (D8), and **any commit inside a divergent gap**, whose no-`stop_at`
walk (D22) legitimately reaches commits that older segments already own.
**I8.** *(revised by D34; split by D41.)* **Assertable from stored state, so this is the
invariant:** a **preceding gap** is disjoint from both bounding commits, `approval_{n-1}` and
`start_n`; **drift** is disjoint from `approval_last`. Both halves must be implemented — the
drift half was missing (D41).
**Not assertable, so it is a fold requirement in F9, not an invariant:** that drift *includes*
the branch tip. No tip is stored on `IssueThread` (M1), so nothing can check it after the fold;
it is guarded by direct tests on the slice instead (D41).
**I9.** *(demoted by D40.)* Rounds 2..n are **expected** to have
`checklist.summary().total > 0` — enforced server-side at creation (A5), not asserted at fold
time. A round comment is an ordinary GitHub comment anyone may edit, so a violation signals the
F4 parse misfired or a human emptied it: log and flag (D32), never fail. Round 1 may have
`total == 0` for a legacy or hand-edited body.
**I10.** *(deleted — subsumed by D26.)*
**I11.** *(deleted — structurally impossible under D9.)*
**I12.** `round_state(i) == Superseded ⇒ rounds[i+1].preceding_gap.commits.is_empty()`. The
former "never on the last round" half is deleted: under D27 it is a tautology.
**I13.** `rounds[0].preceding_gap.commits.is_empty()` and
`!rounds[0].preceding_gap.divergent` — round 1 has no predecessor to diverge from.
**I14.** `rounds.last().state == Unapproved ⇒ drift.commits.is_empty()` and
`!drift.divergent` — with no approval there is no anchor to diverge from. The converse does not
hold: an approved round with nothing committed since also has an empty drift, which is why
consumers dispatch on `RoundState` (D17).

---

## §4 W — Walks

**W1.** `latest_round()` = `rounds.last()`.
**W2.** Post-approval changes = newest `file_changed` commit in `thread.drift`, **only when
`R.is_closed()`** (S0). Drift is never absent (D26), and an empty drift on an open round means
nothing — not "no changes". Served as `drift.newest_file_change` (D35).
**W3.** *Any change after commit X* (archive): over `segments()` newest→oldest, every segment
newer than X's owning segment, plus the part of X's own segment newer than X — test
`file_changed`.
*(D39)* When **any** gap in the thread is `divergent`, segment order no longer implies commit
order — a divergent gap's no-`stop_at` walk can hold commits older than the round before it —
so W3's ordering is unsound. In that case `subsequent_file_changes` is set **conservatively
`true`** with the divergent flag surfaced alongside it (U6). Conservative-true is the safe
direction for an audit artefact: it claims "changes may exist after this commit," never the
reverse.
**W4.** Required branch for the user's checkout = `latest_round().branch`.
**W5.** Slider commits = `round(i).commits`, defaulting to the latest.
**W6.** `segments()` applies two **positional** suppression rules — never emptiness rules:
1. skip `rounds[0].preceding_gap` (round 1 has no predecessor; emitting it would prepend a
   `G` before `R1`);
2. emit `drift` only when `rounds.last().is_closed()`.
Otherwise: for each round, emit `preceding_gap` then the `Round`; after the loop, emit
`drift`. This yields exactly `R1 (G R)* S?`, the two legal tails being `… R(closed) S` and
`… R(open)` — R1's rule, enforced here rather than in storage (D17).
Suppression must be by position, not by emptiness: an **empty `G2` is meaningful** — it is
the D8 overlap case `approval_1 == start_2` — and must still appear in the record.

---

## §5 S — Status

Let `R = latest_round()`. `QCStatus` gains **no new variants**; it is round-relative by
construction.

**S0.** Dispatch on `R.state` **first**, in one spelling throughout: `R.is_closed()` (i.e.
`matches!(state, Approved(_))`) gates S1/S2, `!R.is_closed()` gates S3/S4. Drift emptiness is
**never** used to infer which branch applies — that inference would report `Approved` for an
unreviewed QC, since an open round's drift is also empty (I14).
**S1.** `R.is_closed()` and `drift` has a `file_changed` commit →
`ChangesAfterApproval(newest such hash)`.
**S2.** `R.is_closed()` and `drift` has no `file_changed` commit (including empty) →
`Approved`.
**S3.** `!R.is_closed()` and the issue is closed → `ApprovalRequired`.
**S4.** `!R.is_closed()`, issue open: the **current** algorithm verbatim over `R.commits` —
newest `file_changed` index vs newest status-bearing index; covered ⇒ `ChangeRequested` if
that status commit is `Reviewed`, else `AwaitingReview`; not covered ⇒
`ChangesToComment(newest file_changed)`; no `file_changed` at all ⇒ the existing `None` arm
(`ChangeRequested` / `AwaitingReview` / `InProgress`).
**S5.** Because `R.commits` is bounded by the round (D8), the slider and the status scan no
longer grow with the QC's life. This is the fix for §0.5.
**S6.** `is_approved()` unchanged ⇒ blocking-QC gating reads the *latest* round. A gating QC
that starts round 2 blocks again.
**S7.** `Superseded` never reaches status: under D27 the last round is by definition never
superseded, so S1–S4 see only `Open` and `Approved`.

---

## §6 F — Fold / build

**F1.** Fetch issue + comments (existing cache path).
**F2.** Round 1 from the body: `initial qc commit:`, `git branch:`, checklist section at
`find_checklist_start` (which returns the offset **of** the `# ` line), split per D37/F4.
**F3.** Scan comments for `# QC Round` H1s. Partition the comment list at those indices;
partition `k` → round `k`. **Comment order is authoritative**; a `round: N` that disagrees
with its ordinal logs a warning, never an error.
**F4.** Parse each round comment with the **same key set as the body** (D20). The checklist
**section** runs from the **second H1 to the end of the comment**. Split it per D37: `name` =
that H1's heading text, `content` = everything after that line. Any further H1s remain inside
`content` as subsections. `summary()` is `analyze_checklist_in_text(&content)` per M5 — a raw
checkbox-regex scan that never calls `split_body_into_sections`, so the heading is immaterial
and excluding it is correct. *(Corrected by D42; this clause previously claimed `summary()` must
see the heading.)*
**F5.** *(revised by D33.)* Per partition, parse the existing markers: `current commit:` →
`CommitStatus::Notification`, `# QC Review` + `comparing commit:` →
`CommitStatus::Reviewed`, `approved qc commit:` → **`RoundState::Approved(_)`, not a commit
flag** (D33 removed `CommitStatus::Approved` from storage), `# QC Un-Approval` → reset that
partition's state to `Unapproved`. Because approvedness now lives in exactly one place, an
unapproval cannot leave a stale flag behind. Approval and unapproval are
**partition-local**, not global — this is the core change to
`parse_commits_from_comments`.
**F6.** *(corrected — the original produced the three-variant enum D27/M3 deleted.)* Terminal
state per round: `Approved(approval)` if its partition has a surviving approval, else
`Unapproved`. The fold stores nothing about open-vs-superseded; that is
`IssueThread::round_state(i)`, derived from position (D27).
**F7.** For each distinct branch across rounds, one `get_commits_robust` walk with `stop_at` =
the oldest boundary commit expected on that branch — **except** a divergent gap, walked with
no `stop_at` on its owning round's branch (D22).
**F8.** One `find_or_cache_file_changes` per (branch, path), including `## File History` old
paths, as today.
**F9.** Assign commits by the D8 tables. Populate each round's `preceding_gap` and the thread's
`drift`. **Fold requirement (D41):** `drift` must be exclusive of `approval_last` and
**inclusive of the branch tip**. This cannot be re-checked after the fold — no tip is stored
(M1) — so it must be guarded by direct tests on the slice, not left to I8.
**F10.** Attach statuses per D14.
**F11.** Assert I1–I14; log and flag violations.

---

## §7 A — API

**A1.** `IssueStatusResponse` gains `rounds: Vec<RoundInfo>` and a top-level
`drift: Gap` (always present; empty when the latest round is unapproved — D17).
**A2.** `IssueStatusResponse.commits` is **removed** (D24).
**A3.** *(revised by F2, extended by D35.)* Removed from `IssueStatusResponse` per D24:
`branch`, `checklist_summary`, `commits`, and `issue.checklist_name`. Removed from the nested
`qc_status` object per D35: `approved_commit`, `initial_commit`, `latest_commit` — all three are
round-scoped and duplicate `rounds[last].state`, `rounds[0].start_commit`, and
`rounds[last].archive_commit`. `qc_status` retains only `status` and `status_detail`: it is a
**verdict, not a commit carrier**.
Consumers read `rounds[rounds.length - 1]` and `drift`. Verified affected files: `IssueCard`,
`SwimLanes`, `StatusTab`, `IssueDetailModal`, `FileResolveModal`, `ArchiveTab`, and the
milestone status table — accepted churn. `ArchiveTab`'s `approved_commit ?? latest_commit`
becomes `rounds[i].archive_commit`, which is what it was always approximating.
**A4.**
```ts
// D36: a tagged union, so `state: 'open'` + a non-null approval is INEXPRESSIBLE on the
// wire — mirroring M3's guarantee instead of undoing it. Carries `comment_id` for the
// approval-comment deep-link (O2), which no previous shape exposed.
type RoundStateWire =
  | { kind: 'open' }
  | { kind: 'approved'; commit: string; comment_id: number }
  | { kind: 'superseded' }

interface RoundInfo {
  index: number
  branch: string
  start_commit: string
  state: RoundStateWire                 // sole encoding of approvedness (D36)
  checklist_name: string                // "" when the round has no checklist
  checklist_content: string             // excludes its `# ` heading line (D37)
  checklist_summary: ChecklistSummary
  commits: IssueCommit[]
  preceding_gap: Gap                    // the one Gap shape, not inlined fields (D30)
  archive_commit: string                // = Round::latest_commit().hash (M8) — never restated
  subsequent_file_changes: boolean      // committed changes only
}

interface Gap {                         // both positions share one shape (D30)
  commits: IssueCommit[]                // [] is normal and meaningful
  divergent: boolean                    // anchor not in this branch's ancestry (D31)
  newest_file_change: string | null     // D35: the hash S1 reports in ChangesAfterApproval
}
```
The status card reads `drift`; the round switcher reads `rounds[]`. `drift` is never null — the
frontend checks `rounds[rounds.length - 1].state.kind` to know whether it is meaningful, the
same dispatch the backend uses (S0). `drift.newest_file_change` is the **only** legal source
for the `ChangesAfterApproval` hash: `SwimLanes` currently rebuilds it by scanning
`status.commits`, which A2 deletes, and rescanning `drift.commits` client-side would violate
U7.

**A5.** `POST /api/issues/:number/rounds`
```ts
{ start_commit: string, branch: string,
  checklist: { name: string, content: string },
  notify: boolean, note: string | null, include_diff: boolean }
→ { round_index: number, comment_url: string, notification_url: string | null }
```
Server-side: **validate the checklist is non-empty** (D40 — I9 is no longer a fold-time
assertion), post the round comment, re-open the issue, then post the notification if requested.
Invalidate the issue's comment cache.

**A6.** `ArchiveFileRequest` gains `round?: number` and `subsequent_file_changes?: boolean`.
**A7.** `openapi/openapi.yml` must be updated for A1–A6, per `AGENTS.md`.

---

## §8 C — CLI

**C1.** `ghqc issue round create` —
`--milestone --file --checklist-name --base-round <N> --note --notify/--no-notify --no-diff`.
**No `--commit` or `--branch`**: both come from the checkout (D23). Interactive prompts mirror
the modal when flags are absent.
**C2.** `create` is the only `round` subcommand at v1. `round list` is omitted —
`ghqc issue status` covers it (C3). `round edit` / `round checklist` are rejected: they would
mean editing a posted comment, which is audit-hostile.
**C3.** `ghqc issue status` prints a per-round table (round, branch, start, approval, checklist
n/m, preceding-gap commit count, divergent flag), then the latest round's detail as today.
**C4.** `ghqc milestone archive` prompts for a round per QC-attached file, defaulting to the
latest.
**C5.** `unapprove`, `approve`, `comment`, `review` — flags unchanged; they operate on the
latest round implicitly.

---

## §9 U — UI

**U1.** `IssueCard`: when `qc_status.status ∈ {approved, changes_after_approval}`, render a
**New Round** button. Not in the notification modal.
**U2.** `NewRoundModal` — tabbed, no scrolling:
  - **Round**: branch and start commit shown **read-only** from the checkout; notify checkbox
    + note; `include_diff`. No commit picker (D23).
  - **Checklist**: base-round `<Select>` (shown when >1 prior round), editable name, editable
    content pre-seeded from the chosen round with all `- [x]` reset to `- [ ]`.
  - **Preview**: rendered round comment.
**U3.** The notify checkbox is disabled with an explanation when the checkout commit equals
the prior approval commit — the empty-drift case, nothing to diff.
**U4.** `IssueDetailModal`: a round switcher (segmented control) above `CommitSlider`; the
slider renders the selected round's commits only, defaulting to the latest.
**U5.** `ArchiveTab`: per QC-attached file, a round `<Select>` defaulting to the latest; shows
the resolved commit and a warning badge when `subsequent_file_changes`.
**U6.** A divergent gap renders a distinct **"no cohesive history"** badge — on the round
switcher and in the new-round modal for a divergent `preceding_gap`, and on the status card
for a divergent `drift`, where the badge reads **"approval commit not in branch history"**
(D31) so the reported `ChangesAfterApproval` hash is not read as meaningful.
**U7.** The UI never derives status, round boundaries, or commit ownership. It renders
`rounds[]`, `drift`, and `qc_status` as given. In particular the `ChangesAfterApproval` hash
comes from `drift.newest_file_change` (D35) — never from rescanning `drift.commits`.
**U8.** The approved-commit row deep-links the approval comment via `state.comment_id` (D36).
This is why `Approval.comment_id` is stored (O2/R22); before D36 no wire shape exposed it.

---

## §10 P — Phases

Dependency-ordered. P1→P2 are sequential; P3 depends on P1–P2; P4/P5/P7 are independent of
each other given P3; P6 depends on P3 and P5.

**P1.** `Round` / `RoundState` / `Gap` / `Segment` types, the fold (F1–F11), and
I1–I14. `src/issue.rs`.
**P2.** `QCStatus::determine_status` on the latest round + drift. `src/qc_status.rs`.
**P3.** Round comment body type, `POST /rounds`, `RoundInfo` + `drift` in status responses,
removal of `commits`, openapi. `src/round.rs` (new), `src/api/`.
**P4.** `ghqc issue round create` + interactive prompts + `issue status` round table.
**P5.** Archive: `ArchiveQC` fields, `from_issue_thread(round)`, archive API/CLI/UI round
selection.
**P6.** UI: `NewRoundModal`, round switcher, archive round select, Playwright fixture churn
from A2.
**P7.** Record: `checklist_summary` → all-rounds sum (required for correctness — reading the
body alone would now silently miss rounds 2..n), `latest_qc_commit` → latest round's latest
commit. **No new template sections, no new types.**

---

## §11 Resolved questions

**R1.** Two legal tails: `R(closed) -> G` or `R(open)`; a closed round always has a
possibly-empty trailing segment. — *user*. → D17, I14, S2.
**R2.** Gap branch: the succeeding round's, except the trailing one which takes its own
round's. — *user*. → D9.
**R3.** Reuse the body's exact metadata keys (`initial qc commit:`, `git branch:`) plus
`round:`. Substring collision is a non-issue because detection keys off the `# QC Round` H1.
— *user pushback, my lean reversed*. → D20, F4.
**R4.** Comment order is authoritative; a mismatched `round: N` logs a warning. — *user*. → F3.
**R5.** Checklist = second H1 to **end of comment**; later H1s are subsections, not a
terminator. — *user*. → F4.
**R6.** `Superseded` is real (deleted approval comment). Boundary is `start_{n+1}`; `R[n]` is
the malformed one. — *user*. → D21, I12, M3.
**R7.** Merge-base semantics; `divergent` walks the **whole** branch with no `stop_at`;
surfaced in UI and CLI as "no cohesive history"; an allowable state, since we primarily take
diffs. — *user*. → D22, I6, F7, U6.
**R8.** Status and the milestone table use the latest round only; the record sums all rounds,
to show that not everything was checked off during the QC. — *user*. → A3, M7, P7.
**R9.** `ghqc issue round create`; `create` is the only subcommand, `list` is covered by
`issue status`, `edit` is rejected as audit-hostile. — *user pushback, my lean reversed*.
→ C1, C2.
**R10.** Branch **and** start commit are read-only from the checkout, like `issue create`. No
commit picker anywhere. — *user*. → D23, U2, C1.
**R11.** `subsequent_file_changes` counts committed changes only. — *user*. → A4.
**R12.** Field name `subsequent_file_changes`. — *user*. → M10.
**R13.** Minimal record change; no `RoundRecord`, no new template sections. The record's job is
to capture the GitHub history, which round comments enter naturally. — *user pushback, my lean
reversed*. → M11, P7.
**R14.** Per-round assignees and relevant files are out of scope. Rounds are a speed helper;
when the people or relevant files change, use the Previous QC relevant-file strategy. —
*user*. → D20.
**R15.** Drop `IssueStatusResponse.commits`; one API-side source per derived value, nothing
derived in the frontend. — *user pushback, my lean reversed*. → D24, A2.
**R16.** Keep `segments()`; refine during implementation. — *user*. → M9.
**R17.** Representation: `Vec<Round>` where each round owns its **preceding** gap, and the
thread owns a single **`Drift`** for the trailing segment. — *user proposal, adopted over the
assistant's trailing-gap-on-the-round design*. Rationale: indexing a gap by its owner makes
D9's branch rule uniform (a gap's branch is its owner's branch), which deletes the `branch`
field from both gap types and deletes invariant I11 entirely; no consumer ever looks forward.
The trailing segment earns a distinct type on four behavioral splits — moving vs frozen upper
bound, live vs frozen, sole status input vs never a status input, cannot vs can be divergent —
and a top-level field because there is at most one, ever. The `Drift → Gap` transition *is*
round creation: `S(approval_n..tip] = G[n+1](approval_n..start) ⊎ R[n+1][start..tip]`, a clean
partition. → D9, D17, D18, M1–M4, M9, I11 (deleted), I13, I14.
**R25.** Adversarial review pass before implementation, two independent read-only reviewers
(storage lens and behavioural lens), findings verified against the file and the existing code by
the spec owner. — *user-requested*. Outcome: 1 collapse accepted (`CommitStatus::Approved`,
D33), 1 collapse **declined** (`CommitStatus::Initial` — the S4-verbatim cost was judged not
worth paying), and 8 correctness fixes (D32, D34–D40). The reviewers disagreed on I8: the
behavioural lens called it subsumed by I7+D8 and wanted it deleted; the storage lens showed its
premise was stale, because post-D30 I8 also governs `drift`, whose upper bound is inclusive.
Storage was right — I8 was not redundant, it was **wrong**, and deleting it would have dropped
the only statement of drift's bounds (D34). Both reviewers independently flagged the M8/A4
`archive_commit` priority drift; A4 now names `M8::latest_commit()` rather than restating it.
Also verified clean: §5 faithfully reproduces `qc_status.rs:51–121` including its edge arms;
`Gap.divergent` is correctly stored rather than re-derived per poll; `drift` and
`rounds.last().commits` are genuinely two things; D24's removal list matches the real struct.

**R26.** `CommitStatus::Initial` stays duplicated. — *user: "Don't collapse"*. A deliberate,
documented exception to D26, on the same footing as D29's `Round.index`: the derivation is real
but collapsing it costs a restatement of S4's "verbatim" guarantee, which is load-bearing for
§0.5's fix.

**R21.** `RoundState` collapsed to two variants per F1/D27. — *user: "Changing of F1 is
good."* Confirmed after review of the three-variant form.

**R22.** Keep `Approval.comment_id` (O2) and attach statuses to a superseded round's swallowed
commits (O4). — *user*.

**R20.** Audit of the whole model against "don't store what you can derive" — *user-requested
review*. Two hits: `RoundState`'s third variant stored a positional fact (F1 → D27, deleting
I3 and half of I12), and D24 had been applied to `commits` but not to the identical `branch` /
`checklist_summary` / `issue.checklist_name` (F2 → D24 generalized, A3). Three exemptions
documented so the principle is not over-applied: declared-vs-observed pairs, wire projections,
and serialization boundaries (F3 → D28). One knowing exception recorded with its mitigation:
`Round.index` (F4 → D29). One loose end: `Approval.comment_id` is unused (F5 → O2).
→ D24, D27, D28, D29, M3, M7, M8, A3, I3 (deleted), I12, S7.

**R19.** No `Option` whose emptiness is determined by another field. `Round.checklist`,
`Round.preceding_gap`, and `IssueThread.drift` are all non-optional and default to empty. —
*user pushback on M2, accepted and generalized*. The user's argument was that `None` on
`preceding_gap` is fully derivable from `index == 1` (I13), so the `Option` carried no
information; that is the identical argument the assistant used to delete I11, and it applies
verbatim to I14 (`drift.is_some() ⟺ rounds.last().is_closed()`), so `Drift` lost its `Option`
too. This makes **D17 as originally written wrong**: the distinction between "no drift" and
"empty drift" is real but is carried by `RoundState`, not by an `Option`, and storing it twice
could only desync. Extended by the same reasoning to `RoundChecklist.summary`, a pure
function of `content` that is now a method rather than a stored field. R1's "no trailing
segment after an open round" moves from storage into the `segments()` view (W6). → D17
(restated), D26, M1, M2, M4, M5, I9, I10 (deleted), I13, I14, W6, S0, A1, A3, A4.

**R23.** `Drift` deleted as a type; `IssueThread.drift` is a `Gap`. — *user: "doesn't that make
it the same as Gap? We can then drop the type, but keep the field within the issuethread"*.
Accepting D31 (drift can diverge) removed the only structural split between the two, leaving
four splits that are all positional. → D30, D31, M1, M4, M9, I14, A1, A4, U6, P1. Supersedes
the separate-type half of R17; R17's ownership decision (rounds own their preceding gap, the
thread owns the trailing one) stands unchanged.

**R24.** `drift` can be divergent (O3 accepted). — *user, by accepting the type merge*. → D31,
D25 (superseded), D22 (revised), I14. **Corrects R18**, which claimed the polled status path
could never carry an unbounded walk.

**R18.** *(corrected by R24 — the premise below is false.)* Divergent-gap payload size is a
non-issue: `Drift` cannot be divergent (D22), so the
polled status path can never carry an unbounded walk. No cap needed. → resolved by R17.

---

## §12 Still open

**O1.** *(resolved — the name `Drift` stands. User: "I like Drift enough".)*

**O2.** *(resolved — keep `Approval.comment_id`. User: "Keep.")* Deep-linked from the
approved-commit row per U8; exposed on the wire by D36's tagged union, which is what finally
made this rationale reachable.

**O3.** *(resolved — accepted; see D31 and R24. Retained below for the reasoning.)*
**An orphaned approval commit breaks two claims this spec makes.** If `approval_last` was
removed from its branch by a force-push or rebase, `rev-list approval_last..tip` falls back to
merge-base semantics and `Drift` can be as large as the entire branch. Therefore:
1. **R18/D22 are false in that case.** They claim the polled status path can never carry an
   unbounded walk *because* `Drift` cannot be divergent. It can. And unlike a `Gap`, `Drift`
   is on the status endpoint, for every issue in a milestone, on every poll.
2. **S1 then yields a wrong status, not a missing badge.** A ballooned drift almost certainly
   contains file-changing commits, so the issue reports `ChangesAfterApproval` against a
   commit unrelated to the approval — indefinitely.

The decision is therefore whether **`divergent` belongs on `Drift` as well as `Gap`**,
contradicting D22's "only a `Gap` can be divergent."
➡️ **Lean: yes — drift carries `divergent`, symmetric with `Gap`.** *(This lean was written
when `Drift` was still a separate type; per D30 there is now one `Gap` type and no `Drift`
struct — do not create one from this paragraph.)* The condition is
semantically the same fact as `Gap.divergent` (the approval anchor is not in this branch's
ancestry), so two names for it would be the D26 smell inverted. It fails honestly: status
stays `ChangesAfterApproval`, which may well be true, while the UI badges "approval commit not
in branch history" so the hash is not read as meaningful. Severity resolves itself — a merely
non-ancestor approval still exists and diffs still work, so D22's philosophy holds unchanged;
an approval **absent from the repo** already fails through the existing
`IssueError::CommitNotFound` path, so the two failure modes are already separated and no new
error kind is needed. Costs, stated: R18 must be corrected, and the unbounded-payload case
returns for this broken-repo state. No cap is warranted — D22 already accepted unbounded walks
for divergence, and this is rarer.
Rejected: treating an orphaned drift as *empty* and reporting `Approved`. Silently claiming no
post-approval changes when the file may well have changed is the worst available failure.
**Blocking M4 only if accepted; otherwise D25 stands as written.**

**O4.** *(resolved — attach them. User: "Its likely fine.")* Under D21, statuses posted
against commits a superseded round swallowed are attached to that round rather than dropped,
making the malformed round's history more complete. If the fold makes this awkward in P1,
raise it rather than silently dropping the statuses.

---

## §13 Resolutions from adversarial review (D32–D40)

Two independent read-only reviewers audited §0–§12 before implementation. **These clauses
supersede any earlier text they contradict.** Where an earlier clause was stale it was also
corrected in place and marked; where a decision is new it is defined here.

The largest yield was not more collapses — it was that **D22 and D31 were added late and never
propagated into §3**, leaving two invariants that fail on data the spec declares legal.

**D32.** *(Invariant disposition — the cheapest high-value fix; defuses D39 and D40.)* Every
invariant in §3 is **log-and-flag, never fatal**; F11 is authoritative and §3's preamble
formerly implied the opposite by singling out I6. These assertions run against an append-only
GitHub comment log and a git history users may rewrite, so a fatal assertion converts someone
else's force-push into a tool that refuses to read a real issue. The fold always returns an
`IssueThread`.

**D33.** *(Collapse — accepted.)* `CommitStatus::Approved` is **removed from storage**: it is a
zero-cost function of `RoundState::Approved(a).commit`. It is not protected by D28.1, which
defends keeping a *declared* value alongside its *observed* walk — this flag was neither, only a
memoised join marker recording which walked commit the declared sha matched, while
`Approval.commit` survives untouched. The concrete bug it removes: nothing asserted that an
`Unapproved` round carries no `Approved` flag, so a fold that missed F5's flag-reset passed every
invariant while `M8::latest_commit()` and `archive_commit` returned the stale commit — freezing
the **wrong blob** into an archive. Re-injected at serialization from `state` under D28.2, where
it cannot desync.
**`CommitStatus::Initial` is deliberately NOT collapsed.** S4 reuses the current algorithm
verbatim, and its `!statuses.is_empty()` scan treats an `Initial`-only commit as status-bearing;
removing the flag would flip `AwaitingReview`/`ChangeRequested` into `ChangesToComment`. The
duplication is knowingly retained to keep S4 verbatim. Stored variants: `Initial`,
`Notification`, `Reviewed`.

**D34.** *(Bounds — a D30 leftover.)* `Gap.commits` bounds are **position-dependent**, and M4's
shared doc comment plus I8 were both written when `Gap` was preceding-only. A `preceding_gap` is
exclusive at both ends; **`drift` is exclusive-lower and inclusive-upper** per D8's
`(approval_last .. tip]`. S1 depends on the tip being included — the newest post-approval change
is very often HEAD. An implementer trusting the old text would exclude the tip and S1 would
silently report `Approved` for a file changed in HEAD.

**D35.** *(D24 applied one level deeper.)* The nested `qc_status` object escaped the D24 pass.
Its `approved_commit`, `initial_commit`, and `latest_commit` are all round-scoped, duplicating
`rounds[last].state`, `rounds[0].start_commit`, and `rounds[last].archive_commit`; §7 never
mentioned them, and `ArchiveTab` already read `approved_commit ?? latest_commit` as its archive
commit — giving one payload two independently-computed sources for it. All three are removed;
`qc_status` becomes a **verdict, not a commit carrier** (`status` + `status_detail`).
Consequently the `ChangesAfterApproval` hash, which `QCStatusEnum` drops and `SwimLanes`
currently rebuilds from `status.commits` (deleted by A2), is served as
`Gap.newest_file_change` — on the shared `Gap` shape, so one wire type keeps one shape. Any
client-side rescan of `drift.commits` violates U7.

**D36.** *(Wire mirrors storage.)* `RoundInfo` encoded approvedness three ways — `state`,
`approved_commit`, and `commits[].statuses` — so the projection re-admitted the impossible
combinations M3 had made inexpressible. `state` becomes a **tagged union** carrying the
approval: `{kind:'open'} | {kind:'approved', commit, comment_id} | {kind:'superseded'}`. One
encoding, impossible states unrepresentable on the wire, and `comment_id` exposed — which
retroactively makes O2's deep-link rationale reachable (U8). `RoundInfo` also now embeds
`preceding_gap: Gap` instead of inlining `preceding_gap_*` fields, so the one `Gap` type has
one wire shape.

**D37.** *(One-bit ambiguity, compounding per round.)* `RoundChecklist.content` **excludes** its
`# {name}` heading line, matching `configuration::Checklist`, whose `Display` emits the heading
itself. Therefore `name` is **not** derivable and is correctly stored. Under the opposite
reading — which F2/F4 as written implied — `POST /rounds` would emit the H1 twice, U2 would show
`# Name` inside the editable textarea, and F4's "second H1" rule would latch onto the wrong
heading in the following round, compounding each round.
*(Corrected by D42.)* `summary()` is `analyze_checklist_in_text(&content)` per M5 and needs no
header context; the earlier claim that it must see the heading was wrong and is withdrawn.

**D38.** *(D8 hole.)* D7 permits rounds on different branches, so `[start_n .. start_{n+1})` can
name an upper bound not present on `branch_n`, leaving a superseded round's commit set
undefined. Rule: when `start_{n+1}` is not on `branch_n`, `R[n]` owns
`[start_n .. tip(branch_n)]`.

**D39.** *(D22/D31 propagated into §3 — the review's largest finding.)* Two invariants failed on
legal data:
1. **I5 was unsatisfiable in exactly the state D31 creates.** A divergent drift means the
   approval was rewritten off its branch, so it cannot appear in a walk of that branch. I5 is
   scoped to `!drift.divergent` and otherwise log-and-flag (D32).
2. **I7 fired on an ordinary forked feature branch.** D22 mandates a divergent gap be walked
   with no `stop_at`, which legitimately reaches commits an older segment owns — e.g. round 1 on
   `main` (C1…C5, approved C5) with `feat` forked at C3. I7 gains an exception for commits inside
   a divergent gap. This is the most common shape of divergence, not an exotic case.
3. **W3's ordering is unsound under divergence**, since a divergent gap can hold commits older
   than the round before it. `subsequent_file_changes` is then set conservatively **`true`** with
   the divergent flag surfaced — the safe direction for an audit artefact, claiming "changes may
   exist after this commit," never the reverse.
4. **D21's cause list was too narrow.** It named only a deleted approval comment; F5+F6 also
   admit an unapproval before a later round comment, and D6 makes unapproval legitimate — so
   "superseded ⇒ malformed" is an invalid inference in UI and CLI copy.

**D40.** *(Assertion on user-editable input.)* I9 required a non-empty checklist on rounds 2..n,
but a round comment is an ordinary GitHub comment anyone may edit, and A5 specified no
server-side check — so the sanctioned path could produce an issue the fold refused to read. I9
is demoted to a logged expectation (D32) and the validation moves to A5, where it can actually be
enforced. This also restores D26's stated derivation (`summary().total == 0` meaning "no
checklist"), which I9 had made unreachable for rounds 2..n.

---

## §14 Resolutions from implementation review (D41–D43)

From the adversarial review of P1. **These supersede any earlier text they contradict.**

**D41.** *(I8's drift half.)* I8 as written by D34 had two halves and only one is checkable.
Disjointness — a preceding gap excluding both anchors, drift excluding `approval_last` — is a
property of stored state and stays an invariant; **both halves must be implemented**, and the
drift half was missing entirely. "Drift includes the branch tip" is *not* checkable after the
fold, because M1 stores no branch tip and should not start: it moves to F9 as a **fold
requirement** guarded by direct tests on the slice. The practical stakes are real — the reviewer
mutated drift's bound and it was caught only by explicit field assertions, never by the named
invariant, which is exactly the latent hole R25 set out to find.

**D42.** *(F4/D37 rationale withdrawn.)* F4 and D37 claimed `summary()` must be computed over
the checklist section *including* its heading, "because `split_body_into_sections` discards
everything before the first level-1 header." That rationale is wrong and is withdrawn. M5
defines `summary()` as `analyze_checklist_in_text(&content)` — a raw checkbox-regex scan that
never calls `split_body_into_sections` and needs no header context — so excluding the heading
(D37) is correct. The stale sentence survived from a header-anchored implementation that was
never built. **The code is right; the spec's stated reason was wrong.**

**D43.** *(Degenerate-bound logging.)* The Superseded-round degenerate fallbacks
(`start_{n+1} == start_n`, and D38's branch-mismatch case) must `log::warn!`, as the
Approved-round fallback already does. D32's disposition is log-and-flag, not silently cope; the
asymmetry was an oversight, not a decision.

---

## §15 Resolutions from implementation escalations (D44–D49)

Six items the implementation surfaced that §0–§14 did not settle. **These supersede any earlier
text they contradict.** All six were decided by the spec owner.

**D44.** *(`Approval.comment_id` — no sentinel.)* `GitComment.id` must be `Option<u64>`, and
`Approval.comment_id` with it. The rejected approach was `#[serde(default)]`, which makes a
missing id deserialize to `0` — a **valid-looking `u64` indistinguishable from a real comment
id**, i.e. a stored value that lies. `None` means "unknown" honestly: the wire's `comment_id` is
nullable and the UI omits U8's deep-link when it is absent. Consequences: no disk-cache version
bump, no migration, no forced re-fetch; comment entries cached before this change degrade
gracefully and self-heal on the next refresh. This is D26's principle applied to a sentinel
rather than to an `Option`.

**D45.** *(`POST /rounds` surfaces partial failure.)* A5's sequence is: post the round comment,
re-open the issue, post the optional notification. Steps 2 and 3 stay **non-fatal** — the round
comment is already posted and is what the fold reads — but they must no longer be silent:
- The response carries **`reopened: bool`**. If re-opening fails the issue stays closed with an
  unapproved latest round, and S3 reads that as `ApprovalRequired` — a round that was just
  created reporting "approval required." That must be visible, not logged.
- **`notification_url: null` must stop conflating** "not requested" with "requested but
  failed"; the response distinguishes them so a client can offer to post it manually.
Rationale: D22, D31, and D39.3 all chose *proceed but flag visibly* over failing or hiding.
Silently-wrong status is the one outcome that pattern rejects.

**D46.** *(Interactive round picker uses D12's gate, not a proxy.)* `round create`'s interactive
picker must not filter on the raw GitHub `IssueState`. D12's gate is `is_approved()`, a function
of `RoundState`; the two normally coincide because approval closes the issue, but an approved
issue manually reopened on GitHub without a `# QC Un-Approval` stays round-startable while
vanishing from the picker. The picker therefore lists issues in **any** state and lets the
existing `ensure_round_startable` gate reject the selection with its message. This buys exact
D12 parity for zero extra thread folds — the gate already exists, it merely runs after selection
instead of before — and matches how the `--milestone`/`--file` flag path already behaves.

**D47.** *(Round-comment preview endpoint.)* Add **`POST /api/preview/round`**, mirroring the
existing `/api/preview/issue`. Without it U2's Preview tab re-implements
`QCRound::generate_body` client-side and cannot produce the
`[file contents at initial qc commit](url)` line at all, because that URL comes from
`GitHelpers::file_content_url` and the UI cannot compute it without guessing the host. That is
the same one-fact-two-implementations shape D35 and M8 were written to remove — and here the
drift shows the user a preview that is not what gets posted.

**D48.** *(REVERSED by D50 — see §16. `Issue.branch` is retained.)* This clause originally
removed `Issue.branch` from the wire. That was wrong: it is load-bearing for cheap list paths,
and D50 gives it a precise validity condition instead.

**D49.** *(The body's `## QC Rounds` pointer — an explicit, narrow exception to D3.)*

The issue body gains a **`## QC Rounds`** section, rewritten on each round creation, telling a
human reader that the QC continues below.

**Why this is worth an exception to D3.** D3 forbids declaring rounds by editing the body, and
§0.3 diagnosed body-editing as mutating the audit surface. That reasoning is unchanged for
*authority*. But this section is not for code — it is for a person opening the issue on GitHub.
Every existing user's mental model is "the body is the whole QC"; as rounds roll out, the
content that matters most moves into comments, and nothing in the body says so. A reader who
does not know to scroll will read round 1's checklist as the current state. No amount of
server-side derivation fixes that, because the reader is looking at GitHub, not at this tool.

**Constraints — these are what keep it from becoming authority:**
1. **Comments remain the sole authority for rounds (D3 intact).** The fold MUST NOT parse this
   section for any purpose. Round count, round metadata, and round checklists come only from
   `# QC Round N` comments and the body's round-1 metadata.
2. **Placement: an H2, spliced immediately before the checklist H1**, in the same slot and by
   the same idiom as `## File History` (`splice_file_history` / `find_checklist_start`). It must
   never be an H1 — `find_checklist_start` returns the first H1, so an H1 pointer would be
   parsed as round 1's checklist and break the fold.
3. **Prose, not a metadata key.** Nothing parses it, so it needs no key — and a key would risk a
   `find()` collision with the round comment's `round: {N}` (D20).
4. **Staleness is tolerable and must be visible when it happens.** The body write is best-effort
   and non-fatal, reported like D45's `reopened`. A stale pointer misleads a human, so it is
   surfaced; it can never mislead the fold, per (1).
5. **Required test: the section is inert.** The fold's output must be byte-identical with and
   without a `## QC Rounds` section present, and with a deliberately *wrong* one (e.g. claiming
   5 rounds when 2 exist). This test is what stops "it's only a hint" from drifting into
   authority the way such fields usually do.

---

## §16 Resolutions: the round marker as a fetch hint (D50–D51)

**These supersede D48 and refine D49.**

**D50.** *(`Issue.branch` is retained.)* D48's removal is **reversed**. `Issue.branch` is round
1's declared branch, parsed from the body, and it is load-bearing for **cheap list paths** — the
milestone issue list returns bare issues with no round data, and folding a thread per issue just
to render a branch is a cost those paths should not pay.

Its validity condition, which is what makes retaining it safe rather than sloppy:
- **No `## QC Rounds` marker in the body ⇒ single round ⇒ `Issue.branch` IS the current
  branch.** The body is complete and authoritative for that issue's branch.
- **Marker present ⇒ rounds exist ⇒ `Issue.branch` is round 1's and may be stale.** A consumer
  that needs the current branch must read `rounds[last].branch` (D9/M7), which requires the
  comment fetch.

Inside a rounds-bearing `IssueStatusResponse`, `issue.branch` duplicates `rounds[0].branch`, so
this is a **stated exception to D24's letter**, justified because the same `Issue` type is reused
in list and create flows that never fetch `rounds[]`. Consumers of a rounds-bearing response
MUST read `rounds[last].branch`; `issue.branch` is for the rounds-less contexts only.

**D51.** *(The marker's presence is a legitimate fetch hint — with the write ordered so failure
is safe.)* This refines D49, which said the section is purely human-facing.

**Presence/absence of `## QC Rounds` MAY be read by cheap paths to decide whether a comment
fetch is required.** That is a real, intended machine use and the reason D48 is reversed.

Still forbidden, unchanged from D49: **the section's *content* is never authoritative.** Round
count, round metadata, and round checklists come only from `# QC Round N` comments and the
body's round-1 metadata. The fold itself never consults the section — it already fetches
comments unconditionally. D49's inertness test stands and is now more important, not less.

**Write ordering (this is the substantive change).** Because a cheap path may now *skip* a
comment fetch when the marker is absent, a failed marker write would make a multi-round issue
look single-round and a consumer would trust a stale branch. Under-fetching is the dangerous
direction. Therefore, on round creation the marker is written **before** the round comment is
posted:

    write `## QC Rounds` marker → post `# QC Round N` comment → re-open issue → notify

Failure modes under this order:
- **Marker write fails** ⇒ abort before posting the round comment. Nothing is inconsistent; no
  round was created.
- **Marker written, comment post fails** ⇒ the marker claims rounds that do not exist yet, so a
  cheap path fetches comments unnecessarily. **Over-fetching — the safe direction**, and the
  same conservative-direction reasoning as D39.3.

This supersedes D49's constraint 4: the marker write is **no longer best-effort**. It is a
precondition of round creation, and its failure aborts the round rather than being logged.

---

## §17 Implementation consequences (D52) and a pre-existing defect (N1)

**D52.** *(`Issue.has_qc_rounds_marker` — the mechanism D50/D51 needs.)* **Approved.** D50 makes
`Issue.branch` valid only when no `## QC Rounds` marker is present, and D51 lets a cheap path read
the marker's presence to decide whether a comment fetch is required. But the marker lives in the
issue **body**, and the wire `Issue` does not carry the body — so a consumer had no way to evaluate
either rule. `Issue.has_qc_rounds_marker: bool` closes that: it reports **presence only**, never a
round count, and is explicitly never authoritative for round data (D51). Without it D50 and D51 are
unimplementable on the wire, so this is the mechanism those decisions require rather than a new
decision.

Consumer contract:
- `has_qc_rounds_marker == false` ⇒ single round ⇒ `Issue.branch` is current; no comment fetch needed
  for branch.
- `has_qc_rounds_marker == true` ⇒ rounds exist ⇒ `Issue.branch` is round 1's and may be stale; read
  `rounds[last].branch`, which requires the comment fetch.

Note for clients: round creation now edits the issue body (D51's marker write, which precedes the
round comment), so any cached copy of the body or of this bit is stale immediately after
`POST /rounds`.

**N1.** *(Pre-existing defect, found during D49 implementation — NOT introduced by rounds and NOT
fixed here.)* `splice_file_history`'s existing-section replacement terminates at the next `## `
heading. If a checklist contains an `## ` subsection, the splice **would swallow the checklist H1**.
This predates rounds and is reachable today by any issue whose checklist uses `## ` subheadings.

`splice_qc_rounds` (D49) deliberately does **not** copy that behaviour: it terminates at the next
heading of **any** level. That is the one intentional deviation from D49's "same idiom as
`splice_file_history`" wording, and it is the correct behaviour — D49's own constraint 2 depends on
the checklist H1 surviving the splice.

**Action:** `splice_file_history` should be fixed to terminate at any heading level, matching
`splice_qc_rounds`. Deliberately left out of the rounds work to keep the diff scoped; it needs its
own change with its own regression test.

---

## §18 Resolutions from the divergence report (D53–D59)

Seven items from the post-implementation divergence review. **These supersede any earlier text
they contradict.** All decided by the spec owner.

### The unifying principle

Two separate findings turned out to be the same defect: the fold **degraded silently** where it
should have **represented the problem and told the user**. A round whose start commit could not
be resolved was dropped and the later rounds re-indexed; an approved round whose approval had
been force-pushed off its branch was silently re-bounded at the branch tip. In both cases the
code produced a confident answer that was wrong, rather than an honest "I cannot place this."
D53–D55 replace both with representation.

**D53.** *(`Round.placement` — a round that cannot be placed still exists.)*

```rust
pub enum RoundPlacement {
    Placed,
    /// The declared start commit could not be resolved on `branch` — usually because the
    /// branch is not fetched locally. The round still exists and keeps its declared index.
    Unplaceable { branch: String },
}
```

`Round` gains `placement: RoundPlacement`. Rules:

1. **An unplaceable round is never dropped and never causes re-indexing.** Most commit
   resolution is local; a branch the user has not fetched is a *local* gap, not evidence that a
   round does not exist. The declared round is real — it is in the comment log — and the user's
   remedy is to fetch the branch.
2. **`Round.index` therefore always matches the declared round number.** This is stronger than
   before and removes the audit divergence the review flagged: previously a `# QC Round 3`
   comment could become `Round { index: 2 }` if round 2's comment was unresolvable, so
   `Round.index` and `ArchiveQC.round` disagreed with the GitHub log. That can no longer happen.
3. **`commits` is empty for an `Unplaceable` round**, and `preceding_gap`/`drift` bounded by an
   unplaceable neighbour are empty.
4. **I4 is scoped to `Placed` rounds.** An `Unplaceable` round owns no commits, so it has no
   `Initial` commit to assert.
5. **The branch to fetch must be reportable.** `Unplaceable` carries it so every surface can say
   *which* branch, matching the existing `IssueStatusErrorKind::branch_not_local` idiom.

**D54.** *(`Round::latest_commit()` becomes fallible — one change fixing both findings.)*
`latest_commit()` returns `Option<&IssueCommit>`, and is `None` in exactly two cases:
1. the round is `Unplaceable` (no commits at all); or
2. the round is `Approved` but `approval.commit` is **not present in `commits`** — the
   force-push/rebase case.

Case 2 is the bug this replaces. Previously `latest_commit()` searched `commits` for the
approval, failed, and **fell through** to the newest status-bearing commit — so a divergent
thread's `archive_commit` became a post-approval commit while `ArchiveQC.approved` was still
`true`. The archive then claimed "this content was approved" over content that was not the
approved content: exactly the audit lie D15 and D28.3 exist to prevent.

The existing fallthrough (`newest status-bearing`, else `commits.first()`) is **retained for
`Unapproved` placed rounds** — that is the legitimate case it was written for.

**D55.** *(Consumers refuse rather than guess.)* Every consumer of `latest_commit()` must handle
`None` by reporting, never by substituting:
- **Status** — `determine_status` requires a placed latest round whose representative commit
  resolves. When it does not, the status endpoint returns the existing
  `IssueStatusError { kind: branch_not_local, branch }` for that issue rather than a `QCStatus`.
  **S5 still holds: `QCStatus` gains no new variants.** Note D10 means only the *latest* round
  matters here — an earlier unplaceable round does not block status.
- **Archive** — selecting a round whose commit does not resolve is an error naming the branch,
  e.g. *"fetch `<branch>` to archive round N"*. It must **never** freeze a substitute commit.
- **Record, CLI, UI** — render the round with its index and an explicit
  "fetch `<branch>`" state; never blank and never a substituted hash.
- The fold itself still always returns an `IssueThread` (D32 unchanged).

**D56.** *(Inherited branch must be visible.)* A round comment with no `git branch:` key inherits
the previous round's branch (this behaviour is kept — it is the sensible default). But because
branch is load-bearing for both the round's and its gap's commit walk (D7/D9), the inheritance
must be **surfaced wherever that round is the one being viewed** — the CLI round table, the
round switcher, and the new-round modal. A silent inherited branch can mis-scope two walks.

**D57.** *(Notification defaults ON, on both paths.)* Starting a round posts the difference
notification by default. The CLI already does this (`!no_notify`). The API's
`#[serde(default)]` on `notify` yields **`false`**, which contradicts the intent and makes the
two sanctioned paths disagree — fix the API to default `true`. C1's `--notify` flag remains a
conventional paired-flag cancel for `--no-notify`; nothing there is broken.

**D58.** *(I8's divergence scoping — recorded.)* I8's `approval_{n-1}` half does not apply to a
**divergent** preceding gap. A divergent gap is walked with no `stop_at` (D22), so it sweeps the
whole branch and will legitimately contain the previous approval when that approval is older
than `start_n`. D39 scoped I5 and I7 for divergence but omitted I8; the implementation already
carves it out correctly, and this clause records it. The `start_n` half of I8 still applies
unconditionally.

**D59.** *(The archive quartet is a breaking change and must be documented as one.)* A6 said only
that `ArchiveFileRequest` "gains `round?` and `subsequent_file_changes?`". The shipped rule is
stronger: `milestone`, `approved`, `round`, `subsequent_file_changes` must **all** be present or
**all** absent, else `400`. The rule is correct — only the client knows which round it selected,
and inventing a default would freeze a false claim (D28.3) — and it widens a pre-existing
`milestone`+`approved` pairing. But it **breaks any existing client posting `milestone` +
`approved` alone.** It must appear under Breaking Changes in `CHANGELOG.md`, alongside the other
wire removals from this work: `IssueStatusResponse.commits`/`branch`/`checklist_summary`,
`Issue.checklist_name`, and `qc_status.approved_commit`/`initial_commit`/`latest_commit`.

### Withdrawn

*`--notify` is parsed and discarded.* Reported as a defect; it is not one. `notify: bool` with
`overrides_with = "no_notify"`, read as `!no_notify`, is the conventional clap paired-flag idiom
and its default matches D57. Discarding the flag's own value is stylistic, not behavioural.

*Abbreviated declared start sha drops F7's `stop_at`.* Accepted as-is by the owner: the walk runs
unbounded, as it did before rounds existed.

---

## §19 Resolutions: unresolved-round handling (D60–D61)

**These supersede D55's archive clause and the D54/D55 error-reporting text they contradict.**

**D60.** *(A distinct error for an off-branch approval.)* D55 reused
`IssueError::LocalBranchNotFound` for both `None` cases of `latest_commit()` (D54). Its text —
*"Branch 'X' is not checked out locally"* — is correct for an **unplaceable round** but **wrong
for an off-branch approval**, where the branch *is* local and the approval was force-pushed or
rebased off it. Telling a user to fetch a branch they already have reads as a broken tool, and
this is precisely the force-push case where they most need to understand what happened.

Add a distinct variant — e.g. `IssueError::ApprovalNotOnBranch { commit, branch }` — whose
message says the approval commit is no longer reachable on that branch and that the history was
likely rewritten. It **still classifies as `branch_not_local` on the wire** so the existing
`IssueStatusErrorKind` and the UI's `StatusErrorDisplay` affordance keep working unchanged; only
the human-readable message differs. The two `None` cases of D54 are therefore distinguishable in
the message while remaining one case for clients.

**D61.** *(Archive skips an unresolvable file and records the omission.)* This **replaces** D55's
"archive errors naming the branch" for the multi-file path.

`ghqc milestone archive` must **not** abort the whole run when one selected round's commit does
not resolve. It **skips that file, warns, and continues.** Rationale, from the owner: a user's
remedy is often not "fetch the branch" but "select a different round that resolves" — and
aborting a fifty-file milestone over one stale branch denies them both the archive and the
choice. The CLI round picker already renders `fetch <branch>` per round (D53.5), so an
unresolvable round is visible *before* selection.

**The skip must be recorded in the archive itself, not only on stderr.** A partial archive whose
own manifest does not say it is partial is the audit hazard this spec repeatedly designs against
(cf. D39.3's conservative flag, D45's `reopened`). A terminal warning scrolls away; the archive
outlives it. Therefore `ArchiveMetadata` gains:

```rust
pub struct SkippedFile {
    pub repository_file: PathBuf,
    pub round: u32,
    pub branch: String,
    pub reason: String,   // e.g. "start commit could not be placed on 'feature/x'"
}
```

carried as `skipped: Vec<SkippedFile>` on `ArchiveMetadata`, serialized into
`ghqc_archive_metadata.json`. Use `#[serde(default, skip_serializing_if = "Vec::is_empty")]` so a
complete archive's manifest keeps its existing shape byte-for-byte and older archives still
deserialize; an incomplete one carries the record. This is a frozen observation at archive time,
consistent with D15/D28.3 — it is never a claim that goes stale.

The single-file API path (`POST /archive/generate` for one QC-attached file) keeps D55's
behaviour: an unresolvable round is an error naming the branch. It must still **never** freeze a
substitute commit — that prohibition is unchanged and absolute.

---

## §20 Resolutions: batch archive skips and test typechecking (D62–D63)

**D62 supersedes D61's treatment of the API path.**

**D62.** *(The batch archive endpoint skips per file and records it.)* D61 gave the CLI
skip-and-record and left the API refusing. But the web UI posts **every file in one**
`POST /archive/generate`, so it is neither of D61's two shapes — and the result was an interface
inconsistency: the CLI skipped one bad file and archived the rest, while the UI refused the whole
archive. That is the same class of defect D57 fixed for `notify` defaults, and the owner's
reasoning for D61 applies identically here: a user should be able to proceed, and to pick a
different round for the one file that failed.

**Where the skip decision lives.** `ArchiveFileRequest` carries an explicit `commit` and **no
issue number**, so the server cannot resolve a round for the batch path — it has only what the
client sends. The skip decision therefore *must* be the client's, and the server's job is to
record it. This is the same footing D28.3 already established for `round` and `approved`: only
the client holds the round it selected.

Contract:

```rust
pub struct SkippedFileRequest {   // mirrors archive::SkippedFile
    pub repository_file: PathBuf,
    pub round: u32,
    pub branch: String,
    pub reason: String,
}

// ArchiveGenerateRequest gains:
#[serde(default)]
pub skipped: Vec<SkippedFileRequest>,
```

- The server writes them **verbatim** into `ArchiveMetadata.skipped` (D61), so a partial archive
  declares itself partial regardless of which interface produced it.
- `#[serde(default)]` keeps existing clients working unchanged.
- The response reports what was skipped so a client can confirm what it asked for was recorded.
- **Unchanged and absolute: no path may freeze a substitute commit.** A file that appears in
  `files` must carry a resolvable commit; a file that cannot is omitted from `files` and declared
  in `skipped`. Skipping is not falling back.

**UI behaviour.** The UI must not silently omit files, and must not block the whole archive
either. Before generating, it shows which files will be skipped and why; on generate, it drops
them from `files` and declares them in `skipped`. The user sees the omission, can proceed, or can
select a different round for the affected file — which is often the real remedy (D61).

**D63.** *(Test files must be typechecked.)* `ui/tsconfig.json` has
`include: ["src", "vite.config.ts"]`, so **`npx tsc --noEmit` never typechecks `ui/tests/`.**
Verified empirically during the divergence review: 0 test files versus 69 under `src`.

This matters because every wire change in this work — the A2/A3 removals, D36's tagged union,
D44's nullable `comment_id`, D45's notification union, D52's marker bit, D53's `placement` and
nullable `archive_commit` — was reflected in hand-maintained Playwright fixtures with **nothing
mechanically checking the result**. A fixture carrying a removed field or missing a new required
one compiles and runs green. The project has already had one incident of this class: a fixture
whose `drift` could not distinguish the correct implementation from the forbidden rescan, which
passed a mutation that should have failed.

Add a `tsconfig.tests.json` (extending the base config, including `tests`) plus an npm script, so
fixture drift becomes a compile error. Two **pre-existing** type errors must be fixed first:
`"relevant"` is not a `RelevantFileKind` (`ui/tests/archive/flatten.spec.ts`), and untyped
`kind: string` tree entries (`ui/tests/record/record.spec.ts`).

**D64.** *(An archive with nothing in it is refused.)* If every file in a
`POST /archive/generate` request was skipped (`files` empty, `skipped` non-empty), the request is
refused with a `400` naming what was skipped. A tarball holding a manifest and no files is an
error report in archive form, not an archive.

This deliberately does **not** reintroduce D62's defect: it fires only when *nothing* is
archivable, so a **partially** skipped archive still succeeds — which is the case that must never
be blocked. The refusal lives on the server, where `files.is_empty()` is visible, rather than as a
re-added client gate.

*Note:* implementing this exposed that `test_declared_skips_are_recorded_in_the_manifest_verbatim`
had been written with an all-files-skipped request — the degenerate shape — so it was tightened to
carry a real archived file alongside the skip, i.e. the actual D62 scenario, asserting both that
the resolvable file is archived and that the skipped one never appears in `files`.

## §21 New-round modal review (D65–D69)

Post-implementation UI review of `NewRoundModal`. §9's U1–U8 stand except where a clause below
supersedes them; where §9's text and this section disagree, this section wins.

**D65.** *(The checkout facts are facts, not inputs.)* The branch and the start commit render as
read-only text in a bordered block captioned "read from the current checkout", **not** as
`TextInput readOnly`. D23 gives the round flow no commit picker; a control that looks like a field
implies one exists. Regression pinned by asserting the Round tab contains **no** text input at all,
rather than by asserting a `readonly` attribute — the attribute-based assertion passes on exactly
the control this decision removes.

**D66.** *(Notification gets its own tab.)* Notify-the-difference, include-diff, the note, and a
notification preview move to a `Notify` tab. The notification is a second comment with its own
commit pair, so sharing the round's tab with it made the round's own identity harder to read.

**D67.** *(The Round tab carries the change, not just the commit.)* The first screen answers the
question the user actually has — *is there anything here worth a round?* — so it renders the file's
diff between the prior round's approval and the checked-out commit, plus a count of the commits in
`drift` and how many touched the file. Without this the default tab was read-only context with no
action on it, and every path required a tab click.

- **D67.1.** New endpoint `POST /api/preview/round-diff`, request `{ issue_number, start_commit }`.
  Only the **new** end of the comparison is sent; the old end is the prior round's approval,
  derived server-side exactly as `create_round` derives the notification's `previous commit` (D5).
  A client that could pass both ends could show a diff for a transition that is not the one about
  to happen.
- **D67.2.** `diff_utils::file_diff_between_commits` becomes the single implementation of "what
  changed in this file between these two commits", and `QCComment::file_diff` delegates to it. The
  modal shows this diff next to the button that posts a notification embedding it; two
  implementations would be two answers. Pinned by a test that pulls the diff back out of
  `QCComment::generate_body` and asserts equality — it is the assertion that catches a reversed
  pair, which the content-level assertions do not.
- **D67.3.** `None` from the diff helper means **unreadable**, never *unchanged*: an unchanged text
  file still yields `Some`, carrying diff_utils' own "No difference between file versions." So
  `None` is a `400` naming the branch to fetch (D60), never an empty diff. Collapsing the two would
  hide a fetch problem behind a reassuring "nothing changed".
- **D67.4.** No request is made at all when the checkout **is** the approval: U3 already proves
  there is no difference, so the tab says so without a round-trip.
- **D67.5.** An unapproved latest round is a `409`, not a diff against a substituted commit — §18's
  rule applied to a new surface.

**D68.** *(Footer is Preview and the action.)* `Cancel` is dropped and `Preview` takes its place,
matching the notify modal. The header close button and Escape are the documented exits, and a test
pins Escape so dropping Cancel stays safe. `Preview` opens a nested modal whose tabs are *Round
comment* and — **only when a notification will actually be posted** — *Notification*; previewing a
comment nobody will send misrepresents what Start Round does. The selected preview tab is therefore
**derived**, not stored, so turning notify off cannot leave a vanished tab selected.

The notification preview needs no new endpoint: the round notification is a plain `QCComment`, and
`POST /preview/:n/comment` builds that same struct.

**D69.** *(The checklist seed's provenance is always stated.)* With more than one prior round the
base-round `Select` stands. With exactly one, a static line names it instead of rendering nothing:
a user seeding round 2 is still looking at a checklist that came from somewhere. Round 1 is named
"Round 1 (the initial QC)", since its checklist lives in the issue body rather than in a
`# QC Round 1` comment (D2).

*Note:* `/api/preview/round` is a **prefix** of `/api/preview/round-diff`, so unanchored matchers
answer the diff request with the round comment. Three test-side matchers and one route mock had to
be anchored; an unanchored mock is mutation-confirmed to break the D47 request-count assertion.

## §22 Cross-round commit selection (D70–D84)

Supersedes **W5** and **U4**: the slider is no longer bounded to one round, and the round
`SegmentedControl` is replaced by a History dropdown. Everything else in §9 stands.

### §22.0 Diagnosis

1. W5 bounds the slider to `round(i).commits` and U4 picks exactly one `i`, so the cross-round
   comparison is unreachable: with round 3 open there is no way to notify the delta since **round
   1's** approval. Round-scoping was the right fix for §0.5 but it over-corrected.
2. Gap and `drift` commits are selectable **nowhere**. `RoundSwitcher` maps `rounds` only, so
   commits between rounds, and everything after the latest approval, are invisible in all three
   tabs.
3. A `SegmentedControl` of round numbers cannot express a gap, a break, or anything that is not a
   round.
4. `ApproveTab` renders a round switcher and posts the commit selected in **any** round. Nothing
   client-side or server-side (`src/api/routes/comments.rs`, which only parses the hash) constrains
   the approval to the latest round's span. D8 gives an `Approved` round `[start_n ..= approval_n]`,
   so an approval before `start_n` gives the round a negative span — reachable in two clicks today,
   and it lands in exactly D54's state (`archive_commit: null`, status refusing with a "fetch the
   branch" remedy that is wrong, because the branch is local).

### §22.1 Decisions

**D70.** The `SegmentedControl` becomes a **History dropdown** whose rows are segments in W6 order
(`R1, G2, R2, G3, R3, drift`). Selection is a **set**; the slider renders the union of the selected
segments' commits.

**D71.** The default selection is **the latest round only** — byte-identical to today's view. §0.5
is not reopened: the bounded view is the default and widening is deliberate.

**D72.** Granularity is the **segment**, never the commit. The slider already filters commits
(`showAll` hides those with no file change and no statuses); a per-commit picker in the dropdown
would be a second answer to that question.

**D73.** Gaps and `drift` are selectable rows, not decoration. A gap commit is a real commit on the
round's branch (D9).

**D74.** **One visual language for discontinuity.** Two causes produce "these commits are not
adjacent in history": a divergent gap (D22/D31), and *the selection skipping a segment in the
middle*. Both render as the same break marker. The second is self-inflicted and previously unnamed
— selecting R1 and R3 makes R1's newest commit look adjacent to R3's oldest.

**D75.** Across a break, **content survives and order degrades** — D22's "diffs still work, which is
what matters." `current_commit`/`previous_commit` stay valid (a blob-to-blob diff needs no
ancestry). `fileChangedInRange`, which walks the commits between the handles, does not: it goes
conservative-`true` (as D39.3) so the diff is offered rather than silently dropped, and the range
says its commits are not one history.

**D76.** *(revised by D83.)* One control, on the tabs that can use it.

**D77.** An unplaceable round (D53) is a row that contributes zero commits and keeps its
`FetchBranchBadge`. Never hidden — the remedy is the point.

**D78.** Two concepts, previously conflated in one control:
- **History selection = what the slider *displays*.**
- **Landing rules = where each *handle* may rest**, per tab, because the selected commit means
  something different in each.

**D79.** Notify: `to` ⇒ `current_commit` is confined to the **tail block**; `from` may reach into
any displayed segment. Enforced as *the max handle is constrained to tail positions*, checked
continuously, so handles still cross freely and dragging the max handle left stops at the tail's
first commit. Clamped through the existing `snapToVisible`, so an illegal range is never
constructible rather than constructible-then-rejected.

**D80.** The tail block is therefore **pinned** in the dropdown — deselecting it would leave no
legal `to`. This makes D71's default exactly the minimum selection.

**D81.** The tail block is **latest round ∪ drift**, one contiguous region: both are "now". With the
latest round approved and drift non-empty (`ChangesAfterApproval`), drift is the tail.

**D82.** Review: the selected commit is the **old** end — `QCReview` diffs it against the *working
tree* — so it is `from`-like and takes **no** constraint. This is the largest capability the change
unlocks: "review my working tree against what was approved in round 1."

*Known gap, deliberately not fixed here:* the posted review body records `comparing commit: X` with
no round attribution, so a reader seeing round 1's hash on a round-3 review has nothing saying it
was deliberate. A round-attributing line in the review body is a possible follow-up.

**D83.** *(supersedes D76 for one tab.)* **Approve has no segment selection at all.** An approval
must lie after its round's start commit (D8), so every row other than the latest round would be
unselectable — and today's switcher does not merely offer them, it posts them (§22.0.4). The tab is
fixed to the latest round. I14 also makes `drift` empty whenever the latest round is unapproved, so
the tail collapses to the latest round there anyway.

*Not fixed here:* the server still accepts an out-of-span approval. Removing the switcher closes the
reachable path; a `409` in `approve_issue` is the durable fix and is left as its own change.

**D84.** No cap on how many segments may be selected — the user opted in explicitly — but the
selected commit count is displayed, because §0.5 was about unusability at volume.

### §22.2 Wire

**M2.** `IssueStatusResponse` gains `history: Vec<SegmentRef>`, where
`SegmentRef { kind: 'round' | 'gap' | 'drift', round_index }` — **pointers, not payload**, so no
commit is duplicated. For a gap, `round_index` is the round it precedes; for drift, the latest
round.

W6's ordering carries two *positional* suppression rules (skip `rounds[0].preceding_gap`; emit
`drift` only when the last round is closed). Rebuilding those in TypeScript is the derivation
D30/U7 pushes to the server, so `history` is projected from `IssueThread::segments()` and W6 keeps
exactly one implementation. To make that possible, `Segment::Gap` and `Segment::Drift` carry their
**owning round** rather than only a `&Gap`: which round owns a gap is part of its identity (D9 —
a gap's branch *is* its owning round's branch), not merely its position.

## §23 The approval span rule reaches the write endpoint (D85)

**D85.** *(closes §22.0.4, which §22/D83 only made unreachable.)* `POST /api/issues/{n}/approve`
**refuses** an approval commit that is not among `latest_round().commits`.

D8 gives an `Approved` round `[start_n ..= approval_n]`, so an approval outside that set gives the
round a negative span, and the fold then lands in D54's state: `latest_commit()` is `None`,
`archive_commit` is null, and status refuses with a "fetch the branch" remedy that is *wrong*
because the branch is local and fine. D83 removed the UI control that could post one, but removing
a control only makes a state unreachable from one screen — the endpoint still accepted it, so the
CLI, a script, or a future caller could still write it.

- **D85.1.** This is not a new rule. `QCApprove::from_args` has always resolved a user-supplied
  commit against `latest_round().commits` and errored otherwise, and `from_interactive` only ever
  offers that set. D85 makes the API agree with the CLI rather than inventing a constraint.
- **D85.2.** Deliberately **not** waivable by `?force=true`. `force` exists to bypass blocking-QC
  *policy*; this is a model invariant, and no caller has standing to waive it. Pinned by its own
  test.
- **D85.3.** `409 Conflict`, and the refusal is **total** — no approval comment, and the issue is
  not closed. A posted approval is an audit record, so a partial write here would be the lie the
  guard exists to prevent. Also pinned by asserting no writes occurred.
- **D85.4.** When the latest round is `Unplaceable` there is nothing to verify against, so the
  message names the branch to fetch instead (D55) rather than reporting the commit as wrong.

*Cost:* one extra comments fetch per approval (cached), to build the thread. Correctness on a write
that produces an audit record outranks a cached round-trip.

## §24 History dropdown presentation (D86–D89)

Refines §22's first-pass UI (Q7 was explicitly left to be tuned against a POC). No decision in
§22 is reversed; D86–D88 are presentation, D89 is a correctness fix.

**D86.** *(timeline, not a list.)* Rounds render as **dots**; the gap between two rounds renders as
the **line connecting** them. A gap is the distance between two rounds, so drawing it as the
connector rather than as another list item makes the alternating `R (G R)*` structure visible
instead of merely stated. Selection checkboxes are right-justified into their own grid column, so
every row's control lands on one edge regardless of how many badges the row carries.

**D87.** *(newest-first.)* The dropdown reads top-down newest→oldest: the round in progress is the
top row, round 1 the bottom. Only the **display** is reversed — `status.history` keeps W6's order
(M2) and `flattenSelection` still walks it oldest→newest, because the slider's axis is time.

**D88.** *(divergence is drawn on the connection.)* D74's break is two slashes with space between
them, struck across the connecting line — the conventional "these ends are not continuous" mark.
Drawing it on the *line* rather than beside either round makes it a property of the connection,
which is what `preceding_gap.divergent` actually is.

A gap belongs to the round it **precedes** (D9), which in newest-first order is the row directly
*above* it; the label names that round ("before round 2") so the descriptor cannot be read as
hanging off the older round below.

The rail's horizontal stub meets the label at the row's **vertical centre**, which is exactly where
the label sits because the row is `align-items: center`. A first attempt drew the stub at a fixed
`top: 7px` to bias it toward the round above — it landed 6px above the text it was supposed to
point at. Every rail graphic is now positioned from a single `RAIL_CENTER` constant with explicit
widths, rather than from `borderLeft` offsets plus a content-box ring, so the dot's centre and the
line's centre are the same number by construction instead of by coincidence.

**D89.** *(superseded by D91 — the single derived count became two raw facts.)* A row's count, and
the total beside the control, count commits that **changed the file or carry a QC status** —
`isOfInterest` — not raw commits in the segment's span.

The dropdown exists to say what ticking a row adds to the slider, and the slider hides commits that
touched nothing relevant behind "Show all commits". Counting raw commits therefore advertised
commits that never appeared: a gap holding one irrelevant commit read as `1` while selecting it
added nothing visible.

*This was a display bug, not a fold bug.* The gap walk is `walk[start_index + 1 .. anchor]` —
exclusive at both ends per D8/D34 — with an `index > start_index + 1` guard, so adjacent rounds
already yield an empty gap. The arithmetic that exposed it: a round whose initial commit **is** its
approval contributes one commit carrying both statuses, so a two-round QC with one irrelevant
commit between the rounds has **two** commits of interest, and the raw count claimed three.

**D90.** *(a gap that would add nothing is not selectable.)* A gap or drift row whose
**of-interest** count (D89) is `0` is disabled. Ticking it would change neither the slider nor the
total, so offering the tick is a control that does nothing.

- **D90.1.** "Zero" means the number *on screen* — commits of interest — not raw commits in the
  span. **Accepted cost:** a gap owning only irrelevant commits becomes unreachable, because "Show
  all commits" widens what the *selected* segments contribute and this gap can no longer be
  selected. Judged acceptable because an uninteresting commit's diff is identical to its nearest
  interesting ancestor's, so no comparison becomes impossible — only the specific hash nameable as
  `previous_commit`. The disabled row's tooltip states what is being withheld and why, so nothing
  is silently dropped.
- **D90.2.** **Rounds are exempt.** A round showing `0` is an *unplaceable* one (D53.3), where `0`
  means "fetch the branch", not "nothing here" — and selecting it is how D77's remedy notice is
  surfaced. Same digit, different meaning, so the rule keys on segment kind rather than on the
  count alone.
- **D90.3.** Divergence must therefore be legible on a gap that can never be selected: the
  dropdown's rail carries the break (D88) and the badge sits on the row, independent of selection.
  The slider's own break still appears, via the omitted-segment half of D74 rather than the
  divergent half.

*Consequence for `flattenSelection`:* the break-carry across an empty selected segment becomes
effectively unreachable, since the only selectable empty segments are unplaceable rounds and an
omitted neighbouring gap re-triggers the break anyway. The carry is retained as defensive code, not
because a current path exercises it.

**D91.** *(supersedes D89; revises D90.1.)* A row states **two raw facts** rather than one derived
number: `5 commits (3 file changing)`. The parenthetical is dropped when the segment owns nothing.

D89's single count could not distinguish "nothing here" from "nothing *interesting* here" — a gap
owning one irrelevant commit read as `0`, which is what made D90.1 look like a choice between a
useless control and an unreachable commit. With both numbers shown there is no such choice: the row
says it owns a commit and that the commit changed nothing, and the reader can see why the slider
hides it.

- **D91.1.** The parenthetical is the **file-changing** count, not the count of visible ticks: the
  slider also draws commits that carry a QC status without touching the file. File-changing is the
  question a reviewer actually asks of a span, and the two raw numbers stay honest where a
  "what you will see" number would have to track the `showAll` toggle.
- **D91.2.** **D90 now keys on the raw count.** Only a segment owning *no commits at all* is
  disabled, so D90.1's accepted cost is withdrawn — a gap holding a hidden commit stays selectable
  and "Show all commits" reaches it. D90.2 (rounds exempt) and D90.3 (divergence legible without
  selection) stand unchanged.
- **D91.3.** Labels are `white-space: nowrap` and the dropdown is 520px wide. A wrapped label
  doubled the row height, which dragged the rail's centred stub (D88) away from the text it points
  at and truncated the divergence badge — the same alignment failure D88 fixed, reintroduced through
  layout rather than through arithmetic.

**D92.** *(the commit slider is centred.)* The slider wrapper pads **symmetrically**. It padded
16px left and 40px right in all three tabs — room for the last mark's label but not the first's —
which inset the track 24px further on the right and read as an off-centre slider.

Both end labels are centred on their own marks and overhang the track by roughly half a hash-width,
so the padding must leave equal room at both ends or the track cannot be centred. Pre-existing, and
unrelated to §22 — it became noticeable once the History control drew attention to the row above it.

Pinned by **measuring** the rendered track's insets against its scroll container rather than by
asserting the padding literal, so the property under test is the geometry a viewer sees. Mutation
check reproduces exactly the 24px asymmetry.

---

## §25 Archive card status is round-scoped (D93)

**D93.** *(The archive card's status describes the selected round, not the QC.)* `ArchiveTab`'s
per-file card labelled `Status:` from `qc_status.status` while its `<Select>` chose a round. With
round 2 open and round 1 approved, picking round 1 froze round 1's approved commit under a card
that read **"awaiting review"** — the card contradicted the archive it was about to write.

- **D93.1.** `qc_status` is scoped to the **latest** round and its drift (S0–S4), so it is the
  correct label only while the latest round is the selected one. That case keeps it, and keeps its
  full nuance (`changes_after_approval` is a distinction only the latest round can carry, S1).
- **D93.2.** Every earlier round has already resolved, and `RoundState` is the sole encoding of
  approvedness (D36). So the label is **read off `round.state.kind`** — `approved` → `approved`,
  `superseded` → `superseded (never approved)`. This is a mapping of a server-sent field, not a
  derivation, so U7 holds: nothing rescans commits.
- **D93.3.** `superseded` is **not** rendered as approved. Under D21 it is an *unapproved* round a
  later round replaced; labelling it "approved" would be the same audit lie D55 removes.
- **D93.4.** The card's "not approved" warning triangle follows the same rule (`isRoundApproved`) —
  it warns about the round the archive will freeze, so it disappears when an approved earlier round
  is selected. A non-latest `open` round is forbidden by D27; the fallback keeps the QC label rather
  than inventing a state for something that cannot occur.
- **D93.5.** Milestone-level visibility (`includeNonApproved`) stays **QC-scoped** and is
  deliberately not re-pointed at the selection. It gates which files enter the grid at all; making
  it follow a per-card selection would let choosing a round change the checkbox counts of a
  milestone filter. Consequence: a QC whose latest round is open needs "Include non-approved" to
  appear, even when the round the user wants is approved.

---

## §26 The tail clamp must be visible (D94)

**D94.** *(The thumbs render from the clamped pair.)* D79 clamped the **derived** `to` and left
the thumbs at their raw handle positions. The max thumb therefore stayed wherever it was dragged
while `To:`, the preview and the posted comment all used a different commit — the control displayed
a range it was not going to send, which is worse than an unenforced constraint because the user's
own reading of the picker was wrong.

- **D94.1.** Both thumbs are driven off `fromPos`/`toPos` — the same clamped pair the `From:`/`To:`
  rows and `commentRequest` read. Disagreement between what the picker shows and what it sends is
  now unrepresentable rather than merely unlikely.
- **D94.2.** Which thumb is the max still follows the drag (`snapA >= snapB`) rather than being
  pinned to one handle, so D78's free crossing survives. Dragging the max thumb below the tail
  visibly snaps it back to the tail's first commit — the clamp *is* the feedback.
- **D94.3.** The stored handle positions stay unclamped on purpose: they are what makes crossing
  work, and re-clamping them into state would fight the pointer during a drag.
- **D94.4.** `CommitBlock` carries `data-testid="commit-block-{from,to}"`; its label and hash are
  separate flex children with no whitespace between them, so the rendered text is `From:1a11111`
  and a text-substring assertion silently never matches.
