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

## See Also

- [`ghqc issue status`](issue-status.md) — detailed status for a single issue
- [`ghqc milestone record`](milestone-record.md) — generate a PDF record once issues are approved
