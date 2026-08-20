# QC Segment Model — Spec v1

Status: **implemented.** Branch `rounds`, uncommitted. Sections §0–§12 were agreed *before* implementation; **§13 (D12–D15) and §14 were added during it**, and several original statements are corrected in place — every correction is marked as such rather than silently rewritten, so a reader can see which claims changed and why.

This replaces the current `IssueThread` model. It was designed in review after a
series of status bugs that all turned out to be one bug. Read `§0` before
implementing — the diagnosis is the justification for every decision below.

---

## §0 Diagnosis — why this exists

`IssueThread::from_issue_comments` parses the same comment thread **twice**, into two
independent representations:

| | Built by | Shape |
|---|---|---|
| Flat model | `parse_commits_from_comments` (`src/issue.rs`) | `commits: Vec<IssueCommit>`, each with `HashSet<CommitStatus>` — `Initial \| Notification \| Approved \| Reviewed` |
| Round model | `fold_rounds_from_comments` (`src/round.rs`) | `rounds: Vec<Round>` with `RoundEvent`, `RoundState`, retractions, extensions |

Those are the *same four facts* from the *same comments*. `CommitStatus::Approved` is
`RoundState::Closed.commit`; `Notification`/`Reviewed` are `RoundEvent`s; `Initial` is
round 1's anchor. Nothing checks the two against each other.

The round model was added **additively** — `src/issue.rs` still carries the comment
"Round-derived accessors (additive; nothing else calls these yet)". Status still reads
the flat model. So every consumer needing round scope re-derives it:

| # | Where | State |
|---|---|---|
| 1 | `IssueThread::round_membership` / `draft_gap` (`src/issue.rs`) | dead — tests only |
| 2 | `determine_status`'s hand-rolled `commits[..=anchor]` window (`src/qc_status.rs`) | live |
| 3 | `latest_commit`'s `approval_outranks` gate (`src/issue.rs`) | live |
| 4 | `roundWindow` / `roundWindows` / `openRoundWindow` / `draftGapRuns` (`ui/src/utils/rounds.ts`, 155 lines) | live |
| 5 | `postApprovalFileCommit` (`ui/src/components/SwimLanes.tsx:50`) | live |

**Five re-derivations of round scope, in two languages, one of them dead.** #1 and #4
are full segmentation implementations — the model below already exists, twice, as a
derived view instead of as the data structure.

Three "status leaks" were patched separately before this review; all three were the
same defect (a thread-wide scan where round scope was needed), and each patch was
justified with the same legacy-safety argument. That repetition is the signal that the
rule belonged in the model, not at the call sites.

### The anchor problem

`round_membership` deliberately excludes a round's own anchor. With commits `A..E`
(newest-first `E,D,C,B,A`), one closed round anchored at `A` closing at `B`, and an
open round 2 anchored at `D`:

| Segment | Members |
|---|---|
| Round 1 (anchor `A`, closed at `B`) | `[B]` |
| gap | `[C]` |
| Round 2 (anchor `D`, open) | `[E]` |

**`A` and `D` — both anchors — are owned by nothing.** Pinned by
`membership_and_draft_gap_split_commits_between_rounds` in `src/round.rs`. Worse, round
2's notification diffs `B→D`, so the commits it exists to review (`C`, `D`) are in the
gap or unowned, while `E` (drift arriving after it opened) is "in" the round.

Nothing in the current model says what a round's scope *is*. That is the hole this
spec closes.

---

## §1 Decisions (D)

| ID | Decision |
|---|---|
| **D1** | Boundary commits may be shared: a Round's `opened_at` may equal the previous Round's closing commit (reachable when HEAD has not moved since approval — legitimate, e.g. re-QC against a stricter checklist) |
| **D2** | Segments own their commits, walked on the segment's own branch |
| **D3** | A trailing Gap walks on the previous Round's branch — **not** the viewer's checkout (see **D8**) |
| **D4** | Unresolvable/malformed segments render grayed, not as errors. Precedent: the card already grays on branch mismatch |
| **D5** | A branch is **always** declared. Round 1's is the issue body's `git branch`; Round ≥ 2's is its round comment's `git branch`. Absent ⇒ malformed input, **not** a legacy case — nothing before `v0.7.1` has round comments |
| **D6** | Strict alternation; empty Gaps are legal and expected |
| **D7** | All derivation lives in the backend. The API shapes data for the UI; the UI renders |
| **D8** | Status is reproducible — never a function of the viewer's checkout. Two people on different branches must see the same status for the same issue |
| **D9** | Hard API swap. Nothing before `v0.7.1` is supported; no dual-shipping, no deprecation window |
| **D10** | Anchor belongs to the **Round**, not the preceding Gap — symmetric with Initial QC, whose `initial qc commit` starts round 1 |
| **D11** | No intermediate green states required during migration; land the backend swap as one coherent change |

---

## §2 Types (M)

**M1**

```rust
pub enum Segment {
    Round(Round),
    Gap(Gap),
}

pub struct IssueThread {
    pub file: PathBuf,
    pub milestone: String,
    pub open: bool,                    // GitHub issue state
    pub blocking_qcs: Vec<BlockingQC>,
    pub segments: Vec<Segment>,
    pub anomalies: Vec<RoundAnomaly>,
}
```

**M2** `IssueThread.branch` and `IssueThread.commits` are **removed**. Per **D5** every
Round carries its own branch, so "the issue's branch" ceases to exist as a concept.
This is what makes the card-graying bug (`§7`) unrepresentable rather than fixed.

**M3**

```rust
pub struct Round {
    pub index: u32,
    pub opened_at: ObjectId,           // the anchor; name kept deliberately (Q2)
    pub branch: String,                // always resolved; see D5
    pub opened: RoundOpen,
    pub checklist: ChecklistSource,
    pub checklist_name: Option<String>,
    pub state: RoundState,             // Open | Closed { commit, by, at, .. }
    pub events: Vec<RoundEvent>,
    pub retractions: Vec<Retraction>,
    pub extensions: Vec<Extension>,
    pub commits: Vec<IssueCommit>,     // owned, newest-first
    pub placement: Placement,
}
```

`previous_approval` is **removed** from `Round` — it becomes an accessor (**M8**).
`branch_declared` was considered and rejected: **D5** makes it meaningless.

**M4**

```rust
pub struct Gap {
    pub branch: String,
    pub commits: Vec<IssueCommit>,     // owned, newest-first
    pub continuity: GapContinuity,
    pub placement: Placement,
}

pub enum GapContinuity {
    /// Bounds are ancestrally connected — the normal case.
    Linear,
    /// Bounds diverge; the histories meet at `merge_base`.
    Diverged { merge_base: ObjectId },
    /// Bounds share no history; no diff between them is meaningful.
    Unrelated,
}
```

`GapContinuity` **replaces `BranchDivergence`** (`src/start_round.rs`) and
`BranchDivergenceResponse` (`src/api/types/responses.rs`) — same fact, one type.

**M5** One degradation path shared by every failure mode (**D4**, **D5**):

```rust
pub enum Placement {
    Placed,
    /// `commits` is empty; the segment renders grayed.
    Unplaceable(UnplaceableReason),
}

pub enum UnplaceableReason {
    BranchNotDeclared,       // D5 violation — malformed round comment
    BranchUnavailable,       // not fetched, deleted, or an empty walk
    AnchorUnreachable,       // not on the branch; force-push, gc
    MergeBaseUnreachable,    // both bounds resolved; their common ancestor did not
    NeighbourUnplaceable,    // a Gap whose bounding Round could not be placed
}
```

**`MergeBaseUnreachable` — added after implementation review.** An off-walk merge-base
previously degraded to `AnchorUnreachable`, which is a **different fact**: both of the
Gap's anchors resolved, and what failed is that their common ancestor lies outside the
walked range. Since walks stop at Initial QC, this fires whenever a later Round's branch
forked *before* Initial QC — ordinary history, not a force-push or a gc. **D4**/**U2**
render `placement.reason` so the user knows the remedy, so reporting an unreachable
anchor sent them hunting a commit that is not missing.

Relatedly, the **empty-walk** case (a Gap whose branch yielded no commits at all) now
reports `BranchUnavailable` rather than `AnchorUnreachable`: nothing is unreachable *on*
the branch — the branch produced nothing. Every reason's wording comes from the shared
`UnplaceableReason::describe()`, which is the one string the CLI, the repair's per-step
report and the API's `skipped_reason` all render.

**M6** `IssueCommit` keeps its name but **drops `statuses`** — it becomes
`{ hash, message, file_changed }`. Statuses become an API projection (**A4**). Keeping
the name shrinks the diff; orphaning it into a new `SegmentCommit` is acceptable if
that proves cleaner in practice (Q11 — implementer's choice).

**M7** New `RoundAnomaly` variants: `BranchNotDeclared { comment_index }`,
`SegmentUnplaceable { position, reason }`.

**M8** `IssueThread` accessors — positional indexing throughout (Q6):

| Accessor | Definition |
|---|---|
| `active_segment()` | `segments.last()` — total by **I3** |
| `active_branch()` | the active segment's branch |
| `previous_approval_of(pos)` | closing commit of the Round at `pos - 2` |
| `standing_approval()` | `Gap` last ⇒ previous Round's closing commit; `Round` last ⇒ `None` |
| `last_approved_commit()` | newest closing commit across all Rounds, ungated |
| `latest_commit()` | newest commit of the active segment |
| `next_notification_from()` | `Round` last ⇒ its newest event commit, else its `opened_at`; `Gap` last ⇒ `standing_approval()` |
| `rounds()` | iterator over `Round` segments |

---

## §3 Invariants (I)

- **I1** `segments[0]` is always `Round` (Initial QC)
- **I2** Strict alternation Round, Gap, Round, Gap … (**D6**)
- **I3** The last segment is `Round(Open)` or `Gap`; therefore `Round(Closed)` is never last
- **I4** Every known commit is owned by exactly one segment, except per **D1**
- **I5** `Placed` segments partition the known commits totally; `Unplaceable` segments own none

> **Correction to I5 as originally worded.** The partition claim does **not** hold
> literally for a thread containing an `Unrelated` Gap. Such a Gap is `Placed` yet owns
> nothing — no bound-to-bound range is meaningful when its ends share no history
> (**M4**) — so the commits between its bounds are owned by no segment while every
> segment is placed. `Unrelated` Gaps therefore stand *outside* the partition, exactly as
> `Unplaceable` segments do, and the fold's invariant check must excuse holes they span
> or it fires on valid input. Read I5 as: *`Placed` segments partition the known commits,
> except that an `Unrelated` Gap owns none.*
- **I6** Debug assertion after every fold: **I4** + **I5** hold

**I3 is what makes `§5` total.** If a Gap could be absent, `active_segment()` would
have to handle `Round(Closed)`, reintroducing the case analysis this model deletes.

---

## §4 Walks (W)

**W1** Branch selection, one rule: **a Gap walks on the branch of the Round bounding
its newer end; if there is none, the Round bounding its older end** (**D3**). Rounds
walk their own branch.

**W2** Bounds:

| Segment | From (newer) | To (older) |
|---|---|---|
| `Round(Open)` | branch tip | `opened_at`, inclusive |
| `Round(Closed)` | closing commit | `opened_at`, inclusive |
| `Gap`, interior | next Round's `opened_at`, exclusive | previous Round's closing commit, exclusive |
| `Gap`, trailing | branch tip, inclusive | previous Round's closing commit, exclusive |

**W3** Cost is bounded by **distinct branches**, not segments. `find_commits`
(`src/git/file_ops.rs:544`) already caches per branch and supports `stop_at`;
`find_or_cache_file_changes` likewise. **A single-branch issue does one walk —
identical to today**, which is why existing issues cannot regress.

**W4** Placement uses the existing `get_commits_robust` chain
(`src/git/file_ops.rs:609`) per segment — declared branch → `find_merged_into_branch`
→ `get_branches_containing_commit` — fed **that segment's own anchor**. That is a
better input than today's single arbitrary thread-wide reference commit, so
`Unplaceable` should be rare.

**W5** Divergence is a **Gap** property, not a special case. A Gap whose bounds are not
ancestrally connected walks from its newer bound back to the merge-base and records
`Diverged { merge_base }`, or `Unrelated` when there is none. The `merge_base`
primitive (added to `GitCommitOps` in `src/git/file_ops.rs`) lands here: it stops being
a fallback and becomes the Gap's natural lower bound.

**W6** A Gap whose bounding Round is `Unplaceable` is itself
`Unplaceable(NeighbourUnplaceable)`. **No substitution onto the other Round's branch**
— walking R1's branch would produce a real commit set that is *not* the Gap, and
showing plausible-but-wrong commits in an audit tool is worse than showing none. The
useful information is surfaced as a diagnostic message instead:

> Round 2's branch `feature/x` is unavailable; `main` has 4 commits since Initial QC's approval.

**W7** Cross-segment consumers need **pairs of commits to diff**, never a walk spanning
two segments (verified: notification diff, archive, record). `git diff A B` is
ancestry-independent, so divergent Rounds never need connecting.

---

## §5 Status (S)

**S1**

```rust
if !issue.open && standing_approval().is_none() { return ApprovalRequired }

match active_segment() {
    // S4: an unplaceable gap yields no status at all
    Gap(g) if !g.is_placed() => None,
    Gap(g)   => match g.commits.iter().find(|c| c.file_changed) {
        // the newest *file-changing* commit: drift that never touched the file is not
        // something a reviewer is asked to comment on
        Some(changed) => ChangesAfterApproval(changed.hash),
        None          => Approved,
    },
    Round(r) => r.status(),
}
```

**S2** `Round::status()` — from the Round's own commits and events only:

| Condition | Status |
|---|---|
| no commits and no events | `InProgress` — **unreachable from the fold** (see below) |
| **commits, but none file-changing, and no events** | **`InProgress`** |
| newest covering event is a `Review` | `ChangeRequested` |
| newest file-changing commit is covered | `AwaitingReview` |
| newest file-changing commit is not covered | `ChangesToComment(newest)` |

**Row 2 was added after implementation review; the table was incomplete.** A `Placed`
Round always owns at least one commit, so row 1 cannot arise from the fold — but row 2
can, and nothing in the original four rows covered it. Reachable shape, verified by probe
through `resolve_segments`: Initial QC at `A` approved at `B`; **Round 2 opened at `C` on
a declared branch with no notification** (`NotificationMode::None` is supported and is the
default in both surfaces); `C` did not touch the QC'd file. Round 2 is `Placed`, owns one
commit, has zero events.

`InProgress` is the honest answer there: nothing was announced, so not `AwaitingReview`;
there is no file change to comment on, so not `ChangesToComment`. The round is simply open
and in progress.

> **This supersedes an earlier reading recorded during migration**, which held that
> `InProgress` was unreachable and should be deleted. That reading examined only row 1.
> Mapping row 2 to `None`/`unknown` instead would be a **new** semantic lie of the `§0`
> shape — the round is perfectly placeable and fully understood — and would break
> **D15**'s second addendum, which ties `Unknown` specifically to *clause 2, an
> `Unplaceable` active segment*. Row 1 is kept and documented as unreachable rather than
> deleted, so that a future reader does not remove row 2 by association.

**S3** *Covered* = some event in this Round names a commit at the same or newer
position **within this Round's `commits`**. No index arithmetic against a global array.

**S4** An `Unplaceable` active segment yields no status; the UI grays it (**D4**).

**S5** `ChangesAfterApproval` stops being an eighth status concept and becomes "the
trailing Gap is non-empty" — which is also why *start a new round* is its natural
resolution.

**S6 — deleted by this section:**

- the `standing_approval` open-round gate (`src/qc_status.rs`)
- the hand-rolled `commits[..=anchor]` search window (`src/qc_status.rs`)
- `latest_commit`'s `approval_outranks` tier (`src/issue.rs`)
- `next_notification_from`'s four-candidate `min_by_key` (`src/issue.rs`)
- `round_membership`, `draft_gap` (`src/issue.rs`)
- `parse_commits_from_comments` (`src/issue.rs`)
- `postApprovalFileCommit` (`ui/src/components/SwimLanes.tsx`)
- the window/gap half of `ui/src/utils/rounds.ts`

---

## §6 Fold (F)

**F1** Stage 1 stays pure over comment text and emits the segment list. Comments yield
Rounds, events, approvals, retractions, extensions; **Gaps are implied by adjacency**
and inserted between Rounds.

**F2** Events land on the current Round — unchanged from today.

**F3** Retraction reopens the current Round and **absorbs the Gap following it**. Safe
because the fold already rejects retraction unless the current Round is `Closed`
(`RoundAnomaly::RetractWithNothingClosed`), and by **I3** that Gap is the last segment.
`[R1(closed), Gap]` → `[R1(open)]`, with the Gap's commits becoming drift-during-review.
Reversible.

**F4** Extension — a round comment posted over an open Round extends it rather than
opening a new one. Unchanged.

**F5** A round comment with no `git branch` is malformed (**D5**): emit
`BranchNotDeclared` and mark the Round `Unplaceable(BranchNotDeclared)`.

**F6** Stage 2 performs the per-segment walks, resolves SHAs, places commits, and sets
`Placement` and `GapContinuity`.

**F7** `parse_commits_from_comments` and the `CommitStatus` *parse* are deleted. Round
events become the sole representation of what the comments said.

**F8** Stage 1 must still run **before** any walk — it supplies each segment's branch,
and the walk is what lets stage 2 resolve anchors. This ordering already exists
(`newest_recorded_branch` was hoisted above the walk in `src/issue.rs` during the
cross-branch work); the segment model generalises it from one branch to one per segment.

---

## §7 API (A)

**A1** `IssueStatusResponse`: `segments[]` replaces **both** `rounds[]` and the
top-level `commits[]`. `open_round_index` is dropped — it is
`segments.at(-1).kind === 'round'`.

**A2** Top-level `branch` → **`active_branch`**.

> **The bug this fixes.** `IssueStatusResponse.branch` is currently
> `parse_branch_from_body(issue.body)` — the issue body's branch. With cross-branch
> rounds: issue created on `main`, round 2 opened on `feature/x`, user checks out
> `feature/x`, and `isWrongBranch` (`ui/src/components/IssueCard.tsx:25`) grays out the
> issue they are actively QC'ing. One card then holds three answers to "what branch":
> status computed on `feature/x`, label showing `main`, graying because they differ.

Deliberately **not** `active_branch_is_checked_out: bool`. `/api/repo` has a
`refetchInterval` (`ui/src/api/repo.ts:50`) while issue statuses sit behind a 5-minute
`staleTime` (`ui/src/api/issues.ts:392`). Putting a volatile fact in the slow response
would mean branch switches do not show until statuses refetch. The compare belongs at
render time — it is a join of stable server data with cheap polled data, not derivation,
so it does not violate **D7**.

**A3** `qc_status` exposes **both** `standing_approval` (null while a Round is open) and
`last_approved_commit` (newest ever), plus `latest_commit`. `approved_commit` is
**removed** — it silently meant both, and was the field that contradicted `status`.

Rationale: the archive needs a *user choice* between the last approval and the current
latest (e.g. at round 3, choosing between round 2's approval and HEAD). Exposing both
makes that possible without another shape change. **The archive UX itself is a separate
discussion, not settled here.** Current ungated consumers to revisit then:
`ui/src/components/ArchiveTab.tsx` (4 sites) and `FileResolveModal.tsx` (4 sites), all
using `approved_commit ?? latest_commit`.

**A4** Per-commit `statuses` become a **projection** computed by the API from the owning
segment's events and state, not a stored parse (**F7**). The wire shape is unchanged, so
the picker's status dots keep working.

**A5** Each segment projects: `kind`, `branch`, `commits[]`, `placement`, and for Gaps
`continuity` plus derived `lower_bound` / `upper_bound` (the bounding commits, for
rendering the detached handle).

**A6** `RoundSeedResponse` keeps `branch` / `comparison_base` / `divergence` but
`divergence` re-types to `GapContinuity` per **M4**.

---

## §8 UI (U) — rendering only, per D7

**U1** Picker renders segment by segment from `segments`. A `Diverged`/`Unrelated` Gap
renders the previous-approval handle **detached** — visibly not connected, which is the
truth about divergent history where today's continuous slider implies a linear path
that may not exist. **No windowing, coverage, or gap-run computation in TypeScript.**

**U2** Rail renders `segments`. A branch line is shown when a segment's branch differs
from the previous segment's. Gaps are unnamed, rendered as *"N commits between Round 1
and Round 2"* (Q10). `Unplaceable` segments render grayed with their reason.

**U3** Card renders `qc_status`; grays on `active_branch` ≠ `/api/repo` branch.

**U4** `ui/src/utils/rounds.ts` loses `roundWindow`, `roundWindows`, `openRoundWindow`,
`draftGapRuns` (~100 of 155 lines).

---

## §9 CLI (C) — segment-aware, high level

**C1** `ghqc issue status` gains a segment summary: each Round's index, branch, state
and approval commit; each Gap's commit count and continuity.

**C2** The `Branch:` line becomes `active_branch`.

> `src/cli/status.rs` currently contains **zero** occurrences of "round" — it is
> entirely round-blind while the UI has a rail, per-round checklists and a round-scoped
> picker. That divergence is what prompted this whole redesign; **C1/C2** close it.

---

## §10 Migration (P)

No intermediate green states required (**D11**). An earlier plan with six
green-at-each-step phases was discarded as unnecessary ceremony.

| ID | Step |
|---|---|
| **P1** | Backend model: `Segment`, `Gap`, `Placement`, `GapContinuity`, `IssueCommit` shrink; per-segment walks in the fold; delete `parse_commits_from_comments` and the flat `commits`/`CommitStatus` parse. One coherent change. |
| **P2** | `determine_status` per `§5`; accessors per **M8**; delete `round_membership`, `draft_gap`, both open-round gates. Rewrite `src/qc_status.rs` tests (971 lines, 11 tests, 4 table cases pinned to the flat model). |
| **P3** | API hard swap per `§7`. Card-graying bug dies here. |
| **P4** | UI per `§8`. |
| **P5** | CLI per `§9`. |

**X0 was explicitly skipped**: patching the card-graying bug standalone would patch a
field **P3** deletes, and the bug only affects cross-branch rounds, which are unreleased.

### Files in scope

| Area | Files |
|---|---|
| Model | `src/issue.rs`, `src/round.rs` |
| Status | `src/qc_status.rs` |
| Write path | `src/start_round.rs` (`round_basis`, `RoundBasis`, `BranchDivergence`), `src/repair_round.rs`, `src/new_round.rs` |
| Git | `src/git/file_ops.rs` (`GitCommitOps::merge_base`, `find_commits`, `get_commits_robust`) |
| API | `src/api/types/responses.rs`, `src/api/routes/rounds.rs`, `src/api/routes/issues.rs`, `openapi/openapi.yml` |
| API test fixtures | `src/api/tests/cases/rounds/*.yaml` |
| CLI | `src/cli/status.rs`, `src/cli/context.rs` (slices `thread.commits` at 4 sites — needs review), `src/cli/new_round.rs` |
| Other consumers | `src/archive.rs`, `src/record/mod.rs`, `src/main.rs` (2 `approved_commit` filters) |
| UI | `ui/src/api/issues.ts`, `ui/src/api/rounds.ts`, `ui/src/utils/rounds.ts`, `ui/src/components/{SwimLanes,IssueCard,IssueDetailModal,RoundRail,RoundCommitPicker,StartRoundModal,ArchiveTab,FileResolveModal}.tsx` |
| UI fixtures | `ui/tests/fixtures/rounds.ts`, `ui/tests/status/*.spec.ts` |

### Baseline at time of writing

Rust **471 pass**, `cargo fmt --check` clean, `tsc` clean, `npm run build` clean,
Playwright **245 pass**, zero flaky. 28 files uncommitted on `rounds` (+1108/−87) from
the cross-branch round work, on top of commit `416d07c`.

---

## §11 Resolved questions

| ID | Resolution |
|---|---|
| Q1 | `previous_approval` becomes an accessor, not a stored field |
| Q2 | Keep the name `opened_at` — "the round was opened at"; "anchor" is prose only |
| Q3 | Hard swap (**D9**) |
| Q4 | Expose both `standing_approval` and `last_approved_commit`; archive UX deferred |
| Q5 | CLI segment-aware but high level |
| Q6 | Positional indexing; Gaps need no stable IDs |
| Q7 | `branch_declared` dropped — **D5** makes it meaningless |
| Q8 | Unplaceable neighbour ⇒ gray the Gap + diagnostic; no substitution (**W6**) |
| Q9 | Compare in the card, not a bool in the API (**A2**) |
| Q10 | Gaps unnamed; rendered "N commits between Round 1 and Round 2" |
| Q11 | Implementer's choice: shrink `IssueCommit` or orphan it |
| Q12 | No intermediate greens (**D11**) |

---

## §12 Still open (not blocking P1)

- Archive UX: how the user chooses between last approval and current latest (**A3**)
- Whether `record` formatting work (previously deferred as "P5") interacts with `§9`
- `docs/` has no `new-round` / `repair-round` page; the docs site also needs updating
  for per-round `git branch`, divergence, `--notification-note`, `--from-round`

---

## §13 Resolutions from implementation review (D12–D15)

Four questions surfaced during implementation that `§0`–`§12` did not answer. All four
are resolved; they supersede any earlier reading.

**D12 — Event-named commits get their own API fields.** `IssueCard.tsx:55,64` render
`latest_commit` under the labels *Reviewed* and *Last Posted*. **M8** redefines
`latest_commit` as the newest commit of the active segment (≈ the branch tip), where it
previously meant *a commit named by a comment* — so those rows would label unreviewed
drift as reviewed. Field name and type were unchanged, so this produced **zero** type
errors and was invisible to the migration work list.

Resolution: the API exposes `last_reviewed_commit` and `last_notified_commit`, derived
from the **active Round's** events, both nullable. The UI renders them; it does not walk
`events` itself (**D7**). Note that `IssueThread::latest_notified_commit` and
`latest_reviewed_commit` existed before this refactor and were deleted by **S6** — these
facts were already first-class, and the card's labels always depended on them. Consolidate
into model accessors if `§9`'s CLI summary comes to want them; an API-layer projection is
sufficient for now and is sanctioned by **A4**.

**D13 — W6's diagnostic sentence is withdrawn.** Its example — *"`main` has 4 commits
since Initial QC's approval"* — needs a commit count on the *other* Round's branch, which
**W6** itself forbids the Gap from walking, and `RoundAnomaly::SegmentUnplaceable
{ position, reason }` (**M7**) carries no count. Exposing `anomalies` would not unblock
it. **D4** and **U2** are fully implementable without it, from `placement.kind` and
`placement.reason`. The advisory count is additive whenever it is wanted; **W6**'s
no-substitution rule is unaffected and still governs.

**D14 — Boundary-commit statuses are computed per owning segment, and the asymmetry is
intentional.** When R1's closing commit equals R2's `opened_at` (legal per **D1**), the R1
copy projects `["approved"]` and the R2 copy `[]`. `RoundCommitPicker` therefore draws the
same hash with and without its approval dot depending on which segment renders it. This is
correct: in R2's frame that commit genuinely is not R2's approval. Document it at the
projection site so it is not "fixed" later as a bug.

Corollary, already implied by **S3**: Gap commits can never carry
`notification`/`reviewed`/`approved`, because Gaps have no events or state. A `RoundEvent`
naming a commit outside its Round's walked range silently loses its dot.

**Addendum — flattening to one row takes the union, and that does not contradict the
above.** A flat list (the CLI picker, the UI's `flattenSegmentCommits`) can only show a
hash **once**; showing it twice is itself a bug, since a picker's positional defaults then
resolve two indices to the same commit. So a flattening consumer must dedupe by hash, and
having done so it must **union** the statuses — there is no way to render half a dot, and
dropping one copy's statuses would make a dot vanish depending on which copy survived.

The two rules apply at different layers and both are right:

| Layer | Rule | Why |
|---|---|---|
| API projection (**D14**) | per owning segment; the same hash is annotated differently in each | in R2's frame that commit is not R2's approval |
| Flattening consumer | dedupe by hash, union the statuses, attribute the row to the **newer** owning segment | one row cannot carry two frames; the newer owner keeps a Round's anchor scoped to that Round |

A per-segment renderer (the segment-by-segment rail, **U2**) keeps **D14**'s asymmetry
untouched, because it never flattens.

Also record the behaviour change **D14** implies but does not state: the old flat parse
tracked a single `approved_commit` and stripped `Approved` from every other commit, so
exactly **one** `approved` dot existed thread-wide. The projection dots **every** closed
Round's closing commit, so `[R1(closed@b), Gap, R2(closed@d), Gap]` now yields `approved`
on both `b` and `d`. That is the intended reading — each round's approval belongs in its
own frame — but it is a change, and consumers that assumed uniqueness must not.

**D15 — An `Unrelated` trailing Gap reads `Approved`, and the card grays.** The Gap is
`Placed`-but-empty (no bound-to-bound range is meaningful when the bounds share no
history), so **S1** gives `Approved` and the status stays reproducible per **D8**.

But an `Unrelated` trailing Gap means the approval commit and the branch tip share no
ancestor — the record is telling us the reviewed history is not the history we are looking
at. That is exactly the "cannot be trusted at face value" state **D4** grays for, so
graying extends beyond a checkout mismatch:

> **U3 (revised).** The card grays when `active_branch` ≠ the `/api/repo` branch **or**
> when the active segment is `Unplaceable` **or** when the active segment is a Gap with
> `continuity: unrelated`. The reason is named, distinguishing *not on this branch* from
> *history unrelated to the approval* — they have different remedies.

No new wire field is needed: `continuity` is already on every Gap per **A5**. Status
remains a function of the record alone (**D8**); only display consults the checkout.

**Addendum — the third clause is unreachable, and that is the right answer.** Adversarial
review proved a trailing Gap's continuity is **always `Linear`**: a trailing Gap walks the
previous Round's branch, and a `Placed`/`Closed` Round necessarily has its closing commit
on that same walk (otherwise it would be `Unplaceable`), so the Gap's lower bound always
resolves. The `Unrelated` trailing Gap therefore cannot arise from the fold, and an
approval whose commit has vanished degrades through `Unplaceable` — clause 2 — instead.

Keep clause 3 as a cheap defensive guard in the UI (the API could emit it after a future
model change) but do **not** write a test that depends on the fold producing it — a UI test
may hand-build such a response, and the distinction should be stated so nobody reads that
fixture as a reachable state.

**Second addendum — `Approved`-but-grayed is only reachable through clause 1. This
supersedes the paragraph originally written here, which was wrong.** The claim that
clause 2 delivers an `Approved`-but-grayed card does not survive **S4**: `determine_status`
matches `Segment::Gap(gap) if gap.is_placed()` and then `Segment::Gap(_) => None`, so an
`Unplaceable` trailing Gap yields **no status at all** rather than `Approved`
(`src/qc_status.rs`, and pinned by an explicit assertion in `src/round.rs` — *"an
unplaceable trailing gap must not report as approved"*). S4 short-circuits before S1's Gap
arm can ever return `Approved`.

So the three clauses divide cleanly, and the division is the honest one:

| Clause | Status alongside it | Meaning |
|---|---|---|
| 1 — `active_branch` ≠ checkout | **any real status**, including `Approved` | the record is fine; *you* are standing somewhere else |
| 2 — active segment `Unplaceable` | no status ⇒ `Unknown` (**D12**/**§13**) | the record cannot be placed; nothing can be claimed about it |
| 3 — Gap `continuity: unrelated` | unreachable from the fold | defensive guard only |

That is a better outcome than the version it replaces: "grayed" no longer overloads two
unrelated conditions. Clause 1 qualifies a *trustworthy* status with a viewer-local
caveat; clause 2 accompanies the *absence* of a status. A card can therefore never say
`Approved` while the model is admitting it cannot locate the approval — which is exactly
the contradiction an audit tool must not present.

Consequence for tests: a fixture pinning `status: "approved"` together with an
`Unplaceable` active segment is **not producible by the backend** and must not be used to
pin clause 2. The reachable clause-2 fixture is `status: "unknown"` with
`latest_commit: null` and `initial_commit: null`.

---

## §14 Known defect — a `Diverged` Gap can over-claim an older Round's commits

**Status: pre-existing, reproduced, not fixed.** It panics identically before and after the
segment migration, so it is not a regression — but it is real and should be fixed.

`build_gap`'s `Diverged` arm sets the Gap's older bound to the merge-base. When the newer
Round's branch forked **before** the previous approval, the merge-base is older than that
approval's position on the walk, so the Gap reaches back past commits an earlier Round
already owns:

```
main = A,B,C,y   (R1 approves B)
feature/x branched at A — before B — with R2 on it
R3 back on main, anchored at y

The Gap before R3 walks main; its lower bound (R2's approval) is off main;
merge_base(R2's approval, y) = A  ⇒  the Gap claims C *and B*.

segment invariants violated: Err("commit bbbbbbb…0002 is owned by segments [0, 3]")
```

Caught by the **overlap** clause, so it is a `debug_assert` panic in dev and test builds
and a silent double-count in release. **I4** is the invariant it breaks.

**Why it was not fixed here.** The over-claim is against R1, which is *two segments back*,
while `build_gap` receives only its immediate `older`/`newer` neighbours. A correct fix has
to thread the already-built segments' ownership on that walk into the bound, so that `end`
stops at the newest commit a preceding segment owns. That is a change to the fold's bound
computation, and it wants its own adversarial review — the same standard every other change
in this migration was held to.

**Intended fix.** `resolve_segments` builds segments in order, so when it calls `build_gap`
it already holds every preceding segment. Pass those in and clamp the Gap's older bound to
the newest position on this walk owned by a preceding segment. In the example that yields
`[C]`, which is exactly the drift on `main` between R1's approval and R3's anchor.

Note this is **not** a licence to let W5 override W2. **W2** already fixes an interior
Gap's older bound at *the previous Round's closing commit, exclusive*; **W5**'s merge-base
is a refinement that applies when the merge-base is *newer* than that bound. The clamp
restores W2's precedence rather than inventing a new rule.

---

## §15 Picker reach and density (D16, U5–U8)

Added after use. The picker's single `showAll` boolean did **three** jobs at once
(`RoundCommitPicker.tsx`): dropped the pertinence filter, left the round scope, and
enabled the collapsible-gap markers. The three are orthogonal, and welding them together
produced the complaint that motivated this section — *the whole history is too busy*, yet
the only way to see one extra commit inside the current round was to show everything.

**D16 — reach and density are separate axes, and reach is expanded per segment, in place.**

- **Density** (a checkbox, scoped): show every commit in the scoped round, not only the
  file-changing or comment-named ones. Never leaves the scope.
- **Reach** (no mode, no dropdown): every segment outside the scope renders as a collapsed
  marker on the track — `Round 1 ·4`, `gap ·3` — and clicking one expands it **in place**.
  This generalises the interior-gap markers that already existed, which were previously
  available *only* in the noisy mode, exactly backwards.

  > **Corrected after use — twice. See §16.** The on-track markers were wrong, and so was
  > the horizontal rail that replaced them. The axis split above is the only part of D16
  > that survived; everything about *where* and *how* reach is exposed is superseded by
  > **D17**.

**U5 — scope survives approval.** Scope was `activeRoundPos`, which is null unless the
last segment is a Round. Approving closes the round, **I3** appends a trailing Gap, and the
picker therefore silently fell back to the entire history — the jump a user sees the moment
they approve. Scope is now the newest Round **plus the trailing Gap when one is last**, so
approving does not change what is on screen, and drift landing after the approval appears
without touching a control.

**U6 — what may be selected depends on what the action does.** Not one rule: the
constraint follows from whether an action mutates round state, records an observation, or
asks a question.

| Tab | Handles | `from` | `to` |
|---|---|---|---|
| **Notify** | range | anything visible | the scoped round **and newer** (so trailing-gap drift qualifies) |
| **Review** | single | — | **anything visible**, including an earlier round |
| **Approve** | single | — | **the scoped round only** |

*Notify* asks someone to read a range, so `to` bounds the question — it means "the newest
state I claim to have addressed", which cannot sit in a round that already closed. *Review*
merely records what the reviewer read, and reading an older round's commit is legitimate.
*Approve* closes a round **at** a commit, so that commit must belong to that round —
including excluding the trailing gap, since approving drift no round covers would bypass
the model (**S5** makes *start a new round* the resolution for `changes_after_approval`).

**U7 — the constraint is keyed on position, not on segment identity.** "A different round
is from-only" is nearly right and fails for the trailing Gap, whose commits are *newer* than
the scope and are precisely what **U5** exists to surface. The rule is therefore
older-than-scope ⇒ from-only; scope-or-newer ⇒ eligible as `to`. Deriving it from what the
handle means rather than from a segment-kind check is what keeps it correct when a new
segment shape appears.

**U8 — the UI is stricter than the CLI, deliberately.** `QCApprove::from_interactive`
(`src/cli/context.rs`) offers the flat all-segments list, commented *"an approval may name
an older one"*. The UI blocks that; the CLI remains the escape hatch. This is a differing
**guardrail**, not a differing model — a flat list has no scope to be strict about — but it
is a real CLI/UI divergence and is recorded here rather than left to be rediscovered,
because `§0` exists precisely because such divergences went unrecorded.


## §16 Reach as a vertical timeline (D17)

Added after use, and it **supersedes D16's placement of reach** (§15). The axis split
itself — density is a checkbox, reach is not — was right and stands. Putting reach *on the
track* was not, and neither was putting it in a horizontal strip beside it.

### Two failed designs, one failure

**Attempt 1 — chips pinned to a track boundary.** Every out-of-scope segment is older than
the scope, so the boundary they attached to was always slot 0. The chips never moved, so
their position meant nothing; a third round piled four of them into the corner; and because
they were derived *from* the scope, an `Unplaceable` scope round (**D4/S4**) emptied the
scope and erased every one of them — no structure shown at the one moment the structure is
confusing. The track was also 24px left of centre (`paddingLeft: 16, paddingRight: 40`),
since fixed and pinned by a symmetry assertion rather than pixel literals.

**Attempt 2 — a horizontal rail of round chips joined by gap connectors.** This fixed the
scope-erasure and the pile-up, and it was still wrong. At that size a connector is a few
pixels of dashes, so the continuity distinction it existed to carry was illegible; and pills
reading `Round 2 ·2` beside `Initial QC ·5` look like a **tab bar**, so the implied action
was *switch to that round* when the actual action is *add it alongside*. An affordance that
misstates its own action is worse than no affordance.

Both are the same failure: **1D leaves room for glyphs but not for words.** `·5` is correct,
compact and unreadable — the whole problem in miniature.

### D17 — reach is a vertical timeline behind a `History ▾` button

Vertically there is room to simply say `2 commits`, `no shared history`, `its branch is
unavailable locally`. Nothing is a glyph to decode, and a spine drawn as a `border-left` on
each row's gutter joins up **by construction** — consecutive rows meet where the rows meet,
so there is no position arithmetic to get wrong. That is the structural reason the vertical
form works where two horizontal ones did not.

Ordered **newest-first**, matching `git log` rather than the track's left-to-right age
order: the segment being worked in is the one that should be under the cursor. `pos` still
indexes the oldest-first model, so the displayed order is the model's reversed.

The load-bearing distinction survives from attempt 2 intact, because it was never the part
that was wrong:

- **Order is round order**, and the model always knows it. `segments` is strictly
  alternating and oldest-first (**I1/I3**), derived from *comment order in the issue* — not
  from git ancestry. So a row's position is honest even for rounds whose branches meet
  nowhere. This is what makes the design safe for the cross-branch case, which is the
  question that produced it: *how does this work if Round 2 and Round 3 are not on the same
  branch and a consistent history does not exist?*
- **Connection is a claim about commits**, and belongs to the **gaps alone**. A gap is not a
  station on the timeline; it is the join between two rounds, so it is drawn *as the spine*
  and its `continuity` is the spine's line style:

  | Gap state | Spine through that row | Says |
  |---|---|---|
  | `linear` | solid grey | the commit count, when non-empty |
  | `diverged { merge_base }` | dashed orange | `histories diverge — they meet at <sha>` |
  | `unrelated` | **severed** — no spine, an orange bar across | `no shared history — no diff across this point is meaningful` |
  | `Unplaceable` | no spine | `commits between these rounds could not be listed` |

  `unrelated` is severed rather than dashed deliberately: *"these rounds are on branches
  that meet nowhere"* is a different fact from *"they diverged and meet upstream"*, and a
  dashed line reads as the second. An `Unplaceable` gap gets no spine either — its commits
  are *unknown*, not absent, and a plain spine would assert a continuity nothing
  established.

Consequences:

- **An `Unplaceable` round stays on the timeline**, its dot hollow orange, unselectable,
  reading `could not be placed · <reason>` as ordinary text. It owns no commits (**I5**), so
  there is nothing to put on the track. This inverts attempt 1's worst failure.
- **The scoped round is not a toggle** — it reads `always shown`. The round being worked in
  is the point of the picker.
- **In the fallback there is no round being worked in.** When the scope fell back, an older
  round must not be described as *just closed* merely because it was the newest *placeable*
  segment; the rows then say only what is true of the round itself. This was a live
  misstatement, caught by looking at the rendered output rather than at the tests.
- **No menu for a single-round thread**, which has no structure to navigate. Those threads
  behave exactly as before rounds existed.

**Reach is never blocked, including across a severed gap.** You may put an `unrelated`
neighbour's commits on the track and select one; the receipt then refuses to claim a result
— `⇢` instead of `→`, commit count withheld, *"histories not connected"*. One rule instead
of a special case, and the same principle as **U8**: the guardrail that matters is
per-action (**U6** — Approve remains `scope-round-only`), not a blanket ban on looking.

**The density checkbox is relabelled** from *"Show every commit in this round"* to *"Show
every commit on the track"*. The menu can put more than the round on the track, so the old
label had become false — and *"in this round"* was itself a correction of *"Show all
commits"*, whose ambiguity between the two axes is what §15 removed.

### Known rough edge

**Escape closes the whole issue modal, not just the menu.** Mantine's `Modal` implements
`closeOnEscape` with a `window` listener, which neither a React `stopPropagation` on the
dropdown nor a capture-phase `window` listener registered by the menu preempts — verified,
not assumed. Scoping it properly means plumbing `closeOnEscape` down through
`IssueDetailModal`, which is a change to the modal rather than to the picker, so it is
recorded here rather than done silently. The menu is dismissed by clicking `History` again.
