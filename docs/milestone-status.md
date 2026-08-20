# Milestone: Status

```shell
ghqc milestone status
```

Prints a tabular summary of all QC issues across one or more milestones — including QC status, git status, and checklist progress for each file.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select Milestones

Choose all milestones or select specific ones.

```shell
📊 Welcome to GHQC Milestone Status Mode!
? 📊 How would you like to select milestones?
  📋 Select All Milestones
> 🎯 Choose Specific Milestones
```

```shell
? 📊 Select milestones to check:
> [x] Milestone 1 (3)
  [x] QC Round 2 (1)
  [ ] EDA (8)
```

### 2. Summary Table Printed

```
File                   | Milestone   | Branch   | Issue State | QC Status          | Git Status | Checklist
-----------------------+-------------+----------+-------------+--------------------+------------+------------
scripts/file_1.qmd     | Milestone 1 | analysis | open        | Changes to comment | Up to date | 0/5 (0.0%)
scripts/file_2.qmd     | Milestone 1 | analysis | open        | Changes to comment | Up to date | 6/8 (75.0%)
scripts/file_3.qmd     | Milestone 1 | analysis | open        | In progress        | Up to date | 3/10 (30.0%)
scripts/file_4.qmd     | QC Round 2  | QC       | closed      | Approved           | Up to date | 15/15 (100.0%)
```

### 3. Archive Readiness Printed

After the table, the same one-line summary [`ghqc milestone archive`](milestone-archive.md) prints before it writes:

```shell
── Archive readiness ─────────────────────────
  12 files · 9 approved & current · 2 approved but superseded · 1 unapproved (round 3 open)
  Counted at each file's latest round — what `ghqc milestone archive` would produce with no `--round` override.
```

| Bucket | Meaning |
|---|---|
| `approved & current` | The file's latest round is approved and nothing newer exists — ready to archive as-is |
| `approved but superseded` | An approval would be archived, but it is **not provable** that nothing newer exists. Deliberately not enumerated here — the causes are listed once, under [`round.superseded`](milestone-archive.md#archive-metadata), and an abbreviated list that looks complete is worse than a pointer |
| `unapproved` | The latest round is open, so **unapproved** content is what an archive would take. A file approved in an earlier round and now under review again counts here, because that is what would be archived; target the earlier round to archive its approval instead |
| `not placeable` | The file's latest round has no locatable commits, so it cannot be archived at all until its branch is available (see [Files That Cannot Be Archived](milestone-archive.md#files-that-cannot-be-archived)) |

The counts come from the same categorization the archive itself uses, so the check and the archive cannot disagree. Two things it deliberately does **not** claim: it speaks only for each file's **latest** round, because this command has no `--round` flag, so a retarget you intend to make at archive time is not reflected here; and it counts nothing about additional files you may add at archive time, which carry no QC status.

## Non-interactive Usage

Pass milestone names as positional arguments or use `--all-milestones` to skip interactive mode.

```shell
# Specific milestones
ghqc milestone status "Milestone 1" "QC Round 2"

# All milestones
ghqc milestone status --all-milestones
```

| Argument / Flag | Description |
|---|---|
| `[milestones...]` | Milestone names to check (positional, repeatable) |
| `--all-milestones` | Check all milestones |

## Columns

| Column | Description |
|---|---|
| File | Repository-relative file path |
| Milestone | Milestone the issue belongs to |
| Branch | Git branch the issue was created on |
| Issue State | `open` or `closed` |
| QC Status | Current QC status (see [Issue: Status](issue-status.md) for values) |
| Git Status | Whether the file is up to date with its tracked remote |
| Checklist | Completed checklist items out of total |

## File Rename Alerts

Before computing status, `ghqc` checks open issues for files that have been renamed in a committed change. If any are found, a warning is printed above the table:

```shell
⚠️  Detected 1 file rename(s):
  `scripts/file_b.R` → `scripts/file_b_renamed.R` (issue #42)
  Run `ghqc issue rename` to confirm.
```

Run [`ghqc issue rename`](issue-rename.md) to update the issue title and record the rename in the issue body.

## See Also

- [`ghqc issue status`](issue-status.md) — detailed status for a single issue
- [`ghqc issue rename`](issue-rename.md) — confirm a detected file rename
- [`ghqc milestone record`](milestone-record.md) — generate a PDF record once issues are approved
- [`ghqc milestone archive`](milestone-archive.md) — the archive this command's readiness line predicts
