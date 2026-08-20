# Archive under Round Semantics — Divergence Report

Spec: `design/archive-rounds.md` · Contract: `design/archive-api-contract.md`
Baseline: `03498d0` (branch `rounds`) · **Nothing committed** — all work is in the working tree.

24 files changed, **+6881 / −693**. `cargo test --features cli,api` (the literal CI command):
**665 passed, 0 failed**, build warning-free, `cargo fmt --check` clean.

---

## 1. The headline: the spec itself moved

You approved **§0–§10**. What shipped is documented in **§0–§32** — **22 appended sections**.
That is the single largest divergence from what you signed off on, and it breaks down as:

| Category | Count | What it means |
|---|---|---|
| Owner-adjudicated | 3 | You decided it mid-run |
| Orchestrator calls | ~20 | I decided it and recorded it |
| **Spec errors corrected** | **6** | The approved text was *wrong*; the code is right |
| Accepted limits recorded | 8 | Known holes, deliberately not closed |
| Previously-unrecorded decisions ratified | 8 | Shipped without sanction, now spec |

The six corrections matter most, because in each case following the approved text literally
would have produced a defect:

| Approved text | Why it was wrong | §  |
|---|---|---|
| S3 clause 4: "a Gap that owns no commits despite being `Placed`" | Also describes an ordinary empty `Linear` gap — the *normal* approved state — making `superseded: false` **unreachable** for every closed-round selection | §20.1 |
| §18.1: refused when the round "resolves to no commit" | Would archive a round that is `Unplaceable` yet closed at a real sha | §20.2 |
| §18.5: CLI "drops its `partition_placeable`/`UnplaceableSelection` logic" | Read literally, deletes the type carrying the callout data | §22.1 |
| C4: status reports "the same summary as U5" | Implies equal totals, which is impossible — the two commands see different file sets | §23.1 |
| A3: "No new status-endpoint fields are required" | Two were added | §11.2 |
| Clause-4 prose: "part of the history could not be read" | An `Unrelated` gap is `Placed` and *readable* — it spans histories with no common ancestor | §28.2 |

---

## 2. Your three decisions

1. **`metadata_version` exists** (§12.1) — structure version, not tool version; value **1**,
   absent ⇒ **0**, serialized first. You corrected my 1/2 numbering to 0/1 before any code.
2. **§16.1 → option (c)** (§18) — gate on the **selected round**, and assert `superseded: true`
   whenever currency cannot be determined. This closed a real CLI/API divergence (§17.6): the two
   surfaces had drifted to *opposite* answers inside one wave, which is §0.2 reappearing.
3. **§25.1 → option (b)** (§26) — the backend projects `archive_preview` per round; the UI's
   TypeScript re-derivation of S1/I2/S3 is deleted.

---

## 3. What shipped, by layer

**Backend** (`archive.rs`, `round.rs`, `issue.rs`, `lib.rs`) — M1–M5, I1–I5, S1–S3, S6,
`metadata_version`, `selected_round()`, `archive_preview()`. The strongest form of §18.4:
`archive_derivation()` is **one derivation with two shapes**, so the metadata and the preview
cannot disagree — pinned by a test walking every round of 10 fixtures with a `saw_older_approval`
guard so the fixtures can't silently stop exercising the I2 case.

**API** (`api/**`, `openapi.yml`) — A1's tagged `mode` enum (`untagged` rejected after testing
that a mixed request silently discards the commit), A2, A3, A4, D6, the `{"error": …}` envelope on
every rejection class, parallel fetches with request-order errors, `archive_preview` projection.

**CLI** (`cli/**`, `main.rs`, docs) — C1–C5, `--round <issue#>=<n>`, `--skip-unplaceable`,
per-file round picker, provenance report, `milestone status` archive-readiness block. The status
report's caveat is enforced **structurally**: two mutations now fail to *compile* (private fields,
one print site) because tests couldn't reach the wrappers.

**UI** (`ui/**`) — A1/A2 request shape, U1–U8, U4/U5 pulled forward from P5, and all **five** §0
predicates deleted. §0 named four; this run found a fifth (`FileResolveModal.resolvedCommitOf`)
and a sixth that is out of scope (below).

---

## 4. Verification status — read this before trusting the number

| Layer | Verified how |
|---|---|
| Backend / API / CLI | **665 tests**, adversarially reviewed, mutation-proved, cross-surface agreement confirmed empirically (one mutation to the shared predicate breaks both surfaces simultaneously) |
| **UI** | **291/291 Playwright tests pass** (0 failed), incl. 27/27 archive specs; `tsc --noEmit` clean; exhaustive static review |

**The UI suite ran clean: 291 passed, 0 failed** (baseline was 280; this run adds 11). The blocker was never the
port: the day-old `vite dev` binds **IPv6 only**, and `playwright.config.ts` launches its own
`vite preview --host 127.0.0.1`, which binds a *different* socket. It works — provided `CI` is
**unset** (with `CI=true`, `reuseExistingServer:false` makes Playwright refuse the occupied
port outright). My earlier workaround pointed at the stale dev server instead of letting
Playwright build its own, which is why it produced failures in files this run never touched.

**Running it found three things a static review could not**, all in the *new* guard's
instrumentation rather than in the product:

1. The `/api/files/content` intercept was registered **before** `setupRoutes`, so Playwright —
   which runs the most recently added matching handler first — let the `/api/**` catch-all
   shadow it and the handler never fired. Fixed by ordering it after, matching
   `flatten.spec.ts`'s proven pattern.
2. The preview modal was left open, and Mantine's portal overlay swallowed the subsequent
   click on Generate Archive. Fixed with an explicit dismiss.
3. `issue-detail-modal.spec.ts:129` failed three times in one earlier parallel run and
   **passes in isolation** — a load-dependent flake in a spec this run did not touch, not a
   regression. Recorded so it is not mistaken for one later.

**What the guard proved, which is the point of it:** every substantive assertion passed on the
first execution — the card renders the wire's `f9f9f9f`, `Round 7`, `wire-only` and
`changed_since` even though a correct local derivation of S1/I2/S3 would yield `b2b2b2b`,
round 1, `reviewer1` and `[]`. So **§26.6 holds in behaviour, not just by inspection**: the UI
renders the projection and does not re-derive it.

Two caveats on `tsc` stand unchanged: **`ui/tests/` is type-checked by nothing** (§31.3), and
the fixtures' internal consistency is why the guard in §31.1 had to be added at all.

## 5. Accepted limits (deliberate, recorded)

- **§19.2** — the version gate is on the metadata *envelope*, not the file entries. A v1 document
  with a v0-shaped entry parses to `qc: None`. Closing it needs `deny_unknown_fields`, which
  **cannot** combine with the `flatten` A4 requires. Latent: nothing writes hybrids.
- **§19.1** — the typed `UnsupportedMetadataVersion` survives on `from_json`; a direct
  `serde_json::from_str` refuses with the same message stringified. Both refuse.
- **§29.4** — `ArchiveFile`'s `pub` fields leave a *type-level* bypass of §20.3. The design that
  closes it (private `qc` + a single constructor) is recorded.
- **§20.4** — S3 clause 4 is **dormant**: not fold-reachable in isolation today. Recorded so it is
  not deleted as untriggerable.
- **§19.3** — S1 row 1's named error is the **only** I1 enforcement in a release build, since
  segment-spec I6 is `debug_assert`-only.

---

## 6. Found by auditing, not by any spec clause

- **`metadata_version` was `pub`** while every sibling was private — this build could *emit* a file
  it would refuse to *read*. Sealed (§29.1); the new test asserts on **emitted JSON** across all
  three construction paths.
- **`502` and `413` were reachable but unlisted** in `openapi.yml` (§29.2, §29.7). 413 exists
  because `DefaultBodyLimit::max(50 MB)` is applied only to `/record/upload`.
- **A key-order assertion was testing the alphabet** (§27.8) — `serde_json::Value` is a `BTreeMap`,
  and `commits < latest_actioned_commit < placement` is alphabetical by coincidence. Contract §12's
  pinned order was unenforced from the moment it was written.
- **`superseded`'s documentation drifted four separate times** (§28.1, §30.8). Every drift was a
  *partial restatement* of a rule documented in full elsewhere. Now: one enumeration, and every
  paraphrase audited against the code.
- **The docs said "zip"/`.zip`** in four places while the code writes `.tar.gz`, and claimed the
  tarball bundles the PDF record, which `archive()` never writes (§32.4). Pre-existing.

---

## 7. Deferred and still open

| Item | Blocks | Note |
|---|---|---|
| ~~Run the UI Playwright suite~~ | — | **Done.** Run it with `CI` unset: `cd ui && npx playwright test`. Do **not** set `CI=true` while another server holds 3103. |
| **§29.6 — `api/routes/preview.rs:295`** still runs `last_approved_commit() ?? latest_commit()` | Nothing here | A §0-family predicate on the *notification preview* endpoint. Unmodified from baseline, out of this spec's scope — **deserves its own decision** |
| §26.5 — projection carries `commit` + `approval` | Nothing | **My** widening of your §25.1 decision; reversible |
| §32.3 — twelfth prune candidate (`get_milestone_issue_threads`) | Nothing | Meets the criterion; not enumerated, so not removed |
| As-of archive (R12), `docs/` `new-round`/`repair-round` pages | Nothing | Deferred from the original spec |

**Nothing is committed.** Review the working tree, then commit as you see fit.
