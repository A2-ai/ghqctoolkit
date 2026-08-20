# Record under Round Semantics — Spec v1

Status: **agreed, not implemented.** Branch `rounds`.

This spec is the **authority** for the record rework. Implementers follow it and do not
redesign. Where an implementation review changes a decision, append a new numbered
section (`## §7 Resolutions from implementation review`) saying which earlier clause it
supersedes — do not silently edit earlier text.

Read alongside `design/segment-model.md` (the `IssueThread` → `Vec<Segment>` model) and
`design/archive-rounds.md`.

**Scope is deliberately minimal.** A larger rework — per-round sections, comments
partitioned by round, per-round timelines, gap/drift reporting — was drafted and
**rejected by the author** as more change than wanted. `§5` lists what was rejected so a
later agent does not reintroduce it as an "improvement". The record keeps its current
shape and feel; only the facts that are *wrong* under the segment model are corrected,
plus one additive column the author asked for.

---

## §0 Diagnosis — why this exists

Record was written for a thread that was *one issue, one branch, two interesting
commits*. The segment model removed all three assumptions. Three fields still encode
them:

- **§0.1 The checklist summary ignores every round ≥ 2.** `src/record/mod.rs:325` calls
  `analyze_issue_checklists(issue.body)`. Round ≥ 2 checklists live in the round comment
  (`ChecklistSource::Comment { comment_index }`), so a re-QC'd file reports Round 1's
  completion forever — and drives the milestone table's red **C** flag from a stale
  checklist. This is a live bug, not a design gap.
- **§0.2 `Latest QC Commit` compresses two independent facts.**
  `src/record/mod.rs:397` — `last_approved_commit().or(latest_commit())`. The reader
  cannot tell which they got. Approved-R1/open-R2 prints R1's approval commit under
  "Latest QC Commit" and never mentions that R2 is open. Same shape as
  `archive-rounds` §0.1: *were these bytes approved* (permanent) and *what is the newest
  QC state* (perishable) are one field.
- **§0.3 `Git Status` mixes branches.** `src/record/mod.rs:331` intersects the local
  checkout's ahead/behind with `file_commits()`, which flattens commits across **all**
  segments — and segments no longer share a branch (segment-spec **M2** removed
  `IssueThread.branch`). A commit belonging to an older round's branch can report
  "Local commits" for the current one.

Two further facts are simply dropped: the round number (nothing in the PDF says how many
rounds a file took), and `UnplaceableReason::describe()` — record prints a bare
`Unknown` where the reason is known and already rendered by the CLI, the repair report
and the API.

**Not bugs — verified, do not "fix".** The milestone table's `U` flag
(`tables.rs:29`, `!qc_status.contains("Approved")`) and `C` flag
(`!checklist_summary.contains("100.0%")`) are string-sniffing and fragile, but correct:
no `QCStatus` display string contains `Approved` except the two approved ones
("Approval required" contains *Approval*, not *Approved*), and `ChecklistSummary`'s
`Display` emits `100.0%`. Rewriting them onto `QCStatus::is_approved()` /
`ChecklistSummary::is_complete()` is optional cleanliness, out of scope here.

---

## §1 Decisions (D)

| ID | Decision |
|---|---|
| **D1** | Record is **all-rounds** and is never pinned to a round. This is what distinguishes it from archive (`archive-rounds` **D7/D8**). No round selector is added to `RecordRequest` or the CLI. |
| **D2** | The record's **shape is preserved**: same sections, same bullet list, same flat comment dump, same tables. Changes are confined to the value of existing fields, one added column, and one added field. |
| **D3** | The milestone summary table reports the **current round's** status. `QCStatus::determine_status` already does exactly this — **no code change to the status cell**. |
| **D4** | Everything a round-aware field needs is read from the segment model. No new derivation of round scope; use the existing accessors. |
| **D5** | Where a fact is unavailable, the record prints `Unknown` **with its reason** (`UnplaceableReason::describe()`) — never blank, never a plausible substitute. Extends the existing `UNKNOWN` const's stated intent. |
| **D6** | Round numbering and unplaceable-reason wording are shared with archive metadata, so the PDF and `ghqc_archive_metadata.json` cannot tell different stories about one issue. |
| **D7** | `Git Status` **stays**, including the working-tree dirtiness note — the client asked for it. It is rebased onto the current round's branch (**R4**). The objection that it is a property of the generating machine (segment-spec **D8**) is noted and deliberately **not** acted on. |

---

## §2 Changes (R)

### R1 — Checklist summary derives from the current round

**Site:** `src/record/mod.rs:325`.

The **current round** is the last `Round` in the thread: `thread.rounds().next_back()`.
This is the last round whether or not the active segment is a trailing gap — an approved
file's summary is the checklist it was approved against.

| `round.checklist` | Text analysed |
|---|---|
| `ChecklistSource::IssueBody` | `issue.body` — today's behaviour |
| `ChecklistSource::Comment { comment_index }` | `comments[comment_index].body` |

Then `analyze_issue_checklists` and `ChecklistSummary::sum` exactly as today: all
checklists **within that one round's source** are summed. Summing across rounds is
**rejected** — it double-counts items re-checked each round and makes the **C** flag mean
"some round somewhere was incomplete."

`checklist_summary` stays a single `String`. No type change, no template change.

When there is no round to read (a thread with no placed rounds), the summary is `UNKNOWN`.

### R2 — `Latest QC Commit` is qualified in place

**Site:** `src/record/mod.rs:397`.

Same field, same bullet, same position in the template. The string states which fact it
is reporting:

| Source | Rendered |
|---|---|
| `last_approved_commit()` | `abc1234 (approved, round N)` — `N` = the round that closed on it |
| `latest_commit()` fallback | `abc1234 (latest commit, round N open)` |
| neither | `Unknown` — with reason per **D5**/**R5** |

Renaming the bullet to *Approved Commit* plus a conditional *Open Round* bullet was
considered and **rejected**: it changes the template, and **D2** keeps template churn at
zero here.

`Initial QC Commit` is unchanged — it is a genuine permanent anchor.

### R3 — Round number in the milestone summary table

Add one field:

```rust
pub struct IssueInformation {
    // ...
    pub current_round: u32,   // thread.rounds().next_back().index
}
```

**Sites:** `src/record/mod.rs` (`IssueInformation`, `create_issue_information`),
`src/record/tables.rs:169` (`render_issue_summary_table_rows`),
`src/templates/record.typ` (issue-summary table header and `columns`).

Five columns become six: `File Path | QC Status | Round | Author | QCer | Issue Closer`.
Widths need a nudge — drop `File Path` from `1.7fr` to about `1.5fr` and give `Round`
about `0.4fr`. The column is numeric and narrow; do **not** run it through
`insert_breaks`.

The **status cell is not touched** (**D3**).

### R4 — `Git Status` is scoped to the current round's branch

**Site:** `src/record/mod.rs:331`.

Replace `issue_thread.file_commits()` — which flattens across all segments, hence across
branches — with the file-changing commits of **every segment whose `branch` equals
`thread.active_branch()`**.

That set is the current round *plus its trailing gap*, deliberately: using the last
`Round` alone would miss post-approval commits, which is precisely the case the client's
"Local commits" warning exists to surface.

Everything else is unchanged: `GitState::format_for_file` as-is, and the
`(file has uncommitted local changes)` suffix as-is (**D7**).

When the active segment is unplaceable, its commits are empty and `format_for_file`
would report a confident "Up to date". Print `Unknown` with the reason instead
(**D5**).

### R5 — Unplaceable reasons are rendered

`UnplaceableReason::describe()` already exists and is the one string the CLI, the repair
per-step report and the API's `skipped_reason` all render. Record appends it wherever it
prints `UNKNOWN`:

```
Unknown — branch 'feature/x' not available locally
```

Applies to `qc_status`, `latest_qc_commit`, `initial_qc_commit`, `checklist_summary`
(when there is no round to read) and `git_status` (**R4**). No new field: the reason is
appended to the existing string.

---

## §3 Invariants (I)

- **I1** No field of `IssueInformation` is derived from `issue.body` where the current
  round's `ChecklistSource` says otherwise (**R1**).
- **I2** `latest_qc_commit` never renders a bare hash — it is always qualified with
  which fact it reports, or is `Unknown` with a reason (**R2**).
- **I3** No commit from a segment on a branch other than `active_branch()` contributes to
  `git_status` (**R4**).
- **I4** Every `Unknown` in the rendered record carries a reason when one is derivable
  (**D5**).
- **I5** Every string added or changed is `escape_typst`'d exactly once. `body` and
  comment bodies go through `format_markdown` and are **not** additionally escaped —
  the qualified strings of **R2**/**R5** are plain text and **are** escaped.
- **I6** The record's section structure, bullet order and comment rendering are
  byte-identical to today except for the values changed here and the column added by
  **R3** (**D2**).

---

## §4 Phases (P)

- **P1** — `src/record/mod.rs`: **R1**, **R2**, **R4**, **R5**, and the
  `current_round` field. Unit tests per change; a multi-round fixture where Round 1's
  checklist is complete and Round 2's is not, pinning that the summary follows Round 2.
- **P2** — `src/record/tables.rs` + `src/templates/record.typ`: the **R3** column and
  widths. Update the record snapshot tests.
- **P3** — `CHANGELOG.md`. No CLI or API change (**D1**): `RecordRequest` gains no field.

`IssueInformation` gains one field and changes no field's type, so a user-supplied
`record.typ` at `configuration.record_path()` keeps rendering — it simply will not show
the new column until updated. No hard swap, no deprecation window needed.

---

## §5 Rejected — do not reintroduce

Drafted, reviewed, and cut by the author as more change than wanted. Listed so a later
agent does not re-propose them as improvements:

- `rounds: Vec<RoundInformation>` on `IssueInformation`, and per-round PDF sections.
- Partitioning comments by round via `comment_index` (the mapping exists; the grouping
  is still out of scope).
- Per-round timelines replacing the flat `Detailed Timeline`; dropping that section.
- Gap/drift reporting — "commits no round reviewed" as a rendered fact.
- An anomalies block from `IssueThread.anomalies`.
- Splitting viewer/checkout state into a labelled "generation environment" block, or
  removing `Git Status` from the audit body (**D7** keeps it).
- Reassembling `(i/N)` split notification comments.
- Rewriting the `U`/`C` table flags off string matching — verified correct today (**§0**).

---

## §6 Resolved questions

| # | Question | Answer | Who |
|---|---|---|---|
| 1 | Scope: full per-round rework, or minimal correctness? | **Minimal.** Keep the record's current shape and feel; fix only what the segment model made wrong. | author |
| 2 | Does record ever pin to one round? | **No** — all-rounds always (**D1**). | author |
| 3 | Summary table status: current round, or aggregate? | **Current round's status** (**D3**). | author |
| 4 | Round number in the table? | **Yes**, add it (**R3**). | author |
| 5 | **R1** — current round's checklist only, or all rounds summed? | **Current round only.** | author |
| 6 | **R2** — qualified string, or renamed bullet plus a new one? | **Qualified string**, no template change. | author |
| 7 | **R4** — drop `Git Status`, relabel it, or keep it? | **Keep it** — a client requirement — but derive it from the current round's branch, not a thread-wide commit set. | author |
| 8 | **R5** — render unplaceable reasons? | **Yes.** | author |

## Still open

None. The frontier is empty; **P1** may start.
