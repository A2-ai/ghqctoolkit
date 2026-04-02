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

### 5. Edit Collaborators

`ghqc` always records the issue creator as the `author` metadata entry. By default it also derives collaborator entries from the git author history for the selected file, filters out malformed or infrastructure-style email entries, and lets you trim or add collaborators before creating the issue. The author is shown separately from collaborators and is not removable from the collaborators editor.

```shell
? 🤝 Select collaborators to keep:
> Jane Doe <jane@example.com>
  John Smith <john@example.com>
? 🤝 Add collaborator (Name <email>, Enter to finish):
> Analyst Two <analyst@example.com>
```

### 6. Add Relevant Files

Optionally attach related files for context. These can be supporting files or references to other QC issues. Press Enter when finished.

If you add a `PreviousQC` reference in interactive mode, `ghqc` will also ask whether to post an automatic diff comment comparing the previous QC commit to the current issue's starting commit. That diff comment is enabled by default.

```shell
? 📁 Enter relevant file path (Tab for autocomplete, directories shown with /, Enter for none): scripts/
  scripts/file_2.qmd
  scripts/file_3.qmd
```

### 7. Issue Created

`ghqc` posts the issue to GitHub and prints the URL.

```shell
✨ Creating issue with:
   📊 Milestone: Milestone 1
   📁 File: scripts/file_1.qmd
   📋 Checklist: Code Review
   👥 Assignees: QCer
   🤝 Collaborators: Jane Doe <jane@example.com>

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
| `--add-collaborator` | Add collaborator metadata entry, format: `Name <email>` (repeatable) |
| `--remove-collaborator` | Remove a detected collaborator entry, format: `Name <email>` (repeatable) |
| `-D, --description` | Description for the milestone (only used when creating a new milestone) |
| `--previous-qc` | Previous QC issue URL, format: `<url>[::description][::no_diff]` (repeatable). By default, `ghqc` posts an automatic diff comment unless `::no_diff` is added. |
| `--gating-qc` | Gating QC issue URL — must be approved before this issue can be approved, format: `<url>[::description]` (repeatable) |
| `--relevant-qc` | Related QC issue URL for informational reference, format: `<url>[::description]` (repeatable) |
| `--relevant-file` | Plain file reference with justification, format: `file_path::justification` (repeatable) |

The issue body metadata always uses the authenticated issue creator as `author` when available. If the current GitHub user cannot be determined, `ghqc` falls back to the first git-derived author for the file. Collaborators default from cleaned git author history and can be edited interactively or with the collaborator flags above.

## Relevant File Categories

When adding relevant files, `ghqc` supports several relationship types:

| Type | Description |
|---|---|
| `GatingQC` | Another QC issue that must be approved before this one can be approved |
| `PreviousQC` | A prior QC issue for reference. It also blocks approval of the new issue, and by default `ghqc` posts a diff comment comparing the previous QC commit to the new issue's starting commit. Use `::no_diff` to suppress that automatic diff comment in non-interactive mode. |
| `RelevantQC` | An informational reference to another QC issue |
| `File` | A plain file reference (requires a justification note) |
