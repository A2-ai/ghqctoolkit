# Archive under Round Semantics — Spec v1

Status: **agreed, not implemented.** Branch `rounds`.

This spec is the **authority** for the archive rework. Implementers follow it and do not
redesign. Where an implementation review changes a decision, append a new numbered
section (`## §11 Resolutions from implementation review`) that says which earlier clause
it supersedes — do not silently edit earlier text.

Read alongside `design/segment-model.md` (the `IssueThread` → `Vec<Segment>` model) and
`design/segment-api-contract.md`. This spec closes that spec's deferred **A3/Q4** —
"archive UX: how the user chooses between last approval and current latest."

Read `§0` before implementing. The diagnosis is the justification for every decision.

---

## §0 Diagnosis — why this exists

Archive has **four different predicates all named "approved,"** and they disagree in
exactly the case rounds were built for: *approved in R1, R2 now open.*

| # | Site | Predicate | Reopened case |
|---|---|---|---|
| 1 | `src/archive.rs:63` `ArchiveFile::from_issue_thread` | `last_approved_commit().is_some()` — ever-approved, ungated | **true**, commit = R1's closing commit |
| 2 | `src/cli/archive.rs:105`, `src/main.rs:1144/1170/1201` | same | **included** by default |
| 3 | `ui/src/components/ArchiveTab.tsx:54` `isApprovedStatus` | `status ∈ {approved, changes_after_approval}` — *currently* settled | **false** → hidden unless per-milestone "include non-approved" is on |
| 4 | `ui/src/components/ArchiveTab.tsx:138` `milestoneFileSets.approvedOnly` | `issue.state === 'closed'` — **GitHub issue state** | **false** (reopened) |

- **§0.1 The `approved` bit lies.** When the UI includes a reopened file it sends
  `commit = archiveCommitOf()` = `last_approved_commit` (R1's approval) together with
  `approved: false` (#3): approved content, labeled unapproved. The CLI writes
  `approved: true` for the same issue. Same repo, same issue, two answers.
- **§0.2 CLI and GUI ship different archives** — #1/#2 versus #3/#4 are different
  concepts, not a flag mismatch.
- **§0.3 `approved: true` is unfalsifiable** after the first approval. Rounds correctly
  make R1's approval permanent, so the bit is `true` for every archive generated
  afterwards regardless of what happened since. It stops carrying information.
- **§0.4 The metadata cannot represent the reopened state.**
  `ArchiveQC { milestone, approved }` plus `ArchiveFile.commit` names no round, no
  open-ness, no drift. An archive cut mid-R2 is byte-identical in metadata to one cut
  when R1 was the whole story.
- **§0.5 `changes_after_approval` silently archives non-current content.**
  `archiveCommitOf` prefers the approval, so archived bytes ≠ working tree, and nothing
  in the card or the metadata says so.
- **§0.6 Conflict detection is built on the wrong fact.** The flatten /
  include-non-approved conflict predictor (`ArchiveTab.tsx:197–245`) partitions by
  GitHub issue state (#4), so it mispredicts for every approved-then-reopened file.
- **§0.7 No round is addressable.** You can archive "the newest approval, ever" and
  nothing else. Once R3 approves, the R1-era archive is unreproducible.
- **§0.8 Purpose.** An archive captures a repository at an approved period such that the
  analysis can be re-run and reproduced exactly. Every rule below is judged against this.

**Root cause** is the segment spec's `§0` shape recurring: **one bool compressing two
independent facts** — *were these bytes approved* (permanent) and *were they the newest
QC state at archive time* (perishable). The fix is two fields, not a better bool.

---

## §1 Decisions (D)

| ID | Decision |
|---|---|
| **D1** | Two independent facts per file, never one bool: the **provenance** of the archived bytes, and **whether those bytes were the newest QC state at archive time**. |
| **D2** | The open/approved line is **not a filter** — it is a **per-file round selection**. The user chooses a round; the bytes follow from that round (**S1**). |
| **D3** | Provenance is a **point-in-time snapshot**, never recomputed by a reader. |
| **D3a** | **No unknowable upper bound is recorded.** No `rounds_total`, no "round 2 of 4" — the total can change after the archive is written. Only facts that stay true forever: which round the selection addressed, and which commit was taken. |
| **D4** | Selection **mode 2 — a file in no milestone — is unchanged**: the user picks a commit directly. |
| **D5** | `approved: bool` is **removed** from `ghqc_archive_metadata.json`. Hard swap, matching segment-spec **D9**, noted in `CHANGELOG.md`. No external reader of that file exists (confirmed with the author). |
| **D6** | The backend computes everything derivable. The frontend sends only what the backend needs in order to derive, and renders the result. All four §0 predicates and `issue.state === 'closed'` are deleted as approval signals. |
| **D7** | A file may be archived at **any** round, not only the newest — this is what **§0.7** costs today. |
| **D8** | A file appears **at most once** per archive. "R1's approval *and* R3's approval of the same file" is newly expressible and is rejected: the archive is a snapshot (**§0.8**); `record` is the history surface. |
| **D9** | **Default target is the latest round** — not the newest approval. There is a reason a round is open, and the archive shows current reality first. A user cutting an archive for a previous QC round knows to go back a round. **No gate and no confirmation dialog** — labeling only (**U3**). |
| **D10** | **The recomputation basis is the repository plus `ArchiveMetadata.created_at`, not the metadata's derived fields.** Anything a reader can reconstruct from those two is not stored. `superseded` is therefore a **convenience flag and a record of the archiver's contemporaneous knowledge** — deliberately *not* an evidentiary structure. This is why it is a bool, and why it must not be re-expanded into a per-cause structure later. |

---

## §2 Types (M)

**M1** Replaces `ArchiveQC` in `src/archive.rs`:

```rust
pub struct ArchiveQC {
    pub milestone: String,
    pub round: RoundProvenance,
}

pub struct RoundProvenance {
    /// The round this selection addressed. Never becomes false (D3a).
    pub round: u32,
    /// Some ⇒ these bytes are a round's closing commit, with who and when.
    /// None ⇒ the bytes were never approved (S1 row 3).
    pub approval: Option<Approval>,
    /// True ⇒ at archive time these bytes were **not** the newest QC state of the
    /// file: a later round had approved, the latest round was open, or the file had
    /// changed since this approval. A glance-level warning, not evidence (D10) —
    /// the cause is recoverable from the thread plus `created_at`.
    pub superseded: bool,
}

pub struct Approval {
    /// The round that closed on this commit — may differ from
    /// `RoundProvenance.round` when a round's anchor is the previous round's
    /// approval (segment-spec D1). See I2.
    pub round: u32,
    pub commit: ObjectId,
    pub by: String,
    pub at: DateTime<Utc>,
}
```

`rounds_total`, `was_latest`, a `thread_state` enum, `superseded_by`, and a
`Superseding` cause enum were all considered and **rejected** — see **D3a**, **D10**,
and *Resolved* R6.

**M2** `RoundProvenance.round` is the round the *selection* addressed, not necessarily
the newest round. This is what makes **D7** representable.

**M3**

```rust
pub fn from_issue_thread(
    thread: &IssueThread,
    flatten: bool,
    target: ArchiveTarget,
) -> Result<Self, ArchiveError>

pub enum ArchiveTarget {
    /// D9 — the latest round.
    Latest,
    /// D7 — an explicitly selected round.
    Round(u32),
}
```

**M4** New accessor on `Round`, and the whole of the "latest update commit" definition:

```rust
/// The round's newest commit carrying an action — its anchor (`initial qc commit`
/// for Round 1, `opened_at` for Round ≥ 2), a notification, or a review.
/// Drift commits nobody acted on are not candidates.
pub fn latest_actioned_commit(&self) -> Option<&IssueCommit>
```

Computed as the newest, by position **within the round's own `commits`**, of
`{opened_at} ∪ {e.commit for e in events}`. This is the same computation as segment-spec
**M8**'s `next_notification_from()` in its `Round` branch — note the identity in both
places so the two cannot drift apart.

**M5** `ArchiveFile.qc: Option<ArchiveQC>` keeps its shape — `Some` for mode-1 milestone
files, `None` for mode-2 manually added files (**D4**).

---

## §3 Invariants (I)

- **I1** **Every non-latest round is closed.** Segment-spec **I3** allows only the last
  segment to be `Round(Open)`, and **F3** makes a retraction reopen the *current* round
  while absorbing its following gap — so a reopened round is always the last one. This
  is what makes **S1** total: "select an older round that was never approved" is
  unreachable. If that shape ever becomes reachable, **S1** row 1 is a lie and the
  fold's invariant broke first.
- **I2** `approval.is_some()` ⟺ `ArchiveFile.commit` is some round's closing commit —
  **and that round is named by `approval.round`, which may be less than
  `RoundProvenance.round`.** Reachable via segment-spec **D1** (a round's `opened_at`
  may equal the previous round's closing commit): selecting an open R2 whose anchor is
  R1's approval yields `round: 2, approval: Some({round: 1, …})`. Read as: *you were on
  round 2; the commit you took happens to be round 1's approval.* Round 2 is not
  asserted approved; the bytes are asserted approved under a different frame.
- **I3** `superseded == false` ⟹ the bytes were the latest round's approval with no
  file changes since, as of archive time.
- **I4** Every file in the request resolves to a commit. An unplaceable thread never
  reaches archive construction (**S5**).
- **I5** Nothing in `RoundProvenance` encodes a round count, a bound on future rounds,
  or a per-cause breakdown of `superseded` (**D3a**, **D10**). Enforced by review, not
  at runtime.

---

## §4 Selection (S)

**S1 — the content rule.** Given a selected round `r` on a thread with `n` rounds:

| Case | Bytes archived | `approval` |
|---|---|---|
| `r < n` — not the latest round | `r`'s closing commit | `Some` |
| `r == n`, closed | `r`'s closing commit — **even if the trailing gap is non-empty** | `Some` |
| `r == n`, open | `r`'s **latest actioned commit** (**M4**) | `None`, except per **I2** |

Row 1 is total by **I1**. Row 2 preserves today's bytes for the approved-with-drift case
and merely labels them. Row 3 defines "latest update commit" per **M4**.

**S2 — default.** `ArchiveTarget::Latest` ⇒ `r = n` (**D9**). For an
approved-then-reopened file the default is therefore **row 3: unapproved bytes**, and the
user retargets to `r = n-1` to get the standing approval. This is the one behavior change
from today's `last_approved_commit ?? latest_commit`, and per **D9** it is intentional
and ungated.

**S3 — `superseded` derivation**, evaluated at archive time. `true` if **any** of:

- a round `> r` has closed;
- the latest round is open;
- `r == n`, `r` is closed, and the trailing gap contains a file-changing commit.

Otherwise `false`.

**S4** `--include-unapproved` / the GUI's "include non-approved" narrows to mean
**threads where no round has ever closed**, only. It no longer governs reopened files;
selection (**D2**) does.

**S5** An unplaceable active segment (segment-spec **S4**) **blocks generation with an
explicit override**, and the file is called out as unplaceable with its
`placement.reason` — replacing today's easily-missed "N files left out" note.

**S6 — behavior change worth stating.** Today an unapproved file archives
`latest_commit`, the active segment's newest commit, whether or not anyone ever put it up
for review. Under **M4** it archives the newest commit someone *acted on*. This changes
the archived bytes for never-approved files, and is the more defensible choice under
**§0.8**.

---

## §5 API (A)

**A1** Mode-1 request becomes `{ issue_number, round: Option<u32> }` — `None` ⇒
`ArchiveTarget::Latest`. The backend derives the commit, `approval`, and `superseded`
(**D6**). Mode-2 files keep `{ repository_file, commit }` (**D4**).

**A2** `approved: bool` is **removed** from `ArchiveFileRequest`.

**A3** No new status-endpoint fields are required: `segments` already carry per-round
`state`, `closing_commit`, `events`, and `placement`, so both the round picker and the
default derive from data the UI already has. `latest_actioned_commit` **is** projected
onto each round segment so the UI can label the default without re-deriving it (**D6**).

**A4** `ghqc_archive_metadata.json` gains `round: RoundProvenance` per mode-1 file and
loses `approved`. Changelog entry per **D5**.

---

## §6 UI (U)

**U1** Each mode-1 card shows both facts: a provenance line
(`Round 2 approval · a1b2c3d · @wes 2026-07-04`, or `Round 3 · unapproved · d4e5f6g`)
and, when `superseded`, a marker that these bytes were not the newest QC state.

**U2** Per-file **round picker**, shown only when `n > 1`, reusing `RoundRail` /
`RoundCommitPicker` from the status work. Default = latest round (**D9**); an explicit
selection is visually marked as an override.

**U3** A reopened file's default is unapproved content — label it clearly and
prominently, but **do not gate generation** (**D9**).

**U4** Round-aware bulk filters: *under review*, *changed since approval*, *approved in
round ≥ 2*, *never approved*, *unplaceable*. (**P5**)

**U5** Pre-generate summary:
`12 files · 9 approved & current · 2 approved but superseded · 1 unapproved (round 3 open)`. (**P5**)

**U6** The conflict / flatten predictor is repointed at **post-selection included
files**, replacing the `issue.state === 'closed'` partition (**§0.6**).

**U7** `isApprovedStatus`, `archiveCommitOf`, `addedFileCommitOf`'s status branch, and
`milestoneFileSets.approvedOnly` are **deleted** from the frontend (**D6**).

**U8** Unplaceable files get an explicit blocking callout carrying `placement.reason`,
not a footnote (**S5**).

---

## §7 CLI (C)

**C1** `--include-unapproved` per **S4**.
**C2** Repeatable `--round <issue#>=<n>` for the non-interactive path.
**C3** `prompt_archive` offers round customization behind a single confirm, prompting
only for files where `n > 1`, so the common case stays one keystroke.
**C4** `ghqc milestone status` reports the same summary as **U5**, so the pre-archive
check and the archive agree.
**C5** `docs/milestone-archive.md` and `CHANGELOG.md` updated for **D5**, **C2**, and
the **S2** / **S6** behavior changes.

---

## §8 Phases (P)

- **P1** Backend: `M1`–`M5`, `I1`–`I5`, `S1`–`S3`, `S6` — one shared derivation function
  used by both surfaces. No surface changes.
- **P2** API: `A1`–`A4`.
- **P3** UI: `U1`–`U3`, `U6`–`U8`.
- **P4** CLI: `C1`–`C5`.
- **P5** Non-blocking: `U4`, `U5`, and the deferred "as-of" archive.

---

## §9 Resolved questions

| ID | Resolution | By |
|---|---|---|
| R1 | Mode 2 (file in no milestone) unchanged — direct commit pick | author |
| R2 | Approved-with-drift archives the **approval**, not the drifted bytes | author |
| R3 | An unapproved round archives its **latest actioned commit** (initial / notify / review), not its newest commit | author |
| R4 | Multi-round files get **user round selection**; a non-latest round ⇒ that round's approval | author |
| R5 | **Default is the latest round**, ungated; retargeting to an older round's approval is explicit | author |
| R6 | No unknowable upper bound and no per-cause structure — `superseded: bool` only. Every cause variant is recomputable from the thread plus `created_at`, so a structure bought nothing an audit could not reconstruct while inviting the §0 misreading of a snapshot as current state | author (challenged the original `Vec<Superseding>` lean; lean withdrawn) |
| R7 | Backend derives everything; the frontend sends `{issue_number, round}` | author |
| R8 | Hard swap of `approved`; changelog note; no external reader | author |
| R9 | `--include-unapproved` = never-approved threads only | author |
| R10 | Unplaceable files block with override and are clearly labeled | author |
| R11 | One file per archive; conflict predictor reads post-selection selections | author |
| R12 | "As-of" archive (cutoff date targeting the approval that stood then) deferred to **P5** | author |
| R13 | `Approval.round` may differ from `RoundProvenance.round` (segment-spec **D1**); recorded as "you were on round 2, the commit was round 1's approval" | author |
| R14 | `superseded: bool` conflating post-approval drift with an open re-review round is **accepted** — both mean "go check the thread", both causes are recomputable, and **U1**/**U5** distinguish them at selection time where a user can still act | author |

---

## §10 Still open (non-blocking)

- **"As-of" archive** (**R12**): a cutoff date, with every file targeting the approval
  that stood at that instant, reproducing a historical sign-off archive after later
  re-QC. Expressible under this model; not scheduled before **P5**.
- Whether `record`'s per-round formatting work interacts with **U5** / **C4**'s shared
  summary.
- `docs/` still has no `new-round` / `repair-round` page (inherited from
  `design/segment-model.md` §12).

---

## §11 Resolutions from contract pinning (wave 0)

Added while pinning `design/archive-api-contract.md`. **These supersede the clauses they
name.** An implementer reading top-to-bottom must not act on the superseded text.

**§11.1 — supersedes S5's "explicit override".** S5 says an unplaceable file "blocks
generation with an explicit override" without saying what the override *does*. It means:
**acknowledge and proceed without that file.** It never means "include it anyway" —
**I4** forbids that, because an unplaceable segment owns no commits and there is
therefore no commit the archive could honestly point at (segment-spec **I5**). The file
is never present in the request. A user who knows the commit they want has mode 2
(**D4**) as the escape hatch. The override exists only to stop the silent omission
diagnosed in **§0.4**/**S5**, not to widen what is archivable.

**§11.2 — supersedes A3's first sentence.** A3 opens "No new status-endpoint fields are
required" and then says `latest_actioned_commit` **is** projected. The second sentence
governs: `latest_actioned_commit` is a **new field** on the round segment of the status
response. The first sentence is withdrawn.

**§11.3 — clarifies A3.** `latest_actioned_commit` is **absent**, not null, on
`GapSegment`: a gap has no anchor and no events, so the concept does not exist there
rather than being unknown. It is `null` on a round exactly when
`placement.kind == "unplaceable"`.

**§11.4 — open, escalated to the spec owner.** Whether `ArchiveMetadata` gains a version
field. **D5**/**R8** establish there is no external reader, so the hard swap is safe
*today*; the residual risk is that a post-**A4** metadata file is silently unreadable by
a pre-**A4** tool, and archives outlive tool versions (**§0.8**). **Until resolved, no
version field is added** — implementers follow **A4** exactly as pinned.

**§11.5 — pre-existing defect, do not patch separately.** `ArchiveTab.tsx:438` sends
`{approved: false}` with no `milestone` for every manually added file, which
`src/api/routes/archive.rs:135` rejects with a 400; the same hazard exists at
`ArchiveTab.tsx:423` when `issue.milestone` is null. **A1**/**A2** delete the field and
the flat struct that cause it, so the migration fixes it. It must **not** be patched
against the old flat struct in the meantime.

---

## §12 Resolutions from implementation review (§11.4 closed)

**§12.1 — closes §11.4, and supersedes it.** `ArchiveMetadata` **gains a metadata
structure version field.** Decided by the spec owner.

```rust
pub struct ArchiveMetadata {
    /// Version of the metadata *structure*, not of ghqc. Serialized first.
    pub metadata_version: u32,
    creator: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    files: Vec<ArchiveFile>,
}
```

- **It versions the JSON shape, never the tool.** It is not the crate version, not a
  semver string, and it must never be derived from `CARGO_PKG_VERSION`. A ghqc release
  that changes no metadata shape does not change this number.
- **Value is `1`.** The pre-**A4** shape was unversioned and is retroactively version
  **0**; the **M1**/**A4** shape is version **1**. *Corrected by the spec owner before
  any code was written — an earlier draft of this clause said 1 and 2. Nothing was
  implemented against those numbers.*
- **Increment rule:** bump by one whenever any field of `ArchiveMetadata`,
  `ArchiveFile`, `ArchiveQC`, `RoundProvenance`, or `Approval` is added, removed,
  renamed, or changes meaning or serde representation. Additive-only changes still bump
  it — a reader must never have to guess which additive generation it is holding.
- **Reader rule:** absent ⇒ version 0 (pre-**A4**). A version **higher than the reader
  knows must be refused with a named error**, not parsed best-effort — a partial read of
  a QC archive is worse than a refusal (**§0.8**).
- Serialized **first** in the object so the version is visible in the first line of a
  `head` on the file.

This does not weaken **D5**/**R8**: the swap stays hard, no compatibility field is added
for the *old* shape, and nothing reads version 0. The field exists so that a reader
encountering a shape it does not know **says so** instead of silently misreading it —
which is the failure **§0.4** describes, displaced from the round model onto the file
format.

---

## §13 Resolutions from P1 implementation review

**§13.1 — extends I2.** When **two** rounds closed on the **same** commit (reachable via
segment-spec **D1**: R1 closes at `B`, R2 opens at `B` and closes at `B` again),
`Approval.round` names **the selected round if it closed there, otherwise the newest
round that closed there.** Rows 1 and 2 of **S1** therefore always name themselves, and
**I2**'s open-round case names the older round — which is exactly the reading
**I2** pins ("you were on round 2; the commit you took is round 1's approval").
"Always the oldest" is rejected: it would make row 2 report R1 for a commit R2 closed on,
contradicting row 2's own definition.

**§13.2 — sanctions new library error surface.** `ArchiveError::RoundSelection { file,
round, rounds }` for a selection outside `1..=n`. Not named by any earlier ID, and
correct: **I4** reserves `CommitDetermination` for "nothing could be placed," which is a
different fact. Maps to the contract's 400 for `round: 0` or `round > n`; the API layer
names the issue number, since `ArchiveError` carries only the path.

**§13.3 — supersedes the "assert, do not fall back" instruction for S1 row 1.** Row 1's
unreachable case (`r < n` and `r` not closed, forbidden by **I1**) is reported as a
**named hard error plus a `debug_assert!`**, not a `panic!`. Reasoning: the fold is fed by
network data and **I6**'s invariant check is `debug_assert`-only, so in a release build a
fold bug would take an axum handler task down rather than return a diagnosable 500. An
error is **not** the defensive fallback the original instruction forbade — a fallback
hides the bug by substituting a plausible commit; a named error surfaces it and archives
nothing. The `debug_assert!` keeps dev and test builds failing loudly.

**§13.4 — §11.4 is closed by §12.1.** The `metadata_version` field is now in scope for
implementation: value **1**, absent ⇒ **0**, serialized first, refuse-on-higher.

---

## §14 Resolutions from the P1 fix pass

**§14.1 — completes §12.1's reader rule.** §12.1 pinned *absent ⇒ 0* and *higher ⇒
refuse* but said nothing about an explicit lower version. The rule is now total:

> **A reader accepts exactly the structure versions it can interpret, and refuses every
> other value with the named error.** Today that set is `{1}`. Version **0** is
> **refused**, not accepted.

Refusing 0 is the point rather than a side effect. A version-0 document carries
`{milestone, approved}` where **M1** now expects `{milestone, round}`, and `ArchiveQC` is
a **flattened `Option`** on `ArchiveFile` — so a v0 file does not fail to parse, it parses
with `qc: None`, silently reporting a QC'd file as a manually added one with no QC
metadata at all. That is **§0.4** reproduced in the file format: one document carrying two
disagreeing accounts of whether QC happened. Accepting v0 to honour "nothing reads v0"
gets the outcome backwards.

This does not soften **D5**/**R8**: refusing the old shape is the opposite of a
compatibility field for it.

**§14.2 — the refusal must be unbypassable.** The version check may not live only in a
helper constructor that a caller can sidestep with `serde_json::from_str::<ArchiveMetadata>`.
It belongs in **deserialization itself** — `#[serde(try_from = …)]` over a raw shadow
struct, or an equivalent — so that every path that produces an `ArchiveMetadata` from JSON
has been version-checked. Rationale is the model's own standard (**M2** in
`design/segment-model.md`): make the defect unrepresentable rather than fixed.

**§14.3 — supersedes §13.3's `debug_assert!` half.** The archive-level `debug_assert!` on
S1 row 1 is **removed**; the named `ArchiveError` stays and is tested in the **default
(debug) profile**. Reason: CI runs `cargo test --features cli,api` and lefthook runs
`cargo test --all`, both debug-only, so a `cfg(debug_assertions)` split leaves the error
arm **never executed in CI** — the test exists but does not run where it matters. The
assertion was in any case a second tripwire for an invariant the fold already checks
(segment-spec **I6**), and §13.3's requirement was that the violation not be *hidden*; a
named error that archives nothing does not hide it. One profile, one behaviour, one test
that actually runs.

---

## §15 Resolutions from P2 implementation review

**§15.1 — mode 2 keeps the handler's own path derivation; it does NOT call
`ArchiveFile::from_file`.** The P2 brief asserted the baseline called `from_file` on this
endpoint; it did not — the handler hand-derived the path. Keeping the handler's
derivation is not inertia, it is required: `from_file` does
`file_name().expect("File to have file name")`, a **panic from a client-supplied string**
(`""`, `"/"`), and strips only a leading `/`, whereas the handler filters to
`Component::Normal` so `..` cannot survive into a tar entry name. Switching would be a
regression in both robustness and path safety on a network-facing endpoint. `from_file`
remains the CLI's constructor, where its input is local.

**§15.2 — a mode-1 error body is always `{"error": …}`.** Axum's own `JsonRejection` for a
malformed `mode` or an unknown field returns **plain text**, so this one endpoint would
answer some client errors in an envelope the UI parses and others in one it cannot. That
is a two-accounts-of-one-fact defect of the **§0** kind, at the transport layer. The
rejection is normalized into the same `{"error": …}` shape as every other failure on this
route. This is an addition to the pinned contract and is recorded here as such.

**§15.3 — mode-1 thread fetches are parallelized.** `build_archive_files` fetched each
issue's thread serially, so an archive over a 60-file milestone became 60 serial round
trips. It uses the same `join_all` pattern as the status endpoint. Error reporting stays
deterministic: results are collected in **request order** and the first failure in that
order is the one reported, so the response does not depend on which fetch lost a race.

**§15.4 — the P2 report's claim that §13.3 is unimplemented is stale**, not a finding. It
read `src/archive.rs` before the P1 fix pass replaced the `panic!`. §14.3 has since
superseded §13.3's `debug_assert!` half in any case. Recorded so the divergence report
does not resurrect it.

---

## §16 Open — escalated to the spec owner

**§16.1 — does an unplaceable *trailing gap* block a file whose selected round is
placed?** Not yet decided; **the literal reading is what currently ships.**

**S5**/**§11.1** are written against the **active segment**, and **U8** gates on it. But a
thread can have a placed, closed round `r` — a real, archivable approval sha — behind a
**trailing gap** that is `Unplaceable` (segment-spec **M5** notes `MergeBaseUnreachable`
fires whenever a later round's branch forked before Initial QC, which is *ordinary
history*, not corruption). The pinned contract's rejection row is phrased more narrowly —
"the selected round resolves to no commit" — which would archive that file.

The reason this is not merely a strictness knob: **S3 clause 3 keys on the trailing gap's
`file_changed`**. An unplaceable gap owns no commits, so clause 3 evaluates **false** and,
absent another clause, the file would be recorded `superseded: false` — a positive claim
that these bytes were the newest QC state, asserted from information we do not have. That
is **§0.4** in the metadata rather than in the status pill.

Options, with the consequence of each:

- **(a) Literal — reject on an unplaceable active segment.** What ships today. Never lies;
  refuses to archive files whose approval is perfectly real. Mode 2 (**D4**) is the escape
  hatch, at the cost of the user re-picking a commit by hand.
- **(b) Narrow — reject only when the selected round resolves to no commit.** Matches the
  contract's phrasing; archives the real approval; risks emitting `superseded: false` on
  incomplete information.
- **(c) Narrow, plus `superseded: true` whenever currency cannot be determined.** Archives
  the real approval and never asserts currency it cannot establish — `superseded` becomes
  "not *provably* the newest state," which is already how a reader is told to act on it
  (**D10**: go check the thread).

Until this is resolved, **(a)** stands as implemented.

---

## §17 Resolutions from P4 implementation review

**§17.1 — `--skip-unplaceable` is sanctioned.** **S5**/**§11.1**'s override was specified
as a pre-flight acknowledgement, which the interactive prompt (**C3**) can express but the
non-interactive path cannot — leaving a block no script could pass. `--skip-unplaceable`
means *acknowledge and archive the rest*; it never includes the file (**§11.1** stands).
Without it the non-interactive path hard-errors. It is ignored in interactive mode, where
the confirm is the override.

**§17.2 — S4's "no round has ever closed" takes the current-state reading.** Segment-spec
**F3** makes a retraction reopen the round, so a round that closed and was later
un-approved reports `closing_commit() == None` and counts as **never approved** for
**S4**, even though `retractions` records that it once closed. This matches how
`RoundProvenance.approval` is derived — a retracted approval is a withdrawn approval, and
reporting it as an approval anywhere would contradict **I3**. The historical reading is
rejected: it would make `--include-unapproved` keep a file that no longer has an approval
to archive.

**§17.3 — `get_milestone_issue_threads` returns `Vec<MilestoneIssueThread { number, thread }>`.**
`IssueThread` carries no issue number and **C2**'s `--round` addresses issues by number.
This is a `pub` library signature change; every current call site is in the CLI. The later
wave owning `src/cli/status.rs` gets the issue number for free from it.

**§17.4 — the branch-mismatch warning stays, with corrected wording.** It is not a status
derivation and nothing archived depends on it, so **D8** is not at risk: it advises the
*additional-file picker*, whose commit list comes from the local checkout. Its message was
also literally truncated in the baseline ("may not have commits of interested"). Kept,
re-worded, comment expanded to record why it is legitimate. Residual known imprecision: it
consults only the active segment, so an older *selected* round living on the checked-out
branch still trips it — harmless for an advisory about a local commit picker.

**§17.5 — a `--round` value error is raised with `clap::Error::raw(ValueValidation, …)`,
breaking the surrounding parser's style deliberately.** clap **discards** a
`ContextKind::Usage` context on a value error, verified against the neighbouring
`--additional-file` parser, which loses its own message and prints only
`error: invalid value 'bogus' for '--additional-file <ADDITIONAL_FILE>'`. The raw form
prints the reason the value was rejected. The style break is the point.

**§17.6 — §16.1 IS NOW A DIVERGENCE BETWEEN SURFACES, not just an open question.** P4
independently gated on the **selected round** (option **b**) while P2 ships the literal
**active segment** (option **a**). So the CLI archives a file whose selected round is
placed behind an unplaceable trailing gap and the API refuses it — the **§0.2** defect
(CLI and GUI shipping different archives from one repo) reintroduced by the very rework
that diagnosed it. Whichever option the owner picks, **both surfaces must be made to
agree**, and the predicate must live in one shared place so they cannot drift again.

---

## §18 §16.1 closed — option (c). Supersedes S3, S5, §11.1's gate, and §17.6

Decided by the spec owner: **option (c)** — gate on the **selected round**, and assert
`superseded: true` whenever currency cannot be determined.

**§18.1 — the gate is the selected round, not the active segment.** A file is refused only
when **the round the user selected resolves to no commit**. An unplaceable *trailing gap*
over a placed, closed round no longer blocks: that round's approval is a real, archivable
sha (**S1** row 2), and **I4**'s actual requirement is that every file in the request
resolve to a commit. This supersedes **S5** and **§11.1** wherever they say "active
segment", and ratifies P4's reading over P2's.

**§18.2 — S3 gains a fourth clause. `superseded` means "not *provably* the newest QC
state."** Clauses 1–3 are unchanged; the new clause fires when they cannot be evaluated on
complete information:

> **Clause 4 — currency undeterminable.** True when any segment **after** the selected
> round either is `Unplaceable`, or is a Gap that owns no commits despite being `Placed`.

The second half is not redundant, and this is why the clause is written as a property of
the commit set rather than of `placement`: segment-spec **I5**'s own correction records
that an **`Unrelated` Gap is `Placed` yet owns nothing**, because no bound-to-bound range
is meaningful when its ends share no history (**M4**). So `Unrelated` reaches **S3**
clause 3 with an empty commit list, clause 3 reads "no file-changing commit," and the file
would be recorded `superseded: false` — the same false claim as the unplaceable case,
arriving by a different route. Both are now clause 4.

**§18.3 — the framing is inverted on purpose.** `superseded` is no longer "something newer
exists" but "**it is not provable that nothing newer exists**". This costs nothing: **D10**
already defines the field as a glance-level "go check the thread," never as evidence, and a
reader's action on `true` is identical in all four clauses. The alternative — a positive
claim of currency derived from segments we could not read — is **§0.4** in the metadata,
which is the defect this spec exists to remove.

**§18.4 — one predicate, one home.** The selected-round placement check (**§18.1**) and the
`superseded` derivation (**§18.2**) live in **`src/archive.rs`** and are called by both
surfaces. Neither `src/api/routes/archive.rs` nor `src/cli/archive.rs` may carry its own
copy. §17.6 recorded that the two surfaces had already drifted to opposite answers within
one wave — the shared home is what makes that unrepresentable rather than fixed, and a
reviewer finding a second implementation of either rule should report it as a defect
regardless of whether the two agree at that moment.

**§18.5 — surfaces to reconcile.** `src/archive.rs` gains both rules; `src/api/routes/archive.rs`
drops its active-segment `unplaceable_error` gate in favour of the shared check;
`src/cli/archive.rs` drops its local `partition_placeable`/`UnplaceableSelection` logic in
favour of the same. The user-visible callouts (**U8**, **C3**'s prompt, `--skip-unplaceable`)
stay where they are — only the predicate moves.

---

## §19 Accepted limits of the §14 version gate

**§19.1 — the named error survives on the library's reader, and is stringified on the
direct route. This is accepted.** `#[serde(try_from = …)]` erases `ArchiveError` into
`serde_json::Error`, so §14.1's "refuse with the **named** error" and §14.2's "the check
belongs in **deserialization**" cannot both hold on every path. As shipped:
`ArchiveMetadata::from_json` returns the matchable
`ArchiveError::UnsupportedMetadataVersion`, and a direct `serde_json::from_str::<ArchiveMetadata>`
returns the same refusal as a message. **Both routes refuse** — §14.2's actual requirement
— and the typed variant exists where a caller can match on it. Making the shadow struct the
public read API (dropping `Deserialize` from `ArchiveMetadata`) would give the variant on
every route; it is a larger surface change than §14.2 describes and is **not** taken.

**§19.2 — the gate is on the envelope, not on the file entries. Known, latent, accepted.**
A version-**1** document containing a version-0-shaped *entry* (`approved` present, `round`
absent) is accepted with `qc: None` — the same false negative §14.1 exists to prevent,
one level down. It is latent, not live: nothing in tree writes a hybrid, and the envelope
gate rejects every real pre-**A4** file. The mechanical fix, `deny_unknown_fields` on the
entry, **cannot** combine with the `flatten` that **A4** requires for a mode-2 file to emit
zero extra keys (contract §2.1). Closing it properly would mean giving up the flattened
`Option`, i.e. re-pinning the wire. Recorded rather than fixed; a reader that must trust
entry shapes should validate them itself.

**§19.3 — S1 row 1's error is the only enforcement of I1 in a release build.** Segment-spec
**I6**'s invariant check is `debug_assert`-only, so after §14.3 removed the archive-level
assertion, nothing catches an `I1` violation in release except a caller that happens to
look — here. That is why this error is reachable at all, and it is the accepted trade:
one named error where the fold is consumed beats a tripwire that never runs (§14.3).

---

## §20 Corrections to §18, and one authorisation

**§20.1 — CORRECTS §18.2's clause 4 wording. The earlier text was wrong; the code is
right.** §18.2 said clause 4 fires when a segment after the selection "is `Unplaceable`, or
is a Gap that owns no commits despite being `Placed`." **The second half as written also
describes an ordinary empty `Linear` gap** — which is the *normal* approved-and-current
state, and which by **I3** always trails a closed latest round. Taken literally it makes
`superseded: false` unreachable for every closed-round selection, contradicting the
contract's own pinned example (§8.1: `superseded: false`, empty trailing gap), **I3**, and
§18.3's rationale (clause 4 fires when clauses 1–3 *cannot be evaluated* — an empty
`Linear` gap is complete information). The clause is:

> **Clause 4 — currency undeterminable.** True when any segment **after the selected
> round** either is `Unplaceable`, or is a Gap that owns no commits **because its
> continuity is `Unrelated`**.

It is a property of the commit set **and the reason the set is empty**. An empty `Linear`
gap proves currency; an empty `Unrelated` gap proves nothing. Pinned by the test
`an_ordinary_empty_trailing_gap_still_proves_currency`.

**§20.2 — CORRECTS §18.1's wording.** §18.1 said a file is refused when the selected round
"resolves to no commit." The ratified predicate is the selected round's **`Placement`**, and
the two differ for a round that is `Unplaceable` yet `Closed` with a real identified sha
(reachable — see `an_anchor_off_its_own_branch_is_identified_but_unplaceable`). The literal
wording would archive it; the placement predicate refuses it. **The placement predicate is
correct**: an identified sha on no walked branch is not a sha whose blob we can be
confident of extracting, and refusing at the gate yields a diagnosable message instead of a
failure during tar construction.

**§20.3 — extends §18.4: the refusal must be unbypassable.** `selected_round()` shares the
round *lookup* with `from_issue_thread` but not the *refusal*, so a caller that skips the
gate can still archive the §20.2 case. That is the same class of defect as §14.2's
bypassable version check, and it is resolved the same way: `from_issue_thread` itself
refuses an unplaceable selected round with a named error. This is new error surface and is
**hereby authorised** — §18.4's "one predicate, one home" is not satisfied by a predicate
callers may decline to consult.

**§20.4 — clause 4 is currently future-proofing, and must not be deleted as
untriggerable.** Neither half is fold-reachable in isolation today: `build_gap` gives a
trailing gap the older round's own branch (so an unplaceable trailing gap implies an
unplaceable round), a trailing gap is always `Linear` (so `Unrelated` gaps are interior),
and by **I1** any segment after a *non-latest* selection already trips clause 1 or 2. Its
isolating tests therefore use hand-built segment lists and say so. Recorded here so a
future reader does not remove it for want of a live case — the fold's shape, not the
clause, is what makes it dormant.

**§20.5 — §18.1 closes a second, previously undiscussed case.** P2's review constructed
this and it was not in view when option (c) was chosen: Round 1 closed and fully placed,
Round 2 open and `Unplaceable(AnchorUnreachable)`. An explicit **D7** retarget to round 1 —
well-formed, resolving to a known commit — was **rejected on account of round 2, which the
client never asked about**, defeating **D7** for exactly the case D7 exists to serve
(retargeting away from a broken round to an older good approval). The active-segment gate
caused it; §18.1 removes it. No new decision needed; recorded because it broadens the
justification for (c) beyond the `superseded` hazard that motivated it.

---

## §21 Accepted shapes from the P2 fix pass

**§21.1 — the unreachable refusal path names the round by index, the reachable one by
name. Accepted.** `ArchiveError::UnplaceableRound` carries the round **index**, so the
API's defensive mapping renders "Round 1" where the primary path renders
`SelectedRound::name` — "Initial QC". Both are kept rather than unified: the indexed
wording is only reachable if a caller skips the gate, which **§20.3** made an error rather
than a supported path, and threading a display name through the library error to improve a
string nobody should see is not worth a second naming authority. The reason text is
identical on both paths, since both come from `UnplaceableReason::describe()`
(segment-spec **M5**).

**§21.2 — `SelectedRound::refusal() -> Option<&'static str>` is kept, with its invariant
unenforced by the type. Accepted, with a note for anyone who revisits it.** Because
`is_archivable()` is the guard and `refusal()` is the accessor, a caller must pair them
correctly, and the API layer therefore carries an `.expect()`. That `.expect()` is
unreachable — it sits one line below its own guard — which is why this is **not** being
changed, even though §14.2 and §20.3 applied the opposite standard elsewhere. The
distinction is deliberate and worth stating: those two closed defects that were
**reachable**; this one is a shape that merely *permits* a future mistake. If the type is
ever revisited, the shape that removes the hazard is a single
`archivable() -> Result<(), &'static str>` accessor replacing the guard/accessor pair, and
the API's `.expect()` disappears with it.

---

## §22 Corrections and accepted shapes from the P4 fix pass

**§22.1 — CORRECTS §18.5's wording.** §18.5 said the CLI "drops its local
`partition_placeable`/`UnplaceableSelection` logic", then said in its next sentence that the
user-visible callouts stay — read literally, the first clause deletes the type that carries
the callout's data. The ratified reading, and what shipped: **only the predicate moved.**
`UnplaceableSelection` and `unplaceable_callout` remain, and `partition_placeable` survives
as a sort-and-render shell that calls the shared `selected_round()`. §18.4's requirement is
one implementation of the **rule**, not one implementation of the presentation.

**§22.2 — `describe_round` reading `Placement` is rendering, not a second gate.** The
CLI's round picker labels an unplaceable round rather than hiding it, so the user can see
why a round is not selectable; choosing it is then refused by the shared gate with the same
`UnplaceableReason::describe()` wording. Reading `Placement` to *display* a round is not the
duplicate predicate §18.4 forbids, and the distinction is recorded here so a reviewer does
not have to adjudicate it each time.

**§22.3 — one wording for both refusal routes.** §20.3 authorised
`ArchiveError::UnplaceableRound` but did not say who renders it. The CLI routes it through
`unplaceable_callout`, so the user sees the same callout whether the refusal came from the
pre-flight gate or from the library's own door. Reaching the second route means the gate was
skipped, which §20.3 made an error rather than a supported path.

**§22.4 — clause 4 gets no distinct label on any surface, for now.** `superseded` renders as
one marker regardless of which of the four clauses fired, so a file flagged because its
history is *unreadable* looks identical to one flagged because newer work exists. Accepted
because **§18.3** fixed the reader's action as identical in all four cases and **§20.4**
records clause 4 as dormant — no fold-reachable input produces it today. **Revisit if clause
4 becomes live:** at that point the distinction belongs at selection time (**U1**, **U5**,
and the CLI's provenance report), where a user can still act on it, not in the metadata,
where **D10** already forbids a per-cause breakdown.

---

## §23 Resolutions from C4

**§23.1 — CORRECTS C4's wording. The two summaries agree per file, never necessarily in
total.** §7's C4 says `ghqc milestone status` reports "the same summary as **U5**", which
reads as though the two lines should match. They cannot: `milestone status` counts every
issue in the milestone, while the archive counts what survived `--include-unapproved`
(**S4**) and the unplaceable partition (**§18.1**). The property that makes the pre-archive
check trustworthy is **per-file agreement of the categorization** — the same thread lands in
the same bucket on both surfaces — and that is what is tested. Equal totals were never the
requirement and must not be asserted.

**§23.2 — the categorization has one home, shared by both CLI surfaces.**
`ArchiveCategory` / `ArchiveSummary` / `categorize()` live in `src/cli/archive.rs` and are
called by both `report_archive_provenance` and `src/cli/status.rs`. `categorize()` asks the
shared `ghqctoolkit::selected_round()` gate and then the same `from_issue_thread` the
archive calls, so a bucket cannot disagree with what would actually be archived. This
extends **§18.4**'s rule from the placement predicate to the presentation categories, for
the same reason: `status.rs` deriving its own buckets is **§0.2** at a smaller scale, and it
is pinned by a test asserting `of_threads == of_files` per thread rather than by convention.

**§23.3 — the readiness line names the target it speaks for.** `milestone status` has no
`--round`, so it can only count at each file's **latest** round (**S2**/**D9**). The output
says so in a second line — *what `ghqc milestone archive` would produce with no `--round`
override* — rather than hiding the caveat in help text, because the count is only useful if
the reader knows it is the archive they would actually get. **No `--round` was added to
`milestone status`**: a check that summarizes a hypothetical override is a worse default
than one that summarizes the real one.

**§23.4 — U5's shape is extended, not followed literally.** U5's example predates §18 and
names three buckets and one open round. As shipped: `not placeable` and `added` are extra
buckets; empty buckets are omitted; the open-round detail renders only when the unapproved
files **agree** on a round and is dropped otherwise rather than inventing a list (`(rounds
2, 3 open)` is a one-line change if wanted); and round 1 renders `Initial QC`, matching
`Round::name()`. The `added` bucket appears only on the archive's line, since additional
files carry no QC status — documented as the one thing the check does not claim, rather than
suppressed to make the two lines look identical.

**§23.5 — `docs/milestone-status.md` was edited outside the enumerated fence, and that was
correct.** The brief asked for `docs/` to reflect the new output, and that command's own
page is the only place a user would look; leaving it silent would have repeated the
stale-documentation defect that §14.1's drift already caused once in this run. No other
agent had touched the file.

---

## §24 Resolutions from P3 (UI)

**§24.1 — U2's "reusing `RoundCommitPicker`" is withdrawn; only `RoundRail` is reused.**
`RoundCommitPicker` is a *commit* slider, and wiring it into the archive would hand the
client a commit for a mode-1 file — which **D6**, **S1** and the contract's request shape
forbid, since the server derives the commit from the selected round. `RoundRail` is reused
inside the picker popover to show thread structure. U2's naming of the wrong component was
an error in the spec, not a shortfall in the implementation.

**§24.2 — a QC'd file added through the relevant-files flow is sent as mode 1.** Deleting
`addedFileCommitOf`'s status branch (**U7**) left such files with no commit, since
`handleSelectRelevantFile` sets `commit: ''`. They are now sent as
`{mode: "issue", issue_number: source_issue_number, round: null}`, so the server derives
everything (**D6**). **A1** supports this — the issue number is the handle for a QC'd file —
though no earlier ID says so explicitly. The alternative, mode 2 with a commit picked at
add time, would reintroduce a client-chosen commit for a file the server can resolve.

**§24.3 — the candidate-milestone conflict predictor falls back to a milestone's full title
set when its statuses are not yet fetched. Accepted, with a behaviour change recorded.**
**U6** repointed the predictor at post-selection included files, which do not exist for a
milestone the user has not selected yet. The conservative fallback means a milestone whose
only overlapping file is unapproved is now **disabled** in the dropdown where it was
previously selectable. Accepted because a false "selectable" produces a flatten collision at
generate time, while a false "disabled" is visible and recoverable.

**§24.4 — U5 and the picker render each round's wire `name`, not a bare index.** So
`Initial QC open`, not `round 1 open` — consistent with `Round::name()` and with §23.4's
same decision on the CLI side. U5's example text `(round 3 open)` is illustrative, not
pinned.

**§24.5 — the UI's report of a contract §2.5 / §15.2 conflict is stale.** It was already
resolved by `design/archive-api-contract.md` **§11**, which withdraws §2.5's plain-text
body sentence while keeping its status table. The client tolerating both shapes is harmless
and may stay.

**§24.6 — process note.** The P3 agent ran `git stash push -- ui` inside a malformed guard
command, against an explicit instruction not to stash. It popped immediately and disclosed
it unprompted. Verified by the orchestrator: `git stash list` empty, `HEAD` unmoved at
`03498d0`, every expected modified and new file present, suite green afterwards. **No work
was lost.** Recorded because the disclosure is the reason it could be verified.

---

## §25 Open — escalated to the spec owner

**§25.1 — the UI now re-derives `superseded`'s shape in TypeScript, and §18.4 forbids
exactly that.** Flagged by the implementing agent itself.

**U1** and **U5** must show, before generating, whether a file's bytes are the newest QC
state and why. The wire cannot tell them: `superseded` is computed by the server at
generate time and the response body is only `{output_path}`. So `ui/src/utils/
archiveSelection.ts` reads the segments and reproduces **S3**'s four clauses to render named
causes at selection time.

It is defensible as far as it goes — **R14** puts cause-distinction at selection time
precisely because that is where a user can act, and the derived value never travels to the
server, so it cannot corrupt an archive. But it is a **second implementation of one rule**,
in a second language, and §18.4 made that a reportable defect on its own after §17.6 showed
two Rust surfaces drifting to opposite answers inside a single wave. TypeScript and Rust
drifting is strictly more likely, not less.

Options:

- **(a) Accept it.** Selection-time only, never authoritative, no wire change. Costs: the
  four clauses live twice, and a future clause 5 must be implemented in both or the UI
  quietly under-reports.
- **(b) Project it onto the status response.** Add a per-round-segment field — "if this
  round were selected, would the result be superseded, and why" — so the UI renders instead
  of deriving. One additive, nullable field per round segment; kills the duplicate rule.
- **(c) A dry-run endpoint** returning the provenance the archive *would* write per file.
  Most faithful to **D6**, and the only option that also lets **U5** state the archive's
  real per-file provenance rather than an approximation. Largest change.

Until resolved, **(a)** stands as implemented.

---

## §26 §25.1 closed — option (b). One authority for S3, projected per round

Decided by the spec owner: **option (b)** — the backend answers "what would archiving this
round produce", and the UI renders it. The TypeScript reimplementation of **S3** is deleted.

**§26.1 — the projection is per round segment on the existing status response.** No new
endpoint (that was option **c**, not taken). One additive, nullable object per
`RoundSegment`, `null` exactly when that round cannot be archived — mirroring
`selected_round()`'s refusal, so a client never has to ask why twice.

**§26.2 — pinned Rust surface.** Lives in `src/archive.rs` beside the rules it projects:

```rust
pub struct ArchivePreview {
    /// The commit archiving this round would use — S1's three rows.
    pub commit: ObjectId,
    /// Some ⇒ those bytes are a round's closing commit (I2: may name an OLDER round).
    pub approval: Option<Approval>,
    /// Empty ⇒ provably the newest QC state. Non-empty ⇒ every reason it is not.
    pub superseding_causes: Vec<SupersedingCause>,
}

pub enum SupersedingCause {
    LaterApproval,    // S3 clause 1
    RoundOpen,        // S3 clause 2
    ChangedSince,     // S3 clause 3
    Undeterminable,   // S3 clause 4 (§20.1)
}

/// None when this round cannot be archived (unplaceable — §18.1/§20.2).
pub fn archive_preview(thread: &IssueThread, round: u32) -> Option<ArchivePreview>;
```

**§26.3 — there is NO `superseded: bool` on the projection, deliberately.**
`superseded ⟺ !superseding_causes.is_empty()`, and shipping both would be two fields that
can disagree about one fact — the **§0** defect this spec exists to remove, reintroduced at
the moment of fixing a different instance of it. Clients derive the bool.

**§26.4 — causes are exposed here and remain forbidden in the metadata.** **D10** bars a
per-cause breakdown from `ghqc_archive_metadata.json`, and that stands: the metadata keeps
its plain bool. **§22.4** and **R14** put cause-distinction at *selection* time, where a user
can still act, and this response is that surface. The two rules are not in tension —
they describe different artifacts.

**§26.5 — `archive_preview` extends the escalated option, and this is my call, not the
owner's.** §25.1 escalated only `superseded`. I widened the projection to also carry
`commit` and `approval` because the UI derives those from **S1**'s three rows and **I2**'s
tie-break in TypeScript as well — the same duplicate-rule defect, in the same file, found by
the same argument. Adding two fields to an object being introduced anyway removes it for
almost nothing, where a narrow field would have left `archiveSelection.ts` re-implementing
S1 and I2 after being rewritten to stop re-implementing S3. **Reversible**: if the owner
wants the projection narrowed to causes alone, drop the two fields and the UI keeps its S1
derivation.

**§26.6 — what the UI must stop doing.** `ui/src/utils/archiveSelection.ts` deletes its S1
row logic, its I2 tie-break, and its four-clause supersession derivation, and renders
`archive_preview` instead. What legitimately stays: filter predicates (**U4**), summary
aggregation (**U5**), and wording.

---

## §27 Resolutions from the archive_preview build

**§27.1 — the wire vocabulary has one home: `src/api/types/responses.rs`.**
`ArchivePreview` and `SupersedingCause` carry **no serde derives**. The spellings
`later_approval` / `round_open` / `changed_since` / `undeterminable` are the *contract's*,
and they are applied by the API projection — the same arrangement `QCStatus` →
`QCStatusEnum` already uses in this codebase. Putting a `rename_all` on the library enum as
well would create a second naming authority for one string, which is the defect this run has
been removing all day. If a non-API consumer ever needs the spelling, it goes through the
projection.

**§27.2 — validation and projection use different functions, deliberately.**
`archive_preview()` returns `None` for **two** distinct reasons — the round cannot be
archived, and the round does not exist — and a caller cannot tell them apart. That is
harmless for its pinned use (the API projects over the thread's *own* round segments, so
"does not exist" is unreachable) but it must never be used to validate a **client-supplied**
round, because the request path answers those two cases with different 400s. **Rule:
`selected_round()` validates; `archive_preview()` projects.** Do not merge them.

**§27.3 — `Approval` gained `PartialEq, Eq`.** Beyond contract §5.2's pinned derive list,
and sanctioned: it touches no field and no serde representation, and it is what lets a test
compare a whole `ArchivePreview` rather than field-by-field.

**§27.4 — clause order is load-bearing on the wire with no type-level guard.** Contract §12
says a client may render `superseding_causes` verbatim, so the order is
`LaterApproval, RoundOpen, ChangedSince, Undeterminable` — enforced only by
`superseding_causes()`'s push order and one co-occurrence test. **If a clause 5 is ever
added, the contract's cause table and that push order must be changed together.** No guard
was invented; this note is the guard.

**§27.5 — the preview and the metadata are one code path, not two that agree.**
`archive_derivation()` holds the shared lookup, the **§20.3** refusal, **S1**'s three rows,
`approval_at` (**I2**/**§13.1**), and the cause list; `from_issue_thread` folds it into
`RoundProvenance` and `archive_preview` returns it directly. This is the strongest form
**§18.4** can take — not one predicate called twice, but one derivation with two shapes —
and it is pinned by a test walking every round of ten fixtures, which additionally asserts a
`saw_older_approval` flag so that if the fixtures ever stop containing the **I2** case the
test fails rather than silently proving less.

---

## §28 Resolutions from the documentation and status-report fixes

**§28.1 — `superseded`'s causes are enumerated in exactly ONE place in the doc set.**
`docs/milestone-archive.md`'s `round.superseded` row is the single authoritative list and says
so in its own text; `docs/milestone-status.md` points at it instead of abbreviating it. This
field's documentation drifted **three times** in this run — "three exhaustive causes" written
before clause 4 existed, `superseded: false` framed as an absence rather than a provable
claim, and a bucket table silently missing clause 2 — and every drift came from a *second,
abbreviated* list that looked complete. Two lists are the mechanism, so the second was
deleted rather than corrected. Non-exhaustive mentions that are explicitly partial in context
("including…", or one cause named inside a bullet about unplaceable segments) are fine and
were left.

**§28.2 — clause 4's prose is corrected everywhere.** "Part of the history could not be
read" is wrong for half of it: an **`Unrelated` gap is `Placed`** and is perfectly readable —
it spans histories with **no common ancestor**, so no bound-to-bound range is meaningful
(segment-spec **M4**, **I5**'s correction). The accurate phrasing covers both halves: a
segment after the selected round *could not be located*, **or** *spans histories with no
common ancestor*.

**§28.3 — the readiness caveat is now unremovable by construction, not by test.**
`src/cli/status.rs` gained a private inner `mod report` with
`MilestoneStatusReport { rows, archive }` (private fields). The caveat lives **inside**
`archive_readiness_block`, both entry points *return* the report and print nothing, and
`src/main.rs` has a single `report.print()` after the match. So a path cannot print the table
without the readiness block, cannot construct a report, and cannot replace its readiness —
two of the reviewer's mutations now fail to **compile** (`E0616`, private field) rather than
failing a test.

This is recorded because of *how* it was reached: the implementer first pinned the behaviour
with ordinary tests, ran the reviewer's mutation, and **the suite stayed green** — no unit
test can drive either wrapper, since one needs the network and the other prompts. It moved
the boundary instead of adding a test that would have to be remembered. That is the standard
**§14.2**, **§20.3** and **§18.4** all reach for, applied to presentation: prefer the shape
that makes the defect unrepresentable over the test that catches it.

The one residual uncovered mutation is deleting the single `report.print()` in `src/main.rs`,
which silences the command outright on all three arms — recorded rather than guarded.

**§27.6 — two reports about `src/lib.rs` conflicted; the re-exports are present.** The P2
report stated `ghqctoolkit::archive_preview` does not resolve because `src/lib.rs` omits the
new items. Verified false: `src/lib.rs:41-42` re-export `ArchivePreview`, `SupersedingCause`
and `archive_preview`. P2 read the file before the backend agent saved. No fix was needed,
and none was made — recorded because the divergence report should not carry a defect that
never existed.

**§27.7 — `SegmentInfo::project` takes `(&IssueThread, usize)` instead of `(&[Segment], usize)`.**
Necessary and correct: a round's `superseding_causes` are decided by the segments *after* it,
so the preview is a fact about a round **within its thread**, not about a segment in
isolation. Both call sites were inside `responses.rs`.

**§27.8 — contract §12's pinned key order was unenforced until the P2 fix pass.** The
key-order assertion read `Value::as_object().keys()`, and `serde_json::Value` is a
`BTreeMap` — it re-sorts, so the test was asserting the alphabet, and
`commits < latest_actioned_commit < placement` is alphabetical by coincidence. It would not
have caught a reordering. Both order assertions now run against serialized **text**. Any
future assertion about emitted key order must do the same; `Value` cannot express it.

---

## §29 Findings from the divergence inventory

**§29.1 — `metadata_version` must be private. Writer-side hole.** `archive.rs` makes
`metadata_version` `pub` while `creator`, `created_at` and `files` are private, so a holder
of an `ArchiveMetadata` can set an arbitrary version before `archive()` writes the tarball.
The **read** side is sealed (§14.2's `try_from` over a private shadow struct); the **write**
side is not, which means this build can *emit* a file it would refuse to *read*. That is
worse than the defect §14.1 closed. The field becomes private, set only by
`ArchiveMetadata::new` from `METADATA_VERSION`.

**§29.2 — `openapi.yml` must enumerate `502` for `POST /api/archive/generate`.** The status
is reachable (`issue_fetch_error` → `ApiError::GitHubApi` → `BAD_GATEWAY`) and **tested**
(`an_upstream_read_failure_is_a_bad_gateway`), but the schema lists only
200/400/404/415/422/500. Contract §9.1 anticipated "the 404 and 502 rows … are new for this
endpoint"; only 404 landed. A client generated from this schema cannot handle a documented,
tested failure.

**§29.3 — `Round::newest_event_commit` is production-orphaned by M4 and is removed.** Its
only remaining callers are three tests. **M4** replaced its role, and contract §10's
instruction that the new accessor would *wrap* it was not followed (the accessor re-walks
instead — provably equal, since `opened_at` is always max position on a placed round). Two
functions computing one fact, one of them dead, is what this run has spent its length
removing. If a caller ever needs it again it is three lines.

**§29.4 — `ArchiveFile`'s `pub` fields leave a type-level bypass of §20.3. Accepted,
recorded.** Every field is `pub` (as in the baseline), so a caller can hand-build a mode-1
`ArchiveFile` with a chosen commit and `qc: Some(…)`, skipping both `selected_round()` and
`from_issue_thread`. §20.3 closed the *function-level* bypass; this is the *type-level* one.
Not closed here because the API's mode-2 arm legitimately constructs `ArchiveFile` directly
(`qc: None`) and sealing the type means a constructor for every legitimate shape — real work,
no current caller doing the wrong thing. **The shape that would close it** is a private
`qc` field with a `from_issue_thread`-only constructor; recorded so a future reader has the
design rather than rediscovering the hole.

**§29.5 — the metadata's key order past the first key is unpinned. Accepted.**
`the_metadata_declares_its_structure_version_first` pins only that `metadata_version` is
emitted first. The rest of the order that contract §5.1 illustrates is not asserted on text
anywhere. Accepted because no reader depends on member order in a JSON object; recorded
because §27.8 showed how easy it is to believe an order is pinned when it is not.

**§29.6 — OUT OF SCOPE, flagged for a follow-up run: `src/api/routes/preview.rs:295-297`
still runs `last_approved_commit() ?? latest_commit()`.** That is the exact expression **S2**
replaced and a member of the **§0** #1 predicate family — ungated, ever-approved. The file is
**unmodified from baseline** and belongs to the *notification preview* endpoint, not the
archive, so no ID in this spec reaches it and **D6**'s "all four §0 predicates are deleted"
does not literally cover it. It is **not** fixed here: this run's scope is the archive, and
changing what a notification preview points at is a behaviour change for a different feature
that deserves its own decision. Recorded because the §0 diagnosis is now known to be
incomplete — it named four sites, and this run found two more (this one, and
`FileResolveModal.resolvedCommitOf` in the UI, which *was* in scope and was fixed).

---

## §30 Decisions that shipped without a written resolution (CLI layer)

Found by the divergence inventory. Each changed what ships and had **no** written sanction.
Ratified here, so the spec matches the code and a later reader is not left guessing.

**§30.1 — `--round` with no milestone selected is a hard error.** No ID said what the flag
does when no milestone is chosen. Refusing is right: the flag selects a round for an issue in
a milestone, so with none selected it can have no effect, and silently dropping it archives
every file at its latest round while the user believes one was retargeted (**C2**'s own
reasoning). One shared message across both arms so they cannot drift.

**§30.2 — a duplicate `--round` for one issue is an error, not last-one-wins**, even when both
name the same round. **D8** (one file per archive) is the spirit but does not cover flag
parsing. Ratified: two values for one issue means the user believes two different things about
one file, and guessing which is worse than asking.

**§30.3 — the dropped-`--round` warning is kept and must be documented.** When `--round` names
an issue that the never-approved filter then drops, the CLI warns rather than staying silent —
consistent with §30.1's reasoning. It is currently **undocumented**; `docs/milestone-archive.md`
must mention it.

**§30.4 — the aggregate "N file(s) are archived at unapproved bytes" warning is kept.** Beyond
**U1**/**C4**'s per-file labelling, and the right call: **S2**/**D9** made unapproved-by-default
the deliberate behaviour change of this whole rework, and a per-file marker is easy to miss
across sixty files. **D9** forbids a *gate*, not emphasis. Must be documented.

**§30.5 — `round_label()` is a second round-naming authority and must go.** `src/cli/archive.rs`
carries its own index→name function beside `Round::name()`. That is precisely the duplicate-rule
defect §18.4 and §27.1 treat as reportable, and it is load-bearing: §23.4 and §24.4 both pin
that round 1 renders `Initial QC`, so two implementations can disagree about the one string the
spec names. Consolidate onto `Round::name()`.

**§30.6 — `ArchiveSummary::of_threads` has no production caller.** §23.2 named
`report_archive_provenance` and `status.rs` as the two callers of the shared categorization;
both call `categorize()` directly. Either give it its caller or remove it — an exported function
exercised only by tests reads as API.

**§30.7 — §17.3's stated payoff did not materialize, and that is accepted.**
`get_milestone_issue_threads`'s signature change to `Vec<MilestoneIssueThread>` was justified
partly by `status.rs` getting the issue number "for free"; `status.rs` re-fetches issues itself.
The change is still required by **C2** (`--round` addresses issues by number), so it stands —
but the claimed second beneficiary does not exist and the spec should not pretend it does.

**§30.8 — §28.2 was left half-applied. This is the FOURTH drift on `superseded`'s
documentation.** §28.2 asserts clause 4's prose was corrected "everywhere"; three sites still
say the wrong thing. The pattern is now conclusive: every drift on this field came from a
*partial restatement* of a rule documented in full elsewhere. §28.1 fixed that for
*enumerations*; the same discipline applies to *paraphrases*.

**§29.7 — `413` was also reachable and unlisted; found by audit, not by any spec ID.**
`create_router` applies `DefaultBodyLimit::max(50 MB)` only to `/record/upload`, so
`POST /api/archive/generate` keeps axum's **2 MB** default and a large body is rejected with
`PAYLOAD_TOO_LARGE`. Verified empirically (a 3 MB body returns `413` with
`{"error":"Failed to buffer the request body: length limit exceeded"}`), now enumerated in
`openapi.yml` and pinned by `an_oversized_body_is_rejected_in_the_error_envelope` — the fifth
`ApiJson` rejection class, the other four having been pinned in the §15.2 pass.

Not a functional limit worth raising: a mode-1 entry is a few dozen bytes, so 2 MB is on the
order of tens of thousands of files. It is documented because a client generated from the
schema would otherwise treat a real, enveloped rejection as an unknown status. The audit that
found it — walking every `ApiError` variant and every rejection class against the schema rows —
also confirmed **nothing listed is unreachable**, and that `403`/`409`/`501` are correctly
absent.

**§29.8 — M4 deviates from contract §10's "wrapping" instruction, deliberately and with
equal behaviour.** Contract §10 said the new accessor would *wrap* `newest_event_commit` with
`unwrap_or(opened_at)` plus an `is_placed` gate. It instead re-walks `events ∪ {opened_at}` by
position, because **M4**'s pinned return type is `Option<&IssueCommit>` and an *unowned* anchor
cannot be returned as one. The two are provably equal on a placed round: `opened_at` is always
the maximum position in `Round.commits` (which run back to it inclusive), so a `min` over the
union equals a `min` over events when any event is owned, and `opened_at` otherwise. That
equality is precisely why the old function ended up orphaned rather than reused (**§29.3**), and
the identity is pinned by `the_notification_base_of_an_open_round_is_its_latest_actioned_commit`.
Recorded as a deviation from the contract's stated mechanism, not a defect.

**§29.9 — `ui/tests/fixtures/rounds.ts:109` carries a comment describing a Rust expression that
no longer exists** (`newest_event_commit().unwrap_or(opened_at)`), stale as of **§29.3**'s
removal. Accurate wording: *the round's `latest_actioned_commit` — the newest of its events and
its own anchor, by position in its own commits*. Owned by the UI fence, not fixed by the backend
agent that noticed it.

---

## §31 Findings from the UI review

**§31.1 — the UI suite cannot detect a re-derivation regression. This is the most important
test finding of the run.** Every fixture builds `archive_preview` to be *internally consistent*
with the round's own `closing_commit` / `latest_actioned_commit` / `opened_at` — i.e. each
fixture models what a faithful projection would return. So a regression that reintroduces a
**correct** TypeScript clone of **S1**'s row rule and **I2**'s tie-break — precisely the
duplicate-rule defect **§25.1** escalated and **§26.6** deleted — produces byte-identical
output on every existing fixture and is caught by **no assertion**.

One narrower mutation *is* caught (`reopenedSegments` distinguishes reading `preview.commit`
from reading `commits[0]`, per **S6**), so the proof holds for "reads the wrong wire field" but
not for "re-derives instead of renders", which is the class that matters.

**The fix is a fixture whose `archive_preview` deliberately DISAGREES with its own round's
fields** — an "obviously wrong wire" case. Rendering follows the wire; re-deriving does not; the
two become distinguishable. Without it, §26.6 is enforced only by review, and the next agent to
"helpfully" restore a local derivation gets a green suite.

**§31.2 — `closeRound()` asserts currency it cannot verify. Dormant landmine.**
`ui/tests/fixtures/rounds.ts:211-240` unconditionally sets `superseding_causes: []` on the round
it closes, with no visibility into what follows. Per contract §12 and **S3** clause 2, a closed
round must carry `round_open` whenever the thread's latest round is open, and `undeterminable`
when a later segment is unreadable. Four scenarios build on it
(`multiRoundSegments`, `crossBranchSegments`, `unplaceableStatus`'s round 1,
`reviewedThenDriftedSegments`) and ship a round-1 preview claiming `superseded: false`.

Currently **inert** — no archive spec imports those constants; the archive specs hand-build
corrected segments. But it is the **§18.3**/**§20.1** hazard reproduced in fixture code: a
positive claim of currency derived from information the code never checked. Fix it at the
source, because a fixture that lies is worse than no fixture — it is a trap set for whoever
trusts it next.

**§31.3 — `ui/tests/` is not type-checked by any command in the project, and was not before this
run either.** `ui/tsconfig.json` includes only `src`, so `npx tsc --noEmit` — the command this
run relied on while the Playwright suite was unavailable — says **nothing** about the test tree.
An ad-hoc check surfaces `TS2783` at the `archive_preview: null` scaffold line (harmless, and the
comment documents the intent) and a missing required `blocking_qc_status` in `rounds.spec.ts`'s
`statusOf()` helper (no runtime effect — `updateBlockingQcMaps` guards on presence). Recorded
because "tsc is clean" was reported as a verification signal in this run and is narrower than it
sounds.

**§31.4 — the central §26.6 verdict is CONFIRMED CLEAN by exhaustive grep.** No S1 row logic, no
I2 tie-break, no four-clause S3 derivation, and no synthesized `superseded` bool survives in
`ui/src`. All five §0 predicates are gone, including the fifth (`FileResolveModal.resolvedCommitOf`)
that the original §0 table never named. A fourth U4 filter (`approved_round_2_plus`) reads
`closing_commit !== null` directly and is legitimate — an existence check, not a re-derivation.

---

## §32 Resolutions from the CLI fix pass

**§32.1 — §30.5 is only partly closable inside the CLI, and the real fix belongs in
`src/round.rs`.** `round_label` had three callers, all holding a bare `u32`. One was routed
through `Round::name()` (it had the thread in hand); the other two render provenance **read back
from the metadata**, where `RoundProvenance.round` and `Approval.round` are `u32` and
`report_archive_provenance(&[ArchiveFile])` holds no threads at all. `Round::name(&self)` needs a
`&Round`, so a projection must exist somewhere.

As shipped: one projection survives, renamed `round_name_of`, documented as *a forced projection
of the one authority rather than a second authority*, and **pinned to `Round::name()` by a test**
asserting equality for indices 1..=3 — so a divergence on the spec-named `Initial QC` string now
fails the suite. **The change that removes it outright is `Round::name_of(index: u32)` in
`src/round.rs`, with `Round::name()` delegating to it.** That is the correct home: the naming rule
is the round model's, and both a `&Round` caller and an index-only caller can then reach the same
function.

**§32.2 — `ArchiveSummary`'s `(Initial QC open)` / `(round N open)` parenthetical is a distinct
string from `Round::name()` and stays separate.** It is U5's own lowercase shape, pinned by
**§24.4**/**§23.4**; folding it into `Round::name()` would change pinned output. Recorded so
§32.1's consolidation is not over-applied.

**§32.3 — `src/cli/mod.rs` re-exports eleven items that `src/main.rs` never imports.**
`ArchiveCategory`, `ArchiveSummary`, `categorize`, `ArchiveSelection`, `UnplaceableSelection`,
`MilestoneIssueThread`, `build_archive_files`, `partition_placeable`, `resolve_selections`,
`unplaceable_callout`, `has_closed_round` — all reached internally by module path or by tests.
This is **§30.6**'s "reads as API" smell one level up, and it is closed the same way: the
re-export list is pruned to what is actually imported across a module boundary. Removing a
`pub use` that nothing imports is compiler-verified, so the risk is nil and the benefit is that
the CLI's public surface stops advertising internals as API.

**§32.4 — a documentation defect found while fixing another: the archive never contained the PDF
record.** `docs/milestone-archive.md` claimed the tarball bundles the generated PDF record;
`archive()` writes only `ghqc_archive_metadata.json` plus each file's bytes. Pre-existing,
corrected, with an explicit "It does **not** include the PDF record; generate that separately."
Recorded because it is the same class as the four `superseded` drifts — documentation asserting a
fact about the artifact that the code never implemented.

**§32.5 — the second CI warning was never a source warning.** It is the macOS linker's
`__eh_frame section too large` message, surfaced under `#[warn(linker_messages)]` when the
**debug binary is linked**. Pre-existing, platform- and profile-specific, with no source change
that addresses it — and it does **not** appear in the CI command. A forced full rebuild of
`cargo test --features cli,api --no-run` is warning-free.

---

## §33 The UI suite ran. §31.1's guard is confirmed in behaviour

**§33.1 — the suite was runnable all along; the invocation was wrong.** The day-old `vite dev`
on 3103 binds **IPv6 only**, while `playwright.config.ts` launches
`vite preview --host 127.0.0.1` — a *different socket*, which binds fine. The requirement is
that `CI` be **unset**: with `CI=true`, `reuseExistingServer:false` makes Playwright refuse the
occupied port outright. The earlier "the suite cannot run" conclusion was wrong, and the
workaround that replaced it (pointing at the stale dev server) is what produced failures in
`ui/src/routes/status.tsx`, a file this run never touched.

**§33.2 — §31.1's guard passed every substantive assertion on first execution.** The card
renders the wire's `f9f9f9f`, `Round 7`, `wire-only` and `changed_since` where a correct local
derivation of **S1**/**I2**/**S3** would produce `b2b2b2b`, round 1, `reviewer1` and `[]`. So
**§26.6 holds in behaviour, not merely by inspection** — the UI renders the projection and does
not re-derive it. All **27** archive specs pass.

**§33.3 — execution found two defects in the guard's own instrumentation that no static review
could have caught.** Both were in the test, not the product:

- The `/api/files/content` intercept was registered **before** `setupRoutes`. Playwright runs the
  most recently added matching handler first, so the `/api/**` catch-all shadowed it and the
  handler never fired — the assertion compared `null`. Fixed by ordering it after, which is what
  `flatten.spec.ts`'s long-standing preview test already does.
- The preview modal was left open, and Mantine's portal overlay swallowed the next click on
  Generate Archive. Fixed with an explicit dismiss.

Recorded because the pattern generalises: an unexecuted test is not weak evidence of
correctness, it is **no evidence** — both failures were in the assertions' plumbing, and either
one would have reported a false pass had the plumbing merely been *absent* instead of wrong.

**§33.4 — `tests/status/issue-detail-modal.spec.ts:129` is a load-dependent flake, not a
regression.** It failed three times in one earlier parallel run and passes in isolation and in
a full run. This run does not touch that spec. Its fixture (`multiCommitStatus`, in the
unmodified `tests/fixtures/index.ts`) *does* transitively consume `segmentFields()` from the
modified `rounds.ts`, so the dependency was real and worth checking — it just is not the cause.
