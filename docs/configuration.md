# Configuration

`ghqc` reads checklists, a logo, and options from a separate **configuration repository**. The configuration repository must be cloned locally before running other commands.

## Init

```shell
ghqc configuration init [PATH]
```

Interactively creates a configuration repository, or edits an existing one. `PATH` is resolved relative to the current directory and prompted for when omitted; if it does not exist, the command warns and asks before creating it.

The wizard writes files only — it never runs `git`. Committing, pushing, and pointing `GHQC_CONFIG_REPO` at the resulting remote are left to the user, and the command prints those next steps when it finishes.

It walks through, in order:

1. **Options** — every `options.yaml` setting, with the current (or default) value pre-filled. Options left unset are written as comments, so they keep falling back to the environment variable and built-in default.
2. **Logo** — a path to an image, copied into the repository at `logo_path`. Optional: pressing Enter skips it, or keeps the existing logo.
3. **Record template** — write the built-in template, copy an existing `.typ` file, or skip it. When skipped, the built-in template is used at runtime.
4. **Checklists** — add or edit checklists until you choose **Done**.

The logo and template prompts browse the filesystem: **Tab** completes to the unique match or the longest shared prefix, directories are listed with a trailing `/`, and `../` is always offered so the tree can be walked in both directions.

New checklists are written as markdown. They can be authored three ways: in your editor starting from a small skeleton, item by item through prompts (optionally grouped into `###` sections), or from a bundled starter checklist (General Script, Code Review, Report) which is then opened in the editor. A checklist with no `- [ ]` items re-opens the editor rather than being saved silently.

The editor is `$VISUAL`, then `$EDITOR`, falling back to `vim` (then `vi`, then `nano`) — set either variable to use something else.

Selecting an existing checklist offers editing its contents, renaming it, or deleting it. Editing opens the **raw file** as it exists on disk, so round-tripping never duplicates `prepended_checklist_note` or flattens YAML sections. Renaming updates the file stem (or the YAML root key, for the older YAML format) as well as the filename.

### Editing an existing configuration repository

When `options.yaml` already exists, the command reports the existing configuration and asks for confirmation before proceeding. Every prompt is then seeded with the current value, so pressing Enter through the wizard leaves the configuration unchanged. Existing files are never overwritten without an explicit confirmation.

To change one thing without walking the whole wizard, use `ghqc configuration edit` below.

## Edit

```shell
ghqc configuration edit              # choose a component interactively
ghqc configuration edit checklists
ghqc configuration edit options
ghqc configuration edit logo
ghqc configuration edit record
```

Runs a single step of the wizard against an existing configuration repository. `edit checklists` goes straight to the checklist menu; `edit options` walks the `options.yaml` settings and rewrites the file; `edit logo` and `edit record` replace those files. Each step behaves exactly as it does inside `ghqc configuration init`, and untouched components are left alone.

With no component named, the components are offered as a menu that returns after each one, so several can be edited in a single session. `options.yaml` is re-read before each step, so a checklist directory changed under **Options** takes effect immediately.

The repository is located, in order:

1. `--config-dir`, when given
2. the current directory, when it contains an `options.yaml`
3. the configured configuration directory (see [Directory Resolution](#directory-resolution))

Unlike `init`, `edit` never creates a repository — if none is found, it says so and points at `ghqc configuration init`.

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

Checklists are placed in the `checklists/` directory (or the directory specified by `checklist_directory`). Files ending in `.md`, `.markdown`, and `.txt` are used as the checklist body verbatim, and their title comes from the filename — wrap the name in backticks (`` `Code Review`.md ``) for titles containing spaces. Files ending in `.yaml` and `.yml` are parsed as a single-key mapping whose root key is the title.

Markdown is the recommended format and the one `ghqc configuration init` writes: the file is passed through untouched, so any GitHub-flavored markdown is available. Checklist items are lines beginning with `- [ ]`; `###` headers group them into sections.

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
