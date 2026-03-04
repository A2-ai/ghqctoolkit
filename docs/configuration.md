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

The **Custom** checklist is always available as a built-in fallback, regardless of the configuration repository contents.

## Directory Resolution

When running `ghqc` commands, the configuration directory is resolved in this order:

1. `--config-dir` flag — uses the provided directory directly
2. `GHQC_CONFIG_REPO` env var — uses `$XDG_DATA_HOME/ghqc/<repo name>`
3. Default — uses `$XDG_DATA_HOME/ghqc/config`

## Example Repository

An example configuration repository is available at:
[https://github.com/a2-ai/ghqc.example_config_repo](https://github.com/a2-ai/ghqc.example_config_repo)
