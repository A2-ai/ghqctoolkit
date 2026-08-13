# Issue: Retract Approval

```shell
ghqc issue unapprove          # or: ghqc issue retract-approval
```

Retracts an approval: posts a retraction comment with a reason and puts the issue back into an open state. Use this when the approval itself was **wrong** — recorded against the wrong commit, by the wrong person, or on an inadequate review. Retracting invalidates the claim that the round was approved, so downstream QCs that relied on it may need to be redone.

If the file simply changed again and needs another QC pass, use `ghqc issue new-round` instead: a new round is an append, and the previous approval remains valid.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select a Milestone

```shell
🚫 Welcome to GHQC Retract Approval Mode!
   This says a past approval was wrong. If the file simply changed again and needs another QC pass, use `ghqc issue new-round` instead.
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

A reason is required and will be included in the retraction comment.

```shell
? 📝 Why is this approval being retracted? Approved against the wrong commit
```

### 4. Approval Retracted

`ghqc` posts the retraction comment and puts the issue back into an open state. Any QCs that relied on the retracted approval are listed, because their own approvals may no longer be valid.

```shell
✨ Retracting approval with:
   🎯 Milestone: Milestone 1
   🎫 Issue: #4 - scripts/file_1.qmd
   🚫 Reason: Approved against the wrong commit

🚫 Approval retracted — the issue is open again!
https://github.com/my_organization/my_analysis/issues/4#issuecomment-192837465

These QCs relied on the approval just retracted, so their own approvals may no longer be valid and may need to be redone:
└── #7 scripts/file_2.qmd (Milestone 1) (previous QC)
```

## Non-interactive Usage

All three of `--milestone`, `--file`, and `--reason` must be provided together to skip interactive mode.

```shell
ghqc issue unapprove --milestone "Milestone 1" --file scripts/file_1.qmd --reason "Approved against the wrong commit"
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (required for non-interactive mode) |
| `-f, --file` | File path of the issue whose approval should be retracted (required for non-interactive mode) |
| `-r, --reason` | Reason the approval is being retracted — included in the comment (required for non-interactive mode) |

## Notes

- `retract-approval` is an alias for `unapprove`; both names work.
- After retracting, the QC workflow continues from the [review/comment cycle](issue-comment.md).
- To approve again, use [`ghqc issue approve`](issue-approve.md).
- The comment marker written to the issue thread is still `# QC Un-Approval`: it is the parse key every existing QC thread already contains, so it is deliberately unchanged.
