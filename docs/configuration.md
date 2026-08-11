# Configuration

`ghqc` reads checklists, a logo, and options from a separate **configuration repository**. The configuration repository must be cloned locally before running other commands.

## Setup

```shell
ghqc configuration setup [GIT_URL]
```

Clones the configuration repository to local storage. Behaves differently depending on how the repository URL is provided:

- **Environment variable** — If `GHQC_CONFIG_REPO` is set, no argument is required. The repository is cloned to `$XDG_DATA_HOME/ghqc/<repository name>`.
- **Argument** — If a `GIT_URL` is provided directly, the repository is cloned to `$XDG_DATA_HOME/ghqc/config`.

### Example

```shell
export GHQC_CONFIG_REPO=https://github.com/a2-ai/ghqc.example_config_repo
ghqc configuration setup
```

## Status

```shell
ghqc configuration status
```

Displays the current state of the configuration repository: directory path, remote repository, sync status, available checklists, and logo.

### Example output

```
== Directory Information ==
📁 directory: /Users/user/.local/share/ghqc/config
📦 git repository: a2-ai/ghqc.example_config_repo
Repository is up to date!
📋 Checklists available in 'checklists': 4
✅ Logo found at logo.png

== Checklists Summary ==
📌 checklist note:
│  Note: edit checklist items as needed

- Code Review: 10 checklist items
- Custom: 1 checklist items
- General Script: 3 checklist items
- Report: 7 checklist items
```

In the Web UI, the Configuration tab shows the same sync status in an always-visible strip at the top of the tab. When the configuration repository is behind or has diverged from its remote, the Configuration tab is marked with a warning badge, and hovering the badge explains why. Deployments that [disable updates from the Web UI](#disabling-updates-from-the-web-ui) show the sync status only within the Configuration tab, without the warning badge.

## Update

```shell
ghqc configuration update
```

Fast-forwards the local configuration repository to match its remote, then reloads the checklists. Runs from any working directory — the configuration repository is resolved with the same rules as `ghqc configuration status`, so there is no need to `cd` into it first.

The update is deliberately conservative: it never creates a merge commit, stashes changes, or leaves conflicts behind. If a fast-forward is not possible, the command reports why, exits non-zero, and leaves the repository untouched. That happens when:

- **Uncommitted or staged changes** — commit or stash them first. The affected files are listed.
- **Local commits not on the remote** — the local repository is ahead, so there is nothing to fast-forward onto.
- **Diverged history** — local and remote have both moved on, and the difference must be resolved manually.

If the repository is already current, the command says so and does nothing.

The Web UI offers the same operation through the **Update** button in the Configuration tab, which is enabled when the configuration repository is behind its remote.

### Disabling updates from the Web UI

Deployments where the configuration repository is managed centrally — and individual users have no write access to it — can hide the **Update** button entirely by setting `allow_ui_config_update: false` in `options.yaml`, or `GHQC_ALLOW_CONFIG_UPDATE=false` in the server's environment. With updates disabled, `POST /api/configuration/update` also returns `403` with `Configuration updates are disabled for this deployment`, so the restriction holds even without the button.

The `ghqc configuration update` command is deliberately unaffected: an administrator maintaining the shared repository uses the CLI, and ordinary git and filesystem permissions already determine who can actually update it.

### Example output

```
✅ Updated configuration repository: 2 commits (a1b2c3d -> 4f1c9ab)
📋 checklists available in 'checklists': 4
```

Already up to date:

```
✅ Configuration repository is already up to date
```

Refused because of uncommitted changes:

```
Error: Cannot update: 2 uncommitted change(s) in /Users/user/.local/share/ghqc/config:
  - checklists/report.yaml
  - options.yaml
Commit or stash them, then retry.
```

Refused because of local commits not on the remote:

```
Error: Cannot update: 1 local commit(s) not on the remote in /Users/user/.local/share/ghqc/config. Push or reset them, then retry.
```

Refused because history has diverged:

```
Error: Cannot update: /Users/user/.local/share/ghqc/config has diverged from its remote (1 ahead, 3 behind). Resolve manually.
```

## Path

```shell
ghqc configuration path
```

Prints the configuration repository directory to stdout and nothing else, so it composes with other shell commands instead of requiring the path to be copied out of `ghqc configuration status` output.

### Example output

```
/Users/user/.local/share/ghqc/config
```

Which makes it usable directly in a shell:

```shell
cd $(ghqc configuration path)
```

## Configuration Repository Layout

The configuration repository must follow this structure:

```
<config-repo>/
├── checklists/         # YAML or GitHub-flavored Markdown checklist files
├── logo.png            # Logo embedded in generated PDFs
└── options.yaml        # Optional settings
```

### Checklist Format

Checklists can be defined as YAML files or GitHub-flavored Markdown files placed in the `checklists/` directory (or the directory specified by `checklist_directory`).

### options.yaml

| Option | Description |
|---|---|
| `prepended_checklist_note` | A note shown at the top of every checklist |
| `checklist_display_name` | Override the display name for "checklists" in the UI |
| `logo_path` | Override the default logo path (`logo.png`) |
| `checklist_directory` | Override the default checklist directory (`checklists`) |
| `ui_repo_refresh_rate_seconds` | Override the UI repository refresh interval in seconds |
| `allow_ui_config_update` | Whether the Web UI may update the configuration repository (default `true`) |

`ui_repo_refresh_rate_seconds` resolves in this order:

1. `options.yaml` `ui_repo_refresh_rate_seconds`
2. `GHQC_UI_REFRESH_RATE`
3. default `15`

Missing, non-numeric, zero, and negative values fall back to the next source, ending at `15`.

`allow_ui_config_update` resolves in this order:

1. `options.yaml` `allow_ui_config_update`
2. `GHQC_ALLOW_CONFIG_UPDATE`
3. default `true`

The environment variable accepts `true`/`false`, `1`/`0`, `yes`/`no`, and `on`/`off`, case-insensitively. Missing and unrecognized values fall back to the next source, ending at `true`, so a typo never silently disables updates.

The **Custom** checklist is always available as a built-in fallback, regardless of the configuration repository contents.

## Directory Resolution

When running `ghqc` commands, the configuration directory is resolved in this order:

1. `--config-dir` flag — uses the provided directory directly
2. `GHQC_CONFIG_REPO` env var — uses `$XDG_DATA_HOME/ghqc/<repo name>`
3. Default — uses `$XDG_DATA_HOME/ghqc/config`

## Example Repository

An example configuration repository is available at:
[https://github.com/a2-ai/ghqc.example_config_repo](https://github.com/a2-ai/ghqc.example_config_repo)
