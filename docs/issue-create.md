# Issue: Create

```shell
ghqc issue create
```

Creates a new QC issue in GitHub for a file in the current repository. Running the command with no arguments enters interactive mode, walking through each step in sequence.

## Steps

### 1. Select or Create a Milestone

Issues must belong to a Milestone. Select an existing one or create a new one.

```shell
🚀 Welcome to GHQC Interactive Mode!
? Select or create a milestone:
  📝 Create new milestone:
> 🎯 Milestone 1
  🎯 QC Round 2
  🎯 EDA
```

### 2. Select a File

Choose the file to be QCed. Files that already have an open issue in the selected milestone are shown as unavailable (only one issue per file per milestone is allowed).

```shell
> Select or create a milestone: 🎯 Milestone 1
? 📁 Enter file path (Tab for autocomplete, directories shown with /): scripts/
> scripts/file_1.qmd
  scripts/file_2.qmd
  🚫 scripts/file_3.qmd (already has issue)
```

### 3. Select a Checklist

Choose a checklist to attach to the issue. Checklists come from the [configuration repository](configuration.md). A **Custom** option is always available as a built-in fallback.

```shell
? Select a checklist:
> 📋 Code Review
  📋 Custom
  📋 General Script
  📋 Report
```

### 4. Assign Reviewers

Optionally assign one or more GitHub users as reviewers. Press Enter to skip or to finish adding reviewers.

```shell
? 👥 Enter assignee username (use Tab for autocomplete, Enter for none): QCer
  QCer
  Reviewer
```

### 5. Add Relevant Files

Optionally attach related files for context. These can be supporting files or references to other QC issues. Press Enter when finished.

```shell
? 📁 Enter relevant file path (Tab for autocomplete, directories shown with /, Enter for none): scripts/
  scripts/file_2.qmd
  scripts/file_3.qmd
```

### 6. Issue Created

`ghqc` posts the issue to GitHub and prints the URL.

```shell
✨ Creating issue with:
   📊 Milestone: Milestone 1
   📁 File: scripts/file_1.qmd
   📋 Checklist: Code Review
   👥 Assignees: QCer

✅ Issue created successfully!
https://github.com/my_organization/my_analysis/issues/4
```

## Non-interactive Usage

All three of `--milestone`, `--file`, and `--checklist-name` must be provided together to skip interactive mode.

```shell
ghqc issue create --milestone "Milestone 1" --file scripts/file_1.qmd --checklist-name "Code Review" [options]
```

| Flag | Description |
|---|---|
| `-m, --milestone` | Milestone name (create new or use existing) |
| `-f, --file` | File path to create the issue for |
| `-c, --checklist-name` | Name of the checklist to attach |
| `-a, --assignees` | Reviewer GitHub usernames (repeatable) |
| `-D, --description` | Description for the milestone (only used when creating a new milestone) |
| `--previous-qc` | Previous QC issue URL, format: `<url>[::description]` (repeatable) |
| `--gating-qc` | Gating QC issue URL — must be approved before this issue can be approved, format: `<url>[::description]` (repeatable) |
| `--relevant-qc` | Related QC issue URL for informational reference, format: `<url>[::description]` (repeatable) |
| `--relevant-file` | Plain file reference with justification, format: `file_path::justification` (repeatable) |

## Relevant File Categories

When adding relevant files, `ghqc` supports several relationship types:

| Type | Description |
|---|---|
| `GatingQC` | Another QC issue that must be approved before this one can be approved |
| `PreviousQC` | A prior QC issue for reference |
| `RelevantQC` | An informational reference to another QC issue |
| `File` | A plain file reference (requires a justification note) |
