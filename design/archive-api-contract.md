# Archive under Round Semantics — API Wire Contract (P2)

Status: **pinned.** Branch `rounds`, baseline `03498d0`. Authority for the JSON exchanged
across every archive boundary while `design/archive-rounds.md` §5 (**A1**–**A4**) is
implemented. That spec is the design authority; this document is the wire authority
derived from it, in the same relationship `design/segment-api-contract.md` has to
`design/segment-model.md`. Where the spec was silent, the choice is marked
**[implementer's choice]** with its reasoning.

Three boundaries are pinned here, and they are not the same kind of boundary:

| Boundary | Shape | Spec IDs |
|---|---|---|
| `POST /api/archive/generate` request | `ArchiveGenerateRequest.files[]` | **A1**, **A2** |
| `POST /api/archive/generate` response | `ArchiveGenerateResponse` | — (**unchanged**, §3) |
| `GET /api/issues/status` round segments | `RoundSegment.latest_actioned_commit` | **A3** |
| `ghqc_archive_metadata.json` inside the tarball | `ArchiveFile` / `ArchiveQC` / `RoundProvenance` / `Approval` | **A4**, **M1** |

The last one is a **file format**, not an HTTP payload, and it is written identically by
the CLI (`src/cli/archive.rs:154`, `src/main.rs:1145/1171/1202`) and by the API
(`src/api/routes/archive.rs:148`). One shape, two producers — **§0.2** of the spec exists
because those two producers disagreed.

**Out of scope, deliberately.** Nothing here designs UI (**U1**–**U8**), CLI flags
(**C1**–**C5**), or the derivation algorithm (**S1**–**S3**, **M4**'s computation). Where
a shape below implies a derivation, the derivation is cited, not restated; the spec
governs it.

Files this contract pins, none of which it edits:

- `src/api/types/requests.rs`, `src/api/routes/archive.rs`, `src/api/types/responses.rs`
- `src/archive.rs`
- `openapi/openapi.yml`
- `ui/src/api/archive.ts`, `ui/src/api/rounds.ts` — TypeScript declarations only

---

## 1 Verified baseline

Every shape below is quoted from the tree at `03498d0`, not from the spec's prose. Where
the two disagree, §9 records it.

| Shape | Where it lives today | Current form |
|---|---|---|
| `ArchiveFileRequest` | `src/api/types/requests.rs:284-289` | one flat struct: `repository_file: PathBuf`, `commit: String`, `milestone: Option<String>`, `approved: Option<bool>` — **no `issue_number` at all** |
| mixed-request rejection | `src/api/routes/archive.rs:134-146` | hand-written: `(Some, None) | (None, Some)` ⇒ `ApiError::BadRequest("milestone and approved must both be provided or both omitted for file: …")` |
| `ArchiveGenerateResponse` | `src/api/types/responses.rs:1663-1666` | `{ output_path: String }` |
| `ArchiveQC` | `src/archive.rs:15-19` | `{ milestone: String, approved: bool }`, `Serialize + Deserialize` |
| `ArchiveFile` | `src/archive.rs:37-49` | `repository_file`, `archive_file`, `commit: ObjectId` (via `display_as_string`/`parse_from_string`, `src/archive.rs:21-35`), `#[serde(flatten)] qc: Option<ArchiveQC>` |
| `ArchiveMetadata` | `src/archive.rs:116-121` | private `creator: Option<String>`, `created_at: DateTime<Utc>` (`Utc::now()`, `src/archive.rs:169`), `files: Vec<ArchiveFile>` |
| `RoundSegment` | `src/api/types/responses.rs:782-818` | 16 fields (17 JSON keys with the `kind` tag), **no** `latest_actioned_commit` |
| `Round::newest_event_commit` | `src/round.rs:285-291` | newest event commit **by position in the round's own `commits`**; `None` when no event names an owned commit |
| `IssueThread::next_notification_from` | `src/issue.rs:325-338` | `!is_placed()` ⇒ `None`; Round ⇒ `newest_event_commit().unwrap_or(opened_at)` |
| TS request type | `ui/src/api/archive.ts:3-8` | `{ repository_file, commit, milestone?, approved? }` |
| OpenAPI request schema | `openapi/openapi.yml:2857-2875` | `required: [repository_file, commit]`, plus nullable `milestone` / `approved` |

Environment facts the shapes below depend on, each verified by probe rather than
recalled: **serde 1** (flatten/tagging behaviour), **chrono 0.4.41** (`Cargo.lock:531`,
timestamp rendering), **axum 0.8.8** (`Cargo.lock:209`, `Json` rejection status).

### 1.1 A live defect the baseline already has, which A2 removes

`ui/src/components/ArchiveTab.tsx:427-439` builds every mode-2 ("manually added") file as
`{ repository_file, commit, approved: false }` — **`milestone` omitted, `approved`
present**. `src/api/routes/archive.rs:135` rejects exactly that pair, so any archive
containing a manually added file returns 400 today. The same hazard exists for a mode-1
file whose `issue.milestone` is null (`ArchiveTab.tsx:423`). Recorded here because it is
the concrete cost of the shape **A1** replaces — two optional fields whose legal
combinations are a convention rather than a type — and because **A2** deletes the field
that causes it. It is **not** a thing to fix separately: fixing the pair while keeping the
flat struct would preserve the representable-invalid shape.

---

## 2 `ArchiveFileRequest` — A1, A2

### 2.1 Tagging decision

`ArchiveFileRequest` becomes an **internally tagged enum on `mode`**, values `issue`
(mode 1) and `file` (mode 2), with `deny_unknown_fields`:

```rust
#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveFileRequest {
    /// Mode 1 (A1): a file under QC in a milestone. The backend derives the path, the
    /// commit, `approval` and `superseded` from the thread (D6).
    Issue {
        issue_number: u64,
        /// `None` ⇒ `ArchiveTarget::Latest` (D9). `Some(n)` ⇒ `ArchiveTarget::Round(n)` (D7).
        #[serde(default)]
        round: Option<u32>,
    },
    /// Mode 2 (D4/R1): a file in no milestone. Unchanged — the user picks the commit.
    File {
        repository_file: PathBuf,
        commit: String,
    },
}
```

**Why tagged, and not the other two candidates.** The requirement is that a mixed request
— one carrying both a round selection and a hand-picked commit — cannot be silently
honoured, because "two sources of truth for one fact, neither checked against the other"
is the **§0** defect this whole spec exists to remove.

| Candidate | Behaviour on `{"issue_number":7,"round":null,"repository_file":"a.R","commit":"abc"}` | Verdict |
|---|---|---|
| `#[serde(untagged)]` | **`Ok(Issue { issue_number: 7, round: None })`** — `repository_file` and `commit` silently discarded (verified) | **rejected.** Reproduces §0.1 exactly: the request claims two things, the server acts on one, nothing reports the discard. Untagged also yields `"data did not match any variant of untagged enum"` for every genuine typo, naming no field |
| flat struct + mutually exclusive `Option`s | representable; rejected only by a hand-written check, as at `routes/archive.rs:134-146` today | **rejected.** This *is* the baseline, and §1.1 is what it costs. Four optional fields have sixteen combinations, of which two are legal; the type asserts nothing |
| internally tagged + `deny_unknown_fields` | **`Err("unknown field `repository_file`, expected `issue_number` or `round`")`** (verified) | **chosen.** The mode is *declared*, never inferred; the invalid shape is unconstructible in Rust and named on rejection |

Internal tagging is also the convention this codebase already pinned for every response
union (`design/segment-api-contract.md` §1: `Segment`, `GapContinuity`, `Placement`, all
`tag = "kind"`), and it is what a TypeScript discriminated union consumes with no
unwrapping.

Serde caveats the implementer must respect:

1. `deny_unknown_fields` **is** honoured on an internally tagged enum's struct variants
   (verified: the error above). It **cannot** be combined with `flatten` — which is why
   §5's `ArchiveFile`, whose `qc` is flattened, does not get it.
2. The tag key is `mode`, not `kind`. **[implementer's choice]** — the spec has no tag
   name; `mode` is the word §4/**D4**/**A1** use for this distinction ("selection mode 2"),
   and reserving `kind` for the segment unions keeps one word per concept. The variant
   values `issue` / `file` match the two existing constructors, `ArchiveFile::from_issue_thread`
   (`src/archive.rs:56`) and `ArchiveFile::from_file` (`src/archive.rs:98`).
3. This type is deserialize-only, as today (`requests.rs:283`). Do not add `Serialize`;
   nothing on the Rust side emits a request.

### 2.2 JSON — both modes in one array

```jsonc
{
  "output_path": "archives/milestone-3.tar.gz",
  "flatten": false,
  "files": [
    // mode 1 — default target: the latest round (D9/S2)
    { "mode": "issue", "issue_number": 42, "round": null },

    // mode 1 — retargeted to round 1's approval (D7); an override, per U2
    { "mode": "issue", "issue_number": 43, "round": 1 },

    // mode 2 — a file in no milestone; the user picked the commit (D4/R1)
    { "mode": "file",
      "repository_file": "scripts/helpers.R",
      "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" }
  ]
}
```

`ArchiveGenerateRequest` itself is **unchanged** (`requests.rs:292-297`): `output_path`,
`flatten`, `files`. `flatten` stays a whole-archive flag; **D8**'s one-file-per-archive
rule and the flatten collision check are enforced server-side by
`ArchiveMetadata::new` (`src/archive.rs:124-172`), not on the wire.

#### Mode 1 — `mode: "issue"`

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `mode` | `"issue"` | no | tag |
| `issue_number` | integer (u64) | no | the QC issue. **The only handle to the file** — the backend reads the thread and takes `IssueThread.file` and `IssueThread.milestone` from it (**D6**) |
| `round` | integer (u32) | **yes** | the round the selection addresses. `null` ⇒ `ArchiveTarget::Latest` (**A1**, **D9**). `1`-based; `1` is Initial QC. Any `n` in `1..=` (number of round segments) is legal (**D7**) |

#### Mode 2 — `mode: "file"`

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `mode` | `"file"` | no | tag |
| `repository_file` | string (path) | no | path to the file in the repository, as today |
| `commit` | string (sha) | no | full 40-char hex commit to take the bytes at, as today. Parsed with `ObjectId::from_hex` (`routes/archive.rs:103`) |

**`round` is present-and-null, not absent.** Rust carries `#[serde(default)]` so an absent
key is tolerated for `curl`/CLI callers, but OpenAPI lists `round` in `required` with
`nullable: true` and TypeScript declares it non-optional, so **the UI has exactly one
encoding of "latest"**. This is a deliberate inversion of the reasoning in
`design/segment-api-contract.md` §8 for `StartRoundRequest.notification`, where the schema
was loosened to match traffic that already existed: here there is no traffic yet, so the
schema defines it, and two encodings of the default target is the kind of redundancy §0
punishes. The server being the more permissive of the two is safe in that direction only.

### 2.3 Fields that must **not** appear on a mode-1 entry

Each of these existed at `requests.rs:284-289` and is deleted. An implementer restoring
any of them reopens a named defect:

| Removed | Why it may not come back |
|---|---|
| `approved` | **A2**. The bit was unfalsifiable after the first approval (**§0.3**) and the two surfaces set it from different predicates (**§0.1**). Replaced by `RoundProvenance.approval` + `superseded` — *two* fields, per **D1** |
| `milestone` | Derivable from the thread (`IssueThread.milestone`); a client-sent copy is a second source of truth for a fact the server already holds (**D6**). Its optionality is also half of §1.1's live 400 |
| `commit` | **D6**/**S1**: the commit *follows from* the selected round. A client-sent commit beside a client-sent round is the mixed request §2.1 exists to forbid |
| `repository_file` | Derivable (`IssueThread.file`); a client-sent path could disagree with the issue title after a rename |

### 2.4 Confirming A2: nothing else on the request carries an approval claim

Exhaustive over the post-**A1** request: `ArchiveGenerateRequest` has `output_path`,
`flatten`, `files`; mode 1 has `issue_number`, `round`; mode 2 has `repository_file`,
`commit`. **None of the seven asserts approval**, and none is derived from an approval
predicate. `round` is a *selection*, not a claim: **M2** — it is the round the selection
addressed, and per **I2** it does not even imply that round was approved. The only
approval statement anywhere in the archive boundary is `RoundProvenance.approval`, in the
metadata file, written by the server (§5).

### 2.5 Rejections — status codes and bodies

The move from a flat struct to a tagged enum moves some rejections from the handler into
the extractor, and axum's `Json<T>` rejection is **not** `ApiError`'s
`{"error": string}` shape. Pinned:

| Cause | Status | Body |
|---|---|---|
| unknown/absent `mode`, unknown field, missing variant field, wrong type | **422 Unprocessable Entity** (axum 0.8 `JsonDataError`) | axum's plain-text `Failed to deserialize the JSON body into the target type: …` |
| malformed JSON | **400** (axum `JsonSyntaxError`) | plain text |
| missing `Content-Type: application/json` | **415** | plain text |
| `round: 0`, or `round` greater than the thread's round count | **400** `ApiError::BadRequest` | `{"error": "…"}`; the message **must name the issue number**, since mode 1 carries no path |
| `issue_number` names no QC issue in the repository | **404** `ApiError::NotFound` | `{"error": "…"}` |
| selected round resolves to no commit — an unplaceable segment (**I4**/**S5**) | **400** `ApiError::BadRequest` | `{"error": "…"}`; must name the issue number and `placement.reason` |
| mode 2: `commit` is not valid hex | **400** `ApiError::BadRequest` | unchanged (`routes/archive.rs:103-105`) |
| flatten collision / duplicate archive path | **400** `ApiError::BadRequest` | unchanged (`ArchiveMetadata::new`) |

`ui/src/api/archive.ts:28-31` already degrades gracefully on a non-JSON body
(`res.json().catch(() => null)` ⇒ `Failed to generate archive: ${status}`), so a 422 is
survivable but *unreadable*. Two consequences, both pinned rather than left to taste:
the UI must construct requests from the tagged types and never hand-assemble a body; and
if an implementer later normalises `JsonRejection` into `ApiError::BadRequest`, **that is
a wire change** (a status code and a body shape) and must come back through this document.
Message wording is not pinned; status code and body shape are.

### 2.6 TypeScript (`ui/src/api/archive.ts`)

```ts
/** Mode 1 (A1): a milestone QC file. The backend derives path, commit and provenance. */
export interface ArchiveIssueFileRequest {
  mode: 'issue'
  issue_number: number
  /** The round this selection addresses; `null` targets the latest round (D9). */
  round: number | null
}

/** Mode 2 (D4): a file in no milestone — the user picks the commit directly. */
export interface ArchiveAddedFileRequest {
  mode: 'file'
  repository_file: string
  commit: string
}

export type ArchiveFileRequest = ArchiveIssueFileRequest | ArchiveAddedFileRequest
```

`round` is **not** `?`-optional (§2.2). `ArchiveGenerateRequest` and
`ArchiveGenerateResponse` are unchanged.

### 2.7 OpenAPI (`openapi/openapi.yml`, replacing lines 2857-2875)

```yaml
    ArchiveFileRequest:
      description: |
        One file to archive. Mode 1 (`mode: issue`) names a QC issue and, optionally, the
        round the selection addresses; the server derives the path, the commit and the
        provenance. Mode 2 (`mode: file`) names a path and a commit directly.
      oneOf:
        - $ref: '#/components/schemas/ArchiveIssueFileRequest'
        - $ref: '#/components/schemas/ArchiveAddedFileRequest'
      discriminator:
        propertyName: mode
        mapping:
          issue: '#/components/schemas/ArchiveIssueFileRequest'
          file: '#/components/schemas/ArchiveAddedFileRequest'

    ArchiveIssueFileRequest:
      type: object
      additionalProperties: false
      required: [mode, issue_number, round]
      properties:
        mode:
          type: string
          enum: [issue]
        issue_number:
          type: integer
          description: The QC issue whose thread supplies the file, milestone and commit
        round:
          type: integer
          nullable: true
          description: |
            1-based round the selection addresses; 1 is Initial QC. Null targets the
            latest round, which for a reopened file is unapproved content — intentional
            and ungated. Always send the key; send null rather than omitting it.

    ArchiveAddedFileRequest:
      type: object
      additionalProperties: false
      required: [mode, repository_file, commit]
      properties:
        mode:
          type: string
          enum: [file]
        repository_file:
          type: string
          example: "scripts/helpers.R"
        commit:
          type: string
          description: Full 40-char commit hash to retrieve the file at
```

`additionalProperties: false` mirrors `deny_unknown_fields`. The endpoint's `responses`
block (`openapi.yml:918-928`) gains `'404'` and `'422'`; `'200'`, `'400'` and `'500'`
stand.

---

## 3 `POST /api/archive/generate` response — UNCHANGED

```jsonc
{ "output_path": "/abs/path/to/repo/archives/milestone-3.tar.gz" }
```

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `output_path` | string | no | absolute path the archive was written to, as today |

**No field is added.** Checked against every spec ID that could plausibly want one:

- **A1**–**A4** name no response change; §8 **P2** is exactly `A1`–`A4`.
- **U5**'s pre-generate summary (`12 files · 9 approved & current · …`) is *pre*-generate:
  it is computed from the status endpoint's segments (**A3**), which the UI already holds.
  It is also **P5**, non-blocking.
- **S5**'s unplaceable block is a **pre**-flight decision too: the UI has
  `segments[].placement` and gates before POSTing (**U8**). A request that names an
  unplaceable thread anyway is rejected per §2.5, not reported in a 200 body.
- The per-file provenance the server derived is not echoed back. It is recorded in the
  tarball (§5), which is the artifact; echoing it would create a second copy of a
  snapshot that **D3** says a reader must never recompute.

Adding a field here — a summary, a per-file result list, a warning array — is a wire
change and must come back through this document.

---

## 4 `RoundSegment.latest_actioned_commit` — A3

**A3** adds exactly one key to the existing status response. Everything else the round
picker and the default need (`state`, `closing_commit`, `events`, `placement`) already
ships — `design/segment-api-contract.md` §4.1, and `responses.rs:782-818`.

### 4.1 JSON

```jsonc
{
  "kind": "round",
  "index": 3,
  "name": "Round 3",
  "opened_at": "dddddddddddddddddddddddddddddddddddddddd",
  "branch": "feature/x",
  "opened": { "kind": "new_round", "comment_id": 12345, "comment_url": "…",
              "author": "alice", "at": "2026-08-01T10:00:00Z", "note": null },
  "checklist_source": { "kind": "comment", "comment_id": 12345, "comment_url": "…" },
  "checklist_name": "Code Review",
  "state": "open",
  "closing_commit": null,
  "closed_by": null,
  "closed_at": null,
  "events": [ { "kind": "notification", "commit": "eeeeeeee…", "by": "alice",
                "at": "2026-08-01T10:00:01Z", "comment_id": 12346, "comment_url": "…" } ],
  "retractions": [],
  "extensions": [],
  "commits": [ /* IssueCommit[], newest first */ ],
  "latest_actioned_commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "placement": { "kind": "placed" }
}
```

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `latest_actioned_commit` | string, full 40-char sha | **yes** | **M4.** This round's newest commit carrying an action — its own anchor (`opened_at`), a notification, or a review — by position **within this round's own `commits`**. Drift nobody acted on is not a candidate. `null` ⟺ `placement.kind == "unplaceable"` |

- **Carried by `RoundSegment` only.** It is **absent** from `GapSegment` — not null there.
  A gap has no anchor, no events and no state, so the fact is not merely unknown for a gap,
  it is undefined; and the union's established style is absent-not-null for keys that
  belong to one variant (`design/segment-api-contract.md` §4.4's `reason`, §4.5's
  `merge_base`, and a gap's missing `index`/`name`). A consumer reads it after narrowing on
  `kind === 'round'`, which it must already do to reach `state` and `closing_commit`.
  Spec **A3** says "projected onto each **round** segment", so gaps are out of its scope;
  the practical guarantee a caller wants — *nothing on the wire attributes an actioned
  commit to a gap* — holds either way.
- **Null exactly when the round is unplaceable.** An unplaceable round owns no commits
  (segment-spec **I5**; `commits: []`) and its anchor is the fold's all-zero OID, already
  nulled at `opened_at` (`responses.rs:882`) precisely so no fake sha reaches an audit
  tool's wire. The converse holds because a *placed* round's `commits` run "from the
  closing commit (or the branch tip while open) back to `opened_at`, **inclusive**"
  (`src/round.rs:252-254`), so `opened_at` is always an owned candidate. Equivalently:
  `latest_actioned_commit === null ⟺ opened_at === null ⟺ placement.kind === 'unplaceable'`.
- **Derivation is fixed by an existing identity, not restated.** **M4** ties it to
  segment-spec **M8**'s `next_notification_from()` in its Round branch, which is
  `src/issue.rs:325-338`:
  `is_placed().then(|| round.newest_event_commit().copied().unwrap_or(round.opened_at))`.
  The two must not drift; the accessor added by **M4** is the shared implementation and
  `next_notification_from` should call it.
- **It is not the round's newest commit.** `commits[0]` is the newest commit full stop;
  this is the newest *actioned* one. **S6** is the whole point of the difference. Do not
  substitute one for the other, in either direction.
- **It is not `closing_commit`.** For a closed round the closing commit is a
  `RoundState::Closed.commit`, not a `RoundEvent`, so it is **not** a candidate unless some
  review or notification also names it. A closed round can therefore report a
  `latest_actioned_commit` *older* than its `closing_commit`. **S1** rows 1 and 2 take the
  **closing commit**; only row 3 uses this field. A consumer that reads it for a closed
  round is reading the wrong field.
- **Why it is on the wire at all**, given that a client could recompute it from `commits`,
  `events` and `opened_at`: **D6**, and segment-spec §0's count of *five* independent
  re-derivations of round scope in two languages. **A3** exists to stop the sixth. This is
  the one place the contract accepts derivable data on the wire, and the reason is
  explicit; it is not a precedent for reviving `event_count`.

### 4.2 TypeScript (`ui/src/api/rounds.ts`, inside `interface RoundSegment`)

```ts
  /**
   * M4: this round's newest commit carrying an action — its anchor, a notification, or a
   * review — by position in its own `commits`. Drift nobody acted on is not a candidate.
   *
   * null exactly when `placement.kind === 'unplaceable'`. Not `commits[0]` (S6) and not
   * `closing_commit`: a closed round's approval is not an event, so this can be older.
   */
  latest_actioned_commit: string | null
```

Insert after `commits`, before `placement`, matching the Rust field order so the emitted
key order and the declaration order agree.

### 4.3 OpenAPI (`openapi/openapi.yml`, `RoundSegment` at 1403)

Add `latest_actioned_commit` to the `required` list (1412-1415) — it is nullable and
always emitted, per the segment contract's "nullable ⇒ present ⇒ `required` +
`nullable: true`" rule — and add the property after `commits`:

```yaml
        latest_actioned_commit:
          type: string
          nullable: true
          description: |
            The round's newest commit carrying an action — its anchor, a notification or a
            review — by position within this round's own commits. Null exactly when the
            round is unplaceable. Not the round's newest commit, and not its closing
            commit: a closed round's approval is not an event, so this may be older.
```

`GapSegment`'s schema (1487) is untouched.

### 4.4 Test seam

`src/api/types/responses.rs:1910-1918` already has a `project(&[Segment]) ->
Vec<serde_json::Value>` helper used by the segment tests, including
`an_unplaceable_round_reports_no_opened_at` (2168-2182). The null-iff-unplaceable claim
and the not-`closing_commit` claim belong there, asserted on emitted JSON.

---

## 5 `ghqc_archive_metadata.json` — A4, M1

One file per archive, at the tarball root (`src/archive.rs:196-198`), written with
`serde_json::to_string_pretty`.

### 5.1 JSON — a mode-1 file and a mode-2 file in one archive

```jsonc
{
  "creator": "wes",
  "created_at": "2026-08-19T18:36:39.248217Z",
  "files": [
    {
      "repository_file": "scripts/analysis.R",
      "archive_file": "scripts/analysis.R",
      "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "milestone": "Milestone 3",
      "round": {
        "round": 2,
        "approval": {
          "round": 2,
          "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "by": "wes",
          "at": "2026-07-04T15:12:09Z"
        },
        "superseded": false
      }
    },
    {
      "repository_file": "scripts/helpers.R",
      "archive_file": "scripts/helpers.R",
      "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    }
  ]
}
```

Note the second entry: mode 2 emits **no** `milestone` and **no** `round` key at all.
`qc` is `#[serde(flatten)] Option<ArchiveQC>` (`src/archive.rs:41-48`), and a flattened
`None` contributes zero keys — verified, including the round-trip back to `None`, which
matters because `src/archive.rs:660` deserializes this file in a test.

### 5.2 Types (M1) with their serde representation

```rust
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveQC {
    pub milestone: String,
    /// Nested, not flattened. See §5.4.
    pub round: RoundProvenance,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RoundProvenance {
    pub round: u32,
    pub approval: Option<Approval>,
    pub superseded: bool,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Approval {
    pub round: u32,
    #[serde(
        serialize_with = "display_as_string",
        deserialize_with = "parse_from_string"
    )]
    pub commit: ObjectId,
    pub by: String,
    pub at: chrono::DateTime<chrono::Utc>,
}
```

`ArchiveFile` keeps its four fields and its `#[serde(flatten)] qc: Option<ArchiveQC>`
(**M5**). All three types derive **both** `Serialize` and `Deserialize`, matching
`ArchiveQC` today (`src/archive.rs:15`) — the round-trip test at `src/archive.rs:660` is
the only reader, and per **D5**/**R8** there is no external one.

#### `ArchiveFile`

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `repository_file` | string (path) | no | the file's path in the repository. Mode 1: `IssueThread.file`. Mode 2: as requested |
| `archive_file` | string (path) | no | its path inside the tarball; the basename when `flatten` |
| `commit` | string, 40-char lowercase hex | no | the commit the archived bytes were read at. **The single fact about the bytes**; for mode 1 it is derived from the selected round per **S1**, never client-supplied |
| `milestone` | string | **absent** for mode 2 | flattened from `qc`. Present ⟺ `round` is present |
| `round` | `RoundProvenance` | **absent** for mode 2 | flattened from `qc`. See below |

`milestone` and `round` are the flattened `ArchiveQC`: **both present, or both absent.**
There is no state where one appears without the other — that is what `Option<ArchiveQC>`
buys over two optional fields, and it is the same guarantee §1.1's live 400 was trying to
enforce by hand on the request side.

#### `RoundProvenance` (the value of `round`)

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `round` | integer (u32) | no | **the round this selection addressed** (**M2**), 1-based. Never becomes false (**D3a**). **Not** an assertion that this round was approved — see §6 |
| `approval` | `Approval` object | **yes — explicit `null`** | `null` ⟺ the archived bytes were never approved (**S1** row 3). Non-null ⟺ `ArchiveFile.commit` is some round's closing commit (**I2**) |
| `superseded` | bool | no | `true` ⟹ at archive time these bytes were **not** the newest QC state. A glance-level flag, not evidence (**D10**). Derived per **S3**; never recomputed by a reader (**D3**) |

#### `Approval` (the value of `round.approval`)

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `round` | integer (u32) | no | **the round that closed on this commit.** May be **less than** `RoundProvenance.round` (**I2**, **R13**) — §6 |
| `commit` | string, 40-char lowercase hex | no | that round's closing commit. Equal to `ArchiveFile.commit` (**I2**) |
| `by` | string | no | the GitHub login that approved |
| `at` | RFC 3339 timestamp, UTC | no | when the approval landed |

### 5.3 Serde representation of `Option<Approval>`

**Explicit `null`, never absent.** No `skip_serializing_if` — there is none anywhere in
`src/archive.rs` today, and this field is the one where absence would be actively
misleading: "no approval" is the *load-bearing* fact of **S1** row 3, and a missing key
reads as "this writer did not know" rather than "these bytes were never approved". Contrast
`design/segment-api-contract.md` §6.1's `skipped_reason`, where absence genuinely *is* the
fact. So the key is always present on a mode-1 file:

```jsonc
"round": { "round": 3, "approval": null, "superseded": true }
```

**Derivable invariant, worth asserting in a test:** `approval == null ⟹ superseded ==
true`. `approval` is null only in **S1** row 3, which requires `r == n` **open**, and
**S3**'s second clause makes an open latest round `superseded`. So an unapproved archive
entry is always flagged. The converse does not hold: a `superseded: true` entry very often
has an approval (§8.2).

### 5.4 `round` is nested, not flattened

`ArchiveQC.round` carries **no** `#[serde(flatten)]`. Flattening it would put `round`,
`approval` and `superseded` at the file level, where the u32 `round` would sit as a
sibling of `commit` and directly collide, in a reader's eye, with `approval.round` — the
one distinction **I2** exists to keep visible. The nesting is what makes
`round.round` vs `round.approval.round` legible as two frames rather than one
contradiction. `ArchiveQC` itself stays flattened into `ArchiveFile`, as today.

### 5.5 Timestamps

`Approval.at` and `ArchiveMetadata.created_at` use the identical encoding: chrono 0.4's
`Serialize for DateTime<Utc>` — RFC 3339, always `Z`, **fractional seconds present only
when non-zero**, with 3, 6 or 9 digits. Verified:

```
"2026-07-04T15:12:09Z"          // whole second
"2026-07-04T15:12:09.123Z"      // milliseconds
"2026-08-19T18:36:39.248217Z"   // chrono::Utc::now(), as created_at is (src/archive.rs:169)
```

In practice `created_at` carries microseconds (it is `Utc::now()`) while `Approval.at`
usually does not (it comes from a GitHub comment timestamp). A reader must accept both —
an exact-length parse of either field is a bug. This matches `closed_at` and the round
events' `at` on the status response (`design/segment-api-contract.md` §4.1).

### 5.6 `ObjectId`

Every `ObjectId` on this file serializes through the existing pair at
`src/archive.rs:21-35`: `display_as_string` (`serialize_str(&value.to_string())` ⇒ 40
lowercase hex characters, no prefix, never abbreviated) and `parse_from_string`
(`ObjectId::from_hex`). `Approval.commit` carries the same attribute pair as
`ArchiveFile.commit` does today; the attributes go on the **field inside `Approval`**, so
`Option<Approval>` needs no wrapper. Do not introduce a second encoding (`Display` on a
newtype, `Vec<u8>`, an abbreviated sha) — an archive is an audit artifact and one hash
encoding is part of that.

### 5.7 Removed

| Removed | Where it was | Replacement |
|---|---|---|
| `ArchiveQC.approved: bool` | `src/archive.rs:18`; written at `:91-94` and `routes/archive.rs:141-144` | `round.approval` (permanent fact) + `round.superseded` (perishable fact) — two fields, per **D1**/**D5** |

Hard swap, no dual-shipping, per **D5**/**R8** and segment-spec **D9**; `CHANGELOG.md`
entry required (**C5**).

---

## 6 I2 on the wire — "you were on round 2, the commit is round 1's approval"

**I2** is the subtle case and the wire must not let a reader collapse it. Segment-spec
**D1** allows a round's `opened_at` to equal the previous round's closing commit (HEAD has
not moved since approval — legitimate, e.g. re-QC against a stricter checklist). Select
that open round 2 and **S1** row 3 takes its latest actioned commit, which is its anchor,
which *is* round 1's approval. Two facts, two fields:

- `round.round = 2` — the frame the selection was made in.
- `round.approval.round = 1` — the round that actually closed on those bytes.

**The pinned reading rule.** An approval claim is **only ever about
`round.approval.round`.** Nothing on this file asserts that `round.round` was approved,
and a consumer may not infer it. Concretely, these are **forbidden** derivations:

| Forbidden | Why |
|---|---|
| `approved = round.approval !== null` paired with `round.round` | renders *"Round 2 approved"* for §6's example. Round 2 is open. This is **§0.1** — approved content relabelled — with the sign flipped |
| `assert(round.approval.round === round.round)` | fires on valid data (**I2**, **R13**) |
| collapsing the two into one `approved_round` | **D1** compressing two facts into one, the root cause in **§0** |

Correct renderings pair each number with its own frame: *"Round 2 · bytes are Round 1's
approval · a1b2c3d"*. Wording is **U1**'s, not this document's.

### 6.1 Worked JSON — exactly the I2 case

Thread: Initial QC closed at `aaa…`; round 2 opened at the same commit (**D1**) and is
open; nothing has landed since. Selection: default (`round: null` ⇒ `Latest` ⇒ r = n = 2).

```jsonc
// request
{ "mode": "issue", "issue_number": 42, "round": null }
```

```jsonc
// ghqc_archive_metadata.json, files[0]
{
  "repository_file": "scripts/analysis.R",
  "archive_file": "scripts/analysis.R",
  "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "milestone": "Milestone 3",
  "round": {
    "round": 2,
    "approval": {
      "round": 1,
      "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "by": "wes",
      "at": "2026-07-04T15:12:09Z"
    },
    "superseded": true
  }
}
```

Three things to read off it, all required:

1. `round.round` (2) `≠` `approval.round` (1) — legal, and the reason **I2** exists.
2. `approval.commit == ArchiveFile.commit` — **I2**'s ⟺, and the only reason the approval
   is claimable at all.
3. `superseded: true` — **S3** clause 2: the latest round is open. The bytes are approved
   *and* not the newest QC state. That combination is unrepresentable in the baseline's
   single `approved` bool, and representing it is the point of **D1**.

---

## 7 Prohibited fields

These are named prohibitions, each with its reason, because each was considered and
rejected in the spec (**M1**'s closing note, **D3a**, **D10**, **I5**, **R6**) and each
looks like an improvement to someone reading only the shape.

| Must not exist | On | Reason it stays out |
|---|---|---|
| `rounds_total`, `round_count`, `"round 2 of 4"` | anywhere in the metadata | **D3a.** It is an **unknowable upper bound**: a fifth round can open the day after the archive is written, making the recorded total false. Only facts that stay true forever are stored |
| `was_latest` | `RoundProvenance` | **D3a**/**D10.** Perishable, and already covered: `superseded` records the archiver's contemporaneous knowledge, and the durable version is recomputable from the repository plus `created_at` |
| any per-cause breakdown of `superseded` — `superseded_by`, `Superseding`, `Vec<Superseding>`, `superseded_reason`, a `thread_state` enum, or promoting the bool to an object | `RoundProvenance` | **D10**/**R6**. `superseded` is a **convenience flag**, deliberately *not* an evidentiary structure. Every cause is recomputable from the thread plus `created_at`, so a structure buys an auditor nothing while inviting the **§0** misreading of a snapshot as current state. **R14** accepts the conflation of post-approval drift with an open re-review round on the record: both mean *go check the thread*, and **U1**/**U5** distinguish them at selection time, where a user can still act. `superseded` is a **plain bool** |
| `approved: bool` | request, metadata, or any status projection | **A2**/**D5**. Unfalsifiable after the first approval (**§0.3**); four disagreeing definitions (**§0**) |
| `milestone`, `commit`, `repository_file` | a **mode-1** request entry | §2.3 — **D6**, one source of truth per fact |

**I5** makes this review-enforced, not runtime-enforced. This table is the review.

---

## 8 Worked examples

Four rows of **S1**/**S3** end to end. Only `files[0]` of the metadata is shown, and the
request entry that produced it.

### 8.1 Approved and current

Thread: 3 rounds, round 3 closed at `ccc…`, trailing gap empty. Selection: default.
**S1** row 2; **S3** all clauses false.

```jsonc
{ "mode": "issue", "issue_number": 42, "round": null }
```
```jsonc
{
  "repository_file": "scripts/analysis.R",
  "archive_file": "scripts/analysis.R",
  "commit": "cccccccccccccccccccccccccccccccccccccccc",
  "milestone": "Milestone 3",
  "round": {
    "round": 3,
    "approval": { "round": 3, "commit": "cccccccccccccccccccccccccccccccccccccccc",
                  "by": "wes", "at": "2026-08-10T09:20:00Z" },
    "superseded": false
  }
}
```

`round.round == approval.round` — the ordinary case — and `superseded: false`, which per
**I3** is exactly the claim *these were the latest round's approval with no file changes
since, as of `created_at`*.

### 8.2 Approved but superseded, retargeted to an older round

Thread: round 1 closed at `aaa…`, round 2 closed at `bbb…`, trailing gap empty. Selection:
an explicit override to round 1 (**D7**/**U2**). **S1** row 1; **S3** clause 1 (a round
`> r` has closed).

```jsonc
{ "mode": "issue", "issue_number": 43, "round": 1 }
```
```jsonc
{
  "repository_file": "scripts/model.R",
  "archive_file": "scripts/model.R",
  "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "milestone": "Milestone 3",
  "round": {
    "round": 1,
    "approval": { "round": 1, "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                  "by": "alice", "at": "2026-06-02T11:00:00Z" },
    "superseded": true
  }
}
```

**§0.7** is what this entry costs today: the R1-era archive is unreproducible once R3
approves, because no round is addressable. Here it is addressed, and flagged as not the
newest QC state — without asserting anything false about round 2.

The same shape with `superseded: true` arises from **S3** clause 3 — `r == n`, closed, and
the trailing gap contains a file-changing commit (the **§0.5** *changes-after-approval*
case). Per **R2** the bytes are still the **approval**, not the drifted working tree; the
difference from today is that the archive now says so.

### 8.3 Never approved, archived at its latest actioned commit (S1 row 3)

Thread: one round, open, anchored at `111…`, one notification naming `ddd…`, later drift
`fff…` that nobody acted on. Selection: default (r = n = 1). **S1** row 3 ⇒ **M4**'s
latest actioned commit = `ddd…`; **S3** clause 2 ⇒ superseded.

```jsonc
{ "mode": "issue", "issue_number": 44, "round": null }
```
```jsonc
{
  "repository_file": "scripts/new.R",
  "archive_file": "scripts/new.R",
  "commit": "dddddddddddddddddddddddddddddddddddddddd",
  "milestone": "Milestone 3",
  "round": { "round": 1, "approval": null, "superseded": true }
}
```

`commit` is `ddd…`, **not** `fff…`: **S6**'s behaviour change, and the reason **A3** puts
`latest_actioned_commit` on the round segment so the UI can label the default without
re-deriving it. `approval: null` is present-and-null (§5.3), and `superseded: true`
follows from **S3** clause 2 — the `approval == null ⟹ superseded == true` implication.

### 8.4 Mode 2 — a manually added file (`qc: null`)

A file in no milestone; the user picked the commit (**D4**/**R1**).

```jsonc
{ "mode": "file", "repository_file": "scripts/helpers.R",
  "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" }
```
```jsonc
{
  "repository_file": "scripts/helpers.R",
  "archive_file": "scripts/helpers.R",
  "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
}
```

**Neither `milestone` nor `round` appears** — `qc: None` flattens to zero keys (verified,
§5.1). No `"round": null` and no `"approved": false`: a manually added file carries no QC
claim at all, which is the honest shape and the one **M5** pins. Note that
`ArchiveTab.tsx:438` sends `approved: false` here today, which is both a false claim and
§1.1's live 400.

---

## 9 Spec tensions and known gaps

Recorded, not silently resolved. Each pins the shape that satisfies the spec's intent.

1. **A1's "mode-1 request becomes `{ issue_number, round }`" describes a bigger delta
   than it sounds, because `issue_number` does not exist today.** The real baseline
   (`requests.rs:284-289`) is `{ repository_file, commit, milestone?, approved? }` — there
   is no issue number anywhere on the request, and the UI keys mode-1 entries by
   `s.issue.title` (`ArchiveTab.tsx:421`). So **A1** removes three fields and adds one, and
   the server must newly resolve issue → thread inside the handler; today `generate_archive`
   never touches an `IssueThread` (`routes/archive.rs:96-157` builds `ArchiveFile` by hand
   and never calls `from_issue_thread`, unlike the CLI at `cli/archive.rs:154`). Pinned as
   **A1** intends: mode 1 carries `issue_number` + `round` and nothing else. Consequences
   for the implementer, both wire-visible: the handler must construct threads (a network /
   cache read the endpoint did not previously perform, so the 404 and 502 rows of §2.5 are
   new for this endpoint), and the 400 messages keyed on `repository_file.display()`
   (`routes/archive.rs:113,127,138`) must key on the issue number for mode 1.
2. **S5's "explicit override" has no wire representation in A1–A4, and I have not invented
   one.** **S5** says an unplaceable active segment *blocks generation with an explicit
   override*; **A1**–**A4** add no request flag and no response field, and §8 scopes **P2**
   to exactly `A1`–`A4`. Pinned reading: the block and its override are **pre-flight**, on
   the surface that builds the request (**U8** has `placement.reason` from **A3**; **C3**
   has the same in the prompt), and the wire stays as §2/§3 describe — a request naming an
   unplaceable thread is rejected per **I4**, since no commit resolves and **§0.8** admits
   nothing else. A user who overrides in order to *include* such a file already has a wire
   path: add it as a **mode-2** entry with an explicit commit, which is exactly what mode 2
   is (**D4**). **If the spec owner intends a server-side override** — a request flag, or a
   409 listing blocked files — that is an additive request/response change and must come
   back through this document. Escalating rather than designing.
3. **A2's completeness claim needed checking against the code, and it holds — but the
   field's *absence* is load-bearing on the response side too.** §2.4 audits the
   post-**A1** request exhaustively. Separately, **U7** deletes `isApprovedStatus`
   (`ArchiveTab.tsx:53`), `archiveCommitOf` (`:71`), `addedFileCommitOf`'s status branch
   (`:84-88`) and `milestoneFileSets.approvedOnly` (`:131-138`, which partitions on
   `issue.state === 'closed'` — **§0.6**). Those are frontend deletions (**P3**), not wire
   shapes, but they are the only remaining producers of an approval claim on this boundary;
   if any survives, **A2** is cosmetic.
4. **A3 says "no new status-endpoint fields are required" and then adds one.** The two
   halves of **A3** read as a contradiction in isolation. Resolved as written in the
   second sentence: `latest_actioned_commit` **is** projected. The first sentence is about
   the *picker and the default* being derivable from `segments`; the addition exists so the
   UI need not re-derive **M4** (**D6**). §4 pins the addition. No other status field
   changes.
5. **The parent brief asked for `latest_actioned_commit` to be "null for gaps"; the wire
   makes it *absent* for gaps.** §4.1 gives the reasoning (a gap cannot own an actioned
   commit, and the union's style is absent-not-null for single-variant keys). If the spec
   owner wants a literal `"latest_actioned_commit": null` on `GapSegment`, that is a
   one-line change to `GapSegment` + its schema + its TS interface and needs saying; I have
   not done it, because it would be the first nullable key on the wire whose null means
   *undefined by construction* rather than *unresolvable*.
6. **`ArchiveMetadata` has no schema version field, and this contract does not add one.**
   **D5**/**R8** make this a hard swap with no external reader, so a version field would be
   a claim nothing consumes. Noted because a post-**A4** file is *silently* incompatible
   with a pre-**A4** reader: `approved` vanishes and `round` appears, with no marker. If a
   reader ever appears, versioning is a new wire discussion.

---

## 10 Consumers to migrate

Grepped, not guessed, at `03498d0`. Every site that reads or writes a field this contract
changes.

### `approved` — request side (**A2**)

| Site | What breaks |
|---|---|
| `src/api/types/requests.rs:288` | field deleted |
| `src/api/routes/archive.rs:134-146` | the `(milestone, approved)` pairing check is deleted with the fields; mode discrimination moves into the deserializer (§2.1) |
| `ui/src/api/archive.ts:7` | field deleted from the TS type |
| `ui/src/components/ArchiveTab.tsx:423` | `approved: isApprovedStatus(s)` — **§0.1**'s exact line |
| `ui/src/components/ArchiveTab.tsx:438` | `approved: false` on every added file — §1.1's live 400 |
| `openapi/openapi.yml:2872-2875` | property deleted |

### `milestone` / `repository_file` / `commit` — mode-1 request side (**A1**)

| Site | What breaks |
|---|---|
| `src/api/types/requests.rs:285-287` | `repository_file`, `commit`, `milestone` become mode-2-only |
| `src/api/routes/archive.rs:103-132` | commit parsing and `archive_file` derivation run for mode 2 only; mode 1 goes through `ArchiveFile::from_issue_thread` (**M3**) |
| `ui/src/api/archive.ts:3-8` | replaced by the tagged union (§2.6) |
| `ui/src/components/ArchiveTab.tsx:414-425` | mode-1 entries become `{ mode, issue_number, round }`; `archiveCommitOf` disappears with **U7** |
| `ui/src/components/ArchiveTab.tsx:427-439` | mode-2 entries gain `mode: 'file'`, lose `approved` |
| `openapi/openapi.yml:2857-2875` | schema replaced (§2.7) |
| `ui/tests/archive/flatten.spec.ts:646,691` | assert `bodies[0].files.map(f => f.repository_file)`; mode-1 entries no longer carry a path — assert on `issue_number` |
| `ui/tests/archive/flatten.spec.ts:143-147` | `captureArchiveRequests` types bodies as `ArchiveGenerateRequest`; compiles against the new union with no change, but every literal it asserts moves |

### `ArchiveQC.approved` — metadata side (**A4**/**D5**)

| Site | What breaks |
|---|---|
| `src/archive.rs:15-19` | `ArchiveQC` re-shaped; `RoundProvenance` / `Approval` added |
| `src/archive.rs:56-96` | `from_issue_thread` gains **M3**'s `target: ArchiveTarget` and returns provenance instead of a bool; `last_approved_commit()`-then-`latest_commit()` (`:63-71`) is replaced by **S1** |
| `src/api/routes/archive.rs:14` | imports `ArchiveQC` directly to hand-build it; mode 1 must stop doing that (**D6**) |
| `src/api/routes/archive.rs:141-144` | writes `ArchiveQC { milestone, approved }` |
| `src/cli/archive.rs:105` | `--approved-only` prompt filters on `last_approved_commit().is_some()` — **§0** predicate #1, narrowed by **S4** |
| `src/cli/archive.rs:154` | `from_issue_thread(&thread, flatten)` — new arity (**M3**) |
| `src/main.rs:1144-1146`, `1170-1172`, `1201-1203` | three copies of the same filter + `from_issue_thread` call (**§0** predicate #2); **C2**'s `--round` map arrives here |
| `src/archive.rs:348-360`, `:479-527`, `:611-623` | six test fixtures and two named tests (`test_archive_file_from_issue_thread_approved` / `_not_approved`) assert on `qc.approved` |
| `src/archive.rs:660` | round-trips the metadata through `serde_json::from_str::<ArchiveMetadata>` — the one deserializing reader, and the reason all three new types need `Deserialize` |
| `CHANGELOG.md` (v0.8.0, Unreleased) | **D5**/**C5** require the entry |
| `docs/milestone-archive.md:69,83` | `--include-unapproved` wording, narrowed by **S4** (**C5**) |

### `RoundSegment.latest_actioned_commit` — additive (**A3**)

Additive, so nothing *breaks*; these are the sites that must learn the new key.

| Site | What to do |
|---|---|
| `src/api/types/responses.rs:782-818` | add the field; `src/api/types/responses.rs:879-896` (`SegmentInfo::project`'s Round arm) populates it |
| `src/round.rs:285-291` | `newest_event_commit` is the existing half of **M4**; the new accessor wraps it with `unwrap_or(opened_at)` and the `is_placed` gate |
| `src/issue.rs:325-338` | `next_notification_from`'s Round branch is **M4**'s identity — call the new accessor so the two cannot drift (**M4** says to note the identity in both places) |
| `ui/src/api/rounds.ts:106-140` | add the field to `interface RoundSegment` (§4.2) |
| `openapi/openapi.yml:1403-1415` | add to `required` and to `properties` (§4.3) |
| `ui/tests/fixtures/rounds.ts:76-110` | `roundSegment()` must default the new key, or every fixture fails to type-check; `closeRound` (`:152`) and `approvedRoundFields` (`:265`) build round segments through it |
| `src/api/types/responses.rs:1910-1918` | the `project()` test helper — where §4.4's assertions go |

No Rust YAML case under `src/api/tests/cases/` asserts a segment shape (checked: the only
`segments` hits are path-traversal fixtures), so none needs updating.

---

## 11 Correction — §2.5's error-body statement is superseded

**§2.5 above states that a malformed-`mode` or unknown-field rejection returns axum's
plain text, and that normalizing it "is a wire change … and must come back through this
document." It came back through the document and was authorised.**
`design/archive-rounds.md` **§15.2** requires every client error on
`POST /api/archive/generate` to carry the `{"error": <message>}` envelope, including
axum's own `JsonRejection`.

As implemented:

- **Statuses are unchanged** from §2.5's table — axum's own status is preserved (422 for a
  data error, 400 for malformed JSON, 415 for a bad content type). Only the **body shape**
  changed.
- **serde's message is preserved verbatim** inside the envelope, because it names the
  offending field (`files[0].mode: unknown variant \`bogus\``) and that is the part a
  client can act on.
- `415` is now enumerated on the endpoint alongside `400`, `404`, `422`, and `500`.

Read §2.5's status table as current and its body-shape sentence as withdrawn. A reader who
follows §2.5 literally will write a client that parses plain text for two error classes and
JSON for the rest — the inconsistency **§15.2** exists to remove.

---

## 12 `RoundSegment.archive_preview` (archive-rounds §26)

Additive, nullable object on **`RoundSegment` only** — never on `GapSegment`, which cannot
be selected for an archive. Emitted after `latest_actioned_commit`, before `placement`.

```jsonc
{
  "index": 2,
  "commits": [ /* … */ ],
  "latest_actioned_commit": "ddd…",
  "archive_preview": {
    "commit": "aaa…",
    "approval": { "round": 1, "commit": "aaa…", "by": "wes", "at": "2026-07-04T18:22:09Z" },
    "superseding_causes": ["round_open"]
  },
  "placement": { "kind": "placed" }
}
```

| Field | Type | Null? | Meaning |
|---|---|---|---|
| `archive_preview` | object | **yes** | What archiving **this** round would produce. `null` ⟺ `placement.kind == "unplaceable"` — the round cannot be archived (archive-rounds **§18.1**/**§20.2**), and the reason is already in `placement.reason`. |
| `.commit` | string (40-hex) | no | The commit that would be archived — **S1**'s three rows: this round's `closing_commit` when closed, its `latest_actioned_commit` when open. **Not** necessarily `commits[0]`. |
| `.approval` | object | **yes** | `null` ⇒ those bytes were never approved. Non-null ⇒ they are a round's closing commit. |
| `.approval.round` | number | no | **May be LESS than the enclosing `index`** — see §6.1. Read as "you are on round N; these bytes are round M's approval". Never render the enclosing round as approved on the strength of this field. |
| `.superseding_causes` | array of enum | no | Possibly **empty**. Each of `later_approval`, `round_open`, `changed_since`, `undeterminable`. |

**There is no `superseded` boolean here, by decision (§26.3).** `superseded` is
`superseding_causes.length > 0`. Two fields for one fact is the defect this contract's
parent spec exists to remove; a client that wants the bool computes it.

**Cause vocabulary**, mapping to **S3**'s clauses:

| Value | Fires when |
|---|---|
| `later_approval` | a round **after** the previewed one has closed |
| `round_open` | the thread's latest round is open |
| `changed_since` | the previewed round is the latest, is closed, and its trailing gap holds a file-changing commit |
| `undeterminable` | a segment after the previewed round is `Unplaceable`, or is a Gap owning no commits **because its continuity is `Unrelated`** (**§20.1** — not merely any empty gap) |

Causes are **not** mutually exclusive; `later_approval` and `round_open` co-occur routinely.
Order is the clause order above, so a client may render the array verbatim.

**Relationship to the metadata file.** `ghqc_archive_metadata.json` keeps its plain
`superseded` **bool** and gains **no** cause breakdown — archive-rounds **D10** forbids it
there and **§26.4** keeps that intact. This projection is a *selection-time* surface; the
metadata is an *evidentiary* one. The bool in the metadata and the emptiness of this array
must agree for the same thread, round and moment.

**Two clarifications a client implementer will want (archive-rounds §27.2, §20.2).**

1. `archive_preview == null` means *this round cannot be archived*. It does **not** mean the
   round is unknown — the segment is right there in the array. Server-side, the distinction
   between "cannot be archived" and "no such round" is answered by a different function
   (`selected_round()`), which is why a client-supplied round number is validated by the
   request path and rejected with its own 400, not by reading a null preview.
2. A round may legitimately carry a non-null `closing_commit` **and** a null
   `archive_preview`. That is the **§20.2** shape: the round closed at a real, identified
   sha, but its anchor is not on any walked branch, so the commit cannot be confidently
   extracted and the round is refused. Do not treat `closing_commit != null` as proof a
   round is archivable — `archive_preview != null` is the only such proof.
