# Data-Driven Test Harness for API Testing

## Context

The current API route tests have maintainability issues:
- Test logic mixed with test data in Rust code
- Repetitive setup code across test files
- Difficult for non-programmers to contribute tests
- Hard to see at a glance what scenarios are covered

**Solution:** Build a data-driven test harness where:
- Test cases are defined in YAML files (readable, with comments)
- Mock data (issues, milestones, users) stored as JSON fixtures (reusable)
- Single parameterized test discovers and runs all YAML test cases
- New tests can be added by just creating YAML files

**Benefits:**
- Non-programmers can write tests
- Clear separation of test data from test logic
- Fixtures reusable across multiple tests
- Easy to see test coverage by browsing YAML files
- Better error reporting with test file context

## Architecture Overview

### File Organization

```
src/
├── api/
│   ├── tests/                    # Unit tests module (#[cfg(test)])
│   │   ├── mod.rs               # Single parameterized test: run_all_test_cases()
│   │   ├── helpers.rs           # Existing MockGitInfo (keep this!)
│   │   ├── harness/             # Test harness implementation
│   │   │   ├── mod.rs          # Module exports
│   │   │   ├── types.rs        # TestCase, Fixtures, GitState, etc.
│   │   │   ├── loader.rs       # FixtureLoader (loads JSON fixtures)
│   │   │   ├── mock_builder.rs # MockBuilder (builds MockGitInfo)
│   │   │   ├── runner.rs       # TestRunner (executes tests)
│   │   │   └── assertions.rs   # ResponseAsserter (validates responses)
│   │   ├── fixtures/            # JSON fixture files (reusable)
│   │   │   ├── issues/
│   │   │   │   ├── test_file_issue.json
│   │   │   │   ├── config_file_issue.json
│   │   │   │   └── main_file_issue.json
│   │   │   ├── milestones/
│   │   │   │   └── v1.0.json
│   │   │   └── users/
│   │   │       └── repository_users.json
│   │   └── cases/               # YAML test cases (one per file)
│   │       ├── health/
│   │       │   └── health_check.yaml
│   │       └── issues/
│   │           ├── get_issue_success.yaml
│   │           ├── get_issue_not_found.yaml
│   │           └── get_blocked_issues.yaml
```

**Note**: This is a unit test module (runs with `cargo test --features api`), not integration tests.

### Important Implementation Notes

**Status Code Behavior:**
- Missing issues return **502 Bad Gateway**, not 404
- Current implementation: `GitHubApiError::NoApi` → `ApiError::GitHubApi` → status 502
- See: `get_issue` handler in `src/api/routes/issues.rs` and `IntoResponse` impl for `ApiError` in `src/api/error.rs`
- Tests should expect 502 for missing resources

**MockGitInfo Limitations:**
- Currently returns empty data for milestones/assignees
- Write operations return `GitHubApiError::NoApi`
- Start with GET endpoint tests only
- Extend MockGitInfo incrementally for POST/PUT testing

**Fixture Path Strategy:**
- New harness loads from `src/api/tests/fixtures/`
- Existing helper loads from `src/tests/github_api/`
- Both can coexist during migration

### YAML Test Case Format

Each test is a single YAML file with this structure:

```yaml
# src/api/tests/cases/issues/get_issue_success.yaml
name: "GET /api/issues/{number} - success"
description: "Retrieve an existing issue successfully"

# Reference JSON fixtures (loaded from fixtures/ directory)
fixtures:
  issues:
    - test_file_issue.json      # Loaded as issue #1
    - config_file_issue.json    # Loaded as issue #2
  milestones: []                 # Optional
  users: []                      # Optional
  blocking:                      # Define blocking relationships
    - issue: 1                  # Issue 1 blocks issue 2
      blocks: [2]

# Git repository state for MockGitInfo
git_state:
  owner: "test-owner"           # Defaults if omitted
  repo: "test-repo"
  commit: "abc123"
  branch: "main"
  dirty_files: []               # Optional

# HTTP request specification
request:
  method: GET
  path: "/api/issues/1"
  query: {}                     # Optional query params
  body: null                    # Optional JSON body for POST

# Expected response
response:
  status: 200
  body:
    match_type: partial         # "exact" | "partial" | "schema"
    fields:
      number: 1
      title: "src/test.rs"
      state: "open"
```

### JSON Fixture Format

Reuse existing GitHub API JSON format:

```json
// src/api/tests/fixtures/issues/test_file_issue.json
{
  "id": 1,
  "number": 1,
  "title": "src/test.rs",
  "state": "open",
  "labels": [{"name": "ghqc"}],
  "body": "Quality check issue\n\n## Metadata\ninitial qc commit: def456\ngit branch: main",
  "created_at": "2011-04-22T13:33:48Z",
  "updated_at": "2011-04-22T13:33:48Z"
}
```

### Test Execution Flow

1. **Test Discovery**: Scan `src/api/tests/cases/**/*.yaml` recursively
2. **Fixture Loading**: Load JSON fixtures from `src/api/tests/fixtures/` (cached)
3. **Mock Building**: Create `MockGitInfo` with git state + fixtures + blocking relationships
4. **Request Execution**: Build HTTP request, execute via `ServiceExt::oneshot()`
5. **Response Validation**: Assert status code and body (exact/partial/schema matching)
6. **Error Reporting**: Show test file path, validation errors, summary

## Critical Files

**Files to Create:**
- `src/api/tests/harness/mod.rs` - Module exports
- `src/api/tests/harness/types.rs` - Rust structs for YAML schema
- `src/api/tests/harness/loader.rs` - Fixture loading and caching
- `src/api/tests/harness/mock_builder.rs` - MockGitInfo builder
- `src/api/tests/harness/runner.rs` - Test execution
- `src/api/tests/harness/assertions.rs` - Response validation
- `src/api/tests/cases/` - Directory for YAML test cases
- `src/api/tests/fixtures/` - Directory for JSON fixtures

**Files to Modify:**
- `src/api/tests/mod.rs` - Add parameterized test runner, update module declarations
- `src/api/tests/routes/mod.rs` - Remove deleted module declarations OR delete entirely
- `src/api/tests/helpers.rs` - Update fixture path if needed (keep `load_test_issue()`)

**Files to Migrate:**
- Copy `src/tests/github_api/issues/*.json` → `src/api/tests/fixtures/issues/`
- Convert existing tests to YAML format in `src/api/tests/cases/`

**Files to Delete (Phase 1 - after YAML conversion):**
- `src/api/tests/routes/issues_tests.rs` - converted to YAML
- `src/api/tests/routes/health_tests.rs` - converted to YAML

**Files to Keep (Phase 2+ - until MockGitInfo supports them):**
- `src/api/tests/routes/comments_tests.rs` - needs write operation support
- `src/api/tests/routes/milestones_tests.rs` - needs milestone fixtures
- `src/api/tests/routes/status_tests.rs` - needs assignee fixtures
- `src/api/tests/routes/configuration_tests.rs` - needs config fixtures
- `src/api/tests/routes/mod.rs` - updated to remove deleted modules
- `src/api/tests/routes/` directory - kept until all tests converted

**Files to Reference:**
- `src/api/tests/helpers.rs` - MockGitInfo API (keep this!)
- `src/api/state.rs` - AppState creation pattern
- `src/api/server.rs` - Router creation with `create_router()`
- `src/api/types/requests.rs` - Request body structs
- `src/api/types/responses.rs` - Response types for assertions
- `src/api/error.rs` - ApiError types and status code mappings

## Implementation Steps

### Step 1: Create Test Harness Type Definitions

**File**: `src/api/tests/harness/types.rs`

Define Rust structs that deserialize YAML test cases:

**Core types:**
- `TestCase` - Complete test specification (name, description, fixtures, git_state, request, response)
- `Fixtures` - Fixture references (issues, milestones, users, blocking)
- `BlockingRelationship` - Defines `issue` blocks `Vec<u64>`
- `GitState` - Repository state (owner, repo, commit, branch, dirty_files)
- `HttpRequest` - Request spec (method, path, query, body)
- `HttpMethod` - Enum (GET, POST, PUT, DELETE, PATCH)
- `ExpectedResponse` - Expected result (status, body)
- `ResponseBody` - Assertion type (Exact, Partial, Schema)
- `SchemaAssertion` - Schema validation (type, min_length, item_fields)
- `SchemaType` - Enum (Object, Array, String, Number, Boolean)

**Defaults:**
- `GitState`: owner="test-owner", repo="test-repo", commit="abc123", branch="main"
- Empty vecs for optional collections
- Use `#[serde(default)]` extensively

**Dependencies:**
All required dependencies (serde, serde_yaml, serde_json, anyhow) are already in Cargo.toml. No new dependencies needed.

---

### Step 2: Implement Fixture Loader

**File**: `src/api/tests/harness/loader.rs`

Build `FixtureLoader` to load and cache JSON fixtures:

**Struct:**
```rust
pub struct FixtureLoader {
    base_path: PathBuf,
    issue_cache: HashMap<String, Issue>,
    milestone_cache: HashMap<String, Milestone>,
    user_cache: HashMap<String, Vec<RepoUser>>,
}
```

**Key methods:**
- `new(base_path)` - Initialize with `src/api/tests/fixtures` path (from `env!("CARGO_MANIFEST_DIR")`)
- `load_fixtures(&Fixtures)` - Load all referenced fixtures, return `LoadedFixtures`
- `load_issue(filename)` - Load from `fixtures/issues/*.json`, cache results
- `load_milestone(filename)` - Load from `fixtures/milestones/*.json`
- `load_users(filename)` - Load from `fixtures/users/*.json`

**Return type:**
```rust
pub struct LoadedFixtures {
    pub issues: HashMap<u64, Issue>,           // Keyed by issue number
    pub milestones: HashMap<u64, Milestone>,   // Keyed by milestone number
    pub users: Vec<RepoUser>,
    pub blocking: Vec<BlockingRelationship>,   // From YAML, not JSON
}
```

**Error handling:**
- Use `anyhow::Context` for file read/parse errors
- Include file path in error messages
- Cache successful loads to avoid re-reading

**Path resolution:**
- Accept `PathBuf` in `new()` (caller provides full path with `env!("CARGO_MANIFEST_DIR")`)
- This ensures fixture loading works regardless of working directory

---

### Step 3: Implement Mock Builder

**File**: `src/api/tests/harness/mock_builder.rs`

Convert test specification into configured `MockGitInfo`:

**Core function:**
```rust
impl MockBuilder {
    pub fn build(git_state: &GitState, fixtures: &LoadedFixtures) -> MockGitInfo {
        let mut builder = MockGitInfo::builder()
            .with_owner(&git_state.owner)
            .with_repo(&git_state.repo)
            .with_commit(&git_state.commit)
            .with_branch(&git_state.branch);

        // Add all issues
        for (number, issue) in &fixtures.issues {
            builder = builder.with_issue(*number, issue.clone());
        }

        // Add blocking relationships
        for blocking in &fixtures.blocking {
            let blocked_issues: Vec<_> = blocking.blocks
                .iter()
                .filter_map(|num| fixtures.issues.get(num).cloned())
                .collect();
            builder = builder.with_blocked_issues(blocking.issue, blocked_issues);
        }

        // Add dirty files
        for file in &git_state.dirty_files {
            builder = builder.with_dirty_file(PathBuf::from(file));
        }

        builder.build()
    }
}
```

---

### Step 4: Implement Response Assertions

**File**: `src/api/tests/harness/assertions.rs`

Build `ResponseAsserter` to validate HTTP responses:

**Core struct:**
```rust
pub struct ResponseAsserter<'a> {
    expected: &'a ExpectedResponse,
}
```

**Key methods:**
- `validate(StatusCode, Option<&Value>)` - Main validation entry point
- `validate_exact(&Value, &Value)` - Full JSON equality check
- `validate_partial(&Value, &HashMap<String, Value>)` - Check specified fields only
- `validate_schema(&Value, &SchemaAssertion)` - Validate structure (type, required fields, array length)

**Return type:**
```rust
pub struct ValidationError {
    pub message: String,
    pub details: Vec<String>,  // All validation failures
}
```

**Validation logic:**
- Always check status code first
- For `Exact`: Full JSON equality with pretty-printed diff on failure
- For `Partial`: Check only specified fields exist and match, ignore extras
- For `Schema`: Check type matches, required fields present, array min_length

---

### Step 5: Implement Test Runner

**File**: `src/api/tests/harness/runner.rs`

Build `TestRunner` that executes test cases:

**Core struct:**
```rust
pub struct TestRunner {
    loader: FixtureLoader,
}
```

**Key method:**
```rust
pub async fn run_test(&mut self, test_case: TestCase) -> Result<TestResult>
```

**Execution steps:**
1. Load fixtures using `FixtureLoader`
2. Build `MockGitInfo` using `MockBuilder`
3. Create `AppState::new(mock, Configuration::default(), None)`
4. Create router with `create_router(state)`
5. Build HTTP `Request` with method, URI (path + query), body
6. Execute request with `ServiceExt::oneshot()`
7. Extract status and parse body as JSON
8. Validate response using `ResponseAsserter`
9. Return `TestResult` (name, passed, status_code, validation)

**Helper method:**
```rust
fn build_request(&HttpRequest) -> Result<Request<Body>>
```
- Build URI with query parameters: `path?key1=val1&key2=val2`
- Set `content-type: application/json` header if body present
- Serialize body to JSON with `serde_json::to_vec()`

---

### Step 6: Implement Test Discovery and Runner

**File**: `src/api/tests/mod.rs`

Update the existing test module to add the parameterized test runner:

**Test discovery:**
```rust
fn discover_test_cases() -> Vec<PathBuf> {
    // Use CARGO_MANIFEST_DIR to ensure paths work from any cwd
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let cases_dir = Path::new(manifest_dir).join("src/api/tests/cases");

    // Recursively scan src/api/tests/cases/**/*.yaml
    // Sort alphabetically for deterministic order
    // Return Vec of test file paths
}
```

**Main test:**
```rust
#[tokio::test]
async fn run_all_test_cases() {
    let test_files = discover_test_cases();
    assert!(!test_files.is_empty(), "No test cases found");

    // Use CARGO_MANIFEST_DIR for fixture path
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = Path::new(manifest_dir).join("src/api/tests/fixtures");
    let mut runner = TestRunner::new(fixture_path);
    let mut all_passed = true;
    let mut results = Vec::new();

    for test_file in test_files {
        println!("\nRunning test: {}", test_file.display());

        // Parse YAML
        let yaml_content = fs::read_to_string(&test_file).expect("Failed to read test file");
        let test_case: TestCase = serde_yaml::from_str(&yaml_content)
            .expect("Failed to parse test file");

        // Run test
        match runner.run_test(test_case).await {
            Ok(result) => {
                if result.passed {
                    println!("  ✓ PASSED (status: {})", result.status_code);
                } else {
                    println!("  ✗ FAILED");
                    if let Err(e) = result.validation {
                        println!("    {}", e);
                    }
                    all_passed = false;
                }
                results.push(result);
            }
            Err(e) => {
                println!("  ✗ ERROR: {}", e);
                all_passed = false;
            }
        }
    }

    // Print summary
    println!("\n========================================");
    println!("Test Summary");
    println!("========================================");
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    println!("Total: {}  Passed: {}  Failed: {}", results.len(), passed, failed);

    if !all_passed {
        println!("\nFailed tests:");
        for result in results.iter().filter(|r| !r.passed) {
            println!("  - {}", result.name);
        }
    }

    assert!(all_passed, "Some tests failed");
}
```

---

### Step 7: Create Module Exports

**File**: `src/api/tests/harness/mod.rs`

```rust
pub mod types;
pub mod loader;
pub mod mock_builder;
pub mod runner;
pub mod assertions;

pub use types::{TestCase, Fixtures, GitState};
pub use loader::FixtureLoader;
pub use mock_builder::MockBuilder;
pub use runner::TestRunner;
pub use assertions::ResponseAsserter;
```

**Update**: `src/api/tests/mod.rs`

Add harness module:
```rust
pub mod helpers;
pub mod harness;  // Add this line

// ... rest of test code
```

---

### Step 8: Migrate Fixtures

Copy existing JSON fixtures to new location:

```bash
# Create directory structure
mkdir -p src/api/tests/fixtures/{issues,milestones,users}
mkdir -p src/api/tests/cases/{health,issues}

# Copy existing fixtures (keep originals for now)
cp src/tests/github_api/issues/*.json src/api/tests/fixtures/issues/

# Create placeholder user fixture
cat > src/api/tests/fixtures/users/repository_users.json << 'EOF'
[
  {
    "login": "octocat",
    "name": "The Octocat"
  },
  {
    "login": "reviewer1",
    "name": "Reviewer One"
  }
]
EOF
```

**Note on fixture paths:**
- Existing `load_test_issue()` helper reads from `src/tests/github_api/issues/`
- New `FixtureLoader` reads from `src/api/tests/fixtures/issues/`
- Both can coexist during migration
- Once migration is complete, optionally update `load_test_issue()` or deprecate it

---

### Step 9: Create Initial YAML Test Cases

Convert existing tests to YAML format. Start with simple cases:

**Health check:**
```yaml
# src/api/tests/cases/health/health_check.yaml
name: "GET /api/health - success"
description: "Health check endpoint returns OK"

fixtures:
  issues: []

git_state: {}  # Use all defaults

request:
  method: GET
  path: "/api/health"

response:
  status: 200
  body:
    match_type: partial
    fields:
      status: "ok"
```

**Get issue success:**
```yaml
# src/api/tests/cases/issues/get_issue_success.yaml
name: "GET /api/issues/{number} - success"
description: "Retrieve an existing issue by number"

fixtures:
  issues:
    - test_file_issue.json

git_state:
  commit: "abc123"
  branch: "main"

request:
  method: GET
  path: "/api/issues/1"

response:
  status: 200
  body:
    match_type: partial
    fields:
      number: 1
      title: "src/test.rs"
      state: "open"
```

**Get issue not found:**
```yaml
# src/api/tests/cases/issues/get_issue_not_found.yaml
name: "GET /api/issues/{number} - not found"
description: "Return 502 when issue doesn't exist (GitHubApiError maps to ApiError::GitHubApi)"

fixtures:
  issues: []

git_state: {}

request:
  method: GET
  path: "/api/issues/999"

response:
  status: 502  # Current API behavior: GitHubApiError -> 502 Bad Gateway
```

**Blocked issues:**
```yaml
# src/api/tests/cases/issues/get_blocked_issues_success.yaml
name: "GET /api/issues/{number}/blocked - success"
description: "Retrieve issues blocked by a given issue"

fixtures:
  issues:
    - test_file_issue.json
    - config_file_issue.json
  blocking:
    - issue: 1
      blocks: [2]

git_state: {}

request:
  method: GET
  path: "/api/issues/1/blocked"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      min_length: 1
      item_fields:
        - issue
        - qc_status
```

**Note on test scope and phased rollout:**
- **Phase 1 (initial harness)**: Basic GET endpoints only (health, get_issue, get_blocked_issues)
- **Deferred to Phase 2**: Milestones, configuration, status routes - these remain as traditional tests until MockGitInfo can support them
- **Deferred to Phase 3**: POST/PUT endpoints (comments, approve, unapprove, review) - require MockGitInfo write operation extensions
- **DO NOT** delete all route tests immediately - only delete what's been converted to YAML
- This ensures no loss of test coverage during migration

---

### Step 10: Delete Converted Test Files and Update Module Structure

**IMPORTANT**: Only delete tests that have been converted to YAML. For Phase 1, this means:

**1. Delete only converted route test files:**
```bash
# Phase 1: Delete only what's been converted to YAML
rm src/api/tests/routes/issues_tests.rs      # Converted: get_issue, get_blocked
rm src/api/tests/routes/health_tests.rs      # Converted: health check

# Phase 2+: Keep these until MockGitInfo supports them
# - milestones_tests.rs (needs milestone fixture support)
# - status_tests.rs (needs assignee fixture support)
# - configuration_tests.rs (needs config fixture support)
# - comments_tests.rs (needs write operation support)
```

**2. Update `src/api/tests/routes/mod.rs`:**

Remove module declarations for deleted test files:
```rust
// src/api/tests/routes/mod.rs

// Phase 1: Remove these (converted to YAML)
// mod issues_tests;
// mod health_tests;

// Phase 2+: Keep these until converted
mod milestones_tests;
mod status_tests;
mod configuration_tests;
mod comments_tests;
```

**3. Update `src/api/tests/mod.rs`:**

Keep the routes module (still has unconverted tests), add harness module:
```rust
pub mod helpers;
mod harness;  // Add this line for new harness module
mod routes;   // Keep - still has unconverted tests

// Add the parameterized test runner here (from Step 6)
```

**Future cleanup:**
When ALL route tests are converted, then delete `src/api/tests/routes/` entirely and remove `mod routes;`.

Keep `src/api/tests/helpers.rs` - still needed for MockGitInfo.

---

## Verification

### Step-by-Step Testing

**1. Test harness compiles:**
```bash
cargo test --all-features --no-run
```

**2. Run initial tests:**
```bash
cargo test --all-features run_all_test_cases
```

**3. Verify test discovery:**
```bash
# Should find all .yaml files in src/api/tests/cases/
# Check output shows: "Running test: src/api/tests/cases/..."
```

**4. Test fixture loading:**
```bash
# Add debug output to confirm fixtures load correctly
# Verify issue numbers match YAML references
```

**5. Test assertion types:**
Create test cases for each assertion type:
- Exact match test (health check)
- Partial match test (get issue)
- Schema match test (blocked issues)

**6. Test error reporting:**
Create intentionally failing test:
```yaml
name: "Failing test for validation"
fixtures: {issues: []}
git_state: {}
request: {method: GET, path: "/api/issues/1"}
response:
  status: 200  # Should be 502
```

Verify error output shows:
- Test file path
- Validation error details (e.g., "Status code mismatch: expected 200, got 502")
- Summary with failed test list

---

## Expected Outcomes

After implementation:
- **Single test function** discovers and runs all YAML tests
- **Clear test organization** by browsing `src/api/tests/cases/` directory
- **Reusable fixtures** in `src/api/tests/fixtures/`
- **Easy to add tests** - just create YAML file
- **Good error messages** with test file context
- **Migration complete** - old test files deleted
- **Robust path resolution** using `env!("CARGO_MANIFEST_DIR")`

### Success Criteria

1. ✅ All harness modules compile without errors
2. ✅ Test discovery finds YAML files recursively
3. ✅ Fixtures load correctly from JSON files
4. ✅ MockGitInfo configured properly from test specs
5. ✅ HTTP requests execute via router
6. ✅ All three assertion types work (exact, partial, schema)
7. ✅ Failures show detailed validation errors
8. ✅ Summary reports total/passed/failed counts
9. ✅ Initial YAML test cases pass
10. ✅ Old test files successfully deleted

### Future Enhancements

After initial implementation works:
- Add support for custom assertion functions
- Add support for multiple test cases per YAML file
- Add test tags/filters for running subsets
- Add parallel test execution
- Add response header assertions
- Add request header specification
- Extend MockGitInfo with configurable write operations
- Add milestone/user fixture support once MockGitInfo can use them

---

## Addressing Review Findings

This updated plan addresses all identified issues:

### First Round Fixes:
1. ✅ **Test location**: Kept under `src/api/tests/` (unit tests), not `tests/` (integration tests)
2. ✅ **Module compilation**: Added explicit steps to update `src/api/tests/mod.rs` and delete `src/api/tests/routes/`
3. ✅ **Status codes**: Updated examples to expect 502 (current behavior), not 404
4. ✅ **MockGitInfo scope**: Noted limitations, start with GET endpoints only
5. ✅ **Fixture migration**: Using `cp` (copy), both paths can coexist during migration
6. ✅ **Dependencies**: Removed redundant dependency step - all deps already present

### Second Round Fixes:
1. ✅ **Path consistency**: Fixed all Steps 1-5 to use `src/api/tests/harness/...` (was `tests/api/harness/...`)
2. ✅ **Reference brittleness**: Changed line number citations to symbol+file references (e.g., `get_issue` handler, `IntoResponse` impl)
3. ✅ **Path robustness**: Added `env!("CARGO_MANIFEST_DIR")` for test discovery and fixture loading
4. ✅ **Module removal**: Made Step 10 deterministic - **must** remove `mod routes;` when deleting routes/ directory

### Third Round Fixes:
1. ✅ **Stale path string**: Updated Step 2 fixture path comment to `src/api/tests/fixtures`
2. ✅ **Deletion consistency**: Unified approach - routes/ directory kept until all tests converted (phased migration)
3. ✅ **Scope clarity**: Explicit Phase 1 (GET only) vs Phase 2+ (deferred endpoints) - prevents premature deletion of unconverted test coverage
