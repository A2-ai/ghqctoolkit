# Milestone: Record

```shell
ghqc milestone record
```

Generates a PDF QC record for one or more milestones. The record summarizes the QC history: approved commits, checklists, comments, and reviewer information for each issue in the selected milestones.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select Milestones

Choose all milestones or select specific ones.

```shell
📊 Welcome to GHQC Milestone Record Mode!
? 📊 How would you like to select milestones?
  📋 Select All Milestones
> 🎯 Choose Specific Milestones
```

```shell
? 📊 Select milestones to check:
> [x] Milestone 1 (3)
  [ ] QC Round 2 (1)
  [ ] EDA (8)
```

### 2. Name the Output File

Optionally provide a custom file name. Press Enter to use the default: `<repo-name>-<milestone-names>.pdf`.

```shell
? 📁 Enter record file name (Enter for default):
```

### 3. Record Generated

```shell
✅ Record successfully generated at my_analysis-Milestone-1.pdf
```

## Non-interactive Usage

Pass milestone names as positional arguments or use `--all-milestones` to skip interactive mode.

```shell
# Specific milestones
ghqc milestone record "Milestone 1" --record-path my_analysis-m1.pdf

# All milestones with context files
ghqc milestone record --all-milestones \
  --prepended-context cover.pdf \
  --appended-context appendix.pdf
```

| Argument / Flag | Description |
|---|---|
| `[milestones...]` | Milestone names to include (positional, repeatable) |
| `--all-milestones` | Include all milestones |
| `-r, --record-path` | Output file path (default: `<repo>-<milestones>.pdf`) |
| `--only-tables` | Include only summary tables; skip detailed issue content |
| `--prepended-context` | PDF to prepend before the main findings (repeatable, rendered in order) |
| `--appended-context` | PDF to append after the main findings (repeatable, rendered in order) |

## Output

The generated PDF includes:
- Repository and milestone metadata
- For each issue: file path, assigned checklist, reviewer(s), approval commit, and comment history
- Optional logo from the [configuration repository](configuration.md)

## Web UI

The Record tab in the web UI offers additional options:
- Upload files to prepend or append to the record (e.g., a cover page or appendix)
- Preview the record before downloading

## See Also

- [`ghqc milestone archive`](milestone-archive.md) — bundle the record and files into a zip archive
- [`ghqc milestone status`](milestone-status.md) — verify all issues are approved before generating a record
