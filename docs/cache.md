# Cache

`ghqc` keeps a small on-disk cache of GitHub data and per-commit file-change records to speed up repeated operations. The cache lives under the system cache directory (typically `~/.cache/ghqc` on Linux/macOS via XDG; honors `XDG_CACHE_HOME`) and is namespaced per repository as `<root>/<owner>/<repo>/`.

Within a repository's cache directory, data is grouped by **element**:

| Element | Contents |
|---|---|
| `commits` | Per-commit file-change records (drives the "commits that changed file X" list) |
| `issues` | Issue comments and events |
| `users` | Repo assignees and user details |
| `labels` | Repo labels |

TTL defaults to 1 hour (3600s). Override with the `GHQC_CACHE_TIMEOUT` environment variable (in seconds). Some entries (issue comments/events, user details) are stored without a TTL and refresh based on GitHub-side timestamps instead.

## Status

```shell
ghqc cache status
```

Show the cache root, total size, configured TTL, and a per-element table for the current repo.

### Example output

```
── Cache ─────────────────────────────────────
root:     /home/user/.cache/ghqc
size:     1.4 MB (87 files)
ttl:      3600s (default; override with GHQC_CACHE_TIMEOUT)
── Repository ────────────────────────────────
repo:     A2-ai/ghqctoolkit
path:     /home/user/.cache/ghqc/A2-ai/ghqctoolkit

  element          size    files
  -------          ----    -----
  commits      612.4 KB       12
  issues       780.2 KB       64
  users          3.1 KB        2
  labels             —        —
```

When run outside a git repository, only the global section is shown.

## Dir

```shell
ghqc cache dir [--global]
```

Print the cache directory for the current repo (default) or the cache root (`--global`). Useful for piping into other tools, e.g. `du -sh "$(ghqc cache dir)"`.

Aliases: `ghqc cache directory`.

### Examples

```shell
# Per-repo cache directory
ghqc cache dir
# /home/user/.cache/ghqc/A2-ai/ghqctoolkit

# Cache root
ghqc cache dir --global
# /home/user/.cache/ghqc
```

When run outside a git repository, the per-repo form errors; pass `--global` instead.

## Remove

```shell
ghqc cache remove [ELEMENT] [--global]
```

Remove cached data from disk. The cache is reconstructible — entries will be re-fetched on next use — so deletion is safe. The command prints what was removed and exits 0 even if no matching entries existed.

Aliases: `ghqc cache rm`.

### Behavior

| Invocation | Effect |
|---|---|
| `ghqc cache remove` | Remove the entire per-repo cache for the current repo |
| `ghqc cache remove <element>` | Remove just `<element>` for the current repo |
| `ghqc cache remove --global` | Wipe the entire `ghqc` cache directory (all repos, all elements) |
| `ghqc cache remove <element> --global` | Remove `<element>` for **every** repo under the cache root |

When run outside a git repository, the repo-scoped forms error; use `--global` instead.

### Examples

```shell
# Drop just the commits cache for this repo (e.g. after a force-push or rebase)
ghqc cache remove commits

# Drop everything cached for this repo
ghqc cache remove

# Drop the labels cache for every repo on this machine
ghqc cache remove labels --global

# Wipe the entire ghqc cache
ghqc cache remove --global
```

### When to clear

In normal use the cache refreshes itself on TTL expiry or when GitHub-side timestamps change. You typically only need to clear it when:

- A `ghqc` upgrade changes how something is cached and you want the new logic applied immediately to already-cached commits/issues.
- A force-push or rebase rewrote history and the per-commit cache no longer reflects the branch.
- You're debugging unexpected behavior and want to rule out a stale cache.
