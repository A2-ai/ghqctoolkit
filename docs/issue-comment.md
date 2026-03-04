# Issue: Comment

```shell
ghqc issue comment
```

Posts a comment to an existing QC issue documenting changes made between two commits. Typically used by the author to notify the reviewer that changes requested during review have been implemented.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
💬 Welcome to GHQC Comment Mode!
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

### 3. Select Commits

Choose two commits to diff. The legend shows each commit's state:

```
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed
```

The first commit defaults to the most recent commit that changed the file. The second defaults to the most recent previously commented-on commit. If both would be the same, the second defaults to the next earlier file-changing commit.

```shell
📝 Select first commit (press Enter for latest file change):
? Pick commit:
>    📝 00eadb9b - commit 3
    💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

```shell
📝 Select second commit for comparison (press Enter for second file change):
? Pick commit:
     📝 00eadb9b - commit 3
>   💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

### 4. Add Context

Optionally include a note and/or embed the commit diff in the comment body.

```shell
? 📝 Enter optional note for this comment (Enter to skip):
? 📊 Include commit diff in comment? (Y/n)
```

### 5. Comment Posted

`ghqc` posts the comment and prints the URL.

## Non-interactive Usage

Both `--milestone` and `--file` must be provided together to skip interactive mode.

```shell
ghqc issue comment --milestone "Milestone 1" --file scripts/file_1.qmd [options]
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (required for non-interactive mode) |
| `-f, --file` | File path of the issue to comment on (required for non-interactive mode) |
| `-c, --current-commit` | Newer commit in the diff (defaults to most recent file commit) |
| `-p, --previous-commit` | Older commit in the diff (defaults to second most recent file commit) |
| `-n, --note` | Note to include in the comment |
| `--no-diff` | Do not include the commit diff in the comment |

```shell
✨ Creating comment with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   📁 File: scripts/file_1.qmd
   📝 Current commit: 00eadb9bf2747dffade4415e63e689c1450261bd
   📝 Previous commit: bf8e8730a66f7be13aa0c895bf8dc2acd033751a
   📊 Include diff: Yes

✅ Comment Created!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-123456789
```
