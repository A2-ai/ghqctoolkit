# API Testing Infrastructure Plan

## Context

The API server (`src/api/`) needs unit tests for its routes. Currently, `AppState` contains `Arc<GitInfo>` (concrete type), which makes it impossible to inject mock implementations for testing. The codebase already uses trait-based architecture with `GitHubReader`, `GitHubWriter`, `GitHelpers`, `GitRepository`, `GitFileOps`, and `GitStatusOps` traits, but these can't be used for testing API routes without modifying how `AppState` stores its git provider.

**Problem**: AppState's concrete `GitInfo` type prevents dependency injection for testing
**Solution**: Use trait object wrapper to allow both real `GitInfo` (production) and `MockGitInfo` (tests)
**User preference**: Trait object wrapper with small production code change (cleaner than alternatives)

## Recommended Approach: Trait Object Wrapper

We'll create a `GitProvider` super-trait that combines all git traits, then modify `AppState` to hold `Arc<dyn GitProvider + Send + Sync>` instead of `Arc<GitInfo>`. This allows:
- Production code to use real `GitInfo`
- Test code to use `MockGitInfo` implementing the same traits
- Minimal performance overhead (trait objects have negligible cost in this context)
- Clean, idiomatic Rust design

## Critical Files

### Files to Create
- `src/git/provider.rs` - GitProvider super-trait definition and implementation on GitInfo
- `src/api/tests/helpers.rs` - MockGitInfo and test utilities
- `src/api/tests/mod.rs` - Test module organization
- `src/api/tests/routes/issues_tests.rs` - Example tests for issue routes

### Files to Modify
- `src/api/state.rs` - Change git_info field type, add test constructor
- `src/git/mod.rs` - export provider module
- `src/configuration.rs` - Add test_default() helper for minimal test configs
- `src/api/mod.rs` - Add test module with #[cfg(test)]

## Implementation Steps

### Step 1: Create GitProvider Super-Trait

**File**: `src/git/provider.rs`

Create a trait that combines all git traits using trait inheritance:

```rust
//! Git provider trait combining all git operations.

use super::{GitFileOps, GitHelpers, GitRepository, GitStatusOps};
use super::api::{GitHubReader, GitHubWriter};

/// Super-trait combining all git/GitHub operations.
///
/// This trait enables dependency injection for testing by allowing both
/// GitInfo (production) and MockGitInfo (tests) to be used interchangeably.
pub trait GitProvider:
    GitHubReader
    + GitHubWriter
    + GitHelpers
    + GitRepository
    + GitFileOps
    + GitStatusOps
    + Send
    + Sync
{}

// Blanket implementation: any type implementing all traits automatically implements GitProvider
impl<T> GitProvider for T
where
    T: GitHubReader
        + GitHubWriter
        + GitHelpers
        + GitRepository
        + GitFileOps
        + GitStatusOps
        + Send
        + Sync
{}
```

**Modify**: `src/git/mod.rs`
- Add `mod provider;`
- Add `pub use provider::GitProvider;` to exports

### Step 2: Modify AppState to Use GitProvider

**File**: `src/api/state.rs`

Change the git_info field type and add a test constructor:

```rust
use crate::{Configuration, DiskCache, GitInfo, GitProvider};
use crate::api::cache::StatusCache;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Application state shared across all request handlers.
#[derive(Clone)]
pub struct AppState {
    /// Git repository and GitHub API access.
    git_info: Arc<dyn GitProvider>,  // CHANGED: was Arc<GitInfo>
    /// Configuration loaded at startup.
    pub configuration: Arc<Configuration>,
    /// Disk-based cache for GitHub API responses.
    disk_cache: Option<Arc<DiskCache>>,
    /// In-memory cache for issue status responses.
    pub status_cache: Arc<RwLock<StatusCache>>,
}

impl AppState {
    /// Create a new AppState with the given configuration.
    pub fn new(
        git_info: GitInfo,
        configuration: Configuration,
        disk_cache: Option<DiskCache>,
    ) -> Self {
        Self {
            git_info: Arc::new(git_info),  // GitInfo implements GitProvider via blanket impl
            configuration: Arc::new(configuration),
            disk_cache: disk_cache.map(Arc::new),
            status_cache: Arc::new(RwLock::new(StatusCache::new())),
        }
    }

    pub fn git_info(&self) -> &dyn GitProvider {  // CHANGED: was &GitInfo
        &**self.git_info
    }

    pub fn disk_cache(&self) -> Option<&DiskCache> {
        self.disk_cache.as_ref().map(|d| &**d)
    }

    /// Create AppState for testing with a mock GitProvider.
    #[cfg(test)]
    pub fn test_new(
        git_info: impl GitProvider + 'static,
        configuration: Configuration,
    ) -> Self {
        Self {
            git_info: Arc::new(git_info),
            configuration: Arc::new(configuration),
            disk_cache: None,
            status_cache: Arc::new(RwLock::new(StatusCache::new())),
        }
    }
}
```

**Note**: Route handlers should continue to work without changes since they use trait methods, not GitInfo-specific methods.

### Step 3: Create MockGitInfo Test Helper

**File**: `src/api/tests/helpers.rs`

Create a comprehensive mock implementing all traits with builder pattern:

```rust
//! Test helpers for API route testing.

use crate::git::{
    GitFileOps, GitFileOpsError, GitHelpers, GitRepository, GitRepositoryError,
    GitStatusOps, GitStatusError, GitProvider
};
use crate::git::api::{GitHubReader, GitHubWriter, GitHubApiError};
use crate::git::types::{GitAuthor, GitCommit, GitStatus};
use crate::CommentBody;
use gix::ObjectId;
use octocrab::models::{issues::Issue, Label};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Mock implementation of all git traits for testing.
#[derive(Clone)]
pub struct MockGitInfo {
    // Repository metadata
    owner: String,
    repo: String,
    current_commit: String,
    current_branch: String,

    // Mock data storage
    issues: Arc<Mutex<HashMap<u64, Issue>>>,
    blocked_issues: Arc<Mutex<HashMap<u64, Vec<Issue>>>>,

    // Status
    dirty_files: Arc<Mutex<Vec<PathBuf>>>,

    // Call tracking (for assertions)
    calls: Arc<Mutex<Vec<String>>>,
}

impl MockGitInfo {
    /// Create a new mock with default values.
    pub fn builder() -> MockGitInfoBuilder {
        MockGitInfoBuilder::new()
    }
}

/// Builder for MockGitInfo.
pub struct MockGitInfoBuilder {
    owner: String,
    repo: String,
    commit: String,
    branch: String,
    issues: HashMap<u64, Issue>,
    blocked_issues: HashMap<u64, Vec<Issue>>,
    dirty_files: Vec<PathBuf>,
}

impl MockGitInfoBuilder {
    pub fn new() -> Self {
        Self {
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            commit: "abc123".to_string(),
            branch: "main".to_string(),
            issues: HashMap::new(),
            blocked_issues: HashMap::new(),
            dirty_files: Vec::new(),
        }
    }

    pub fn with_owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = owner.into();
        self
    }

    pub fn with_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = repo.into();
        self
    }

    pub fn with_commit(mut self, commit: impl Into<String>) -> Self {
        self.commit = commit.into();
        self
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    pub fn with_issue(mut self, number: u64, issue: Issue) -> Self {
        self.issues.insert(number, issue);
        self
    }

    pub fn with_blocked_issues(mut self, issue_number: u64, blocking: Vec<Issue>) -> Self {
        self.blocked_issues.insert(issue_number, blocking);
        self
    }

    pub fn with_dirty_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.dirty_files.push(file.into());
        self
    }

    pub fn build(self) -> MockGitInfo {
        MockGitInfo {
            owner: self.owner,
            repo: self.repo,
            current_commit: self.commit,
            current_branch: self.branch,
            issues: Arc::new(Mutex::new(self.issues)),
            blocked_issues: Arc::new(Mutex::new(self.blocked_issues)),
            dirty_files: Arc::new(Mutex::new(self.dirty_files)),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Default for MockGitInfoBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Implement all required traits for MockGitInfo
// (GitProvider is auto-implemented via blanket impl)

impl GitRepository for MockGitInfo {
    fn commit(&self) -> Result<String, GitRepositoryError> {
        Ok(self.current_commit.clone())
    }

    fn branch(&self) -> Result<String, GitRepositoryError> {
        Ok(self.current_branch.clone())
    }

    fn owner(&self) -> &str {
        &self.owner
    }

    fn repo(&self) -> &str {
        &self.repo
    }
}

impl GitHelpers for MockGitInfo {
    fn file_content_url(&self, git_ref: &str, file: &Path) -> String {
        format!(
            "https://github.com/{}/{}/blob/{}/{}",
            self.owner, self.repo, git_ref, file.display()
        )
    }

    fn commit_comparison_url(&self, current: &ObjectId, previous: &ObjectId) -> String {
        format!(
            "https://github.com/{}/{}/compare/{}..{}",
            self.owner, self.repo, previous, current
        )
    }

    fn issue_url(&self, issue_number: u64) -> String {
        format!("https://github.com/{}/{}/issues/{}", self.owner, self.repo, issue_number)
    }
}

impl GitStatusOps for MockGitInfo {
    fn status(&self) -> Result<GitStatus, GitStatusError> {
        Ok(GitStatus {
            dirty: !self.dirty_files.lock().unwrap().is_empty(),
        })
    }

    fn dirty(&self) -> Result<Vec<PathBuf>, GitStatusError> {
        Ok(self.dirty_files.lock().unwrap().clone())
    }
}

impl GitFileOps for MockGitInfo {
    fn commits(&self, _branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
        Ok(vec![]) // Can be extended as needed
    }

    fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
        Ok(vec![])
    }

    fn file_bytes_at_commit(&self, _file: &Path, _commit: &ObjectId) -> Result<Vec<u8>, GitFileOpsError> {
        Ok(vec![])
    }
}

impl GitHubReader for MockGitInfo {
    fn get_milestones(&self) -> impl Future<Output = Result<Vec<octocrab::models::Milestone>, GitHubApiError>> + Send {
        async { Ok(vec![]) }
    }

    fn get_issues(&self, _milestone: Option<u64>) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send {
        async { Ok(vec![]) }
    }

    fn get_issue(&self, issue_number: u64) -> impl Future<Output = Result<Issue, GitHubApiError>> + Send {
        let issues = self.issues.clone();
        let calls = self.calls.clone();

        async move {
            calls.lock().unwrap().push(format!("get_issue({})", issue_number));

            issues
                .lock()
                .unwrap()
                .get(&issue_number)
                .cloned()
                .ok_or_else(|| GitHubApiError::NotFound(format!("Issue {} not found", issue_number)))
        }
    }

    fn get_assignees(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
        async { Ok(vec![]) }
    }

    fn get_user_details(&self, _username: &str) -> impl Future<Output = Result<(String, Option<String>), GitHubApiError>> + Send {
        async { Ok(("test-user".to_string(), None)) }
    }

    fn get_labels(&self) -> impl Future<Output = Result<Vec<Label>, GitHubApiError>> + Send {
        async { Ok(vec![]) }
    }

    fn get_issue_comments(&self, _issue: &Issue) -> impl Future<Output = Result<Vec<octocrab::models::issues::Comment>, GitHubApiError>> + Send {
        async { Ok(vec![]) }
    }

    fn get_issue_events(&self, _issue: &Issue) -> impl Future<Output = Result<Vec<octocrab::models::events::Event>, GitHubApiError>> + Send {
        async { Ok(vec![]) }
    }

    fn get_blocked_issues(&self, issue_number: u64) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send {
        let blocked = self.blocked_issues.clone();
        let calls = self.calls.clone();

        async move {
            calls.lock().unwrap().push(format!("get_blocked_issues({})", issue_number));

            Ok(blocked
                .lock()
                .unwrap()
                .get(&issue_number)
                .cloned()
                .unwrap_or_default())
        }
    }
}

impl GitHubWriter for MockGitInfo {
    fn create_milestone(&self, _name: &str, _desc: &Option<String>) -> impl Future<Output = Result<octocrab::models::Milestone, GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }

    fn post_issue(&self, _issue: &crate::QCIssue) -> impl Future<Output = Result<Issue, GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }

    fn post_comment<T: CommentBody + 'static>(&self, _comment: &T) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }

    fn close_issue(&self, _issue_number: u64) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }

    fn open_issue(&self, _issue_number: u64) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }

    fn create_label(&self, _name: &str, _color: &str) -> impl Future<Output = Result<Label, GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }

    fn block_issue(&self, _blocked: u64, _blocking: u64) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        async { Err(GitHubApiError::NotImplemented) }
    }
}

/// Helper to load test issue fixtures from JSON
pub fn load_test_issue(name: &str) -> Issue {
    let json = std::fs::read_to_string(
        format!("src/tests/github_api/issues/{}.json", name)
    ).expect("Failed to load test fixture");

    serde_json::from_str(&json).expect("Failed to parse test fixture")
}
```

### Step 4: Create Test Structure for All Routes

**File**: `src/api/tests/mod.rs`

```rust
//! Tests for API routes.

pub mod helpers;

#[cfg(test)]
mod routes;
```

**File**: `src/api/tests/routes/mod.rs`

```rust
mod comments_tests;
mod configuration_tests;
mod health_tests;
mod issues_tests;
mod milestones_tests;
mod status_tests;
```

#### Issues Tests

**File**: `src/api/tests/routes/issues_tests.rs`

Tests for:
- `GET /api/issues/{number}` - get_issue
- `GET /api/issues/{number}/blocked` - get_blocked_issues
- `GET /api/issues/status?issues=1,2,3` - batch_get_issue_status

```rust
use crate::api::{server::create_router, state::AppState, types::Issue};
use crate::api::tests::helpers::{MockGitInfo, load_test_issue};
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for oneshot()

// Helper to load configuration from test fixtures
fn load_test_config() -> Configuration {
    Configuration::from_file("tests/config/options.yaml")
        .expect("Failed to load test configuration")
}

#[tokio::test]
async fn test_get_issue_success() {
    // Setup mock with test issue
    let test_issue = load_test_issue("test_file_issue");
    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue.clone())
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = load_test_config();
    let state = AppState::test_new(mock, config);
    let app = create_router(state);

    // Make request
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/1")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap();

    // Assert response
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let issue: Issue = serde_json::from_slice(&body).unwrap();
    assert_eq!(issue.number, 1);
}

#[tokio::test]
async fn test_get_issue_not_found() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::test_new(mock, config);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/999")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap();

    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_blocked_issues_success() {
    let main_issue = load_test_issue("test_file_issue");
    let blocking_issue = load_test_issue("config_file_issue");

    let mock = MockGitInfo::builder()
        .with_issue(1, main_issue)
        .with_blocked_issues(1, vec![blocking_issue])
        .build();

    let config = load_test_config();
    let state = AppState::test_new(mock, config);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/1/blocked")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let issues: Vec<crate::api::types::BlockedIssueStatus> =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(issues.len(), 1);
}

#[tokio::test]
async fn test_get_blocked_issues_empty() {
    let main_issue = load_test_issue("test_file_issue");

    let mock = MockGitInfo::builder()
        .with_issue(1, main_issue)
        .with_blocked_issues(1, vec![])
        .build();

    let config = load_test_config();
    let state = AppState::test_new(mock, config);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/1/blocked")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let issues: Vec<crate::api::types::BlockedIssueStatus> =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(issues.len(), 0);
}

#[tokio::test]
async fn test_batch_get_issue_status() {
    let issue1 = load_test_issue("test_file_issue");
    let issue2 = load_test_issue("config_file_issue");

    let mock = MockGitInfo::builder()
        .with_issue(1, issue1)
        .with_issue(2, issue2)
        .build();

    let config = load_test_config();
    let state = AppState::test_new(mock, config);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/status?issues=1,2")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let statuses: Vec<crate::api::types::IssueStatusResponse> =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(statuses.len(), 2);
}
```

#### Comments Tests

**File**: `src/api/tests/routes/comments_tests.rs`

Tests for:
- `POST /api/comments` - create_comment
- `POST /api/approve` - approve_issue
- `POST /api/unapprove` - unapprove_issue
- `POST /api/review` - review_issue

```rust
use crate::api::{server::create_router, state::AppState};
use crate::api::tests::helpers::MockGitInfo;
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

// Helper to load configuration from test fixtures
fn load_test_config() -> Configuration {
    Configuration::from_file("tests/config/options.yaml")
        .expect("Failed to load test configuration")
}

#[tokio::test]
async fn test_create_comment() {
    // TODO: Implement test for POST /api/comments
    // Needs: mock with issue, commits for diff generation
}

#[tokio::test]
async fn test_approve_issue() {
    // TODO: Implement test for POST /api/approve
    // Needs: mock with issue to approve, verify issue closed
}

#[tokio::test]
async fn test_unapprove_issue() {
    // TODO: Implement test for POST /api/unapprove
    // Needs: mock with closed issue, verify reopened
}

#[tokio::test]
async fn test_review_issue() {
    // TODO: Implement test for POST /api/review
    // Needs: mock with dirty files for diff generation
}
```

#### Milestones Tests

**File**: `src/api/tests/routes/milestones_tests.rs`

Tests for:
- `GET /api/milestones` - list_milestones
- `POST /api/milestones` - create_milestone
- `GET /api/milestones/{number}/issues` - list_milestone_issues

```rust
use crate::api::{server::create_router, state::AppState};
use crate::api::tests::helpers::MockGitInfo;
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn load_test_config() -> Configuration {
    Configuration::from_file("tests/config/options.yaml")
        .expect("Failed to load test configuration")
}

#[tokio::test]
async fn test_list_milestones() {
    // TODO: Implement test for GET /api/milestones
    // Needs: mock with list of milestones
}

#[tokio::test]
async fn test_create_milestone() {
    // TODO: Implement test for POST /api/milestones
    // Needs: mock that can create milestones
}

#[tokio::test]
async fn test_list_milestone_issues() {
    // TODO: Implement test for GET /api/milestones/{number}/issues
    // Needs: mock with milestone and its issues
}
```

#### Status Tests

**File**: `src/api/tests/routes/status_tests.rs`

Tests for:
- `GET /api/assignees` - list_assignees

```rust
use crate::api::{server::create_router, state::AppState};
use crate::api::tests::helpers::MockGitInfo;
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn load_test_config() -> Configuration {
    Configuration::from_file("tests/config/options.yaml")
        .expect("Failed to load test configuration")
}

#[tokio::test]
async fn test_list_assignees() {
    // TODO: Implement test for GET /api/assignees
    // Needs: mock with list of assignees
}
```

#### Configuration Tests

**File**: `src/api/tests/routes/configuration_tests.rs`

Tests for:
- `GET /api/checklists` - list_checklists
- `GET /api/configuration/status` - get_configuration_status

```rust
use crate::api::{server::create_router, state::AppState};
use crate::api::tests::helpers::MockGitInfo;
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn load_test_config() -> Configuration {
    Configuration::from_file("tests/config/options.yaml")
        .expect("Failed to load test configuration")
}

#[tokio::test]
async fn test_list_checklists() {
    // TODO: Implement test for GET /api/checklists
    // Tests reading from configuration, not mocking
}

#[tokio::test]
async fn test_get_configuration_status() {
    // TODO: Implement test for GET /api/configuration/status
    // Tests configuration validation
}
```

#### Health Tests

**File**: `src/api/tests/routes/health_tests.rs`

Tests for:
- `GET /api/health` - health_check

```rust
use crate::api::{server::create_router, state::AppState};
use crate::api::tests::helpers::MockGitInfo;
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn load_test_config() -> Configuration {
    Configuration::from_file("tests/config/options.yaml")
        .expect("Failed to load test configuration")
}

#[tokio::test]
async fn test_health_check() {
    let mock = MockGitInfo::builder().build();
    let config = load_test_config();
    let state = AppState::test_new(mock, config);
    let app = create_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let health: crate::api::types::HealthResponse =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(health.status, "ok");
}
```

**Modify**: `src/api/mod.rs`

Add test module:
```rust
#[cfg(test)]
mod tests;
```

### Step 6: Verify Basic Infrastructure

After implementing Steps 1-4, verify that the basic infrastructure works:

**Compilation checks:**

1. **Production code compiles**: `cargo build --all-features`
2. **Tests compile**: `cargo test --all-features --no-run`
3. **Existing tests still pass**: `cargo test --all-features` (check that API changes don't break existing tests)
4. **New tests run**: `cargo test --all-features api::tests`

### Step 5: Implement Remaining Tests

After verifying the basic infrastructure works (Step 6), implement the remaining TODO tests:

**Priority 1 - Issues (already implemented):**
- ✅ `get_issue` - success and error cases
- ✅ `get_blocked_issues` - with and without blocking issues
- ✅ `batch_get_issue_status` - multiple issues

**Priority 2 - Comments:**
- `create_comment` - requires file bytes at commit mocking
- `approve_issue` - verify issue gets closed
- `unapprove_issue` - verify issue gets reopened
- `review_issue` - requires dirty file handling

**Priority 3 - Milestones & Status:**
- `list_milestones` - mock milestone list
- `create_milestone` - mock milestone creation
- `list_milestone_issues` - mock milestone with issues
- `list_assignees` - mock assignee list

**Priority 4 - Configuration & Health:**
- `list_checklists` - reads from actual config, minimal mocking
- `get_configuration_status` - configuration validation
- ✅ `health_check` - simple status check (implemented)

**Testing Scenarios to Cover:**
- Cache hit/miss scenarios (batch_get_issue_status)
- Dirty working directory detection (review_issue)
- Error cases (GitHub API errors, invalid inputs)
- Blocking QC status determination (batch_get_issue_status)
- Integration with `IssueThread` and status cache

## Verification

### Testing the Changes

Run the new API tests:
```bash
cargo test --all-features api::tests::routes::issues_tests
```

Verify specific test cases:
```bash
# Test get_issue endpoint
cargo test --all-features test_get_issue_success

# Test get_blocked_issues endpoint
cargo test --all-features test_get_blocked_issues

# Test error cases
cargo test --all-features test_get_issue_not_found
```

### Ensure No Regressions

Run full test suite to ensure trait object changes don't break existing code:
```bash
cargo test --all-features
```

### Manual API Server Testing

Start the API server and verify routes still work:
```bash
cargo run --all-features -- serve
curl http://localhost:3000/api/health
```

## Benefits of This Approach

1. **Minimal Production Impact**: Only changes AppState field type from concrete to trait object
2. **Type Safety**: GitProvider trait ensures all required methods are implemented
3. **Consistent Pattern**: Follows existing trait-based architecture
4. **Easy to Test**: Builder pattern makes test setup readable and maintainable
5. **Extensible**: Easy to add more mock behaviors or test scenarios
6. **Zero Runtime Cost**: Arc<dyn Trait> has negligible overhead vs Arc<ConcreteType>

## Alternative Approaches Considered

- **Generic AppState**: Would require generics throughout API module, affecting all route signatures
- **Newtype wrapper**: Extra abstraction layer with method forwarding
- **Conditional compilation**: Duplicated struct definitions, confusing maintenance

The trait object wrapper was chosen as the cleanest solution with the best balance of simplicity and testability.
