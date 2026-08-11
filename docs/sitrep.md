# Sitrep

```shell
ghqc sitrep
```

Prints a situation report summarizing the current state of the `ghqc` binary, the git repository in the working directory, and the configuration repository. Useful for diagnosing setup issues or sharing environment details when reporting a bug.
It also reports the current authentication store and the auth sources available for the repository host.

## Output

The report is divided into four sections:

### Binary

```
── Binary ────────────────────────────────────
Version: 0.2.0
Path: /usr/local/bin/ghqc
```

| Field | Description |
|---|---|
| Version | Installed version of `ghqc` |
| Path | Absolute path to the running executable |

### Repository

```
── Repository ────────────────────────────────
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

### Auth

```
── Auth ──────────────────────────────────────
store directory: /home/user/.local/share/ghqc/auth
stored tokens:
  ▶ github.com (ghp_abcd...wxyz)

repository host: github.com
available auth sources
  ▶ ✓  ghqc auth store            (ghp_abcd...wxyz)
    ✓  GITHUB_TOKEN               (ghp_1234...7890)
    ✗ gh auth token
    ✗ gh stored auth
    ✗ git credential manager
    ✗ .netrc
```

| Field | Description |
|---|---|
| store directory | Path to the local `ghqc` auth store |
| stored tokens | Hosts with tokens stored by `ghqc`, with the selected repository host highlighted when applicable |
| repository host | Host inferred from the current repository remote |
| available auth sources | Authentication sources checked for the repository host, shown in priority order. The active source is marked with `▶`. |

### Configuration

```
── Configuration ─────────────────────────────
Directory: /home/user/.local/share/ghqc-config
Repository: owner/ghqc-config (https://github.com/owner/ghqc-config)
Checklists: 3
  - Default: 12 items
  - Abbreviated: 5 items
  - Custom: 1 items
Options:
  - Prepended Checklist Note:
     │ All items must be reviewed before approval.
  - Checklist Display Name: checklists
  - Include Collaborators: no
  - Logo Path: logo.png
  - Checklist Directory: checklists
  - Record Template Path: record.typ
  - UI Repo Refresh Rate: 15s
  - Allow Config Update From UI: yes
```

Every configuration option is listed. `Prepended Checklist Note` shows `(none)`
when it is unset. `UI Repo Refresh Rate` and `Allow Config Update From UI` show
the *effective* value after resolution: the value in `options.yaml` if set,
otherwise `GHQC_UI_REFRESH_RATE` / `GHQC_ALLOW_CONFIG_UPDATE`, otherwise the
default (`15` seconds and `yes`). `Allow Config Update From UI: no` explains why
the web UI shows no Update button.

| Field | Description |
|---|---|
| Directory | Path to the configuration directory (marked ❌ if not found) |
| Repository | GitHub owner/repo of the configuration repo, if it is a git repository |
| Checklists | Number of checklists found, with item counts for each |
| Options | All configuration options: prepended note, checklist display name, include collaborators, logo path, checklist directory, record template path, and the resolved UI repo refresh rate and UI config-update toggle |

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
- [`ghqc auth status`](auth.md) — focused view of auth storage and source resolution
- [`ghqc milestone status`](milestone-status.md) — detailed issue status across milestones
