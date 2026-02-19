# Plan: Add YAML Tests for POST /api/milestones with Mock Write Support

## Context

The API test harness currently supports POST requests with JSON bodies, but `MockGitInfo` returns `NoApi` errors for all write operations (`GitHubWriter` trait methods). This prevents testing of write endpoints like milestone creation.

**Current state:**
- YAML harness supports POST with `request.body` field (✓)
- MockGitInfo implements `GitHubWriter` but all methods return `Err(NoApi)` (✗)
- No call tracking for write operations (✗)
- Only Rust tests exist for POST endpoints, expecting failures (✗)
- No YAML tests for any POST endpoint (✗)

**Why this change is needed:**
- Test that POST /api/milestones endpoint correctly handles milestone creation
- Verify request body parsing (name, description fields)
- Validate response format (201 status, Milestone object)
- Establish patterns for testing other POST endpoints (comments, approvals, etc.)
- Move from failure-expectation tests to success-path testing

**Endpoint details:**
- **Route:** `POST /api/milestones`
- **Location:** `src/api/routes/milestones.rs` (lines 24-36)
- **Request:** `CreateMilestoneRequest { name: String, description: Option<String> }`
- **Response:** `(StatusCode::CREATED, Json<Milestone>)` with milestone number, title, state, etc.

## Implementation Approach

### Step 1: Add Write Call Tracking to MockGitInfo

**File:** `src/api/tests/helpers.rs`

Enhance `MockGitInfo` to track write operation calls, similar to how it tracks read operations.

**Current read tracking** (lines 294-299):
```rust
calls.lock().unwrap().push(format!("get_issue({})", issue_number));
```

**Add write tracking field to MockGitInfo struct** (around line 35):
```rust
pub struct MockGitInfo {
    // ... existing fields ...
    calls: Arc<Mutex<Vec<String>>>,  // Existing - tracks all calls

    // NEW: Track write operation parameters for verification
    write_calls: Arc<Mutex<Vec<WriteCall>>>,
}

#[derive(Debug, Clone)]
pub enum WriteCall {
    CreateMilestone { name: String, description: Option<String> },
    PostComment { comment_type: String },
    CloseIssue { issue_number: u64 },
    OpenIssue { issue_number: u64 },
    // Add others as needed
}
```

**Add helper method for verification:**
```rust
impl MockGitInfo {
    // ... existing methods ...

    /// Check if a specific write operation was called
    pub fn was_called(&self, expected: &WriteCall) -> bool {
        let calls = self.write_calls.lock().unwrap();
        calls.iter().any(|call| matches_write_call(call, expected))
    }

    /// Get all write calls for inspection
    pub fn write_calls(&self) -> Vec<WriteCall> {
        self.write_calls.lock().unwrap().clone()
    }
}

fn matches_write_call(actual: &WriteCall, expected: &WriteCall) -> bool {
    match (actual, expected) {
        (WriteCall::CreateMilestone { name: n1, description: d1 },
         WriteCall::CreateMilestone { name: n2, description: d2 }) => {
            n1 == n2 && d1 == d2
        }
        // Add other variants as needed
        _ => false,
    }
}
```

**Update MockGitInfoBuilder:**
```rust
impl MockGitInfoBuilder {
    pub fn build(self) -> MockGitInfo {
        MockGitInfo {
            // ... existing fields ...
            calls: Arc::new(Mutex::new(Vec::new())),
            write_calls: Arc::new(Mutex::new(Vec::new())),  // NEW
        }
    }
}
```

### Step 2: Implement Successful create_milestone in MockGitInfo

**File:** `src/api/tests/helpers.rs` (lines 378-383)

Replace the error-returning implementation with a success simulation:

**Before:**
```rust
async fn create_milestone(
    &self,
    _name: &str,
    _desc: &Option<String>,
) -> Result<octocrab::models::Milestone, GitHubApiError> {
    Err(GitHubApiError::NoApi)
}
```

**After:**
```rust
async fn create_milestone(
    &self,
    name: &str,
    desc: &Option<String>,
) -> Result<octocrab::models::Milestone, GitHubApiError> {
    // Track the call with parameters
    self.write_calls.lock().unwrap().push(WriteCall::CreateMilestone {
        name: name.to_string(),
        description: desc.clone(),
    });

    // Load base milestone from fixture and mutate key fields
    // This is more maintainable than hardcoding all octocrab model fields
    let milestone_number = {
        let milestones = self.milestones.lock().unwrap();
        milestones.len() as i64 + 1
    };

    // Use fixture as template (avoids brittle field-by-field construction)
    let template = r#"{
        "url": "https://api.github.com/repos/test-owner/test-repo/milestones/1",
        "html_url": "https://github.com/test-owner/test-repo/milestone/1",
        "labels_url": "https://api.github.com/repos/test-owner/test-repo/milestones/1/labels",
        "id": 1000,
        "node_id": "MDk6TWlsZXN0b25lMQ==",
        "number": 1,
        "title": "placeholder",
        "description": null,
        "creator": null,
        "open_issues": 0,
        "closed_issues": 0,
        "state": "open",
        "created_at": "2011-04-10T20:09:31Z",
        "updated_at": "2011-04-10T20:09:31Z",
        "due_on": null,
        "closed_at": null
    }"#;

    let mut milestone: octocrab::models::Milestone = serde_json::from_str(template)
        .expect("Failed to parse milestone template");

    // Update with actual parameters
    milestone.number = milestone_number;
    milestone.title = name.to_string();
    milestone.description = desc.clone();
    milestone.url = format!("https://api.github.com/repos/{}/{}/milestones/{}",
                            self.owner, self.repo, milestone_number)
        .parse().unwrap();
    milestone.html_url = format!("https://github.com/{}/{}/milestone/{}",
                                 self.owner, self.repo, milestone_number)
        .parse().unwrap();

    // Persist to mock state for multi-create realism
    // (so repeated creates increment milestone numbers correctly)
    self.milestones.lock().unwrap().push(milestone.clone());

    Ok(milestone)
}
```

**Key aspects:**
- Records call with full parameters in `write_calls`
- Uses JSON template approach (more maintainable than manual struct construction)
- Mutates only the fields that matter (number, title, description, URLs)
- Avoids brittle dependencies on exact octocrab model structure

### Step 3: Create YAML Test - Success with Name and Description

**File:** `src/api/tests/cases/milestones/create_milestone_success.yaml`

Basic success test with both name and description:

```yaml
name: "POST /api/milestones - create milestone with description"
description: "Successfully create a new milestone with name and description"

fixtures:
  milestones: []  # Start with no existing milestones

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: POST
  path: "/api/milestones"
  body:
    name: "v2.0"
    description: "Second release milestone"

response:
  status: 201
  body:
    match_type: partial
    fields:
      number: 1
      title: "v2.0"
      state: "open"
      description: "Second release milestone"
      open_issues: 0
      closed_issues: 0
```

**Validation:**
- Status 201 CREATED
- Response includes all required Milestone fields
- Milestone number is 1 (first milestone)
- Title matches request name
- Description matches request

### Step 4: Create YAML Test - Success without Description

**File:** `src/api/tests/cases/milestones/create_milestone_no_description.yaml`

Test with only name field (description is optional):

```yaml
name: "POST /api/milestones - create milestone without description"
description: "Successfully create a milestone with only a name (no description)"

fixtures:
  milestones:
    - v1.0.json  # One existing milestone

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: POST
  path: "/api/milestones"
  body:
    name: "v2.0"

response:
  status: 201
  body:
    match_type: partial
    fields:
      number: 2
      title: "v2.0"
      state: "open"
      open_issues: 0
      closed_issues: 0
```

**Key differences:**
- Request body omits `description` field
- Response validates `description` can be null/absent
- Milestone number is 2 (one existing milestone in fixtures)

### Step 5: Create YAML Test - Error with Missing Name

**File:** `src/api/tests/cases/milestones/create_milestone_missing_name.yaml`

Test request validation - name is required:

```yaml
name: "POST /api/milestones - error on missing name"
description: "Return 422 Unprocessable Entity when name field is missing"

fixtures:
  milestones: []

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: POST
  path: "/api/milestones"
  body:
    description: "Missing the required name field"

response:
  status: 422
```

**Validation:**
- Tests Axum Json extraction validation
- Required field `name` missing returns 422 (Axum's default for invalid JSON payloads)
- Note: Not 400 - Axum's `Json<T>` extractor returns 422 for deserialization failures

### Step 6: Update Existing Rust Test

**File:** `src/api/tests/routes/milestones_tests.rs`

Update `test_create_milestone` to expect success and verify call tracking.

**Before** (lines 15-42):
```rust
#[tokio::test]
async fn test_create_milestone() {
    // ... creates mock and request ...
    let response = app.oneshot(request).await.unwrap();
    assert_ne!(response.status(), StatusCode::OK);  // Expects failure!
}
```

**After:**
```rust
#[tokio::test]
async fn test_create_milestone() {
    use crate::api::tests::helpers::{MockGitInfo, WriteCall};
    use crate::api::types::Milestone;  // Public re-export, not ::responses::

    let mock = MockGitInfo::builder().build();
    let config = Configuration::default();
    let state = AppState::new(mock.clone(), config, None);
    let app = create_router(state);

    let request_body = json!({
        "name": "Test Milestone",
        "description": "A test milestone"
    });

    let request = Request::builder()
        .method("POST")
        .uri("/api/milestones")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Verify success response
    assert_eq!(response.status(), StatusCode::CREATED);

    // Verify response body structure
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let milestone: Milestone = serde_json::from_slice(&body).unwrap();
    assert_eq!(milestone.title, "Test Milestone");
    assert_eq!(milestone.description, Some("A test milestone".to_string()));
    assert_eq!(milestone.number, 1);
    assert_eq!(milestone.state, "open");

    // Verify write call was tracked
    let expected_call = WriteCall::CreateMilestone {
        name: "Test Milestone".to_string(),
        description: Some("A test milestone".to_string()),
    };
    assert!(mock.was_called(&expected_call), "create_milestone should have been called");
}
```

**Key changes:**
- Expects `StatusCode::CREATED` instead of non-OK
- Deserializes and validates response body
- Uses `was_called()` helper to verify write operation

### Step 7: Future Enhancement - Write Call Verification in YAML Runner

**Status:** DEFERRED - Not needed for initial implementation

**Rationale:**
- YAML tests validate HTTP responses (status, body structure)
- Rust tests validate write call tracking (using `was_called()`)
- This division of responsibility is sufficient for current needs
- Adding write call verification to YAML runner requires non-trivial refactoring

**If implemented later:**
- Would need to expose mock from AppState in test runner
- TestResult struct would need write_calls field
- YAML test cases would need optional `expected_calls` section

**For now:** Use Rust tests to verify call tracking, YAML tests to verify API contract.

## Critical Files

**Files to modify:**
- `src/api/tests/helpers.rs` - Add `WriteCall` enum, tracking, and successful `create_milestone()` implementation
- `src/api/tests/routes/milestones_tests.rs` - Update Rust test to expect success

**Files to create:**
- `src/api/tests/cases/milestones/create_milestone_success.yaml` - Success with description
- `src/api/tests/cases/milestones/create_milestone_no_description.yaml` - Success without description
- `src/api/tests/cases/milestones/create_milestone_missing_name.yaml` - Error validation

**Existing files referenced:**
- `src/api/routes/milestones.rs` - Endpoint implementation
- `src/api/types/requests.rs` - CreateMilestoneRequest type
- `src/api/types/responses.rs` - Milestone response type
- `src/api/tests/harness/runner.rs` - Test execution (optional enhancement)

## Test Strategy

### YAML Tests (Primary)
✅ Success case: Create milestone with name and description
✅ Success case: Create milestone with name only (optional description)
✅ Error case: Missing required name field (422 validation - Axum Json extraction)

**Benefits:**
- Declarative test specification
- Easy to add more test cases
- Consistent with existing GET endpoint tests

### Rust Tests (Verification)
✅ Updated `test_create_milestone` expects success
✅ Verifies write call tracking works correctly
✅ Can be used for complex scenarios not suitable for YAML

## Verification

### Run YAML Tests

```bash
# Run all YAML-based tests
cargo test --features api run_all_test_cases -- --nocapture
```

**Expected output:**
```
Running test: .../milestones/create_milestone_success.yaml
  ✓ PASSED (status: 201)
Running test: .../milestones/create_milestone_no_description.yaml
  ✓ PASSED (status: 201)
Running test: .../milestones/create_milestone_missing_name.yaml
  ✓ PASSED (status: 422)

Total: 19  Passed: 19  Failed: 0
```

### Run Rust Test

```bash
# Run updated Rust test
cargo test --features api test_create_milestone -- --nocapture
```

**Expected output:**
```
test api::tests::routes::milestones_tests::test_create_milestone ... ok
```

### Run All API Tests

```bash
# Verify no regressions
cargo test --features api
```

Should show 229 tests passing (226 existing + 3 new YAML tests).

## Success Criteria

✅ MockGitInfo implements successful `create_milestone()` returning mock data
✅ Write call tracking records method name and parameters
✅ YAML test creates milestone with description (201 response)
✅ YAML test creates milestone without description (201 response)
✅ YAML test validates missing name returns 422 error
✅ Rust test verifies write call tracking works
✅ All tests pass without regression
✅ Total YAML test count increases from 16 to 19

## Future Expansion

This establishes the pattern for testing other POST endpoints:

**Next phase (after milestones):**
- POST /api/issues/{number}/comment - Add `WriteCall::PostComment`
- POST /api/issues/{number}/approve - Add `WriteCall::PostComment` + `WriteCall::CloseIssue`
- POST /api/issues/{number}/unapprove - Add `WriteCall::PostComment` + `WriteCall::OpenIssue`
- POST /api/issues/{number}/review - Add `WriteCall::PostComment`

**Pattern to follow:**
1. Add `WriteCall` variant for each write operation
2. Update `GitHubWriter` impl to track calls and return success
3. Create YAML tests for success/error cases
4. Update existing Rust tests to expect success
