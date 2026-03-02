# Issue: Status

```shell
ghqc issue status
```

Displays the current QC status, git status, and checklist progress for a single issue.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
? Select a milestone:
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

### 2. Select an Issue

```shell
> Select a milestone: 🎯 Milestone 1
? 🎫 Enter issue title (use Tab for autocomplete):
> scripts/file_1.qmd
  scripts/file_2.qmd
  scripts/file_3.qmd
```

### 3. Status Printed

```shell
- File:         scripts/file_1.qmd
- Branch:       analysis
- Issue State:  open
- QC Status:    File change in `bb23a12` not commented
- Git Status:   File is up to date!
- Checklist Summary: 0/5 (0.0%)
    - Code Quality: 0/2 (0.0%)
    - Scientific Review: 0/3 (0.0%)
```

## Non-interactive Usage

Both `--milestone` and `--file` must be provided together to skip interactive mode.

```shell
ghqc issue status --milestone "Milestone 1" --file scripts/file_1.qmd
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (required for non-interactive mode) |
| `-f, --file` | File path of the issue to check (required for non-interactive mode) |

## QC Status Values

| Status | Meaning |
|---|---|
| `In Progress` | Review is underway; checklist items are being completed |
| `Changes to Comment` | A file-changing commit has not yet been documented with a comment |
| `Awaiting Review` | Waiting for the reviewer to begin |
| `Change Requested` | Reviewer has requested changes |
| `Approval Required` | All checklist items complete; waiting for final approval |
| `Approved` | Issue has been approved and closed |
| `Changes After Approval` | File changed after approval was given |

## See Also

- [`ghqc milestone status`](milestone-status.md) — tabular summary across multiple milestones
