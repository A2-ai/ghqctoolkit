# Implementation Plan: Relevant Files Downstream Features

## Overview

This plan covers the downstream features for Blocking QC issue relationships (Gating QC + Previous QC combined):
1. **Status** - Display approval status of Blocking QCs
2. **Approval** - Block approval when Blocking QCs are unapproved
3. **Unapproval** - Display impact tree of potentially affected downstream issues

## Design Principles

- **GHE Compatibility**: Parse issue body for relationships (don't rely on blocking API for reading)
- **GUI Support**: Return structured results (like `CreateResult`) instead of just logging errors
- **Graceful Degradation**: Handle API failures without crashing
- **Unified Blocking Concept**: Gating QC and Previous QC are merged into "Blocking QCs" - they have the same functionality (must be approved before current issue)

---

## 1. Parsing Blocking QCs from Issue Body

### Location: `src/issue.rs` - Extend `IssueThread`

Add new field to `IssueThread`:
```rust
pub struct IssueThread {
    // ... existing fields ...

    /// Blocking QC issues parsed from issue body
    /// Includes both Gating QC and Previous QC sections
    /// (issue_number, file_name, relationship)
    pub blocking_qcs: Vec<(u64, String, BlockingRelationship)>,
}
```

### Parsing Logic

Parse the "## Relevant Files" section from issue body:
- Look for `### Gating QC` subsection
- Look for `### Previous QC` subsection
- Combine both into single `blocking_qcs` list
- Extract from **markdown links**: `[file_name](url_with_issue_number)`
  - Regex pattern: `\[([^\]]+)\]\([^)]*\/issues\/(\d+)[^)]*\)`
  - Captures: group 1 = file_name, group 2 = issue_number
  - Works for any host (github.com, GHE, etc.)

Create parser function:
```rust
/// Returns Vec of (issue_number, file_name, relationship)
fn parse_blocking_qcs(body: &str) -> Vec<(u64, String, BlockingRelationship)>
```

**Parsing approach**:
1. Find `### Gating QC` section, extract all markdown links → tag as `BlockingRelationship::GatingQC`
2. Find `### Previous QC` section, extract all markdown links → tag as `BlockingRelationship::PreviousQC`
3. Combine into single list

### Files to Modify
- `src/issue.rs`: Add field and parsing to `IssueThread::from_issue()`

---

## 2. Status Feature

### 2.1 Individual Issue Status

**Location**: `src/qc_status.rs` or new status display logic

When displaying individual issue status, add a section showing Blocking QC status:

```
📋 Issue Status: src/analysis.R
   State: Open
   QC Status: Pending Review

   🔗 Blocking QCs:
      ✅ #123 - upstream.R (Approved)
      ❌ #124 - dependency.R (Open - Pending)
      ✅ #100 - analysis_v1.R (Approved)
```

**Implementation**:
1. Get `IssueThread` for the target issue
2. For each `blocking_qcs`, fetch their status
3. Use existing `qc_status.rs` logic to determine approval status (reuse `QCStatus::Approved` check)

**Return Type** - Simple struct with three HashMaps:
```rust
pub struct BlockingQCStatus {
    pub approved: HashMap<u64, PathBuf>,              // issue_number -> file_name
    pub not_approved: HashMap<u64, (PathBuf, String)>, // issue_number -> (file_name, status)
    pub errors: HashMap<u64, GitHubApiError>,         // issue_number -> error
}
```
- Easy to check: `status.errors.is_empty()`, `status.not_approved.is_empty()`
- GUI can display partial results with per-issue error details

### 2.2 Milestone Status Table

Add new column `Blocking QCs` to the existing milestone status table:

```
File                   | Milestone | Branch  | Issue State | QC Status                         | Blocking QCs      | Git Status | Checklist
-----------------------+-----------+---------+-------------+-----------------------------------+-------------------+------------+------------
src/cli/interactive.rs | test1     | comment | closed      | Approved; subsequent file changes | 2/2 (100%)        | Up to date | 0/19 (0.0%)
src/lib.rs             | test1     | main    | open        | Changes to comment                | 1/3 (33%)         | Up to date | 0/1 (0.0%)
src/main.rs            | test1     | main    | open        | Pending                           | -                 | Up to date | 0/5 (0.0%)
src/utils.rs           | test1     | main    | open        | Pending                           | 1/3 (+1 err)      | Up to date | 0/3 (0.0%)
```

- Show `approved/total (percent%)` format where `total` = all blocking QCs (approved + not_approved + errors)
- Show `-` when issue has no blocking QCs
- Show `(+N err)` suffix when some status fetches failed
  - Example: `1/3 (+1 err)` means 1 approved out of 3 total blockers, with 1 of the 3 having a fetch error
  - The denominator includes ALL blocking QCs, including those with errors
  - Percentage is calculated as `approved / total * 100`

**Implementation**:
1. For each issue in milestone, parse `blocking_qcs` from body
2. Fetch status for each blocking QC issue
3. Count approved vs total, calculate percentage
4. Add column to table output

### Files to Modify
- `src/issue.rs`: Add parsing (from section 1)
- `src/qc_status.rs`: Add helper to check if issue is approved, add blocking QC column logic
- `src/main.rs`: Update status table output

---

## 3. Approval Feature

### 3.1 Get Unapproved Blocking QCs

**Location**: `src/approve.rs`

Before approving an issue, check which Blocking QCs are not approved.

Create query function:
```rust
/// Result of checking blocking QC approval status for approval validation
pub struct BlockingQCCheckResult {
    pub unapproved: HashMap<u64, PathBuf>,        // issue_number -> file_name
    pub errors: HashMap<u64, GitHubApiError>,     // issue_number -> error
}

impl BlockingQCCheckResult {
    /// Returns true if all blocking QCs are approved (or there are none)
    pub fn all_approved(&self) -> bool {
        self.unapproved.is_empty() && self.errors.is_empty()
    }

    /// Returns true if there are any errors that prevented status checks
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

pub async fn get_unapproved_blocking_qcs<T: GitHubReader>(
    issue_thread: &IssueThread,
    git_info: &T,
    cache: Option<&DiskCache>,
) -> BlockingQCCheckResult
```
- Returns struct with both unapproved issues AND errors (never fails entirely)
- `all_approved()` returns true only if `unapproved` AND `errors` are both empty
- CLI can decide policy: block on errors, or allow with warning
- Reuses existing `qc_status.rs` approval detection logic

### 3.2 ApprovalResult Struct

**Location**: `src/approve.rs`

```rust
pub struct ApprovalResult {
    pub approval_url: String,
    pub force_used: bool,
    pub skipped_unapproved: HashMap<u64, PathBuf>,    // Unapproved blocking QCs bypassed with --force
    pub skipped_errors: HashMap<u64, GitHubApiError>, // Blocking QCs with fetch errors bypassed with --force
}

impl fmt::Display for ApprovalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "✅ Issue approved and closed!\n")?;

        if self.force_used {
            write!(f, "  ⚠️ --force was used to bypass dependency checks\n")?;
        }

        if !self.skipped_unapproved.is_empty() {
            write!(f, "  ⚠️ Unapproved Blocking QCs: {}\n",
                self.skipped_unapproved.keys().map(|n| format!("#{}", n)).collect::<Vec<_>>().join(", "))?;
        }

        if !self.skipped_errors.is_empty() {
            write!(f, "  ⚠️ Blocking QCs with unknown status: {}\n",
                self.skipped_errors.keys().map(|n| format!("#{}", n)).collect::<Vec<_>>().join(", "))?;
        }

        write!(f, "\n{}", self.approval_url)
    }
}
```

### 3.3 CLI Integration

**Location**: `src/main.rs`

Add `--force` flag to approve command:
```rust
Approve {
    // ... existing fields ...

    /// Force approval even if Blocking QCs are not approved
    #[arg(long)]
    force: bool,
}
```

**Flow**:
1. Build `QCApprove` as before
2. Get `IssueThread` to access `blocking_qcs`
3. Call `get_unapproved_blocking_qcs()` → returns `BlockingQCCheckResult`
4. Check result:
   - If `!result.all_approved()` and `!force`:
     - Return error listing unapproved issues AND errors
     - Error message: "Cannot approve: N blocking QCs are not approved, M could not be checked"
   - If `force` or `result.all_approved()`:
     - Proceed with approval
5. Return `ApprovalResult` with:
   - `skipped_unapproved`: populated from `result.unapproved` if `force` was used
   - `skipped_errors`: populated from `result.errors` if `force` was used

### 3.4 QCApprove Method

Add method to `QCApprove`:
```rust
pub async fn approve_with_validation<T: GitHubWriter + GitHubReader + GitHelpers>(
    &self,
    git_info: &T,
    cache: Option<&DiskCache>,
    force: bool,
) -> Result<ApprovalResult, ApprovalError>
```

### Files to Modify
- `src/approve.rs`: Add `ApprovalResult`, `get_unapproved_blocking_qcs()`, validation logic
- `src/main.rs`: Add `--force` flag, use new approval method
- `src/cli/context.rs`: Update `QCApprove::from_interactive()` to show dependency status

---

## 4. Unapproval Feature

### 4.1 Query Blocked Issues

**Location**: `src/git/api/read.rs`

Add method to `GitHubReader` trait:
```rust
/// Get issues that are blocked by the given issue
/// Uses GitHub's issue dependencies API - may not be available on all deployments
fn get_blocked_issues(
    &self,
    issue_number: u64,
) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send;
```

**API Endpoint**: `GET /repos/{owner}/{repo}/issues/{issue_number}/dependencies/blocking`

### 4.2 UnapprovalResult Struct

**Location**: `src/approve.rs`

Only the selected issue is unapproved, but we display a tree of potentially impacted downstream issues.

```rust
pub struct UnapprovalResult {
    pub unapproval_url: String,
    pub impacted_issues: ImpactedIssues,
}

pub enum ImpactedIssues {
    /// No downstream issues found
    None,
    /// API not available (GHE), couldn't check
    ApiUnavailable,
    /// Found downstream issues - display as tree
    Some(Vec<ImpactNode>),
}

/// A node in the impact tree
pub struct ImpactNode {
    pub issue_number: u64,
    pub file_name: PathBuf,
    pub milestone: String,                   // Milestone name for easy unapproval
    pub relationship: BlockingRelationship,  // GatingQC or PreviousQC
    pub children: Vec<ImpactNode>,           // Recursive children
    pub fetch_error: Option<String>,         // Error if children couldn't be fetched
}

pub enum BlockingRelationship {
    GatingQC,
    PreviousQC,
    /// Relationship could not be determined (issue not found in child's body)
    Unknown,
}

impl fmt::Display for UnapprovalResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "🚫 Issue unapproved and reopened!\n")?;
        write!(f, "{}\n", self.unapproval_url)?;

        match &self.impacted_issues {
            ImpactedIssues::None => {},
            ImpactedIssues::ApiUnavailable => {
                write!(f, "\n⚠️ Could not check for impacted issues (API may not be supported)\n")?;
            },
            ImpactedIssues::Some(nodes) => {
                write!(f, "\nThe following QCs may be impacted by this unapproval:\n")?;
                for node in nodes {
                    node.fmt_tree(f, 0)?;
                }
            },
        }
        Ok(())
    }
}

impl ImpactNode {
    fn fmt_tree(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        let indent = "   ".repeat(depth);
        let prefix = if depth > 0 { "|- " } else { "" };
        let rel = match self.relationship {
            BlockingRelationship::GatingQC => "gating QC",
            BlockingRelationship::PreviousQC => "previous QC",
            BlockingRelationship::Unknown => "unknown relationship",
        };

        // Format: #issue file (milestone) (relationship)
        // User can unapprove with: ghqc issue unapprove --milestone "milestone" --file "file"
        writeln!(f, "{}{}#{} {} ({}) ({})", indent, prefix,
            self.issue_number, self.file_name.display(), self.milestone, rel)?;

        // Show fetch error if present
        if let Some(err) = &self.fetch_error {
            writeln!(f, "{}   ⚠️ {}", indent, err)?;
        }

        for child in &self.children {
            child.fmt_tree(f, depth + 1)?;
        }
        Ok(())
    }
}
```

**Example Output**:
```
🚫 Issue unapproved and reopened!
https://github.com/owner/repo/issues/25#issuecomment-123

The following QCs may be impacted by this unapproval:
#30 path/to/file1.qmd (Sprint 1) (previous QC)
   |- #35 path/to/script1.R (Sprint 1) (gating QC)
      |- #40 path/to/data.csv (Sprint 2) (gating QC)
#33 path/to/file2.qmd (Sprint 1) (gating QC)
   |- #37 path/to/script2.qmd (Sprint 2) (gating QC)
      ⚠️ Could not fetch children: API rate limit exceeded
   |- #38 (fetch failed) () (gating QC)
      ⚠️ 404 Not Found
```

**To unapprove an impacted issue**, use:
```bash
ghqc issue unapprove --milestone "Sprint 1" --file "path/to/file1.qmd"
```

### 4.3 QCUnapprove Method

Add method to `QCUnapprove`:
```rust
pub async fn unapprove_with_impact<T: GitHubWriter + GitHubReader + GitHelpers>(
    &self,
    git_info: &T,
) -> Result<UnapprovalResult, UnapprovalError>
```

**Flow**:
1. Post unapproval comment and reopen this issue
2. Try to fetch blocked issues via `get_blocked_issues()`
3. If API fails, set `ImpactedIssues::ApiUnavailable` and return
4. If no blocked issues, set `ImpactedIssues::None` and return
5. For each blocked issue, recursively build the impact tree:
   - Fetch the issue to get file name and determine relationship type
   - Recursively call `get_blocked_issues()` for that issue
   - Build `ImpactNode` with children
6. Return `UnapprovalResult` with `ImpactedIssues::Some(tree)`

### 4.4 Building the Impact Tree

The relationship type is stored in the **child's** body (the child lists its blockers under Gating QC or Previous QC sections). So when building a child node, we must parse the **child's** body to determine how it relates to the parent.

```rust
/// A node in the impact tree, with optional error information
pub struct ImpactNode {
    pub issue_number: u64,
    pub file_name: PathBuf,
    pub milestone: String,
    pub relationship: BlockingRelationship,
    pub children: Vec<ImpactNode>,
    pub fetch_error: Option<String>,  // If children couldn't be fetched
}

async fn build_impact_tree<T: GitHubReader>(
    git_info: &T,
    parent_issue_number: u64,  // The issue we're building FROM (the blocker)
    child_issue_number: u64,   // The issue we're building the node FOR
    visited: &mut HashSet<u64>,  // Prevent cycles
) -> Result<ImpactNode, GitHubApiError> {
    // Prevent infinite loops from circular dependencies
    if !visited.insert(child_issue_number) {
        return Ok(ImpactNode {
            issue_number: child_issue_number,
            file_name: PathBuf::from("(circular reference)"),
            milestone: String::new(),
            relationship: BlockingRelationship::Unknown,
            children: vec![],
            fetch_error: None,
        });
    }

    // Fetch the CHILD issue to get its details and body
    let child_issue = git_info.get_issue(child_issue_number).await?;
    let file_name = PathBuf::from(&child_issue.title);
    let milestone = child_issue.milestone.map(|m| m.title).unwrap_or_else(|| "No milestone".to_string());

    // Parse CHILD's body to determine relationship to parent
    // Look for parent_issue_number in child's Gating QC or Previous QC sections
    let relationship = determine_relationship_from_child_body(
        &child_issue.body.unwrap_or_default(),
        parent_issue_number
    );

    // Try to fetch issues blocked by this child
    let (children, fetch_error) = match git_info.get_blocked_issues(child_issue_number).await {
        Ok(blocked) => {
            let mut child_nodes = vec![];
            for blocked_issue in blocked {
                // Recursively build tree - this child becomes the parent for next level
                match build_impact_tree(git_info, child_issue_number, blocked_issue.number, visited).await {
                    Ok(node) => child_nodes.push(node),
                    Err(e) => {
                        // Create error placeholder node
                        child_nodes.push(ImpactNode {
                            issue_number: blocked_issue.number,
                            file_name: PathBuf::from("(fetch failed)"),
                            milestone: String::new(),
                            relationship: BlockingRelationship::Unknown,
                            children: vec![],
                            fetch_error: Some(e.to_string()),
                        });
                    }
                }
            }
            (child_nodes, None)
        },
        Err(e) => (vec![], Some(format!("Could not fetch children: {}", e))),
    };

    Ok(ImpactNode {
        issue_number: child_issue_number,
        file_name,
        milestone,
        relationship,
        children,
        fetch_error,
    })
}

/// Determine relationship by parsing child's body for parent issue number
fn determine_relationship_from_child_body(body: &str, parent_issue_number: u64) -> BlockingRelationship {
    // Parse the body for Gating QC and Previous QC sections
    let blocking_qcs = parse_blocking_qcs(body);

    // Find which section contains the parent issue
    for (issue_num, _file_name, relationship) in blocking_qcs {
        if issue_num == parent_issue_number {
            return relationship;
        }
    }

    // Return Unknown if parent not found in child's body
    // This indicates data inconsistency (API says blocked, but body doesn't list blocker)
    BlockingRelationship::Unknown
}

### 4.5 CLI Integration

**Location**: `src/main.rs`

No new flags needed - unapproval always shows impact tree when available.

### Files to Modify
- `src/git/api/read.rs`: Add `get_blocked_issues()` method
- `src/approve.rs`: Add `UnapprovalResult`, `ImpactedIssues`, `ImpactNode`, `BlockingRelationship`
- `src/approve.rs`: Add `build_impact_tree()` helper function

---

## 5. Implementation Order

1. **Phase 1: Parsing** (Foundation)
   - Add `blocking_qcs` field to `IssueThread`
   - Implement body parsing logic (combines Gating QC + Previous QC sections)
   - Use host-agnostic regex: `/issues/(\d+)`
   - Add unit tests for parsing

2. **Phase 2: Status**
   - Add `BlockingQCStatus` struct with three HashMaps
   - Add helper to check if an issue is approved
   - Update individual issue status to show Blocking QC details
   - Add `Blocking QCs` column to milestone status table
   - Add tests

3. **Phase 3: Approval**
   - Add `BlockingQCCheckResult` struct with `unapproved` and `errors` HashMaps
   - Add `get_unapproved_blocking_qcs()` function returning `BlockingQCCheckResult`
   - Add `ApprovalResult` struct with `skipped_unapproved` and `skipped_errors`
   - Add `approve_with_validation()` method
   - Add `--force` flag to CLI
   - Update interactive mode to show dependency status
   - Add tests

4. **Phase 4: Unapproval**
   - Add `get_blocked_issues()` to `GitHubReader`
   - Add `UnapprovalResult`, `ImpactedIssues`, `ImpactNode`, `BlockingRelationship`
   - Implement `build_impact_tree()` helper
   - Implement `unapprove_with_impact()` method
   - Add tests for impact tree display

---

## 6. Test Plan

### Unit Tests - Parsing
- `test_parse_blocking_qcs_from_body` - Parse both Gating QC and Previous QC sections with relationship tags
- `test_parse_blocking_qcs_empty` - Handle missing sections
- `test_parse_blocking_qcs_partial` - Handle only one section present
- `test_parse_blocking_qcs_ghe_url` - Parse GHE URLs (not just github.com)
- `test_parse_blocking_qcs_multiple_urls` - Handle multiple issues per section
- `test_parse_blocking_qcs_extracts_file_name` - Verify link text (file name) is captured

### Unit Tests - Status
- `test_blocking_qc_status_all_approved` - All HashMaps correct when all approved
- `test_blocking_qc_status_mixed` - Correct categorization of approved/not_approved/errors
- `test_blocking_qc_status_api_errors` - Errors populated correctly
- `test_milestone_status_format_with_errors` - Verify "(+N err)" suffix in display

### Unit Tests - Approval
- `test_blocking_qc_check_result_all_approved` - `all_approved()` returns true when both maps empty
- `test_blocking_qc_check_result_with_unapproved` - `all_approved()` returns false when unapproved present
- `test_blocking_qc_check_result_with_errors` - `all_approved()` returns false when errors present
- `test_blocking_qc_check_result_has_errors` - `has_errors()` returns correct value
- `test_get_unapproved_blocking_qcs_partial_failure` - Returns struct with both unapproved and errors
- `test_approval_result_display` - Verify output format with skipped_unapproved
- `test_approval_result_display_with_errors` - Verify output format with skipped_errors
- `test_approval_result_display_with_force` - Verify force warning includes both skipped types

### Unit Tests - Unapproval (Impact Tree)
- `test_unapproval_result_display_none` - No impacted issues
- `test_unapproval_result_display_api_unavailable` - API failure message
- `test_unapproval_result_display_tree` - Nested tree output format
- `test_impact_node_fmt_tree` - Tree formatting with indentation
- `test_impact_node_fmt_tree_with_errors` - Error messages displayed correctly
- `test_impact_node_fmt_tree_unknown_relationship` - Unknown relationship displayed correctly
- `test_build_impact_tree_circular` - Handles circular dependencies with Unknown relationship
- `test_determine_relationship_from_child_body_gating` - Returns GatingQC when found in Gating QC section
- `test_determine_relationship_from_child_body_previous` - Returns PreviousQC when found in Previous QC section
- `test_determine_relationship_from_child_body_not_found` - Returns Unknown when parent not in child body
- `test_build_impact_tree_partial_failure` - Creates error placeholder nodes with Unknown relationship

### Integration Tests
- Mock `GitHubReader`/`GitHubWriter` for approval flow
- Test `--force` flag bypasses validation
- Test unapproval displays impact tree correctly

---

## 7. API Reference

### Get Blocked Issues (for Unapproval cascade)
- **Endpoint**: `GET /repos/{owner}/{repo}/issues/{issue_number}/dependencies/blocking`
- **Success**: 200 OK with array of issues
- **Failures**: 403, 404, 410 (feature not available)

---

## 8. Verification Steps

1. `cargo build --all-features` - verify compilation
2. `cargo test --all-features` - verify existing + new tests pass
3. Manual test: Create issue with GatingQC, check status shows dependency
4. Manual test: Try to approve issue with unapproved GatingQC (should fail)
5. Manual test: Approve with `--force` (should succeed with warning)
6. Manual test: Unapprove a blocking issue, verify impact tree displays correctly
