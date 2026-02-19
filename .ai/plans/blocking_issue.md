# Implementation Plan: GitHub Issue Dependencies (Blocking/Blocked-By)

## Overview
Add support for GitHub's issue dependencies feature to automatically set blocking relationships when creating issues with GatingQC or PreviousQC relevant files. The new issue will be "blocked by" the referenced issues (they are prerequisites).

## Requirements
1. Add `issue_id: Option<u64>` to RelevantFileClass variants (GatingQC, PreviousQC, RelevantQC)
2. Create `block_issue()` method on `GitHubWriter` trait
3. Add `get_issue()` method on `GitHubReader` to fetch issue details when ID not available
4. Gracefully handle failures at the call site (GitHub Enterprise may not support this feature)
5. After posting an issue, create blocking relationships for GatingQC and PreviousQC types
6. Update tests to verify blocking relationships are created

## API Reference
- **Endpoint:** `POST /repos/{owner}/{repo}/issues/{issue_number}/dependencies/blocked_by`
- **Body:** `{ "issue_id": <blocking_issue_id> }` (internal GitHub ID, not issue number)
- **Success:** 201 Created
- **Failures:** 403, 404, 410, 422 (should be handled gracefully at call site)

## Files to Modify

### 1. `src/relevant_files.rs`
Add `issue_id: Option<u64>` to issue-linked variants:
```rust
pub(crate) enum RelevantFileClass {
    PreviousQC {
        issue_number: u64,
        issue_id: Option<u64>,  // NEW: GitHub internal ID
        description: Option<String>,
    },
    GatingQC {
        issue_number: u64,
        issue_id: Option<u64>,  // NEW
        description: Option<String>,
    },
    RelevantQC {
        issue_number: u64,
        description: Option<String>,
    },
    File { justification: String },
}
```

### 2. `src/git/api/read.rs`
Add method to fetch single issue (for getting issue_id when not available):
```rust
fn get_issue(
    &self,
    issue_number: u64,
) -> impl Future<Output = Result<Issue, GitHubApiError>> + Send;
```

### 3. `src/git/api/write.rs`
Add new method to `GitHubWriter` trait:
```rust
fn block_issue(
    &self,
    blocked_issue_number: u64,
    blocking_issue_id: u64,  // Note: This is the internal ID, not number
) -> impl Future<Output = Result<(), GitHubApiError>> + Send;
```

Implementation notes:
- Use octocrab's raw request API (this endpoint may not be directly supported)
- Return the error properly - do NOT swallow errors here
- Caller is responsible for graceful degradation

### 4. `src/create.rs`
Add helper method to get blocking issues with their IDs:
```rust
pub fn blocking_issues(&self) -> Vec<(u64, Option<u64>)> {
    // Returns Vec of (issue_number, issue_id) for GatingQC and PreviousQC
}
```

### 5. `src/main.rs`
After `git_info.post_issue(&qc_issue)`:
1. Parse created issue number from returned URL
2. For each blocking issue from `qc_issue.blocking_issues()`:
   a. If `issue_id` is None, fetch via `get_issue(issue_number)`
   b. Call `block_issue(new_issue_number, blocking_issue_id)`
   c. **Collect errors but do not fail** - log warning and continue
3. After loop, report any failures that occurred

```rust
// Pseudocode for call site in main.rs
let mut blocking_errors = Vec::new();
for (issue_number, issue_id) in qc_issue.blocking_issues() {
    let id = match issue_id {
        Some(id) => id,
        None => git_info.get_issue(issue_number).await?.id,
    };
    if let Err(e) = git_info.block_issue(new_issue_number, id).await {
        blocking_errors.push((issue_number, e));
    }
}
if !blocking_errors.is_empty() {
    log::warn!("Issue dependencies feature may not be available...");
    // Print which relationships failed
}
```

### 6. `src/cli/context.rs`
When creating RelevantFile from interactive mode (where we have fetched issues):
- Extract `issue.id` and store in RelevantFileClass

### 7. Update all RelevantFileClass construction sites
- `src/cli/context.rs` - interactive mode (has issue_id from fetched Issue)
- `src/cli/context.rs` - from_args (no issue_id, set to None)
- `src/create.rs` - test fixtures

## Implementation Flow

```
1. User creates issue with GatingQC/PreviousQC relevant files
   - Interactive mode: issue_id is populated from fetched Issue objects
   - CLI args mode: issue_id is None (only URL/number provided)
2. post_issue() creates the GitHub issue, returns URL
3. Parse new issue number from URL
4. For each (issue_number, issue_id) from qc_issue.blocking_issues():
   a. If issue_id is None, fetch via get_issue(issue_number) to get the ID
   b. Call block_issue(new_issue_number, blocking_issue_id)
   c. If error, collect it but continue to next relationship
5. After all blocking relationships attempted:
   a. If any errors, log warning about feature availability
   b. Do NOT fail the overall issue creation
6. Return success to user
```

## Graceful Failure Strategy
- `block_issue()` returns `Result<(), GitHubApiError>` - propagates errors properly
- Call site in `main.rs` collects all errors into a Vec
- After attempting all blocking relationships, log collected errors as warnings
- Log message: "Issue dependencies feature may not be available on this GitHub instance. Failed to create blocking relationships for issues: #N, #M"
- Issue creation itself succeeds regardless of blocking relationship failures

## Tests

### Unit Tests (`src/create.rs`)
```rust
#[test]
fn test_blocking_issues() {
    let issue = create_test_issue();
    let blocking = issue.blocking_issues();
    // Should return GatingQC (#2) and PreviousQC (#1), not RelevantQC or File
    assert_eq!(blocking.len(), 2);
    assert!(blocking.iter().any(|(num, _)| *num == 2)); // GatingQC
    assert!(blocking.iter().any(|(num, _)| *num == 1)); // PreviousQC
}
```

### Integration Test (mock `block_issue` calls)
- Verify `block_issue` is called correct number of times
- Verify called with correct issue numbers/IDs
- Verify that when `block_issue` returns error, issue creation still succeeds

## Verification Steps
1. `cargo build --all-features` - verify compilation
2. `cargo test --all-features` - verify existing + new tests pass
3. Manual test with real GitHub repo that supports dependencies
4. Manual test scenario: Create issue with GatingQC, verify blocking relationship appears in GitHub UI
5. Manual test with GitHub Enterprise (if available) to verify graceful failure at call site
