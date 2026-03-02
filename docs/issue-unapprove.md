# Issue: Unapprove

```shell
ghqc issue unapprove
```

Reverses an approval: posts an unapproval comment with a reason and reopens the issue. Used when additional changes are required after an issue was previously approved.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
🚫 Welcome to GHQC Unapprove Mode!
? Select a milestone:
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

### 2. Select a Closed Issue

Only closed (approved) issues are shown.

```shell
> Select a milestone: 🎯 Milestone 1
? 🎫 Enter issue title (use Tab for autocomplete):
> scripts/file_1.qmd
  models/1001.mod
```

### 3. Provide a Reason

A reason is required and will be included in the unapproval comment.

```shell
? 📝 Enter reason for unapproval: Found more changes to be made
```

### 4. Issue Unapproved and Reopened

`ghqc` posts the unapproval comment and reopens the issue.

```shell
✨ Creating unapproval with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   🚫 Reason: Found more changes to be made

🚫 Issue unapproved and reopened!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-192837465
```

## Non-interactive Usage

All three of `--milestone`, `--file`, and `--reason` must be provided together to skip interactive mode.

```shell
ghqc issue unapprove --milestone "Milestone 1" --file scripts/file_1.qmd --reason "Found more changes to be made"
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (required for non-interactive mode) |
| `-f, --file` | File path of the issue to unapprove (required for non-interactive mode) |
| `-r, --reason` | Reason for unapproval — included in the comment (required for non-interactive mode) |

## Notes

- After unapproving, the QC workflow continues from the [review/comment cycle](issue-comment.md).
- To approve again, use [`ghqc issue approve`](issue-approve.md).
