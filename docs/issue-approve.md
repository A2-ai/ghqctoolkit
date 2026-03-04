# Issue: Approve

```shell
ghqc issue approve
```

Approves a QC issue at a specific commit, posts an approval comment, and closes the issue. Used by the reviewer once they are satisfied with the file.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
✅ Welcome to GHQC Approve Mode!
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

### 3. Select a Commit

Choose the commit to approve. Defaults to the latest commit.

```
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed
```

```shell
📝 Select commit to approve (press Enter for latest):
? Pick commit:
>   💬📝 00eadb9b - commit 3
    💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

### 4. Add an Optional Note

```shell
? 📝 Enter optional note for this comment (Enter to skip):
```

### 5. Issue Approved and Closed

`ghqc` posts the approval comment and closes the issue.

```shell
✨ Creating approval with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   📁 File: scripts/file_1.qmd
   📝 Commit: 00eadb9bf2747dffade4415e63e689c1450261bd

✅ Approval created and issue closed!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-987654321
```

## Non-interactive Usage

Both `--milestone` and `--file` must be provided together to skip interactive mode.

```shell
ghqc issue approve --milestone "Milestone 1" --file scripts/file_1.qmd [options]
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (required for non-interactive mode) |
| `-f, --file` | File path of the issue to approve (required for non-interactive mode) |
| `-a, --approved-commit` | Commit to approve (defaults to most recent file commit) |
| `-n, --note` | Note to include in the approval comment |
| `--force` | Force approval even if blocking QC issues are not yet approved |

## Notes

- To reverse an approval, use [`ghqc issue unapprove`](issue-unapprove.md).
- The approved commit hash is recorded in the approval comment and drives the `QCStatus` calculation for downstream record generation.
