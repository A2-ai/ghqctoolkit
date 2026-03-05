# Sitrep

```shell
ghqc sitrep
```

Prints a situation report summarizing the current state of the `ghqc` binary, the git repository in the working directory, and the configuration repository. Useful for diagnosing setup issues or sharing environment details when reporting a bug.

## Output

The report is divided into three sections:

### Binary

```
=== Binary =========================
Version: 0.2.0
Path: /usr/local/bin/ghqc
```

| Field | Description |
|---|---|
| Version | Installed version of `ghqc` |
| Path | Absolute path to the running executable |

### Repository

```
=== Repository =====================
Directory: /projects/myrepo
Repository: owner/repo (https://github.com/owner/repo)
Branch: main
Milestones: 2
  - v1.0 [open]: 4 open | 8 closed
  - v2.0 [open]: 2 open | 3 closed
```

| Field | Description |
|---|---|
| Directory | Resolved path of the working directory (`-d` flag, defaults to `.`) |
| Repository | GitHub owner/repo and remote URL |
| Branch | Current git branch |
| Milestones | All milestones, sorted by number of open issues (descending), then alphabetically. Each entry shows milestone state (`open`/`closed`) and open/closed issue counts. |

If the directory is not a git repository, or the GitHub API cannot be reached, a descriptive error is shown instead.

### Configuration

```
=== Configuration ==================
Directory: /home/user/.local/share/ghqc-config
Repository: owner/ghqc-config (https://github.com/owner/ghqc-config)
Checklists: 3
  - Default: 12 items
  - Abbreviated: 5 items
  - Custom: 1 items
Options:
  - Prepended Checklist Note:
     │ All items must be reviewed before approval.
  - Checklist Display Name:  checklists
  - Logo Path: logo.png
  - Checklist Directory: checklists
  - Record Template Path: record.typ
```

| Field | Description |
|---|---|
| Directory | Path to the configuration directory (marked ❌ if not found) |
| Repository | GitHub owner/repo of the configuration repo, if it is a git repository |
| Checklists | Number of checklists found, with item counts for each |
| Options | Active values from the configuration (prepended note, display name, logo, checklist directory, record template) |

## Flags

| Flag | Description |
|---|---|
| `--json` | Output the full report as pretty-printed JSON |

## Examples

```shell
# Default text output
ghqc sitrep

# JSON output (for scripting or bug reports)
ghqc sitrep --json

# Check a different project directory
ghqc sitrep -d /path/to/project

# Use a custom configuration directory
ghqc sitrep --config-dir /path/to/config
```

## See Also

- [`ghqc configuration status`](configuration.md) — focused view of configuration only
- [`ghqc milestone status`](milestone-status.md) — detailed issue status across milestones
