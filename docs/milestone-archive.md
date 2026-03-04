# Milestone: Archive

```shell
ghqc milestone archive
```

Generates a zip archive for one or more milestones. The archive bundles the PDF record along with associated files for long-term storage or distribution.

Running the command with no arguments enters interactive mode.

## Steps

### 1. Select Milestones

Choose all milestones or select specific ones.

```shell
📊 Welcome to GHQC Milestone Archive Mode!
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

### 2. Select Additional Files

Optionally select additional files from the repository to include in the archive alongside the generated record.

### 3. Name the Output File

Optionally provide a custom archive file name. Press Enter to use the default: `<repo-name>-<milestone-names>.zip`.

```shell
? 📁 Enter archive file name (Enter for default):
```

### 4. Archive Generated

```shell
✅ Archive successfully generated at my_analysis-Milestone-1.zip
```

## Non-interactive Usage

Pass milestone names as positional arguments or use a milestone selection flag to skip interactive mode.

```shell
# Specific milestones
ghqc milestone archive "Milestone 1" --archive-path archive/m1.tar.gz

# All closed milestones, flattened structure
ghqc milestone archive --all-closed-milestones --flatten

# Add specific files at specific commits
ghqc milestone archive "Milestone 1" --additional-file scripts/file_1.qmd:00eadb9b
```

| Argument / Flag | Description |
|---|---|
| `[milestones...]` | Milestone names to include (positional, repeatable) |
| `--all-milestones` | Include all milestones (open and closed) |
| `--all-closed-milestones` | Include only closed milestones |
| `--include-unapproved` | Include issues that have not been approved |
| `--flatten` | Put all files in the archive root directory (no subdirectory structure) |
| `-a, --archive-path` | Output file path (default: `archive/<repo>-<milestones>.tar.gz`) |
| `--additional-file` | Extra file to include at a specific commit, format: `file_path:commit` (repeatable) |

## Archive Contents

The zip archive includes:
- The generated PDF record (equivalent to [`ghqc milestone record`](milestone-record.md))
- Any additional files selected during the interactive flow

## See Also

- [`ghqc milestone record`](milestone-record.md) — generate only the PDF record
- [`ghqc milestone status`](milestone-status.md) — verify all issues are approved before archiving
