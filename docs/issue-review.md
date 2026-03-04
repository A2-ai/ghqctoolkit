# Issue: Review

```shell
ghqc issue review
```

Posts a review comment to a QC issue documenting the reviewer's feedback. While [`ghqc issue comment`](issue-comment.md) is used by the **author** to document changes they made between two commits, `ghqc issue review` is used by the **reviewer** to capture their review of the current working directory state against a specific commit.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
📝 Welcome to GHQC Review Mode!
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

Choose a commit to compare the current working directory against. Defaults to HEAD if not specified.

```
📋 Commit Status Legend:
   🌱 Initial commit  💬 Has comments  ✅ Approved  📍 Latest  📝 File changed
```

```shell
📝 Select commit to review against (press Enter for HEAD):
? Pick commit:
>   💬📝 00eadb9b - commit 3
    💬📝 bf8e8730 - commit 2
    🌱  32cf8fd6 - commit 1
```

### 4. Add Context

Optionally include a note and/or embed the diff between the commit and the current working directory in the review comment.

```shell
? 📝 Enter optional note for this review (Enter to skip):
? 📊 Include diff in review? (Y/n)
```

### 5. Review Comment Posted

```shell
📝 Review comment created!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-123456789
```

## Non-interactive Usage

```shell
ghqc issue review --milestone "Milestone 1" --file scripts/file_1.qmd [options]
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (required for non-interactive mode) |
| `-f, --file` | File path of the issue to review (required for non-interactive mode) |
| `-c, --commit` | Commit to compare against (defaults to HEAD) |
| `-n, --note` | Note to include in the review comment |
| `--no-diff` | Do not include the diff in the comment |

## See Also

- [`ghqc issue comment`](issue-comment.md) — author posts a comment documenting changes between two commits
- [`ghqc issue approve`](issue-approve.md) — approve the issue once review is complete
