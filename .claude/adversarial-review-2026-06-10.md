# Adversarial Review: ghqctoolkit

_Date: 2026-06-10 · Branch: main @ 7ba49f12 · Reviewer: Claude Code (6 parallel reviewers + verification)_

Six parallel reviewers swept the backend (core domain, git layer, API, CLI/record,
supporting modules) and the React frontend; significant findings were verified by
reading callers and, for the diff/archive logic, by executing extracted code. The
codebase is generally solid — Kahn's-algorithm topo-sort, cycle detection, the
AES-GCM auth design, the disk-cache validation logic, and most async/state handling
are correct. The problems cluster into five themes.

**Recommended triage order:** (1) the network-exposure cluster — at minimum bind to
loopback and add `sandbox` to the preview iframes; (2) the audit-record omissions in
`diff_utils`/Excel/config, which silently corrupt the tool's core compliance output;
(3) the UTF-8 byte-slicing panics — a one-line `is_char_boundary`/`.char_indices()`
fix each, but they crash on real GitHub content.

---

## Theme 1 — Network-exposure security cluster (most serious)

These compound: the server is reachable off-box, and several endpoints are dangerous
once reachable.

1. **Server binds all interfaces, not loopback** — `src/api/server.rs:171-177`.
   `bind_dual_stack_ipv6`/`bind_ipv4` use `Ipv6Addr::UNSPECIFIED`/`Ipv4Addr::UNSPECIFIED`
   (`::` / `0.0.0.0`), so it listens on every interface — yet `local_server_url` prints
   `http://127.0.0.1:…` and the CLI help calls it a "loopback URL." Anyone on the same
   host/LAN can reach a server holding the user's GitHub token. The displayed URL
   actively misleads the operator about exposure.

2. **`generate_record` is an arbitrary file-write primitive** —
   `src/api/routes/record.rs:243-261`. Absolute `output_path` is written verbatim;
   relative `../../../etc/foo` traverses out of the repo; it `create_dir_all`s the
   parent. The *sibling* archive route (`src/api/routes/archive.rs:31-75`) deliberately
   rejects `..`, canonicalizes, and checks repo containment — record simply omits all
   of it. Confirmed by direct read.

3. **`context_files.server_path` → arbitrary server-file read/exfiltration** —
   `src/api/routes/record.rs:143-153`. Any readable server path can be named as a
   "context PDF," embedded into the generated PDF, then downloaded.

4. **Stored XSS via unsandboxed `srcDoc` iframes** — `IssueDetailModal.tsx:414`,
   `CreateIssueModal.tsx:501`, `RelevantFilePickerModal.tsx:614`,
   `UnapproveSwimLanes.tsx:336`. Preview HTML comes from `pulldown-cmark` with
   `Options::all()` + `push_html` and **no sanitizer** (no `ammonia` in the tree); raw
   embedded HTML passes through. A `srcDoc` iframe runs same-origin, so
   `<img src=x onerror="fetch('/api/...')">` in checklist/comment/note/path content
   executes against the API.

5. **Wildcard CORS** — `src/api/server.rs:38-41`, `allow_origin/methods/headers(Any)`.
   There's an explicit code comment that this is intentional for local dev — but
   combined with #1 it's a real DNS-rebinding/CSRF vector against the mutating POSTs.
   Worth at least an Origin allowlist.

6. **Lower-tier**: path sanitizer in `files.rs:44` splits only on `/` (misses `..\` and
   symlinks); state-changing one-shot `GET` deletes preview artifacts (`record.rs:200`,
   breaks on prefetch/reload); internal errors leak absolute server paths to clients
   (`api/error.rs:32`).

## Theme 2 — Silent data omission in the QC audit record (integrity-critical)

For a compliance tool, a diff that silently drops a change is worse than a crash.

7. **Final-hunk trim drops real change lines** — `src/diff_utils.rs:447-471` (HIGH,
   reproduced by execution). When the last change sits 4–5 unchanged lines from a prior
   change near EOF, the backward context count cuts at an *internal* context run and
   `truncate`s away trailing `+`/`-` lines. Feeds `QCReview::generate_body` and
   `PreviousQCDiffComment` — the audit diff omits a genuine change.

8. **Excel diff caps at row 20 / column Z** — `src/diff_utils.rs:158,183`.
   Identical-dimension workbooks differing at row 21+ or column AA+ return an
   affirmative *"No differences between Excel file versions."* — not a truncation
   notice. Parameter tables routinely exceed 20 rows.

9. **Trailing-newline / CRLF↔LF reported as "No difference"** — `src/diff_utils.rs:317`
   uses `.lines()` for the early-equal check, which strips `\r` and the final newline;
   genuinely different blobs report identical.

10. **YAML checklist items silently dropped** — `src/configuration.rs:371`.
    `filter_map(|i| i.as_str())` discards any non-string item: `- 42`, `- true`, or the
    common `- Check: verify against spec` (parses as a mapping) vanish from the rendered
    checklist — a QC step never presented.

11. **Typst `//` strips user text from the record** — `src/record/typst.rs:200`.
    `escape_typst_inline_text` doesn't neutralize `//` (a Typst line comment), so
    "works either/way // still need sign-off" loses everything after `//`. Lines
    beginning `=` become headings polluting the outline.

## Theme 3 — UTF-8 byte-slicing panics (repeated pattern, untrusted input)

The same `len() > N` byte-check followed by `&s[..M]` byte-slice appears in several
places; any multibyte char straddling the cut panics on routine GitHub content
(accents, em dashes, CJK, emoji):

- `src/record/typst.rs:384-404,425-466` — code-fence line wrapping, **crashes record
  generation** (HIGH).
- `src/cli/interactive.rs:574`, `src/cli/archive.rs:457` — 50-byte commit-message
  truncation (duplicated snippet).
- `src/issue.rs:640-649` — the CRLF offset bug: `pos += line.len()+1` assumes `\n`, but
  `.lines()` strips `\r\n` (2 bytes), so for browser-edited bodies the splice point
  drifts and can land mid-codepoint on the `→` the tool itself writes — panicking, or
  corrupting the body written back to GitHub on rename (`cli/rename.rs:238`,
  `api/routes/issues.rs:483`).

## Theme 4 — Parsing fragility on user-controlled issue/comment text

12. **Approval state corrupted by quote-replies** — `src/issue.rs:329-386`. State keys
    off bare substrings (`approved qc commit: `, `# QC Un-Approval`) over *every*
    comment from *any* user with no author/format filter. A GitHub "quote reply"
    (`> approved qc commit: <sha>`) matches and can re-mark an issue Approved or strip a
    valid approval. For a QC tool this is fabricated/destroyed approval state on
    plausible input.

13. **`expect()` panics on missing initial/latest commit** — `src/issue.rs:299,316`.
    `initial_commit()`/`latest_commit()` assume the walked commit set contains
    Initial/non-empty-status entries, which a force-push or un-approval can violate —
    panicking inside the API server (`responses.rs:152`), record, archive, and preview.

14. **Blocking-QC regex matches links inside descriptions** — `src/issue.rs:22,472`.
    `captures_iter` over the whole subsection picks up issue links in free-text
    descriptions, fabricating phantom blocking QCs that then block approval until
    `--force`.

15. **qc_status counts fenced code blocks as checklist items** —
    `src/qc_status.rs:225-331`. No code-fence awareness, so a `- [ ]` or `# comment`
    inside a ``` example inflates the denominator; completion % is wrong and can never
    reach 100%.

16. **CLI substring/state-match bugs** — `find_issue` uses `title.contains(file)` so
    `--file model.R` can hit `model.Rmd` and approve the wrong issue
    (`src/cli/context.rs:949`); the non-interactive `unapprove` state check is
    inverted/dead so it can never target a closed-approved issue (`context.rs:739`);
    `prompt_commits` first-selection omits the `📝` marker in its `skip_while`, silently
    selecting `commits[0]` (`interactive.rs:676`).

## Theme 5 — Data-loss & cache correctness (medium)

17. **`archive()` truncates the destination before fetching contents** —
    `src/archive.rs:185`. `File::create` truncates up front; any mid-loop error (incl.
    #18) leaves a corrupt partial `.tar.gz` and destroys the prior archive. The API
    route's containment check still allows `output_path: "src/analysis.R"`, overwriting
    a tracked source file with a tarball. Should write-temp-then-rename.

18. **Archive fails on paths >100 bytes** — `src/archive.rs:213` (reproduced). Manual
    GNU tar header + `set_path` bypasses long-name extension; nested pharma paths
    routinely exceed 100 chars, aborting mid-archive (→ #17).

19. **`.expect("File to have file name")` panics on issue title `..`** —
    `src/archive.rs:67`. Title is arbitrary user text; one malformed title kills the
    whole CLI archive run (the API route handles this; the library path doesn't).

20. **Cache issues** — `src/cache.rs`: comment cache keyed only on `issue.updated_at`,
    which GitHub does *not* bump on comment edits → stale approval/review bodies served
    indefinitely (`:364`); "atomic" write uses a fixed `.tmp` name so concurrent
    same-key writers can rename a torn file (`:165`), and `file_ops.rs:727` has a
    lost-update read-modify-write; unbounded, never-pruned growth.

## Git shell-out layer (PR #116) — mostly clean, some hardening gaps

No shell/`sh -c` injection (all arg-vector `Command`); tokens not logged. But:
`clone()` omits the `GIT_TERMINAL_PROMPT=0` its siblings set, so it can hang/prompt on
the server (`git/action.rs:113`); branch/ref from unvalidated issue-body text is passed
as a positional revision with no `--end-of-options`, allowing `git log` flag-injection /
wrong output (`action.rs:196,251`); rename detection breaks under default
`core.quotepath` for non-ASCII paths (`git/status.rs:367`); stash no-op detected via
English-string match with no `LC_ALL=C` (`action.rs:236`).

## Notable quality items

- ~250 lines triplicated across `NotifyTab`/`ReviewTab`/`ApproveTab` commit sliders
  (`IssueDetailModal.tsx`); `escape_typst`/`escape_typst_inline_text` are byte-identical
  duplicates (`record/typst.rs:55` vs `:200`).
- `fetchIssueStatuses` ignores `res.ok` and parses unconditionally → raw `SyntaxError`
  on 500s, retried 3× (`api/issues.ts:202`); debug `console.log`s shipped
  (`RelevantFilesTab.tsx:31`).
- Dead/inverted code: `CreateResult.parse_failed` never set true (`create.rs`);
  `bail!("")` empty error (`context.rs:627`); `set_extension(".pdf")` → `report..pdf`
  (`main.rs:898`); `render.rs:338` drops the `TempDir` guard immediately (deletes the
  dir before use). `serde_yaml 0.9` is archived/unmaintained.
