# Issue: Rename

```shell
ghqc issue rename
```

Confirms a detected file rename: updates the issue title to the new file path and appends a `## File History` entry to the issue body recording the old name, new name, and HEAD commit. A timeline comment is also posted to the issue so the rename is visible in the thread.

`ghqc issue status` and `ghqc milestone status` will alert when renames are detected. Run this command to confirm them.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
? Select a milestone:
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

### 2. Renames Detected and Confirmed

`ghqc` checks open issues in the selected milestone against the current git HEAD tree and identifies any files that have been renamed in a committed change.

```shell
⚠️  Detected 1 file rename(s):
  `scripts/file_b.R` → `scripts/file_b_renamed.R` (issue #42)
? Update issue #42 title and record rename in body? (Y/n)
  ✓ Issue #42 updated.

✅ Confirmed 1 rename(s).
```

Choose **Y** to update the issue, or **n** to skip. If no renames are detected, the command reports that and exits.

## Non-interactive Usage

Provide `--milestone` and `--file` to confirm a specific rename without any prompts. The new file path is auto-detected from git history.

```shell
ghqc issue rename --milestone "Milestone 1" --file scripts/file_b.R
```

`--file` requires `--milestone`.

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name. Skips the milestone selection prompt. |
| `-f, --file` | Old file path (current issue title). Auto-detects the rename target. Requires `--milestone`. |

## File History

When a rename is confirmed, a `## File History` section is appended (or updated) in the issue body:

```markdown
## File History
* `scripts/file_b.R` → `scripts/file_b_renamed.R` (commit: abc1234)
```

This history is used internally by `ghqc` to correctly attribute commits made against the old file name when computing QC and git status.

## See Also

- [`ghqc issue status`](issue-status.md) — alerts about detected renames for a single issue
- [`ghqc milestone status`](milestone-status.md) — alerts about detected renames across a milestone
