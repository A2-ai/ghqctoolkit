# Milestone: Archive

```shell
ghqc milestone archive
```

Generates a gzipped tar archive (`.tar.gz`) for one or more milestones, bundling each QC'd file at the commit its selected round points at, plus a metadata file recording where every archived file came from.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select Milestones

Choose all milestones, specific ones, or none (to archive only files you name yourself).

```shell
📦 Welcome to GHQC Milestone Archive Mode!
? 📦 How would you like to select milestones for the archive?
  📋 Select All Milestones
> 🎯 Choose Specific Milestones
  🚫 Select No Milestones
```

```shell
? 📦 Include open milestones? (y/N)
? 📦 Select milestones for the archive:
> [x] Milestone 1
  [ ] QC Round 2
  [ ] EDA
```

### 2. Choose Which Files

Files that have **never** been approved — no round on them has ever closed — are skipped by default:

```shell
? ✅ Skip files that have never been approved? (Y/n)
  Y = skip files no round has ever closed on, n = include them. A file approved in an earlier
  round is kept either way; which round it is archived at is asked separately
```

The prompt governs only never-approved files. A file that *was* approved and is now under review again is always included; which round it is archived at is the next step's decision.

If the round selected for any file has no locatable commits, the files are named and you are asked whether to go on without them — see [Files That Cannot Be Archived](#files-that-cannot-be-archived).

```shell
? 🚫 Proceed without those file(s)? (y/N)
  N = stop here, y = archive the remaining files and leave those out
```

### 3. Choose a Round per File

By default every file is archived at its **latest round**. When any selected file has more than one round, you are asked once whether you want to choose; answering `n` keeps the default for everything.

```shell
? 🎯 Choose which round each file is archived at? (y/N)
```

Answering `y` prompts only for the files that have more than one round, showing what each round would archive:

```shell
? 🎯 Round to archive scripts/analysis.R at:
  Initial QC · closed · approved by @alice on 2026-06-02 · aaaaaaa
> Round 2 · open · latest actioned commit ddddddd
```

### 4. Select Additional Files

Optionally add files from the repository at a commit you pick yourself. These carry no QC status, and no QC claim is recorded for them.

```shell
? 📄 Select additional files? (y/N)
```

### 5. Name the Output File

Choose whether to keep the repository's directory structure, then the output path. Press Enter to accept the default.

```shell
? 📁 Flatten archive directory structure? (y/N)
? 📁 Enter archive path: archive/my_analysis-Milestone-1.tar.gz
  Press Enter to use the default path shown above
```

### 6. Archive Generated

Before the archive is written, every file reports where its bytes came from: the round the selection addressed, whether those bytes were approved and by whom, the commit, and whether they were still the newest QC state when the archive was cut. The block ends with the same one-line summary [`ghqc milestone status`](milestone-status.md) prints, from the same categorization, so the pre-archive check and the archive cannot disagree.

```shell
── Archive contents ──────────────────────────
  scripts/analysis.R
    Round 2 · approved by @wes on 2026-07-04 · aaaaaaa
  scripts/model.R
    Initial QC · approved by @alice on 2026-06-02 · bbbbbbb · ⚠️ not the newest QC state
  scripts/new.R
    Round 3 · unapproved · ddddddd · ⚠️ not the newest QC state

  3 files · 1 approved & current · 1 approved but superseded · 1 unapproved (round 3 open)

⚠️  1 file(s) are archived at unapproved bytes: the round selected for them is open. This is
the default — the latest round, not the newest approval. Target an earlier round to archive
the approval that stands there.

✅ Archive successfully created at archive/my_analysis-Milestone-1.tar.gz
```

The `⚠️` block appears whenever any file is archived at unapproved content. It repeats in aggregate what the per-file lines already say, because that is the deliberate behaviour change of this release and a single marker is easy to miss across sixty files. It does **not** stop the archive: nothing about the default target is gated.

## Which Commit Is Archived

The commit a file is archived at follows from the round the selection addresses — it is never chosen separately.

| Selected round | Commit archived |
|---|---|
| An earlier round | that round's approval commit |
| The latest round, closed | that round's approval commit, even if the file has changed since |
| The latest round, open | that round's latest **actioned** commit — its starting commit, a notification, or a review |

Two consequences worth knowing before you cut an archive:

- **The default is the latest round, not the newest approval.** A file that was approved and then had a new round opened on it archives **unapproved** content by default, because there is a reason the round is open and the archive shows current reality first. Nothing blocks this and nothing asks you to confirm it — but every such file is labelled `unapproved` in the report above. To archive the approval that still stands, target the earlier round: `--round <issue#>=<n>` non-interactively, or the round prompt interactively.
- **Only the round you selected has to be locatable.** A file whose selected round is placed is archived even when a later part of its history **is not provably current** — it could not be read, or it spans histories with no common ancestor. Either way the approval you asked for is a real commit, so what is unestablished costs *currency*, not archivability, and is recorded as `superseded` in the metadata instead.
- **A file with no approval is archived at its latest *actioned* commit.** Previously it was archived at the newest commit on the branch, whether or not anyone had ever put that commit up for review. Commits nobody acted on are no longer archived: an archive captures the repository at a reviewed period, and drift nobody looked at was never part of one.

## Files That Cannot Be Archived

If the round selected for a file has no locatable commits — its branch is not available locally, or its commits are not on that branch — there is no commit the archive could honestly point at, so the file is named and generation stops:

```shell
🚫 1 file(s) cannot be archived — the round selected for each owns no locatable commits:
   - scripts/analysis.R (#42, Round 2): its branch is unavailable locally
   Proceeding leaves these files out of the archive entirely. To archive one anyway, add it at a commit you name with `--additional-file <path>:<commit>`.
```

Acknowledging this — `y` interactively, `--skip-unplaceable` non-interactively — archives the remaining files **without** these ones. It never includes them. If you know the commit you want, add the file directly with `--additional-file <path>:<commit>`.

## Non-interactive Usage

Pass milestone names as positional arguments or use a milestone selection flag to skip interactive mode.

```shell
# Specific milestones
ghqc milestone archive "Milestone 1" --archive-path archive/m1.tar.gz

# All closed milestones, flattened structure
ghqc milestone archive --all-closed-milestones --flatten

# Add specific files at specific commits
ghqc milestone archive "Milestone 1" --additional-file scripts/file_1.qmd:00eadb9b

# Archive issue #42's file at Initial QC and issue #43's at round 2; everything else at its latest round
ghqc milestone archive "Milestone 1" --round 42=1 --round 43=2
```

A `--round` naming a file that the never-approved filter then drops is reported rather than silently unused, since the round you asked for would never have been applied:

```shell
⚠️ --round names issue #42 (scripts/new.R), which no round has ever closed on: it is left out
without --include-unapproved
```

`--round` also requires a milestone selection. With no milestone named there is no issue to retarget, so the flag is refused instead of being ignored.

| Argument / Flag | Description |
|---|---|
| `[milestones...]` | Milestone names to include (positional, repeatable) |
| `--all-milestones` | Include all milestones (open and closed) |
| `--all-closed-milestones` | Include only closed milestones |
| `--include-unapproved` | Include issues **no round has ever closed on**. A file approved in an earlier round is included either way — which round it is archived at is `--round`'s job, not this flag's |
| `--round` | Round to archive one issue's file at, format `issue#=round`, 1-based with `1` being Initial QC (repeatable). Unlisted issues are archived at their latest round. Requires a milestone selection — with no milestone named there is no issue to retarget, and the flag is refused rather than ignored. Naming an issue outside the selected milestones, or a round the issue does not have, is also an error |
| `--skip-unplaceable` | Archive the remaining files when a selected round has no locatable commits, instead of stopping. Never archives such a file |
| `--flatten` | Put all files in the archive root directory (no subdirectory structure) |
| `-a, --archive-path` | Output file path (default: `archive/<repo>-<milestones>.tar.gz`) |
| `--additional-file` | Extra file to include at a specific commit, format: `file_path:commit` (repeatable) |

## Archive Contents

The `.tar.gz` archive includes:
- Every QC'd file from the selected milestones, at the commit its selected round points at
- Any additional files you named, at the commit you picked for each
- `ghqc_archive_metadata.json`, describing where every archived file came from

It does **not** include the PDF record; generate that separately with [`ghqc milestone record`](milestone-record.md).

## Archive Metadata

`ghqc_archive_metadata.json` sits at the root of the archive and records, per file, the commit its bytes were read at and — for a file under QC in a milestone — the round that selection addressed.

```json
{
  "metadata_version": 1,
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

| Field | Meaning |
|---|---|
| `metadata_version` | Version of this **file structure**, not of ghqc. `1` is the shape above; an absent field means the older, pre-round shape (version `0`). A reader accepts exactly the versions it can interpret — today only `1` — and refuses every other value, **higher or lower**, with a named error rather than reading it best-effort. Version `0` is therefore refused too: see [Reading an Older Archive](#reading-an-older-archive) |
| `round.round` | The round the selection addressed. **Not** a claim that this round was approved |
| `round.approval` | The round that closed on the archived commit, with who approved it and when — or `null` when the archived bytes were never approved. Its `round` can be *lower* than `round.round`: selecting an open round whose starting commit is the previous round's approval archives approved bytes under a later round's frame, and both numbers are recorded so neither reading is lost |
| `round.superseded` | `true` when it is **not provable** that nothing newer exists than these bytes. **The four causes, in full — this is the only place they are listed:** (1) a round later than the selected one had closed; (2) the latest round was open, which includes a file approved in an earlier round and now under review again; (3) the selected round was the latest, was closed, and the file changed after that approval; (4) currency could not be determined at all, because a segment after the selected round could not be located or spans histories with no common ancestor. `false` is therefore a positive claim: the latest round's approval, with no file changes since and nothing unreadable after it, as of `created_at`. A pointer to go read the thread, not a finding |

A manually added file (one in no milestone) carries **neither** `milestone` nor `round`: it was never under QC, so the file makes no QC claim about it at all.

The `approved: true`/`false` field that earlier versions wrote is **gone**, replaced by `round.approval` and `round.superseded` — see the [changelog](../CHANGELOG.md).

### Reading an Older Archive

`ghqc_archive_metadata.json` from an archive written **before** this release is **refused**, with an error naming the version found and the version this build reads. It is not read best-effort.

That is the safe outcome, not a gap. The QC block (`milestone` + `round`) is a flattened optional field, so an old document does not fail to parse — it parses *successfully* with that block absent, and every QC'd file in the archive reads back as a manually added file with no milestone and no approval. A false negative on approval inside an audit artifact is the one failure this file exists to prevent, so the reader refuses the shape rather than misreporting it.

In practice this costs nothing today: ghqc never reads archive metadata back, so no command or workflow depends on it. The gate exists so that a future reader — yours or ghqc's — cannot silently misread an old file. The archived files themselves are unaffected: an archive is an ordinary tarball, and only its metadata file is version-gated.

## See Also

- [`ghqc milestone record`](milestone-record.md) — generate only the PDF record
- [`ghqc milestone status`](milestone-status.md) — the pre-archive check, which reports the same readiness summary as this command
