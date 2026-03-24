# Authentication

`ghqc` can use GitHub credentials from several places, and can also store a token in its own local auth store for the current machine.

## Login

```shell
ghqc auth login [TOKEN]
```

Logs in to a GitHub host and optionally stores a token in the `ghqc` auth store.

Behavior depends on how authentication is provided:

- **Direct token argument** — If `TOKEN` is provided, `ghqc` validates it and stores it immediately.
- **GitHub CLI available** — If `gh` is installed, `ghqc` runs `gh auth login`. By default, it then imports the resulting token into the `ghqc` auth store.
- **Fallback prompt** — If `gh` is not installed, `ghqc` opens the personal access token page for the selected host and prompts for a token interactively.

### Flags

| Flag | Description |
|---|---|
| `--host <host>` | GitHub host to use, such as `github.com` or `https://ghe.example.com`. If omitted, `ghqc` resolves the host from the current repository remote. |
| `--no-store` | Skip importing the token into the `ghqc` auth store after a successful `gh auth login`. This only applies to the `gh` login flow. |

### Examples

```shell
# Login for the current repository host
ghqc auth login

# Login for a GitHub Enterprise host
ghqc auth login --host ghe.example.com

# Store a token directly
ghqc auth login ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx

# Use gh auth login, but do not copy the token into ghqc's local store
ghqc auth login --no-store
```

## Logout

```shell
ghqc auth logout
```

Removes the token stored by `ghqc` for a host. This does not log out the GitHub CLI, change `GITHUB_TOKEN`, or remove credentials stored by Git credential helpers.

### Flags

| Flag | Description |
|---|---|
| `--host <host>` | GitHub host to use. If omitted, `ghqc` resolves the host from the current repository remote. |

### Examples

```shell
# Remove the stored token for the current repository host
ghqc auth logout

# Remove the stored token for a specific host
ghqc auth logout --host github.com
```

## Status

```shell
ghqc auth status
```

Shows the local `ghqc` auth store and the authentication sources available for the selected host.

The selected host comes from `--host` when provided, otherwise from the current repository remote.

### Example output

```
=== Auth Store =====================
store directory: /home/user/.local/share/ghqc/auth
stored tokens:
▶ github.com (ghp_abcd...wxyz)

=== Auth Sources ===================
repository host: github.com

available auth sources
▶ ✓ ghqc auth store            (ghp_abcd...wxyz)
  ✓ GITHUB_TOKEN               (ghp_1234...7890)
  ✗ gh auth token
  ✗ gh stored auth
  ✗ git credential manager
  ✗ .netrc
```

### Source Priority

When multiple sources are available, `ghqc` prefers them in this order:

1. `ghqc auth store`
2. `GITHUB_TOKEN`
3. `gh auth token`
4. `gh stored auth`
5. `git credential manager`
6. `.netrc`

## Host Resolution

For `login`, `logout`, and `status`, the host is resolved in this order:

1. `--host` flag
2. Current repository remote (`origin` fetch URL)

If `ghqc` cannot determine a host from the current directory, re-run the command with `--host`.
