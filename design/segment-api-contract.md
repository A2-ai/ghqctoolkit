# Segment Model — API Wire Contract (P3)

Status: **pinned.** Branch `rounds`. Authority for the JSON exchanged between the
Rust API and the UI while `design/segment-model.md` §7 (A1–A6) and §8 are
implemented. The spec is the design authority; this document is the wire authority
derived from it. Where the spec was silent, the choice is marked
**[implementer's choice]** with its reasoning.

Files pinned by this contract, already updated to match it:

- `openapi/openapi.yml` — schemas
- `ui/src/api/issues.ts`, `ui/src/api/rounds.ts` — TypeScript declarations only

Not yet matching it (deliberately, per **D11** — no intermediate green states):
`src/api/types/responses.rs`, `src/api/routes/*`, and every UI component listed in
[§9](#9-known-consumers-to-migrate).

---

## 1 Tagging decisions

| Type | Representation | Serde attribute |
|---|---|---|
| `Segment` | **internally tagged** on `kind`, values `round` / `gap` | `#[serde(tag = "kind", rename_all = "snake_case")]` |
| `GapContinuity` | **internally tagged** on `kind`, values `linear` / `diverged` / `unrelated` | `#[serde(tag = "kind", rename_all = "snake_case")]` |
| `Placement` (API projection) | **internally tagged** on `kind`, values `placed` / `unplaceable` | `#[serde(tag = "kind", rename_all = "snake_case")]` |
| `UnplaceableReason` | plain string enum | `#[serde(rename_all = "snake_case")]` |

**Why internally tagged, not adjacently tagged.** §7 A1 makes the UI test
`segments.at(-1).kind === 'round'`. Internal tagging puts `kind` *alongside* the
variant's own fields (`{ "kind": "round", "index": 1, … }`), which is exactly what
that expression reads and exactly what a TypeScript discriminated union consumes
with no unwrapping. Adjacent tagging (`tag = "kind", content = "data"`) would also
satisfy the `kind` test but nests every field under `data`, forcing the UI to
destructure twice and making the OpenAPI `discriminator` awkward. Internal tagging
also matches the convention already in this codebase: `RoundChecklistSource` and
`GitStatus` both put a `kind`/`status` discriminant next to flat sibling fields.

Serde caveats the Rust implementer must respect:

1. Internally-tagged **newtype** variants only serialize when the inner type
   serializes as a *map*. `Segment::Round(Round)` and `Segment::Gap(Gap)` are fine
   because both wrap structs. This is why the API-side segment types must be
   structs, never tuples or primitives. Note that this constraint is enforced at
   **runtime, not compile time**: an internally-tagged newtype variant wrapping a
   non-map type compiles and fails when serialized, with e.g. `"cannot serialize
   tagged newtype variant … containing an integer"`. There is no type-level
   protection; only a test that actually serializes the value will catch it.
2. `Placement::Unplaceable(UnplaceableReason)` from **M5** is a newtype variant
   wrapping a *string* enum. Serde does **not** reject this — and that is the trap.
   It compiles **and serializes**, silently emitting the **wrong shape**: the
   reason becomes a *key* with a `null` value.

   ```jsonc
   // what #[serde(tag = "kind")] on Placement::Unplaceable(UnplaceableReason)
   // actually produces — verified, no error:
   { "kind": "unplaceable", "branch_unavailable": null }
   ```

   A unit-variant inner enum serializes as a string, and internal tagging of a
   newtype variant merges the inner value's *map* into the tagged object; a string
   is coerced into a single key with a null value rather than erroring. So this
   failure mode is not caught by the compiler and not caught by
   serialization — only by asserting on the emitted JSON.

   > **Do not derive `Serialize` on the domain `Placement` type.** The absence of
   > that derive today is the only thing keeping this trap closed. `Placement` is a
   > domain type (`M5`); it reaches the wire exclusively through the `PlacementInfo`
   > projection below. Adding `#[derive(Serialize)]` to the domain enum — even
   > "just for a test fixture" — would create a second, wrong encoding of the same
   > fact that nothing would flag.

   The domain type keeps M5's shape; the API projection restructures it to a struct
   variant, which serializes correctly:

   ```rust
   #[derive(Debug, Clone, Serialize)]
   #[serde(tag = "kind", rename_all = "snake_case")]
   pub enum PlacementInfo {
       Placed,
       Unplaceable { reason: UnplaceableReasonEnum },
   }
   ```

   **[implementer's choice]** — the spec does not say how `Placement` reaches the
   wire. This shape was chosen over `{ kind, reason: null }` so `reason` is
   *absent*, not null, when placed, which lets TS narrow on `kind` alone.
3. Internally-tagged enums buffer during deserialization. These types are
   serialize-only on the API side today (`#[derive(Serialize)]`), matching the rest
   of `src/api/types/responses.rs`; add `Deserialize` only if a test fixture needs
   it.

---

## 2 `IssueStatusResponse`

```jsonc
{
  "issue": { /* unchanged */ },
  "qc_status": { /* §3 */ },
  "dirty": false,
  "active_branch": "feature/x",
  "checklist_summary": { "completed": 3, "total": 10, "percentage": 0.3 },
  "blocking_qc_status": { /* unchanged */ },
  "segments": [ /* §4 — oldest first */ ],
  "next_notification_from": "abc…",
  "round_repair": null
}
```

| Field | Type | Null? | Notes |
|---|---|---|---|
| `issue` | `Issue` | no | unchanged |
| `qc_status` | `QCStatus` | no | see §3 |
| `dirty` | bool | no | unchanged |
| `active_branch` | string | no | **A2.** The last segment's branch — the branch status was computed on. Never the viewer's checkout (**D8**). Replaces `branch`, which was the issue body's branch. |
| `checklist_summary` | `ChecklistSummary` | no | unchanged |
| `blocking_qc_status` | `BlockingQCStatus` | no | **Corrected:** always present. The Rust field is a plain struct, not an `Option`, and carries no `skip_serializing_if`, so the key is emitted on every response and is never null. It is now in `required`, and the TS `?` is dropped. An earlier revision of this document called it "the one genuinely optional key". |
| `segments` | `Segment[]` | no | **A1.** Replaces **both** `rounds` and top-level `commits`. |
| `next_notification_from` | string (sha) | **yes** | unchanged field, new derivation (**M8**). Null when the active segment cannot supply one: an `Unplaceable` active segment owns no commits (**S4**/**I5**), so there is neither a newest event commit nor a standing approval to fall back to. |
| `round_repair` | `RoundRepairStatus` | yes | unchanged |

**"Null?" means null, not absent.** Serde emits `Option<T>` as an explicit `null`
key, so a nullable field is still **always present**, and belongs in `openapi.yml`'s
`required` list with `nullable: true`. This is the rule that makes the two documents
agree, and the `required` lists in `openapi.yml` are written to it.

> **Correction — this paragraph previously claimed there is "no `skip_serializing_if`
> anywhere in `responses.rs`", and that `blocking_qc_status` is "the one genuinely
> optional key". Both were false.** `responses.rs` has **five**
> `skip_serializing_if = "Option::is_none"` fields, three of which predate this
> refactor: `BlockingQCError.file_name`, `BlockingQCError.branch`,
> `IssueStatusError.branch`, `StepOutcomeResponse.error` and
> `StepOutcomeResponse.skipped_reason`. And `blocking_qc_status` was never optional at
> all — its Rust field is a plain (non-`Option`) struct with no `skip_serializing_if`,
> so the key is always present and never null.
>
> The rule, correctly stated: **`responses.rs` uses `skip_serializing_if` only where
> absence is itself the fact** — the five fields above, where "there is no error" /
> "there was no reason" / "the branch could not be read" is expressed by the key being
> missing. Every other `Option<T>` is emitted as an explicit `null`, is in `required`,
> and is `nullable: true`. Those five are the only fields that are `?`-optional in TS
> and absent from `required`; `blocking_qc_status` is neither.

**Ordering and invariants the UI may rely on** (from §3 I1–I5):

- `segments` is **oldest first**. `segments[0]` is always `kind: "round"` with
  `index: 1` (Initial QC).
- Strict alternation: round, gap, round, gap … Empty gaps are present, not omitted.
- The last element is either an open round or a gap — never a closed round.
  Therefore `segments.at(-1).kind === 'round'` ⟺ a round is open, which is what
  replaces the deleted `open_round_index`.
- Round indices are contiguous and ascending; the *n*-th round segment has
  `index === n`.
- Each segment's `commits` is **newest first**. Concatenating the segments'
  `commits` from the last segment backwards reproduces the old top-level
  `commits` array, except that a boundary commit may appear in two adjacent
  segments (**D1**).
- An `unplaceable` segment has `commits: []`.

### Positional indexing — read this before writing any `pos ± n`

Segments are **oldest first** and strictly alternating, so positions run
`0 = R1, 1 = Gap, 2 = R2, 3 = Gap, 4 = R3, …`. Round *n* is at position
`2 * (n - 1)`; every odd position is a Gap. Therefore:

> **Round at position `p`: the previous Round is at `p - 2`.**
> **Gap at position `p`: its bounding older Round is at `p - 1`, its bounding newer
> Round (if any) at `p + 1`.**

**M8**'s `previous_approval_of(pos)` takes a **Round** position and reads `pos - 2`.
Its documented description — "the closing commit of the round segment two positions
earlier" — is correct **only** for Round positions. Passing a Gap's position lands on
another Gap, which has no closing commit, so the result is `None` for every input.

This is not hypothetical: it already caused a live bug in the Rust implementation,
where a Gap's own position was handed to `previous_approval_of`, making
`standing_approval()` unconditionally `None` and so silently breaking every consumer
of `qc_status.standing_approval`. What a Gap actually wants is the **closing commit of
the Round at `p - 1`** — not `previous_approval_of` of anything. `standing_approval()`
already encodes exactly that; reach for it rather than doing the arithmetic again.

**Not on the wire.** `IssueThread.anomalies` (**M7**) is deliberately **not**
projected: §7 never mentions it, and W6's "diagnostic message" is unspecified.
**[implementer's choice / open]** — if the rail wants to render W6's diagnostic,
that is a follow-up shape change, not part of this contract.

---

## 3 `QCStatus`

```jsonc
{
  "status": "changes_after_approval",
  "status_detail": "Approved but the file changed since",
  "standing_approval": "aaa…",
  "last_approved_commit": "aaa…",
  "initial_commit": "111…",
  "latest_commit": "eee…",
  "changed_commit": "ddd…"
}
```

**`changed_commit` is not `latest_commit`, and that is the whole reason it exists.**
**S1** selects the trailing Gap's newest commit *that touched the file*, because drift
that never touched it is not something a reviewer is asked to comment on. So for a Gap
`[X(file_changed: false), Y(file_changed: true)]` the status is
`ChangesAfterApproval(Y)` while `latest_commit` is `X`. The enum projection
(`From<crate::QCStatus> for QCStatusEnum`) discards the sha the status carries, which
left a client rendering a "Changed" row from `latest_commit` naming a commit that never
touched the file — the backend-knows/frontend-guesses split **D12** exists to close, and
exactly why **D12**'s two event fields were added. `SwimLanes.tsx`'s "Changed" row reads
this field.

| Field | Type | Null? | Notes |
|---|---|---|---|
| `status` | enum, **8 values** | no | `approved`, `changes_after_approval`, `awaiting_review`, `change_requested`, `in_progress`, `approval_required`, `changes_to_comment`, **`unknown`** |

> **`unknown` was added after implementation review; this row previously said "7 values,
> unchanged".** `determine_status` returns `Option<QCStatus>`, and `None` — an
> `Unplaceable` active segment (**S4**) — had been mapped onto `in_progress`. That is a
> semantic lie: the pill read *In Progress* for an issue about which nothing can be
> asserted, while the card was simultaneously grayed, so one response carried two
> disagreeing accounts of one fact — the `§0` defect. `None` now maps to `unknown`.
>
> `unknown` and `in_progress` are **distinct and must not be collapsed**. `in_progress`
> asserts the round is open, placed and understood (spec **S2** row 2: it owns commits,
> none file-changing, and announced nothing). `unknown` asserts nothing at all. A client
> seeing `unknown` should render an **absence**, not an activity, and read
> `segments.at(-1).placement.reason` for the cause. An earlier plan to delete
> `in_progress` and keep the count at seven was withdrawn once **S2** row 2 was shown
> reachable.
| `status_detail` | string | no | unchanged |
| `standing_approval` | string (sha) | **yes** | **A3.** `IssueThread::standing_approval()`: the previous round's closing commit when the last segment is a Gap; `null` when the last segment is a Round — i.e. `null` exactly while the file is back under review. |
| `last_approved_commit` | string (sha) | **yes** | **A3.** `last_approved_commit()`: newest closing commit across all rounds, ungated. `null` when nothing was ever approved. |
| `initial_commit` | string (sha) | **yes** | Round 1's anchor. **[implementer's choice]** kept — A3 does not mention it and nothing suggests removing it. Null when Round 1 is `Unplaceable`: such a segment owns no commits (**S4**/**I5**), so its anchor cannot be resolved to a sha. |
| `latest_commit` | string (sha) | **yes** | newest commit of the **active segment** (**M8**), not of the thread. Null whenever that segment owns no commits — and that is the *normal* approved state, not an edge case (see below). |
| `changed_commit` | string (sha) | **yes** | **Added.** The newest **file-changing** commit of the trailing Gap — the sha `QCStatus::ChangesAfterApproval` carries. `null` in every other state, including `approved`. |

**All four shas are nullable.** `initial_commit` and `latest_commit` were previously
pinned non-nullable in this document; that was **wrong**, and the two nullable ones
are not the only ones. Per **D6**/**S1** the steady approved state is
`[…, Round(Closed), Gap(empty)]` — a *legal and expected* empty trailing Gap, which
has no newest commit, so `latest_commit` is null for every fully-approved issue with
no drift since. Independently, an `Unplaceable` active segment owns no commits at all
(**S4**/**I5**), which nulls `latest_commit` and — when it is Round 1 —
`initial_commit`. The same reasoning nulls top-level `next_notification_from` (§2).
Consumers must handle null on all four; a non-null assertion here is a bug.

`approved_commit` is **REMOVED**. Both nullable replacements can be null on the
same response (nothing ever approved); `standing_approval` is null whenever a round
is open even though `last_approved_commit` is not — that asymmetry is the entire
point of the split.

**Migration rule for the eight ungated `approved_commit ?? latest_commit` sites**
(`FileResolveModal.tsx:184,185,190,323,471` — four uses — and `ArchiveTab.tsx:374,
387,684,808`; §9's table counts them as `×4` per file): the mechanically equivalent
expression is `last_approved_commit ?? latest_commit`.

**This is verified behaviour-preserving at all eight sites, not a judgement call.**
The spec's **A3** says `approved_commit` "silently meant both" `standing_approval`
and `last_approved_commit`. On the wire that is imprecise. `responses.rs:151` built
the field as `issue.approved_commit()`, and that accessor (`src/issue.rs:360`) was an
**ungated** newest-first scan — i.e. exactly **M8**'s `last_approved_commit()`. The
open-round gate that made `approved_commit` *look* like `standing_approval` existed
only as a **local variable inside `determine_status`** (`src/qc_status.rs:60–64`) and
never reached the wire. So the ambiguity A3 describes was in the *name and the
codebase's two readings of it*, never in the emitted JSON: every consumer of
`approved_commit` was reading `last_approved_commit`. The migration rule is therefore
a rename, not a coin-flip, and `standing_approval` is a genuinely **new** fact on the
wire. (Line references are to the pre-**P3** code, before the migration edits them.)

A3 says the *archive UX* (offering the user a choice between the two) is a
separate, unsettled discussion; do not design it here.

---

## 4 `Segment`

`Segment = RoundSegment | GapSegment`, discriminated on `kind`.

### 4.1 `RoundSegment`

```jsonc
{
  "kind": "round",
  "index": 2,
  "name": "Round 2",
  "opened_at": "ddd…",
  "branch": "feature/x",
  "opened": {
    "kind": "new_round",
    "comment_id": 12345,
    "comment_url": "https://github.com/o/r/issues/7#issuecomment-12345",
    "author": "alice",
    "at": "2026-08-01T10:00:00Z",
    "note": "re-QC after refactor"
  },
  "checklist_source": { "kind": "comment", "comment_id": 12345, "comment_url": "…" },
  "checklist_name": "Code Review",
  "state": "open",
  "closing_commit": null,
  "closed_by": null,
  "closed_at": null,
  "events": [ { "kind": "notification", "commit": "ddd…", "by": "alice",
                "at": "2026-08-01T10:00:01Z", "comment_id": 12346,
                "comment_url": "…" } ],
  "retractions": [],
  "extensions": [],
  "commits": [ /* IssueCommit[], newest first */ ],
  "placement": { "kind": "placed" }
}
```

| Field | Type | Null? | From | Notes |
|---|---|---|---|---|
| `kind` | `"round"` | no | tag | |
| `index` | int | no | M3 | 1-based; 1 is Initial QC |
| `name` | string | no | accessor | `"Initial QC"` or `"Round N"`. **[implementer's choice]** kept — §7 does not list it, but U2 renders round labels and Q10 quotes *"N commits between Round 1 and Round 2"*, so the label belongs server-side per **D7**. |
| `opened_at` | string (sha) | **yes — was non-nullable** | M3 | the anchor. **Owned by this round, not by the gap before it** (**D10**). **Corrected:** `null` when the round is `Unplaceable`. The fold leaves an unresolved anchor as the **all-zero OID**, and this field was previously pinned non-nullable, so that placeholder reached the wire as a real-looking sha. Every other unresolvable sha on this response is already nullable (§3), and a fake sha on an audit tool's wire is a trap for the next consumer, so it is nulled at the source. The UI's existing guards (`shortHash` renders `—` for the all-zero sha; the rail suppresses the anchor of an unplaceable round) are kept as belt-and-braces. |
| `branch` | string | **no — was nullable** | M3 / **D5** | always declared. A round comment with no `git branch` is malformed: the branch is still a string — **the empty string**, not a substituted fallback — and the *failure* is reported as `placement.reason == "branch_not_declared"`. Nullability is gone deliberately: it is what made the card-graying bug representable. |
| `opened` | `RoundOpenInfo` | no | M3 | see §4.3 |
| `checklist_source` | `RoundChecklistSource` | no | M3 `checklist` | **[implementer's choice]** the wire keeps the existing name `checklist_source` even though M3's Rust field is `checklist`. Renaming would churn `RoundRail.tsx` and the Playwright fixtures for no behavioural gain, and the API type has always been named `RoundChecklistSource`. Shape unchanged: `{ kind: "issue_body" \| "comment", comment_id, comment_url }`. |
| `checklist_name` | string | yes | M3 | unchanged |
| `state` | `"open" \| "closed"` | no | M3 | **[implementer's choice]** the closing details stay **flattened** into the three `closing_*`/`closed_*` fields, as today, rather than becoming a nested tagged `RoundState`. §7 is silent; this keeps `RoundRail`'s `round.closing_commit` working and matches the existing `RoundStateEnum` + flat-fields convention documented in `responses.rs`. |
| `closing_commit` | string (sha) | yes | M3 | non-null **iff** `state == "closed"` |
| `closed_by` | string | yes | M3 | non-null iff `state == "closed"` |
| `closed_at` | RFC 3339 string | yes | M3 | non-null iff `state == "closed"` |
| `events` | `RoundEventInfo[]` | no | M3 | **oldest first.** Replaces `event_count`. |
| `retractions` | `RetractionInfo[]` | no | M3 | oldest first. Replaces `retraction_count`. |
| `extensions` | `ExtensionInfo[]` | no | M3 | oldest first. Replaces `extension_count`. |
| `commits` | `IssueCommit[]` | no | M3 | **newest first**, walked on this round's own `branch`: closing commit (or branch tip while open) back to `opened_at`, inclusive (**W2**). `[]` when unplaceable. |
| `placement` | `Placement` | no | M5 | see §4.4 |

`previous_approval` is **REMOVED** from the round shape (**M3**/**Q1**: it became
`IssueThread::previous_approval_of(pos)`). A consumer that needs it reads the
closing commit of the segment at `pos - 2`, where `pos` is **this Round's** position
(see [Positional indexing](#positional-indexing--read-this-before-writing-any-pos--n)
— `pos - 2` is only a Round for Round positions), or — for the round the UI is
actually rendering — `qc_status.standing_approval`.

The three `*_count` fields are **REMOVED**: the spec's A5/M3 list projects the
lists themselves, and a count plus its list on the same object is redundant data
that can disagree. Consumers use `events.length` etc.

### 4.2 `GapSegment`

```jsonc
{
  "kind": "gap",
  "branch": "feature/x",
  "commits": [ /* IssueCommit[], newest first; may be [] */ ],
  "continuity": { "kind": "diverged", "merge_base": "bbb…" },
  "lower_bound": "bbb…",
  "upper_bound": "ddd…",
  "placement": { "kind": "placed" }
}
```

| Field | Type | Null? | Notes |
|---|---|---|---|
| `kind` | `"gap"` | no | |
| `branch` | string | no | **W1**: the branch of the round bounding the gap's **newer** end; for a trailing gap, of the round bounding its older end (**D3**). Never the viewer's checkout. |
| `commits` | `IssueCommit[]` | no | newest first. Empty is normal (**D6**) as well as the unplaceable case. |
| `continuity` | `GapContinuity` | no | see §4.5. Divergence is a gap property, not a special case (**W5**). |
| `lower_bound` | string (sha) | **yes** | **A5**, derived: the **older** bound — the previous round's closing commit, *exclusive*. `null` when unresolvable (typically `neighbour_unplaceable`). |
| `upper_bound` | string (sha) | **yes** | **A5**, derived: the **newer** bound — the next round's `opened_at`, *exclusive*; for a trailing gap, the branch tip, *inclusive*. `null` when unresolvable. |
| `placement` | `Placement` | no | see §4.4 |

**A Gap that is itself `unplaceable` reports `lower_bound: null` and
`upper_bound: null` — both, unconditionally.** The projection filters the *neighbours*
on `is_placed()`, which nulls a bound whose bounding Round could not be placed (**W6**);
it must **also** consult the Gap's **own** placement. A Gap can be `unplaceable` while
both neighbours are placed and both bounds therefore resolve — reachable through
`build_gap`'s `BranchUnavailable`, `AnchorUnreachable` and `MergeBaseUnreachable`
paths — and reporting real bounds there hands **U1** a detached handle to draw across a
segment it is simultaneously graying. **W6**'s rule is that plausible-but-wrong is worse
than nothing. Note this makes the neighbour filter redundant for every shape the fold
emits, since `build_gap` marks a Gap beside an unplaceable Round as
`neighbour_unplaceable`; the per-neighbour rule is kept as a guard.

**[implementer's choice]** the names `lower_bound`/`upper_bound` come from A5 and
are ordered by **commit ancestry**, not by array position: `lower_bound` is the
older commit, `upper_bound` the newer. Their bounds are the same commits as W2's
table, which is why they are the right handles for U1's detached previous-approval
handle. Nullability is not stated in A5; it is required because W6 gives gaps whose
neighbour is unplaceable, which have no resolvable bound.

A gap carries **no** `index`, `name`, or id — Gaps are addressed positionally
(**Q6**, **Q10**).

### 4.3 `RoundOpenInfo`

```jsonc
{ "kind": "issue_created", "comment_id": null, "comment_url": null,
  "author": null, "at": null, "note": null }
```

`kind` is `"issue_created"` (Initial QC) or `"new_round"`. Every other field is
nullable and is `null` for `issue_created`; `comment_id`/`comment_url` are also
`null` for comments loaded from the disk cache, which carry no identity — the same
rule already documented on `RoundChecklistSource`.

**[implementer's choice]** — §7 lists `opened` but not its projected fields.
`comment_index` (a fold-internal position into the comment slice) is deliberately
**not** exposed; `author`/`at`/`note` are, because they are the only round
provenance the UI has no other route to. Serde: emit as a struct with a
`RoundOpenKind` enum field rather than an internally-tagged enum, so that the six
keys are always present and TS need not narrow to read `comment_url`.

### 4.4 `Placement` / `UnplaceableReason` (M5)

```jsonc
{ "kind": "placed" }
{ "kind": "unplaceable", "reason": "branch_unavailable" }
```

`reason` ∈ `branch_not_declared` | `branch_unavailable` | `anchor_unreachable` |
`merge_base_unreachable` | `neighbour_unplaceable`. `reason` is **absent** when
`kind == "placed"`.

**`merge_base_unreachable` was added after this contract was first pinned** (**M5**). An
off-walk merge-base previously reported `anchor_unreachable`, which names a different
failure: both of the Gap's anchors resolved, and what lies outside the walked range is
their common *ancestor*. Because walks stop at Initial QC this fires for any later Round
whose branch forked before Initial QC — ordinary history, not a force-push — so the old
reason sent the user hunting a commit that is not missing. Independently, a Gap whose
branch yielded an **empty walk** now reports `branch_unavailable` rather than
`anchor_unreachable`.

Every reason's user-facing wording is `UnplaceableReason::describe()`, and the UI's
`unplaceableReasonText` (`ui/src/utils/rounds.ts`) is a **verbatim copy** of it, pinned
by `round::tests::every_reason_wording_matches_the_ui_copy`. The backend's strings are
authoritative because they are on the wire (`skipped_reason`, §6.1) and the CLI prints
them; the two previously disagreed on all four arms, so one view could explain a single
degradation two different ways.

An unplaceable segment owns no commits and renders grayed with its reason, never as
an error (**D4**). `neighbour_unplaceable` marks a gap whose bounding round could
not be placed; per **W6** the API must **not** substitute the other round's branch.

### 4.5 `GapContinuity` (M4)

```jsonc
{ "kind": "linear" }
{ "kind": "diverged", "merge_base": "bbb…" }
{ "kind": "unrelated" }
```

`merge_base` is a **required, non-null** string on `diverged` and **absent** on the
other two variants — that is what makes the TS union ergonomic:
`if (c.kind === 'diverged') use(c.merge_base)` needs no null check. Contrast the
deleted `BranchDivergenceResponse`, whose `merge_base: string | null` encoded
"unrelated" as a null and forced every consumer to handle both.

`GapContinuity` **replaces** `BranchDivergence` (`src/start_round.rs`) and
`BranchDivergenceResponse` (`src/api/types/responses.rs`) — both are deleted.

**`previous_branch` is dropped in the re-type, and needs no replacement field** — it
is recoverable from data the same consumer already holds. See the UI wiring note in
§7.6.

---

## 5 `IssueCommit` — wire shape UNCHANGED (A4)

```jsonc
{
  "hash": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "message": "fix rounding",
  "statuses": ["notification"],
  "file_changed": true
}
```

| Field | Type | Notes |
|---|---|---|
| `hash` | string | full 40-char sha, as today |
| `message` | string | as today |
| `statuses` | `("initial" \| "notification" \| "approved" \| "reviewed")[]` | **same four values, same snake_case, same array-of-strings shape.** May be empty. |
| `file_changed` | bool | as today |

What changed is only the *origin*: `statuses` is now an **API-computed projection**
of the owning segment's `events` and `state` (**A4**, **F7**), not a stored
`HashSet<CommitStatus>` parsed from comments. The mapping the Rust side must
implement:

| Status | Emitted when |
|---|---|
| `initial` | the commit is `segments[0].opened_at` (round 1's anchor) |
| `notification` | some `RoundEvent::Notification` in the owning round names it |
| `reviewed` | some `RoundEvent::Review` in the owning round names it |
| `approved` | it is the owning round's `RoundState::Closed.commit` |

**[implementer's choice — a tightening]** the array is emitted in the fixed order
`initial, notification, approved, reviewed`, deduplicated. The old field came from
a `HashSet` and so had **non-deterministic** order; `RoundCommitPicker.tsx:346`
re-sorts it through `STATUS_ORDER` for exactly that reason. Pinning the order makes
API responses and fixtures byte-stable and lets that re-sort eventually go away. A
consumer that keeps sorting stays correct either way.

Because the top-level `commits` array is gone, `IssueCommit` now appears **only**
inside a segment's `commits`. `IssueThread`'s own `commits` field is removed
(**M2**), and `IssueCommit`'s domain type drops `statuses` (**M6**) — the wire
field survives as a projection.

---

## 6 `RoundSeedResponse` / `StartRoundResponse` (A6)

`RoundSeedResponse` keeps every field it has. The only change:

| Field | Old type | New type |
|---|---|---|
| `divergence` | `BranchDivergenceResponse \| null` | **`GapContinuity \| null`** |

`branch`, `comparison_base`, `previous_approval`, `anchor`, `checklist_*`,
`can_start`, `blocked_reason`, `file`, `next_round`, `next_round_name` are
unchanged — but they are **not** uniformly nullable, and an earlier revision of this
section wrongly said they were. Per field, on `RoundSeedResponse`:

| Nullability | Fields |
|---|---|
| **non-nullable** | `file` (`responses.rs:861 pub file: String`, `rounds.ts:327 file: string`), `next_round`, `next_round_name`, `checklist_options` (empty array, never null), `can_start` |
| nullable | `checklist_content`, `checklist_name`, `default_round`, `anchor`, `previous_approval`, `branch`, `comparison_base`, `divergence`, `blocked_reason` |

Every key is **present** either way (§2's note: serde emits `Option<T>` as an
explicit `null`), so all fourteen are in `openapi.yml`'s `required` list and none is
`?`-optional in TS. Note the contrast with `StartRoundResponse`, where `branch` and
`comparison_base` are **non**-nullable (`responses.rs:731,735` — plain `String`): a
round cannot be opened without resolving both, whereas the seed endpoint is
side-effect free and reports what it could not resolve as `null`.

**Pinned semantics of the null.** `divergence` is `null` when the previous approval
*is* an ancestor of the anchor; it is `{ "kind": "diverged", "merge_base": … }` or
`{ "kind": "unrelated" }` otherwise. **`{ "kind": "linear" }` is never emitted on
these two responses** — `null` already means linear, and emitting both encodings of
the same fact would let them disagree. **[implementer's choice]** — A6 says only
"re-types to `GapContinuity`"; it does not resolve the resulting
`null`-vs-`linear` redundancy.

Because `linear` is unreachable here, these two properties do **not** `$ref` the
shared `GapContinuity` schema in `openapi.yml`. Each declares its own
`oneOf: [GapContinuityDiverged, GapContinuityUnrelated]` with `nullable: true`, so a
generated client is not forced to handle a variant that is never sent. The shared
`GapContinuity` schema keeps all three variants and is used only by a Gap segment's
`continuity` (§4.5), where `linear` **is** emitted and is in fact the normal case.

The TS declarations deliberately keep the wider `GapContinuity | null` on both
fields: a superset is sound for a consumer, the `linear` branch is simply dead, and
splitting the union would cost a second exported type for no call-site benefit. The
never-`linear` guarantee lives in the fields' doc comments
(`rounds.ts:229`, `rounds.ts:364`). This asymmetry is intentional; do not "fix" it by
re-pointing `openapi.yml` at the shared schema.

`StartRoundResponse.divergence` re-types identically. **[implementer's choice]** —
A6 names only `RoundSeedResponse`, but M4 deletes `BranchDivergenceResponse`
outright, so `StartRoundResponse` cannot keep it; see §7.

### 6.1 `StepOutcome.skipped_reason` — added

```jsonc
{ "status": "skipped", "skipped_reason": "its branch is unavailable locally" }
```

| Field | Type | Present? | Notes |
|---|---|---|---|
| `status` | enum `done \| skipped \| failed` | always | unchanged |
| `error` | string | **only on `failed`** | unchanged |
| `skipped_reason` | string | **only on `skipped`, and only when a reason exists** | **new.** Why the step could not be attempted, when that is not simply "there was nothing to do". |

The field exists because a repair no longer refuses an **unplaceable** round. A
notification names the round's own anchor and the approval before it, and an
unplaceable round owns no commits (**I5**) with a possibly null-OID anchor, so that one
step is skipped — while the reopen (needs the issue number) and the body marker (needs
the round's index and comment URL) still run, because neither reads `placement`. A
blanket 409 was a UI dead-end: `round_repair.needs_repair` is `reopen || body_marker`,
so the API offered a repair the endpoint then refused, for
`UnplaceableReason::BranchUnavailable` — the ordinary "branch not fetched locally"
state. **D4** says such a state degrades and grays; a 409 is an error, so the
notification degrades instead.

Only a repair's `notification` carries a reason today. The wording is
`UnplaceableReason::describe` — one string shared with `ghqc issue status`'s trust
marker, so a user never sees the same degradation explained two ways.

**Absent, not null** — one of the fields where **absence is itself the fact**, and it
follows `error`, which this field is modelled on: both are
`#[serde(skip_serializing_if = "Option::is_none")]`, so the key is genuinely missing
rather than present-and-null. (An earlier revision called this "the one exception to
§2's rule"; it is neither the only one nor an exception — see §2's corrected rule, which
lists all five such fields.) §2's "nullable ⇒ present ⇒ `required` + `nullable: true`"
is conditioned on the key being emitted, so neither field is in `openapi.yml`'s
`required` list and both are `?`-optional in TS.

**Two steps can carry a reason, not one.** `body_marker` has always had one on the CLI —
*"round comment URL unknown, marker left as it is"*, printed by `RepairRoundResult::fmt`
— but it was not projected, so API clients saw a bare `skipped` where CLI users got an
explanation. It is now projected through
`RepairRoundResult::body_marker_skip_reason()`, alongside the notification's.

**The reason rides on the outcome, not on `RepairPlan`.** `RepairPlan` is derived from
the issue and the round alone, so it cannot distinguish *could not* from *was never
asked for* — and getting that wrong was a live bug: `can_notify()` never consulted the
requested `NotificationMode`, so an unplaceable round reported *"its branch is
unavailable locally"* **even when the caller requested no notification at all**. Since
every surface defaults to `NotificationMode::None` (`--notification none`, and
`#[serde(default)]` so `body: {}` means none), the ordinary
branch-not-fetched-locally case blamed the user's branch for a post they never requested,
and suppressed the *"Nothing needed repairing."* reassurance that is the command's whole
point. The cause is therefore computed in `repair_round`, where the mode is known:

```rust
pub enum NotificationSkip { NotRequested, AlreadyNotified, Unplaceable(UnplaceableReason) }
```

**Precedence: already-notified → not-requested → unplaceable.** Placement ranks *last*
because it is the only cause the caller did not choose. `skipped_reason` is emitted
**only** for `Unplaceable`: the other two are ordinary outcomes each surface words for
itself, and reporting them would make the default no-op repair look like a failure.

`RoundRepairStatus` (§2's `round_repair`) is **unchanged**: it reports what is
incomplete, and with the notification skipped rather than refused, nothing it reports
can now fail.

> **Known gap — `RoundRepairStatus` does not project `plan.unplaceable`.**
> `RoundRepairStatus::derive` exposes `notification_missing` but not the placement, so a
> client can offer *"post the missing notification"* on a grayed round and get back a
> `skipped` it did not predict. Harmless today: `needs_repair` is `reopen || body_marker`,
> neither of which reads placement, so the affordance the UI actually gates on is
> unaffected, and the skip is now explained by `skipped_reason` rather than being a 409.
> Recorded rather than fixed — projecting it is an additive wire change and needs the
> spec owner.

---

## 7 Spec tensions, wiring notes, and known gaps

1. **`StartRoundResponse.divergence` is unspecified.** A6 names only
   `RoundSeedResponse`, yet M4 deletes the type both fields use. Re-typed both, as
   the only coherent reading.
2. **A5 vs M3 field naming.** A5 says each segment projects "`kind`, `branch`,
   `commits[]`, `placement`"; M3's round has no `kind` (that is the serde tag) and
   its checklist field is `checklist`, while the existing wire name is
   `checklist_source`. Resolved in §4.1 as an implementer's choice.
3. **`RoundState` on the wire.** M3 shows `state: RoundState` as a data-carrying
   enum (`Closed { commit, by, at, .. }`), which under this document's tagging rule
   would become `{ "kind": "closed", "commit": … }`. §7 does not ask for that
   change, and the existing wire already flattens it. Kept flat; if the Rust agent
   prefers the tagged form, that is a wire change and must come back through this
   document.
4. **Anomalies have no wire home.** M7 adds `RoundAnomaly` variants and W6
   specifies a user-visible diagnostic sentence, but §7 never exposes
   `IssueThread.anomalies`. Left off the wire (§2). See §7.7 — exposing them would
   not actually unblock W6.
5. **`IssueStatusResponse.issue.branch` still exists** and is still the issue
   body's branch (`Issue::from`, `responses.rs:116`). A2 renames the *top-level*
   field only. Nothing in the spec deletes `issue.branch`; it is untouched here, but
   note that after this change one response carries both `issue.branch` (body) and
   `active_branch` (live) — the graying decision must use **`active_branch`**.

### 7.6 UI wiring note for the P4 agent — `previous_branch`

**Not a spec tension and not a blocking question.** `BranchDivergenceResponse`
carried `previous_branch` (the branch the previous round was reviewed on);
`GapContinuity` has none, and this contract drops it per M4. No replacement field is
needed, because the fact is already on the wire:

> `previous_branch` = `segments[i].branch` of the **last closed Round** on the
> `IssueStatusResponse` — i.e. the newest `kind: "round"` segment with
> `state: "closed"`. Every Round declares a branch unconditionally (**D5**), so this
> always resolves.

That response is one the same UI already holds, and `StartRoundModal` renders
alongside it, so this is a **join at render time**, not a shape change and not a
second request. It is exactly the pattern **A2**/**Q9** already established for the
card-graying compare, so it does not violate **D7**.

Concretely for the P4 agent — current users of the removed field:

| Site | What to do |
|---|---|
| `ui/src/components/StartRoundModal.tsx:29` | drop the `BranchDivergence` import (**§9** counts this as its 1 error) |
| `ui/src/components/StartRoundModal.tsx:624,627` | `DivergenceNote` reads the branch name from the last closed Round segment instead of `divergence.previous_branch` |
| `ui/tests/fixtures/rounds.ts:369,377` | fixtures stop setting `previous_branch`; set the closed Round segment's `branch` instead |

If P4 finds the join genuinely awkward at the call site, dropping the branch name
from the note is an acceptable fallback — the note's load-bearing content is the
merge-base, not the branch label. Either way this is P4's call and needs no spec
owner.

### 7.7 Known gaps

Recorded for a later decision. **Neither blocks P3**, and neither is a schema change
in this contract.

**W6's diagnostic sentence is not implementable as specified.** W6's example is:

> Round 2's branch `feature/x` is unavailable; `main` has 4 commits since Initial
> QC's approval.

The count "4" is a number of commits on the **other** Round's branch — but W6 is the
rule that forbids the Gap from walking that branch (`neighbour_unplaceable` ⇒
`commits: []`, and **I5**: an `Unplaceable` segment owns no commits). Nothing else
carries the number either: `RoundAnomaly::SegmentUnplaceable { position, reason }`
(**M7**) has no count field. So **projecting `anomalies` onto the wire would not
unblock this** — the sentence needs a *new advisory field* carrying a count that W6
simultaneously forbids computing.

> **Closed by D13.** This is no longer an open question: the spec **withdrew W6's
> diagnostic sentence** rather than adding the advisory field. `D4`/`U2` are fully
> implementable from `placement.kind` and `placement.reason`, so the count is additive
> whenever it is wanted, and W6's no-substitution rule is unaffected.

Leaving `anomalies` off the wire is nonetheless correct for everything §8 actually
asks for: **D4** wants unresolvable segments grayed rather than errored, and **U2**
wants them grayed *with their reason*. `placement.kind == "unplaceable"` and
`placement.reason` carry both of those per segment, positionally, with no anomaly
list required.

**D1 boundary-commit `statuses` divergence.** **D1** permits a Round's `opened_at` to
equal the previous Round's closing commit, and **I4** exempts exactly that commit
from single ownership — so the same hash legitimately appears in two adjacent
segments' `commits`. But §5 computes `statuses` **per owning segment**, so the two
copies disagree:

| Copy | `statuses` | Why |
|---|---|---|
| in R1's `commits` | `["approved"]` | it is R1's `RoundState::Closed.commit` |
| in R2's `commits` | `[]` | R2 has no event naming it, and it is not R2's closing commit |

`RoundCommitPicker` would therefore render the same hash with its approval dot when
drawing R1 and without it when drawing R2.

> **Closed by D14 and its addendum.** Both halves are decided, and they apply at
> different layers. **At the API projection**, `statuses` stay **per owning segment** —
> the asymmetry is intentional, because in R2's frame that commit is not R2's approval,
> and §5 stands as written. **At a flattening consumer** (the CLI picker,
> `flattenSegmentCommits` in the UI), the row must **dedupe by hash, union the statuses,
> and attribute the row to the newer owning segment** — one row cannot carry two frames,
> and rendering the hash twice corrupts positional defaults, which was a real CLI bug.
> A per-segment renderer such as the rail never flattens, so it keeps the asymmetry.

**Corollary, spec-consistent but worth writing down:** a Gap has no events and no
state, so by the §5 mapping a Gap's commits can **never** receive `notification`,
`reviewed`, or `approved` — only `initial` is reachable, and only for
`segments[0].opened_at`, which is a Round's. Consequently a `RoundEvent` naming a
commit that falls **outside its own Round's walked range** silently loses its status
dot: no Gap will pick it up. **S3** does scope coverage to "within this Round's
`commits`", so this follows the spec rather than contradicting it — but it means the
old thread-wide `HashSet<CommitStatus>` could show dots that the segment projection
will not.

---

## 8 What changed from the old shape

| Location | Old | New |
|---|---|---|
| `IssueStatusResponse.branch` | `string` (issue body's branch) | **renamed** `active_branch: string` (active segment's branch) |
| `IssueStatusResponse.commits` | `IssueCommit[]` (thread-wide) | **removed** → per-segment `commits` |
| `IssueStatusResponse.rounds` | `RoundInfo[]` | **removed** → `segments: Segment[]` |
| `IssueStatusResponse.open_round_index` | `number \| null` | **removed** → `segments.at(-1).kind === 'round'` |
| `QCStatus.approved_commit` | `string \| null` (meant both) | **removed** → `standing_approval` + `last_approved_commit`, both nullable |
| `QCStatus.latest_commit` | `string`, newest thread commit | newest **active-segment** commit, and now `string \| null` — an empty trailing Gap or an unplaceable segment owns no commit (§3) |
| `QCStatus.initial_commit` | `string` | `string \| null` — null when Round 1 is unplaceable (§3) |
| `IssueStatusResponse.next_notification_from` | `string` | `string \| null` — null when the active segment is unplaceable (§2) |
| `RoundInfo` (schema) | flat round list entry | **replaced** by `RoundSegment` (+ `GapSegment`, `Segment`) |
| `RoundInfo.branch` | `string \| null` | `RoundSegment.branch: string` — **non-nullable** (D5) |
| `RoundInfo.previous_approval` | `string \| null` | **removed** (accessor, Q1) |
| `RoundInfo.event_count` / `retraction_count` / `extension_count` | `number` ×3 | **replaced** by `events` / `retractions` / `extensions` arrays |
| `RoundInfo.checklist_source` | `RoundChecklistSource` | unchanged name and shape |
| `RoundInfo.{state,closing_commit,closed_by,closed_at}` | flat | unchanged |
| — | — | **new**: `RoundSegment.opened: RoundOpenInfo` |
| — | — | **new**: `placement: Placement`, `UnplaceableReason` |
| — | — | **new**: `GapSegment` with `continuity`, `lower_bound`, `upper_bound` |
| `BranchDivergence` (schema) | `{ previous_branch, merge_base }`, both nullable | **replaced** by `GapContinuity` (tagged; `merge_base` non-null on `diverged` only); `previous_branch` **dropped**, recoverable from the last closed Round segment (§7.6) |
| `RoundSeedResponse.divergence` | `BranchDivergence \| null` | `GapContinuity \| null` |
| `StartRoundResponse.divergence` | `BranchDivergence \| null` | `GapContinuity \| null` |
| `IssueCommit` | stored parse | **wire shape identical**; now an API projection, with `statuses` order pinned |
| `StepOutcome` | `{ status, error? }` | **new**: optional `skipped_reason` — why a skip could not be attempted; a repair's `notification` on an unplaceable round (now a 200 rather than a 409) **or** its `body_marker` when the round comment URL is unknown (§6.1) |
| `QCStatus.changed_commit` | — | **new**: `string \| null` — the trailing Gap's newest **file-changing** commit, the sha `changes_after_approval` is *about*. Not `latest_commit`, which is that Gap's newest commit full stop (§3) |
| `RoundSegment.opened_at` | `string` | `string \| null` — null when the round is `unplaceable`, rather than emitting the fold's all-zero OID placeholder as a real-looking sha (§4.1) |
| `GapSegment.lower_bound` / `upper_bound` | nulled per unplaceable *neighbour* | **also** both nulled when the Gap is **itself** `unplaceable`, however well the bounds resolve (§4.2) |
| `UnplaceableReason` | 4 values | **5**: adds `merge_base_unreachable`, for a Gap whose bounds resolved but whose common ancestor lies outside the walked range — previously mis-reported as `anchor_unreachable` (§4.4) |
| `StartRoundRequest.notification` / `RepairRoundRequest.notification` | in `required` | **removed from `required`**, and `?`-optional in TS: both Rust fields carry `#[serde(default)]`, so the server accepts absence — the schema was stricter than the server, and `body: {}` is what every default caller sends |
| `IssueStatusResponse.blocking_qc_status` | absent from `required`, TS `?` | **in `required`**, TS `?` dropped — the Rust field is a plain struct and the key is always emitted (§2) |

---

## 9 Known consumers to migrate

`cd ui && npx tsc --noEmit` after this contract landed reports **63 errors in 9
files**, all of them consumers of renamed/removed fields or of the four now-nullable
shas (§3) — no errors in `src/api/*`. This is the expected intermediate breakage
(**D11**) and is the UI wave's work list:

| File | Errors | What breaks |
|---|---|---|
| `src/components/IssueDetailModal.tsx` | 36 | `status.rounds`, `status.commits`, `status.open_round_index`, `status.branch`; cascading `unknown` from `toOrderedCommits` |
| `src/components/SwimLanes.tsx` | 6 | `open_round_index`, `qc_status.approved_commit`, `status.commits` (all inside `postApprovalFileCommit`, which **S6** deletes) |
| `src/components/FileResolveModal.tsx` | 6 | `qc_status.approved_commit` ×4, `status.branch` ×2 |
| `src/components/IssueCard.tsx` | 7 | `status.branch` (→ `active_branch`, **U3**), `status.rounds`, `approved_commit` ×2, and `qc_status.latest_commit` ×3 (lines 50, 55, 64) now `string \| null` where `CommitRow`'s `hash` is `string` — the card must decide what to render when the active segment owns no commit |
| `src/components/ArchiveTab.tsx` | 4 | `qc_status.approved_commit` ×4 |
| `src/utils/rounds.ts` | 1 | imports removed `RoundInfo` (**U4** deletes most of this file) |
| `src/components/RoundRail.tsx` | 1 | imports removed `RoundInfo` |
| `src/components/RoundCommitPicker.tsx` | 1 | imports removed `RoundInfo` |
| `src/components/StartRoundModal.tsx` | 1 | imports removed `BranchDivergence` (see §7.6) |

Not covered by `tsc` (excluded from the app tsconfig) but equally stale:
`ui/tests/fixtures/rounds.ts`, `ui/tests/fixtures/index.ts`,
`ui/tests/status/*.spec.ts`, `ui/tests/archive/flatten.spec.ts`,
`ui/tests/record/record.spec.ts` — every one builds `rounds` /
`open_round_index` / `approved_commit` / `commits` by hand. Rust-side fixtures:
`src/api/tests/cases/rounds/*.yaml`.
