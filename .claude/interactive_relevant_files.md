# Implementation Plan: Interactive Relevant Files Selection

## Overview
Add interactive prompts to `QCIssue::from_interactive` for selecting relevant files during issue creation.

## Requirements Summary
- Prompt user if they want relevant files (avoids unnecessary API calls)
- Loop until user is done adding files:
  1. Auto-complete file selector (repo files only)
  2. If no matching issues: `RelevantFileClass::File` with required justification
  3. If matching issues exist: Select issue or "File" option
  4. If issue selected: Choose type (GatingQC, PreviousQC, RelevantQC) with descriptions
  5. Optional description for issues, required justification for File
- Display issues: `#123 (Milestone Name)`, current milestone first, then by issue number

## Files to Modify

### 1. `src/cli/interactive.rs`
Add new prompt functions:

```rust
/// Asks if user wants to add relevant files
pub fn prompt_want_relevant_files() -> Result<bool>

/// File selector for relevant files (similar to prompt_file but doesn't exclude existing issues)
pub fn prompt_relevant_file_path(current_dir: &PathBuf) -> Result<PathBuf>

/// Select issue or File option for a given file path
/// Returns Some(issue) if issue selected, None if File selected
pub fn prompt_relevant_file_source(
    file_path: &PathBuf,
    issues: &[Issue],
    current_milestone_number: u64,
) -> Result<Option<&Issue>>

/// Select relevant file class type (GatingQC, PreviousQC, RelevantQC)
pub fn prompt_relevant_file_class() -> Result<RelevantFileClassType>

/// Prompt for description (optional for issues)
pub fn prompt_relevant_description(required: bool) -> Result<Option<String>>

/// Asks if user wants to add another relevant file
pub fn prompt_add_another_relevant_file() -> Result<bool>
```

### 2. `src/cli/context.rs`
Modify `QCIssue::from_interactive`:

```rust
pub async fn from_interactive(...) -> Result<Self> {
    // ... existing prompts ...

    // Prompt for relevant files
    let relevant_files = if prompt_want_relevant_files()? {
        // Fetch all issues (need for matching file paths to issues)
        let all_issues = git_info.get_issues(None).await?;

        let mut relevant_files = Vec::new();
        loop {
            let file_path = prompt_relevant_file_path(project_dir)?;

            // Find matching issues (where issue.title == file_path)
            let matching_issues: Vec<_> = all_issues
                .iter()
                .filter(|i| i.title == file_path.display().to_string())
                .collect();

            let relevant_file = if matching_issues.is_empty() {
                // No matching issues - must be File type with justification
                let justification = prompt_relevant_description(true)?
                    .expect("justification required");
                RelevantFile {
                    file_name: file_path,
                    class: RelevantFileClass::File { justification },
                }
            } else {
                // Has matching issues - let user choose
                match prompt_relevant_file_source(&file_path, &matching_issues, milestone.number as u64)? {
                    Some(issue) => {
                        let class_type = prompt_relevant_file_class()?;
                        let description = prompt_relevant_description(false)?;
                        RelevantFile {
                            file_name: file_path,
                            class: match class_type {
                                GatingQC => RelevantFileClass::GatingQC {
                                    issue_number: issue.number,
                                    description
                                },
                                PreviousQC => RelevantFileClass::PreviousQC {
                                    issue_number: issue.number,
                                    description
                                },
                                RelevantQC => RelevantFileClass::RelevantQC {
                                    issue_number: issue.number,
                                    description
                                },
                            }
                        }
                    }
                    None => {
                        // User chose File
                        let justification = prompt_relevant_description(true)?
                            .expect("justification required");
                        RelevantFile {
                            file_name: file_path,
                            class: RelevantFileClass::File { justification },
                        }
                    }
                }
            };

            relevant_files.push(relevant_file);

            if !prompt_add_another_relevant_file()? {
                break;
            }
        }
        relevant_files
    } else {
        Vec::new()
    };

    // Update QCIssue::new call to include relevant_files
    let issue = QCIssue::new(
        file,
        git_info,
        milestone.number as u64,
        assignees,
        checklist,
        relevant_files,  // Add this parameter
    )?;

    Ok(issue)
}
```

## Prompt UI Details

### `prompt_want_relevant_files()`
```
Do you want to add any relevant files? (y/N)
```
- Default: false

### `prompt_relevant_file_path()`
- Same auto-complete as `prompt_file` but without excluding existing issue files
- Message: "Select a relevant file:"
- **File-issue indicator**: Show issue numbers for files that have related issues:
  ```
  > src/main.rs [#42, #15]
    src/lib.rs
    src/utils.rs [#23]
    src/config.rs
    src/complex.rs [#50, #45, #40 +2 more]
  ```
- Limit display to 3 issue numbers max, show "+N more" if exceeded
- Files without related issues show normally (no indicator)
- Requires passing the issues list to the FileCompleter to build the indicator map

### `prompt_relevant_file_source()`
- If matching issues found, show selection:
```
Select the source for 'path/to/file.rs':
> #42 (Current Milestone)
  #15 (Other Milestone)
  #8 (Another Milestone)
  ────────────────────────
  File (not linked to an issue)
```
- Current milestone issues first, then others sorted by issue number descending

### `prompt_relevant_file_class()`
```
Select the relationship type:
> Gating QC - This issue must be approved before the current issue
  Previous QC - A previous version of the QC for this file
  Relevant QC - Related QC that provides context
```

### `prompt_relevant_description()`
- If required: "Provide a justification for this file:"
- If optional: "Add a description? (optional, press Enter to skip):"

### `prompt_add_another_relevant_file()`
```
Add another relevant file? (y/N)
```
- Default: false

## Verification Steps
1. `cargo build --all-features` - verify compilation
2. `cargo test --all-features` - verify existing tests pass
3. Manual test: `cargo run --all-features -- issue create` in interactive mode
   - Test with no relevant files (answer 'n')
   - Test with File type (select file with no matching issues)
   - Test with issue type (select file with matching issues)
   - Test adding multiple relevant files
   - Verify the created issue body contains correct relevant files section
