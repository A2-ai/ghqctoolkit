# API Server Implementation Plan

## Overview

Add an Axum-based REST API to expose ghqctoolkit functionality for GUI consumption. The API will mirror the CLI's interactive workflow capabilities through RESTful endpoints.

## Configuration

- **Framework**: Axum (with existing Tokio runtime)
- **Authentication**: Server-managed from local repo (stored in `OnceLock` at startup)
- **Git Dependency**: Local repository required (server configured at startup)
- **Workflow**: RESTful endpoints (client manages workflow state)
- **OpenAPI**: Manually maintained at @openapi/openapi.yml
- **API Path**: `/api` (no version prefix)

---

## Module Structure

```
src/api/
├── mod.rs              # Module exports, feature gating, OnceLock for GitInfo
├── server.rs           # Axum server setup
├── state.rs            # AppState (config, cache reference)
├── error.rs            # Error types → HTTP status codes
├── routes/
│   ├── mod.rs          # Route registration
│   ├── milestones.rs   # GET/POST milestones, GET milestone issues
│   ├── issues.rs       # Issue CRUD + batch status
│   ├── comments.rs     # Comment/approve/unapprove/review
│   ├── status.rs       # Assignees, labels
│   ├── configuration.rs # Checklists, config status
│   └── health.rs       # Health check
└── types/
    ├── mod.rs          # Type exports
    ├── requests.rs     # Request DTOs
    └── responses.rs    # Response DTOs

openapi/openapi.yml     # Manually maintained OpenAPI spec
AGENTS.md               # Instructions for AI agents (includes OpenAPI update rules)
```

---

## API Endpoints

### Health
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/health` | Health check |

### Milestones
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/milestones` | List all milestones |
| `POST` | `/api/milestones` | Create milestone |
| `GET` | `/api/milestones/{number}/issues` | List issues in milestone |

### Issues
| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/issues` | Create QC issue |
| `GET` | `/api/issues/status?issues=1,2,3` | Batch get issue statuses with QC info |
| `GET` | `/api/issues/{number}` | Get issue details |

### Comments & Actions
| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/issues/{number}/comment` | Post commit-to-commit comment |
| `POST` | `/api/issues/{number}/approve` | Approve and close issue |
| `POST` | `/api/issues/{number}/unapprove` | Unapprove and reopen issue |
| `POST` | `/api/issues/{number}/review` | Post working directory review |

### Supporting Data
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/assignees` | List repository assignees |
| `GET` | `/api/labels` | List repository labels |

### Configuration
| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/configuration/checklists` | List available checklists |
| `GET` | `/api/configuration/status` | Get configuration status |

---

## Key Implementation Details

### 1. Feature Flag (`Cargo.toml`)

```toml
[dependencies]
axum = { version = "0.7", optional = true }
tower = { version = "0.5", optional = true }
tower-http = { version = "0.6", features = ["cors", "trace"], optional = true }

[features]
api = [
    "dep:axum",
    "dep:tower",
    "dep:tower-http",
]
```

### 2. Server-Managed Auth with OnceLock (`src/api/mod.rs`)

```rust
use std::sync::OnceLock;
use crate::GitInfo;

/// Global GitInfo initialized at server startup from local repo
/// Contains auth token resolved from environment/gh CLI/.netrc
static GIT_INFO: OnceLock<GitInfo> = OnceLock::new();

pub fn init_git_info(git_info: GitInfo) {
    GIT_INFO.set(git_info).expect("GitInfo already initialized");
}

pub fn git_info() -> &'static GitInfo {
    GIT_INFO.get().expect("GitInfo not initialized - call init_git_info first")
}
```

### 3. Application State (`src/api/state.rs`)

```rust
pub struct AppState {
    pub configuration: Arc<Configuration>,
    pub disk_cache: Option<Arc<DiskCache>>,
    pub status_cache: Arc<RwLock<StatusCache>>,
}
```

Note: `GitInfo` is accessed via `git_info()` function, not through AppState. This ensures a single authenticated connection shared across all requests.

### 4. Issue Status Memory Cache (`src/api/cache.rs`)

Cache entries are either **Complete** (fetched as a main issue for status) or **Partial** (fetched as a blocking QC). Partial entries are cheaper to compute.

```rust
pub struct StatusCache {
    entries: HashMap<u64, (CacheKey, CacheEntry)>,
}

pub struct CacheKey {
    pub issue_updated_at: DateTime<Utc>,
    pub branch: String,
    pub head_commit: String,
}

pub enum CacheEntry {
    /// Full status data - created when issue is requested for its status
    Complete {
        issue: Issue,
        qc_status: QCStatus,
        commits: Vec<IssueCommit>,
        checklist_summary: ChecklistSummary,
        blocking_qc_numbers: Vec<u64>,
    },
    /// Minimal data - created when issue is fetched as a blocking QC
    Partial {
        qc_status: QCStatus,
        file_name: String,
    },
}
```

**Example flow for `GET /api/issues/status?issues=1`:**

1. Fetch issue 1 from GitHub API → get `updated_at`
2. Get current branch and HEAD commit
3. Look up issue 1 in cache → **Complete** found, key matches
4. Get blocking QC numbers from entry: `[2, 3, 4]`
5. Fetch issues 2, 3, 4 from GitHub API → get their `updated_at`
6. Look up issue 2 → **Complete** (fetched for status before) → extract `qc_status`
7. Look up issue 3 → **Partial** (fetched as blocking QC before) → extract `qc_status`
8. Look up issue 4 → **Miss** (never queried or stale)
9. Compute QC status for issue 4 (lightweight)
10. Store issue 4 as **Partial**
11. Compose and return `IssueStatusResponse` for issue 1

**Result**: Only 1 status computation required (issue 4's partial status).

**Cache entry promotion**: If a Partial entry is later requested for full status, it gets replaced with a Complete entry.

**Cache mutation on writes:**

| Operation | Cache Mutation |
|-----------|----------------|
| `POST /issues/{n}/comment` | Update `key.issue_updated_at`, add commit, update `qc_status` |
| `POST /issues/{n}/approve` | Update `key.issue_updated_at`, set `qc_status = approved` (works for Complete and Partial) |
| `POST /issues/{n}/unapprove` | Update `key.issue_updated_at`, set `qc_status = in_progress` |
| `POST /issues/{n}/review` | Update `key.issue_updated_at`, add commit |

When issue N is approved via this API, its cache entry is updated. Any other issue with N as a blocking QC sees the updated status on next read.

### 5. Error Mapping (`src/api/error.rs`)

| Error Type | HTTP Status |
|------------|-------------|
| Resource not found | 404 Not Found |
| Validation errors | 400 Bad Request |
| Blocking QCs not approved | 409 Conflict |
| GitHub API errors | 502 Bad Gateway |
| Git/config errors | 500 Internal Server Error |

### 6. CLI Integration (`src/main.rs`)

Add `serve` subcommand (gated by `api` feature):
```rust
#[cfg(feature = "api")]
Commands::Serve {
    #[arg(short, long, default_value = "3103")]
    port: String,
}
```

### 7. Server Startup & Configuration

The server requires two paths at startup:
1. **Repository path** - Local git repo (uses current directory or `--directory`)
2. **Config directory** - Resolved via `determine_config_dir()`:
   - Priority: `--config-dir` arg → `GHQC_CONFIG_HOME` env var → platform default

```rust
// At startup:
let config_dir = determine_config_dir(cli.config_dir, &env)?;
let mut configuration = Configuration::from_path(&config_dir);
configuration.load_checklists();  // Must call to populate checklists

let git_info = GitInfo::from_path(&repo_path, &env)?;
init_git_info(git_info);  // Store in OnceLock

let state = AppState {
    configuration: Arc::new(configuration),
    cache: DiskCache::from_git_info(git_info()).ok().map(Arc::new),
};
```

---

## Response Schemas

### `GET /api/issues/status?issues=1,2,3` Response (Array)

Returns an array of `IssueStatusResponse` objects:

```json
[
  {
    "issue": {
      "number": 10,
      "title": "src/main.rs",
      "state": "open",
      "html_url": "https://github.com/owner/repo/issues/10",
      "assignees": ["user1"],
      "labels": ["ghqc", "main"],
      "milestone": "v1.0",
      "created_at": "2024-01-15T10:30:00Z",
      "updated_at": "2024-01-16T14:20:00Z",
      "closed_at": null
    },
    "qc_status": {
      "status": "in_progress",
      "status_detail": "Awaiting approval",
      "approved_commit": null,
      "initial_commit": "abc1234",
      "latest_commit": "def5678"
    },
    "git_status": {
      "status": "ahead",
      "detail": "2 commits ahead of remote",
      "ahead_commits": ["ghi9012", "jkl3456"],
      "behind_commits": []
    },
    "dirty_files": ["src/main.rs", "src/lib.rs"],
    "commits": [
      {
        "hash": "def5678",
        "message": "Fix validation logic",
        "statuses": ["notification"],
        "file_changed": true
      },
      {
        "hash": "abc1234",
        "message": "Initial implementation",
        "statuses": ["initial"],
        "file_changed": true
      }
    ],
    "checklist_summary": {
      "completed": 7,
      "total": 12,
      "percentage": 58.3
    },
    "blocking_qc_status": {
      "total": 2,
      "approved_count": 1,
      "summary": "1/2 (50.0%)",
      "approved": [
        {"issue_number": 5, "file_name": "src/utils.rs"}
      ],
      "not_approved": [
        {"issue_number": 8, "file_name": "src/config.rs", "status": "in_progress"}
      ],
      "errors": []
    }
  }
]
```

**QCStatus enum values:**
- `approved` - Issue approved, no changes after
- `changes_after_approval` - Approved but file changed since
- `awaiting_review` - Latest commit notified but not reviewed
- `change_requested` - Reviewed with changes requested
- `in_progress` - Awaiting approval
- `approval_required` - Closed without approval
- `changes_to_comment` - File changes not yet commented on

**GitStatus enum values:**
- `clean` - Up to date with remote
- `ahead` - Local commits not on remote
- `behind` - Remote commits not local
- `diverged` - Both ahead and behind

**IssueCommit.statuses enum values:**
- `initial` - Initial commit when issue was created
- `notification` - Commit that was commented on
- `approved` - Commit that was approved
- `reviewed` - Commit that was reviewed

### `GET /api/configuration/status` Response

```json
{
  "directory": "/Users/user/.local/share/ghqc/config",
  "git_repository": {
    "owner": "company",
    "repo": "qc-config",
    "status": "clean",
    "dirty_files": []
  },
  "options": {
    "prepended_checklist_note": "Please review carefully",
    "checklist_display_name": "Quality Checklists",
    "logo_path": "logo.png",
    "logo_found": true,
    "checklist_directory": "checklists",
    "record_path": "record.typ"
  },
  "checklists": [
    {"name": "Code Review", "item_count": 15},
    {"name": "Data Quality", "item_count": 8},
    {"name": "Security Audit", "item_count": 22}
  ]
}
```

### `POST /api/issues/{number}/unapprove` Response

Includes impacted issues tree structure:

```json
{
  "unapproval_url": "https://github.com/owner/repo/issues/10#issuecomment-123",
  "impacted_issues": {
    "type": "some",
    "nodes": [
      {
        "issue_number": 15,
        "file_name": "src/utils.rs",
        "milestone": "v1.0",
        "relationship": "gating",
        "children": [],
        "fetch_error": null
      }
    ]
  }
}
```

**ImpactedIssues variants:**
- `{"type": "none"}` - No issues impacted
- `{"type": "api_unavailable"}` - Could not determine impact
- `{"type": "some", "nodes": [...]}` - Tree of impacted issues

---

## Files to Create

1. `src/api/mod.rs` - Module exports + OnceLock for GitInfo
2. `src/api/server.rs` - Axum server setup
3. `src/api/state.rs` - AppState (config, cache)
4. `src/api/error.rs` - Error types and HTTP mapping
5. `src/api/routes/mod.rs` - Route registration
6. `src/api/routes/milestones.rs`
7. `src/api/routes/issues.rs`
8. `src/api/routes/comments.rs`
9. `src/api/routes/status.rs`
10. `src/api/routes/configuration.rs`
11. `src/api/routes/health.rs`
12. `src/api/types/mod.rs`
13. `src/api/types/requests.rs`
14. `src/api/types/responses.rs`
15. `openapi/openapi.yml` - OpenAPI 3.0 spec (YAML format)
16. `AGENTS.md` - Instructions for AI agents

## Files to Modify

1. `Cargo.toml` - Add `api` feature and dependencies
2. `src/lib.rs` - Add `#[cfg(feature = "api")] pub mod api;`
3. `src/main.rs` - Add `serve` subcommand

---

## AGENTS.md Content

Create `AGENTS.md` with the following content:

```markdown
# AGENTS.md

Instructions for AI coding agents working on this repository.

## OpenAPI Specification Maintenance

The API is documented in `openapi/openapi.yml`. This file is manually maintained (not auto-generated).

### After completing any API-related task, you MUST:

1. **Check if `openapi/openapi.yml` needs updates** for:
   - New endpoints added
   - Endpoint paths or methods changed
   - Request body schemas modified
   - Response schemas modified
   - New error responses added
   - Path parameters changed

2. **Update `openapi/openapi.yml`** to reflect the changes:
   - Add/modify path definitions under `paths:`
   - Add/modify schema definitions under `components/schemas:`
   - Ensure request/response examples are accurate
   - Update descriptions to match implementation

3. **Verify consistency** between:
   - Route definitions in `src/api/routes/*.rs`
   - Request types in `src/api/types/requests.rs`
   - Response types in `src/api/types/responses.rs`
   - The `openapi/openapi.yml` specification

### OpenAPI File Location
- Path: `openapi/openapi.yml`
- Format: YAML (OpenAPI 3.0.3)
```

---

## OpenAPI Specification

See @openapi/openapi.yml for the complete OpenAPI 3.0.3 specification.

---

## Request/Response Examples

### Create Issue Request (`POST /api/issues`)
```json
{
  "milestone_number": 1,
  "file": "src/main.rs",
  "checklist_name": "Code Review",
  "assignees": ["user1", "user2"],
  "previous_qc": [{"issue_number": 5, "description": "Related work"}],
  "gating_qc": [],
  "relevant_qc": [],
  "relevant_files": [{"file_path": "src/lib.rs", "justification": "Shared types"}]
}
```

### Create Issue Response
```json
{
  "issue_url": "https://github.com/owner/repo/issues/10",
  "issue_number": 10,
  "blocking_created": [5],
  "blocking_errors": []
}
```

### Create Milestone Request (`POST /api/milestones`)
```json
{
  "name": "v1.0",
  "description": "First release"
}
```

### Approve Request (`POST /api/issues/{number}/approve`)
```json
{
  "commit": "abc1234",
  "note": "Approved after review",
  "force": false
}
```

### Approval Response
```json
{
  "approval_url": "https://github.com/owner/repo/issues/10#issuecomment-123",
  "skipped_unapproved": [15, 16],
  "skipped_errors": []
}
```

---

## Verification Plan

1. **Build**: `cargo build --features api`
2. **Start server**: `cargo run --features api -- serve`
3. **Test endpoints**:
   - `curl http://localhost:3103/api/health`
   - `curl http://localhost:3103/api/milestones`
   - `curl http://localhost:3103/api/configuration/checklists`
4. **Validate OpenAPI**: Use external tool to validate `openapi/openapi.yml`

---

## Implementation Order

1. Add feature flag and dependencies to `Cargo.toml`
2. Create `src/api/mod.rs` with OnceLock for GitInfo
3. Implement `src/api/error.rs` (error types)
4. Implement `src/api/types/` (request/response DTOs)
5. Implement `src/api/state.rs` (app state)
6. Implement routes: health → milestones → issues → comments → status → configuration
7. Implement `src/api/server.rs` (router assembly)
8. Add `serve` command to `src/main.rs`
9. Update `src/lib.rs` to export api module
10. Create `openapi/openapi.yml` (spec in separate file)
11. Create `AGENTS.md` with OpenAPI maintenance instructions
