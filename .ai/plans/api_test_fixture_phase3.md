# Plan: Add Tests for batch_get_issue_status Endpoint

## Context

The `batch_get_issue_status` endpoint is a critical API route that returns detailed QC status for multiple issues in a single request. It implements sophisticated caching with two layers:

1. **Memory cache** (`StatusCache`) - In-memory cache with cache keys based on issue `updated_at`, git branch, and HEAD commit
2. **Disk cache** (optional) - GitHub API response caching

Currently, this endpoint has no YAML-based tests, and the cache behavior is not tested end-to-end.

**Why this change is needed:**
- Ensure the batch endpoint works correctly for basic use cases
- Verify cache population happens on first request
- Verify cache hits work correctly on subsequent requests with same parameters
- Consistent with other API endpoints having YAML tests

**Endpoint details:**
- **Route:** `GET /api/issues/status?issues=1,2,3` (comma-separated issue numbers)
- **Location:** `src/api/routes/issues.rs` (lines 43-116)
- **Response:** Array of `IssueStatusResponse` with QC status, commits, checklist summary, blocking QC status

## Implementation Approach

### Step 1: Create Basic YAML Test

**File:** `src/api/tests/cases/issues/batch_get_issue_status_success.yaml`

This test verifies the endpoint returns valid status for multiple issues.

```yaml
name: "GET /api/issues/status - batch retrieve multiple issues"
description: "Retrieve status for multiple issues in a single request"

fixtures:
  issues:
    - test_file_issue.json    # Issue #1
    - config_file_issue.json  # Issue #2

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: GET
  path: "/api/issues/status"
  query:
    issues: "1,2"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      exact_length: 2
      item_fields:
        - issue
        - qc_status
        - dirty
        - branch
        - commits
        - checklist_summary
        - blocking_qc_status
      first_item:
        dirty: false
        branch: "main"
```

**Key aspects:**
- Tests comma-separated query parameter parsing
- Verifies response includes all required fields
- Uses existing fixtures (test_file_issue.json, config_file_issue.json)
- Checks array has exactly 2 items (one per requested issue)

### Step 2: Create YAML Test for Single Issue

**File:** `src/api/tests/cases/issues/batch_get_issue_status_single.yaml`

This test verifies the endpoint works with a single issue number.

```yaml
name: "GET /api/issues/status - single issue"
description: "Retrieve status for a single issue"

fixtures:
  issues:
    - test_file_issue.json

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: GET
  path: "/api/issues/status"
  query:
    issues: "1"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      exact_length: 1
      item_fields:
        - issue
        - qc_status
        - dirty
        - branch
```

### Step 3: Create YAML Test for Dirty File Detection

**File:** `src/api/tests/cases/issues/batch_get_issue_status_dirty.yaml`

This test verifies dirty file detection in the response.

```yaml
name: "GET /api/issues/status - dirty file detection"
description: "Verify dirty=true when file has uncommitted changes"

fixtures:
  issues:
    - test_file_issue.json

git_state:
  commit: "abc123"
  branch: "main"
  dirty_files:
    - "src/test.rs"

request:
  method: GET
  path: "/api/issues/status"
  query:
    issues: "1"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      exact_length: 1
      item_fields:
        - issue
        - dirty
      first_item:
        dirty: true
```

### Step 4: Create New Fixture with Blocking QC Content

**File:** `src/api/tests/fixtures/issues/issue_with_blocking.json`

Create a new fixture (issue #3) that references existing issues in its body to test blocking QC parsing.

**Base on existing fixture structure** (copy from `config_file_issue.json`) but with:
- `"number": 3`
- `"title": "src/blocking_test.rs"`
- **Keep `"milestone"` field** (required by `IssueThread::from_issue`, see `src/issue.rs:111-115`)
- `"body"` field containing:

```markdown
Quality check issue for src/blocking_test.rs

## Metadata
initial qc commit: abc123def456
git branch: feature/blocking-test
author: Test Developer <test@example.com>

## Relevant Files

### Gating QC
- [src/test.rs](https://github.com/owner/repo/issues/1)

### Previous QC
- [src/config.rs](https://github.com/owner/repo/issues/2)
```

**IMPORTANT:** The links must be markdown format with GitHub issue URLs for `parse_blocking_qcs()` to extract them (regex at `src/issue.rs:22` requires this format). This allows testing that blocking issues #1 and #2 are correctly parsed.

### Step 5: Create YAML Test for Blocking QC Parsing

**File:** `src/api/tests/cases/issues/batch_get_issue_status_blocking.yaml`

This test verifies blocking QC parsing from issue body.

```yaml
name: "GET /api/issues/status - blocking QC parsing"
description: "Verify blocking_qc_status is populated by parsing issue body"

fixtures:
  issues:
    - test_file_issue.json       # Issue #1 (will be blocking)
    - config_file_issue.json     # Issue #2 (will be blocking)
    - issue_with_blocking.json   # Issue #3 (blocked by #1 and #2)

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: GET
  path: "/api/issues/status"
  query:
    issues: "3"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      exact_length: 1
      item_fields:
        - issue
        - blocking_qc_status
      first_item:
        blocking_qc_status:
          total: 2
```

**Note:** This test validates actual blocking QC parsing behavior. The fixture's body explicitly references issues #1 and #2, so `blocking_qc_status.total` should be 2. This requires the nested field validation enhancement in Step 6a.

### Step 6: Create YAML Test for Invalid Input

**File:** `src/api/tests/cases/issues/batch_get_issue_status_invalid.yaml`

This test verifies error handling for invalid input.

```yaml
name: "GET /api/issues/status - no valid issue numbers"
description: "Returns error when no valid issue numbers provided"

fixtures:
  issues: []

git_state: {}

request:
  method: GET
  path: "/api/issues/status"
  query:
    issues: ""

response:
  status: 400
```

### Step 6a: Enhance Schema Matcher for Nested first_item Assertions

**File:** `src/api/tests/harness/assertions.rs`

The current `first_item` implementation only does exact value equality for top-level fields. To test `blocking_qc_status.total: 2`, we need to support nested field validation.

**Current behavior** (lines 201-224):
```rust
if let Some(first_item_expected) = &schema.first_item {
    // ... checks first_obj.get(key) == expected_value (exact equality)
}
```

**Enhancement needed:**
Support nested field paths in `first_item` values. When a field value is itself an object (HashMap), recursively validate nested fields instead of requiring exact equality.

**Implementation approach:**
```rust
// In validate_schema method, update first_item validation:
if let Some(first_item_expected) = &schema.first_item {
    if arr.is_empty() {
        errors.push("Cannot validate first_item: array is empty".to_string());
    } else if let Value::Object(first_obj) = &arr[0] {
        for (key, expected_value) in first_item_expected {
            match first_obj.get(key) {
                Some(actual_value) => {
                    // NEW: If both are objects, do nested partial matching
                    if let (Value::Object(expected_nested), Value::Object(actual_nested)) =
                        (expected_value, actual_value)
                    {
                        for (nested_key, nested_expected) in expected_nested {
                            match actual_nested.get(nested_key) {
                                Some(nested_actual) if nested_actual == nested_expected => {},
                                Some(nested_actual) => {
                                    errors.push(format!(
                                        "First item field '{}.{}': expected {}, got {}",
                                        key, nested_key,
                                        serde_json::to_string(nested_expected).unwrap_or_default(),
                                        serde_json::to_string(nested_actual).unwrap_or_default()
                                    ));
                                }
                                None => errors.push(format!(
                                    "First item field '{}' missing nested field: '{}'",
                                    key, nested_key
                                )),
                            }
                        }
                    } else if actual_value != expected_value {
                        // Original exact equality for non-objects
                        errors.push(format!(
                            "First item field '{}': expected {}, got {}",
                            key,
                            serde_json::to_string(expected_value).unwrap_or_default(),
                            serde_json::to_string(actual_value).unwrap_or_default()
                        ));
                    }
                }
                None => errors.push(format!("First item missing field: '{}'", key)),
            }
        }
    } else {
        errors.push("First array item is not an object (cannot check fields)".to_string());
    }
}
```

This allows YAML like:
```yaml
first_item:
  blocking_qc_status:
    total: 2
```

Which validates that `response[0].blocking_qc_status.total == 2` while ignoring other fields in `blocking_qc_status`.

### Step 7: Create Rust Test for Cache Behavior

**File:** `src/api/tests/routes/issues_tests.rs` (new file)

The YAML test harness doesn't support multi-request scenarios with stateful cache verification. We need a traditional Rust test for cache behavior.

**Comprehensive Test: Cache Population and Cache Hit**

```rust
//! Issue status endpoint tests
//! Tests cache behavior for batch_get_issue_status endpoint

use crate::api::cache::CacheKey;
use crate::api::tests::helpers::{MockGitInfo, load_test_issue};
use crate::api::{server::create_router, state::AppState};
use crate::api::types::IssueStatusResponse;
use crate::Configuration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_batch_get_issue_status_cache_behavior() {
    // Setup: Create mock with test issues
    let test_issue_1 = load_test_issue("test_file_issue");
    let test_issue_2 = load_test_issue("config_file_issue");

    let mock = MockGitInfo::builder()
        .with_issue(1, test_issue_1.clone())
        .with_issue(2, test_issue_2.clone())
        .with_commit("abc123")
        .with_branch("main")
        .build();

    let config = Configuration::default();
    let state = AppState::new(mock, config, None);
    let app = create_router(state.clone());

    // FIRST REQUEST: Cache should be empty, will populate
    let response1 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/issues/status?issues=1,2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response1.status(), StatusCode::OK);

    // Verify cache was populated
    let cache = state.status_cache.read().await;
    let key1 = CacheKey {
        issue_updated_at: test_issue_1.updated_at,
        branch: "main".to_string(),
        head_commit: "abc123".to_string(),
    };
    let key2 = CacheKey {
        issue_updated_at: test_issue_2.updated_at,
        branch: "main".to_string(),
        head_commit: "abc123".to_string(),
    };
    assert!(cache.get(1, &key1).is_some(), "Issue 1 should be cached after first request");
    assert!(cache.get(2, &key2).is_some(), "Issue 2 should be cached after first request");
    drop(cache);

    // SECOND REQUEST: Should hit cache (no new fetches needed)
    let response2 = app
        .oneshot(
            Request::builder()
                .uri("/api/issues/status?issues=1,2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response2.status(), StatusCode::OK);

    // Verify responses are consistent
    let body1 = axum::body::to_bytes(response1.into_body(), usize::MAX).await.unwrap();
    let body2 = axum::body::to_bytes(response2.into_body(), usize::MAX).await.unwrap();
    let status1: Vec<IssueStatusResponse> = serde_json::from_slice(&body1).unwrap();
    let status2: Vec<IssueStatusResponse> = serde_json::from_slice(&body2).unwrap();

    assert_eq!(status1.len(), 2, "First request should return 2 issues");
    assert_eq!(status2.len(), 2, "Second request should return 2 issues");
    assert_eq!(status1[0].issue.number, status2[0].issue.number, "Issue numbers should match");
    assert_eq!(status1[1].issue.number, status2[1].issue.number, "Issue numbers should match");

    // Cache should still have valid entries
    let cache = state.status_cache.read().await;
    assert!(cache.get(1, &key1).is_some(), "Issue 1 should still be cached after second request");
    assert!(cache.get(2, &key2).is_some(), "Issue 2 should still be cached after second request");
}
```

**Why one comprehensive test:**
- Tests the complete workflow: empty cache → populate → hit
- Simpler to maintain than separate tests
- Validates the most common use case
- Cache invalidation scenarios (branch/commit changes) are already tested in unit tests in `src/api/cache.rs`

### Step 9: Update Module Structure

**File:** `src/api/tests/routes/mod.rs`

Add or uncomment the `issues_tests` module if it doesn't exist:

```rust
// Phase 1: Converted to YAML (removed)
// mod health_tests;

// Phase 2: Partially converted to YAML
mod issues_tests;  // ADD THIS - has cache behavior tests
mod milestones_tests;

// Phase 3+: Keep these until converted
mod comments_tests;
mod configuration_tests;
```

**Note:** If `issues_tests.rs` doesn't exist, create it. If it exists and was previously deleted/commented, restore it with only the cache tests.

## Critical Files

**Files to create:**
- `src/api/tests/fixtures/issues/issue_with_blocking.json` - New fixture with blocking QC references in body
- `src/api/tests/cases/issues/batch_get_issue_status_success.yaml` - Basic batch test (multiple issues)
- `src/api/tests/cases/issues/batch_get_issue_status_single.yaml` - Single issue test
- `src/api/tests/cases/issues/batch_get_issue_status_dirty.yaml` - Dirty file detection test
- `src/api/tests/cases/issues/batch_get_issue_status_blocking.yaml` - Blocking QC parsing test
- `src/api/tests/cases/issues/batch_get_issue_status_invalid.yaml` - Error handling test
- `src/api/tests/routes/issues_tests.rs` - Rust test for cache behavior

**Files to modify:**
- `src/api/tests/harness/assertions.rs` - Add nested field validation for `first_item`
- `src/api/tests/routes/mod.rs` - Uncomment issues_tests module

**Existing files referenced:**
- `src/api/routes/issues.rs` - Endpoint implementation
- `src/api/cache.rs` - Cache types and logic
- `src/api/state.rs` - AppState with status_cache
- `src/api/fetch_helpers.rs` - Cache lookup and population logic

## Test Strategy

### YAML Tests (Stateless, Single Request)
✅ Basic functionality with multiple issues
✅ Single issue retrieval
✅ Dirty file detection (working directory changes)
✅ Blocking QC parsing (validates `blocking_qc_status.total` from issue body)
✅ Invalid input error handling
✅ Response structure validation

**Limitations:** Cannot test cache hits across multiple requests

### Rust Tests (Stateful, Multi-Request)
✅ Cache population on first request
✅ Cache hit on second request with identical parameters
✅ Consistent responses across cache hit and miss scenarios

**Note:** Cache invalidation scenarios (branch change, commit change, issue update) are already covered by unit tests in `src/api/cache.rs` (lines 292-493).

**Why Rust tests needed:** The YAML harness is designed for single request/response tests and doesn't support:
- Inspecting cache state between requests
- Making multiple sequential requests to verify stateful behavior
- Controlling git state changes between requests

## Verification

### Run YAML Tests

```bash
# Run all YAML-based tests
cargo test --features api run_all_test_cases -- --nocapture
```

**Expected output:**
```
Running test: .../issues/batch_get_issue_status_success.yaml
  ✓ PASSED (status: 200)
Running test: .../issues/batch_get_issue_status_single.yaml
  ✓ PASSED (status: 200)
Running test: .../issues/batch_get_issue_status_dirty.yaml
  ✓ PASSED (status: 200)
Running test: .../issues/batch_get_issue_status_blocking.yaml
  ✓ PASSED (status: 200)
Running test: .../issues/batch_get_issue_status_invalid.yaml
  ✓ PASSED (status: 400)

Total: 16  Passed: 16  Failed: 0
```

### Run Rust Cache Tests

```bash
# Run cache behavior tests
cargo test --features api test_batch_get_issue_status_cache -- --nocapture
```

**Expected output:**
```
test api::tests::routes::issues_tests::test_batch_get_issue_status_cache_behavior ... ok
```

### Run All API Tests

```bash
# Verify no regressions in new tests
cargo test --features api batch_get_issue_status
```

**Note:** Full `cargo test --features api` may have environment-dependent failures (e.g., keychain access in `git::auth::tests`). The primary verification target is the new focused tests plus `run_all_test_cases`. Full suite pass is best-effort depending on environment.

## Success Criteria

✅ Basic YAML test verifies batch endpoint with multiple issues
✅ Single issue YAML test verifies endpoint works with one issue
✅ Dirty file detection YAML test verifies working directory status
✅ Blocking QC parsing YAML test verifies issue body parsing behavior
✅ Invalid input YAML test verifies error handling
✅ Comprehensive Rust test verifies cache population and cache hit behavior
✅ New focused tests pass: `cargo test --features api batch_get_issue_status`
✅ YAML test runner passes: `cargo test --features api run_all_test_cases`
✅ Total YAML test count increases from 11 to 16
