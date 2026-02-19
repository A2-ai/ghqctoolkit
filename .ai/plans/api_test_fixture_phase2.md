# Plan: Add YAML-Based Tests for Milestone and Assignee GET Endpoints

## Context

The API test harness was recently refactored to use a data-driven YAML approach where test cases are defined in YAML files and fixtures are stored as JSON. This has been successfully implemented for health and issue endpoints (4 test cases converted).

However, milestone and assignee endpoints are still using old-style Rust tests in `src/api/tests/routes/milestones_tests.rs` and `src/api/tests/routes/status_tests.rs`. These tests only verify empty responses because `MockGitInfo` doesn't support populating milestone or assignee data.

**Why this change is needed:**
- Consistency: All GET endpoints should use the same YAML-based testing approach
- Better test coverage: Current tests only verify empty responses
- Maintainability: YAML tests are easier to read and modify
- Reuse existing fixtures: v1.0.json, v2.0.json milestone fixtures and repository_users.json are available but unused

**Target endpoints:**
1. `GET /api/milestones` - List all milestones
2. `GET /api/milestones/{number}/issues` - List issues in a specific milestone
3. `GET /api/assignees` - List repository assignees

## Implementation Approach

### Step 1: Enhance MockGitInfo to Support Milestones

**File:** `src/api/tests/helpers.rs`

Add milestone storage to the MockGitInfo struct:

```rust
pub struct MockGitInfo {
    // ... existing fields ...
    milestones: Arc<Mutex<Vec<Milestone>>>,
}
```

Add builder method:

```rust
impl MockGitInfoBuilder {
    pub fn with_milestone(mut self, milestone: Milestone) -> Self {
        self.milestones.push(milestone);
        self
    }
}
```

Update trait implementations:

```rust
impl GitHubReader for MockGitInfo {
    async fn get_milestones(&self) -> Result<Vec<Milestone>, GitHubApiError> {
        Ok(self.milestones.lock().unwrap().clone())
    }

    async fn get_issues(&self, milestone: Option<u64>) -> Result<Vec<Issue>, GitHubApiError> {
        // Filter issues by milestone if provided
        let issues = self.issues.lock().unwrap();
        if let Some(milestone_number) = milestone {
            Ok(issues.values()
                .filter(|issue| {
                    issue.milestone.as_ref()
                        .map(|m| m.number == milestone_number as i64)
                        .unwrap_or(false)
                })
                .cloned()
                .collect())
        } else {
            Ok(issues.values().cloned().collect())
        }
    }
}
```

**Note:** Issue fixtures need to have milestone data for filtering to work. Check existing fixtures.

### Step 2: Enhance MockGitInfo to Support Assignees

**File:** `src/api/tests/helpers.rs`

Add assignee storage for full user data:

```rust
pub struct MockGitInfo {
    // ... existing fields ...
    users: Arc<Mutex<Vec<RepoUser>>>,
}
```

Add builder method:

```rust
impl MockGitInfoBuilder {
    pub fn with_users(mut self, users: Vec<RepoUser>) -> Self {
        self.users = users;
        self
    }
}
```

Update trait implementations:

```rust
impl GitHubReader for MockGitInfo {
    async fn get_assignees(&self) -> Result<Vec<String>, GitHubApiError> {
        // Return just the logins for the assignees endpoint
        Ok(self.users.lock().unwrap()
            .iter()
            .map(|u| u.login.clone())
            .collect())
    }

    async fn get_user_details(&self, username: &str) -> Result<RepoUser, GitHubApiError> {
        // Find and return the full user from stored fixtures
        // If not found, return a RepoUser with name: None (matches production behavior)
        Ok(self.users.lock().unwrap()
            .iter()
            .find(|u| u.login == username)
            .cloned()
            .unwrap_or_else(|| RepoUser {
                login: username.to_string(),
                name: None,
            }))
    }
}
```

This approach stores full `RepoUser` objects from fixtures, allowing realistic testing of both login and name fields.

### Step 3: Update MockBuilder to Populate Milestones and Assignees

**File:** `src/api/tests/harness/mock_builder.rs`

Update the `build()` method to handle milestones and users from LoadedFixtures:

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

        // Add all milestones
        for (_number, milestone) in &fixtures.milestones {
            builder = builder.with_milestone(milestone.clone());
        }

        // Add users (for assignees endpoint)
        if !fixtures.users.is_empty() {
            builder = builder.with_users(fixtures.users.clone());
        }

        // Add blocking relationships
        for blocking in &fixtures.blocking {
            let blocked_issues: Vec<_> = blocking
                .blocks
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

**Note:** The LoadedFixtures already has milestones and users fields, they're just currently unused (marked with dead_code warnings).

### Step 4: Copy and Update Fixtures

**4a. Copy milestone fixtures:**

```bash
cp src/tests/github_api/milestones/*.json src/api/tests/fixtures/milestones/
```

This will copy v1.0.json and v2.0.json to the fixtures directory.

**4b. Update issue fixtures to include milestone data:**

**Current state check:**
- `test_file_issue.json` has `"milestone": null` ✓ needs update
- `config_file_issue.json` **already has milestone 1** ✓ needs number changed to 2

Edit `src/api/tests/fixtures/issues/test_file_issue.json`:
- Change `"milestone": null` to link to milestone 1 (v1.0)
- Copy the milestone object structure from `config_file_issue.json` and set `number: 1`

Edit `src/api/tests/fixtures/issues/config_file_issue.json`:
- **Change existing milestone from number 1 to number 2**
- Update `title: "v1.0"` to `title: "v2.0"`
- Update any URLs that reference milestone/1 to milestone/2

**Important:** Consult `config_file_issue.json` (which already has a complete milestone object) or the issue JSONs at `src/tests/github_api/issues/` to see the full GitHub API milestone structure. The milestone field includes many fields beyond just number/title (url, html_url, labels_url, id, node_id, creator, dates, etc.).

This enables proper testing of milestone filtering in the `/api/milestones/{number}/issues` endpoint.

### Step 5: Create YAML Test Cases for Milestones

**Directory:** `src/api/tests/cases/milestones/`

Create the following test files:

**`list_milestones_success.yaml`:**
```yaml
name: "GET /api/milestones - success with multiple milestones"
description: "Retrieve all milestones from repository"

fixtures:
  milestones:
    - v1.0.json
    - v2.0.json

git_state: {}

request:
  method: GET
  path: "/api/milestones"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      min_length: 2
      item_fields:
        - number
        - title
        - state
        - open_issues
        - closed_issues
```

**`list_milestones_empty.yaml`:**
```yaml
name: "GET /api/milestones - empty list"
description: "Repository with no milestones returns empty array"

fixtures:
  milestones: []

git_state: {}

request:
  method: GET
  path: "/api/milestones"

response:
  status: 200
  body:
    match_type: exact
    value: []
```

**`list_milestone_issues_success.yaml`:**
```yaml
name: "GET /api/milestones/{number}/issues - success with filtering"
description: "Retrieve issues for milestone 1, verify filtering works"

fixtures:
  issues:
    - test_file_issue.json        # Milestone 1
    - config_file_issue.json      # Milestone 2
  milestones:
    - v1.0.json                   # Milestone 1
    - v2.0.json                   # Milestone 2

git_state: {}

request:
  method: GET
  path: "/api/milestones/1/issues"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      min_length: 1
      item_fields:
        - number
        - title
        - state
```

**Fixture context:** Both issue fixtures are loaded in this test:
- `test_file_issue.json` (milestone 1) → returned for `/api/milestones/1/issues`
- `config_file_issue.json` (milestone 2) → filtered out, would be returned for `/api/milestones/2/issues`

This verifies the filtering logic actually works by loading multiple issues with different milestones.

**`list_milestone_issues_empty.yaml`:**
```yaml
name: "GET /api/milestones/{number}/issues - no issues in milestone"
description: "Milestone 2 exists but has no issues loaded in this test"

fixtures:
  issues:
    - test_file_issue.json        # Milestone 1 only
  milestones:
    - v1.0.json                   # Milestone 1
    - v2.0.json                   # Milestone 2

git_state: {}

request:
  method: GET
  path: "/api/milestones/2/issues"

response:
  status: 200
  body:
    match_type: exact
    value: []
```

**Note:** This test loads only `test_file_issue.json` (milestone 1), so milestone 2 has no issues in this test context. This is different from `list_milestone_issues_success.yaml` which loads both issue fixtures.

### Step 6: Create YAML Test Cases for Assignees

**Directory:** `src/api/tests/cases/assignees/`

Create the following test files:

**`list_assignees_success.yaml`:**
```yaml
name: "GET /api/assignees - success with users"
description: "Retrieve repository assignees with login and name"

fixtures:
  users:
    - repository_users.json

git_state: {}

request:
  method: GET
  path: "/api/assignees"

response:
  status: 200
  body:
    match_type: schema
    schema:
      type: array
      min_length: 2
      item_fields:
        - login
        - name
```

**`list_assignees_empty.yaml`:**
```yaml
name: "GET /api/assignees - empty list"
description: "Repository with no assignees returns empty array"

fixtures:
  users: []

git_state: {}

request:
  method: GET
  path: "/api/assignees"

response:
  status: 200
  body:
    match_type: exact
    value: []
```


### Step 7: Update Module Structure and Partially Delete Old Tests

**Update:** `src/api/tests/routes/mod.rs`

```rust
// Phase 1: Converted to YAML (removed)
// mod issues_tests;
// mod health_tests;

// Phase 2: Partially converted to YAML
mod milestones_tests;  // KEEP - still has test_create_milestone (POST endpoint)
// mod status_tests;    // REMOVE - fully converted to YAML

// Phase 3+: Keep these until converted
mod comments_tests;
mod configuration_tests;
```

**Update:** `src/api/tests/routes/milestones_tests.rs`

Delete only the GET tests, keep the POST test:
- Delete `test_list_milestones()` - converted to YAML
- Delete `test_list_milestone_issues()` - converted to YAML
- **KEEP** `test_create_milestone()` - POST endpoint not yet supported in YAML harness

Add comment at top of file:
```rust
//! Milestone tests - POST endpoints only
//! GET endpoints have been migrated to YAML tests in src/api/tests/cases/milestones/
```

**Delete file:**
```bash
rm src/api/tests/routes/status_tests.rs
```

**Rationale:** The POST `/api/milestones` endpoint requires write operations in MockGitInfo, which is not yet implemented. Keep the existing Rust test for this endpoint until Phase 3 adds write-capable YAML mocking.

## Critical Files

**Files to modify:**
- `src/api/tests/helpers.rs` - Add milestone and assignee support to MockGitInfo
- `src/api/tests/harness/mock_builder.rs` - Populate milestones and users from fixtures
- `src/api/tests/routes/mod.rs` - Remove deleted module declarations

**Files to create:**
- `src/api/tests/cases/milestones/list_milestones_success.yaml`
- `src/api/tests/cases/milestones/list_milestones_empty.yaml`
- `src/api/tests/cases/milestones/list_milestone_issues_success.yaml`
- `src/api/tests/cases/milestones/list_milestone_issues_empty.yaml`
- `src/api/tests/cases/assignees/list_assignees_success.yaml`
- `src/api/tests/cases/assignees/list_assignees_empty.yaml`

**Files to edit:**
- `src/api/tests/fixtures/issues/test_file_issue.json` - Change milestone from null to 1
- `src/api/tests/fixtures/issues/config_file_issue.json` - Change milestone from 1 to 2
- `src/api/tests/routes/milestones_tests.rs` - Delete GET tests, keep POST test with explanatory comment
- `src/api/tests/routes/mod.rs` - Update module declarations

**Files to delete:**
- `src/api/tests/routes/status_tests.rs` (fully converted to YAML)

**Existing fixtures to use:**
- `src/tests/github_api/milestones/v1.0.json` (to be copied in Step 4)
- `src/tests/github_api/milestones/v2.0.json` (to be copied in Step 4)
- `src/api/tests/fixtures/users/repository_users.json` (already exists)

**Note:** The milestone fixtures directory exists but is currently empty. Step 4 will copy them.

## Important Considerations

### Issue Fixture Milestone Data

**Decision:** Update existing issue fixtures to include milestone references for proper filtering tests:
- `test_file_issue.json` → milestone 1 (v1.0)
- `config_file_issue.json` → milestone 2 (v2.0)

**Test fixture combinations:**
1. **`list_milestone_issues_success.yaml`** - Loads BOTH issues:
   - Request `/api/milestones/1/issues` → returns `[test_file_issue]` (config_file_issue filtered out)
   - Request `/api/milestones/2/issues` → returns `[config_file_issue]` (test_file_issue filtered out)

2. **`list_milestone_issues_empty.yaml`** - Loads ONLY test_file_issue:
   - Request `/api/milestones/2/issues` → returns `[]` (no issue with milestone 2 loaded)

This demonstrates that the same milestone can return different results depending on which fixtures are loaded in each test context.

### Assignee User Details

**Decision:** Store full `RepoUser` objects from fixtures in MockGitInfo.

The `/api/assignees` endpoint calls `get_repo_users()` which internally calls `get_user_details()` for each assignee login. By storing the complete user data from `repository_users.json`:
- `get_assignees()` returns the login strings
- `get_user_details()` looks up and returns the full `RepoUser` with real name from fixtures
- **On miss:** Returns `Ok(RepoUser { login, name: None })` instead of error (matches production behavior in `src/cache.rs:230` and `src/git/api/read.rs:240` where user detail lookup is best-effort)
- Tests can validate both login and name fields in the response
- More realistic behavior matching the actual API

### Caching Behavior

The real implementation uses disk caching for assignees. MockGitInfo bypasses this since `AppState` is created with `None` for disk_cache in tests. This is fine for testing the endpoint logic, but means we don't test cache behavior.

If cache testing is desired, that would require a separate approach (potentially using a temporary cache directory).

## Verification

### Build and Test

```bash
# Verify compilation
cargo test --features api --no-run

# Run all YAML-based tests
cargo test --features api run_all_test_cases -- --nocapture
```

**Expected output:**
```
Running test: .../milestones/list_milestones_success.yaml
  ✓ PASSED (status: 200)
Running test: .../milestones/list_milestones_empty.yaml
  ✓ PASSED (status: 200)
Running test: .../milestones/list_milestone_issues_success.yaml
  ✓ PASSED (status: 200)
Running test: .../milestones/list_milestone_issues_empty.yaml
  ✓ PASSED (status: 200)
Running test: .../assignees/list_assignees_success.yaml
  ✓ PASSED (status: 200)
Running test: .../assignees/list_assignees_empty.yaml
  ✓ PASSED (status: 200)

Total: 10  Passed: 10  Failed: 0
```

(10 total = 4 existing health/issue tests + 4 milestone tests + 2 assignee tests)

**Also verify Rust test still works:**
```bash
cargo test --features api test_create_milestone
```
This ensures the POST endpoint test is preserved.

### Manual Verification Steps

1. **Check fixture copying:**
   ```bash
   ls src/api/tests/fixtures/milestones/
   # Should show: v1.0.json, v2.0.json
   ```

2. **Verify issue fixtures have milestone data:**
   ```bash
   grep -l "milestone" src/api/tests/fixtures/issues/*.json
   ```
   If no matches, adjust test expectations for `list_milestone_issues_success.yaml`.

3. **Test discovery:**
   ```bash
   find src/api/tests/cases -name "*.yaml" | wc -l
   # Should show: 10 test files
   ```

4. **Run specific test:**
   ```bash
   cargo test --features api run_all_test_cases 2>&1 | grep "list_milestones"
   # Should show milestone tests passing
   ```

## Success Criteria

✅ MockGitInfo supports milestones (storage, builder method, get_milestones returns data)
✅ MockGitInfo supports assignees (storage, builder method, get_assignees returns logins)
✅ MockGitInfo.get_user_details returns Ok with name:None on miss (production-like behavior)
✅ MockBuilder populates milestones and users from LoadedFixtures
✅ Issue fixtures updated with proper milestone references
✅ 4 milestone test YAML files created and passing
✅ 2 assignee test YAML files created and passing
✅ Milestone GET tests deleted from milestones_tests.rs, POST test preserved
✅ status_tests.rs fully deleted (converted to YAML)
✅ Module declarations updated in routes/mod.rs
✅ All YAML tests pass with `cargo test --features api run_all_test_cases`
✅ POST test still passes with `cargo test --features api test_create_milestone`
✅ Total YAML test count increases from 4 to 10

## Migration Progress Tracking

After this plan:
- ✅ Phase 1 complete: health_tests.rs, issues_tests.rs (4 YAML tests)
- ✅ Phase 2 complete: status_tests.rs (2 YAML tests)
- ⚠️ Phase 2 partial: milestones_tests.rs GET endpoints (4 YAML tests), POST endpoint kept as Rust test
- ⏳ Phase 3 pending: comments_tests.rs, configuration_tests.rs, milestones POST endpoint

**Remaining work:**
- Write operation support in MockGitInfo needed for POST/PUT endpoints
- POST /api/milestones kept as traditional Rust test until write-capable YAML mocking added
