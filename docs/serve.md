# Serve / UI

`ghqc` can run as an HTTP server exposing the full REST API, and optionally serve an embedded React web UI from the same port.

## ghqc ui

```shell
ghqc ui [url] [--port PORT] [--ipv4-only]
```

Starts the embedded web UI server and opens the browser. Requires the binary to be built with the `ui` feature.

```shell
cargo build --features cli,ui --release
./target/release/ghqc ui
# or on a custom port:
./target/release/ghqc ui --port 8080
# or to force IPv4 on hosts with problematic IPv6/localhost behavior:
./target/release/ghqc ui --ipv4-only
# or to print the exact URL the UI would use and exit:
./target/release/ghqc ui url
./target/release/ghqc ui url --port 8080
```

The server starts on port **3103** by default. The browser opens automatically to a literal loopback URL:
`http://127.0.0.1:<port>` on IPv4-only systems or `http://[::1]:<port>` when the listener is bound on IPv6.

`ghqc ui url` uses the same bind logic as `ghqc ui`, so it prints the exact loopback URL selected on the current machine and then exits without starting the server.

### Web UI Tabs

| Tab | Description |
|---|---|
| **Status** | Kanban board of all issues grouped by QC status. Cards are color-coded by git status: cyan (clean), yellow (ahead/behind), red (conflict). Clicking the issue title opens GitHub in a new tab; clicking the rest of the card opens the in-app issue detail modal. |
| **Create** | Wizard for creating new QC issues: select/create milestone, browse file tree, pick checklist, assign reviewers, add relevant files. Previous QC references can optionally post an automatic diff comment, and that diff is enabled by default. Queued issues and saved custom checklists persist while you move between UI tabs, until the page is refreshed. |
| **Record** | PDF record generation: select milestones, upload context files (prepend or append), preview, generate and download. |
| **Archive** | Archive generation: select milestones, set file name, generate zip archive. |
| **Configuration** | Configuration repo setup and status. |

### Web UI Routes

The embedded UI uses route-based navigation:

| Route | Screen |
|---|---|
| `/status` | Status board |
| `/create` | Create workflow |
| `/record` | Record generation |
| `/archive` | Archive generation |
| `/configuration` | Configuration status |

Opening `/` redirects to `/status`.

## ghqc serve

```shell
ghqc serve [--port PORT] [--ipv4-only]
```

Starts the REST API server only, without the embedded UI. Requires the binary to be built with the `api` feature (but not `ui`).

```shell
cargo build --features cli,api --release
./target/release/ghqc serve
# or on a custom port:
./target/release/ghqc serve --port 3104
# or to force IPv4:
./target/release/ghqc serve --ipv4-only
```

The server starts on port **3103** by default.

The API spec is available at `openapi/openapi.yml` in the repository.

## Options

| Flag | Default | Description |
|---|---|---|
| `-p, --port` | `3103` | Port to listen on |
| `--ipv4-only` | `false` | Force an IPv4-only listener and `127.0.0.1` loopback URL |
| `-d, --directory` | `.` | Git project directory to serve |
| `--config-dir` | (auto-resolved) | Configuration directory path |

## Configuration Resolution

Both commands resolve the configuration directory in the same order as the CLI:

1. `--config-dir` flag
2. `GHQC_CONFIG_REPO` env var → `$XDG_DATA_HOME/ghqc/<repo name>`
3. Default: `$XDG_DATA_HOME/ghqc/config`

## Build Features

| Feature | Command available |
|---|---|
| `cli,ui` | `ghqc ui` |
| `cli,api` (no `ui`) | `ghqc serve` |
| `cli,api,ui` | `ghqc ui` |

## Frontend Dev Server

When developing the frontend, run the API server separately and start the Vite dev server:

```shell
cargo run --features cli,api -- serve --port 3104
cd ui && bun run dev
```

The dev server proxies API requests to port 3104 and supports hot module reload.
