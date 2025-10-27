use std::{io::Cursor, path::Path};

use calamine::{Data, Range, Reader, open_workbook_auto_from_rs};
use diff::{Result as DiffResult, lines};
use gix::ObjectId;
use rust_xlsxwriter::{Color, Format, Workbook, worksheet::IntoExcelData};

use crate::GitFileOps;

// Excel diff format constants
const ADDED_COLOR: u32 = 0xD4F6D4; // Light green
const MODIFIED_COLOR: u32 = 0xD4E6F6; // Light blue
const REMOVED_COLOR: u32 = 0xF6D4D4; // Light red

/// Generate an Excel file with visual diff highlighting
pub fn create_excel_diff(
    file: impl AsRef<Path>,
    from_commit: &ObjectId,
    to_commit: &ObjectId,
    git_info: &impl GitFileOps,
    output_path: impl AsRef<Path>,
) -> Result<(), DiffError> {
    let file = file.as_ref();

    // Get bytes from both commits
    let from_bytes = git_info
        .file_bytes_at_commit(file, from_commit)?;
    let to_bytes = git_info
        .file_bytes_at_commit(file, to_commit)
        .map_err(|error| DiffError::ToCommitReadError { error })?;

    // Create cursors for reading Excel files
    let from_cursor = Cursor::new(from_bytes);
    let to_cursor = Cursor::new(to_bytes);

    // Open both workbooks
    let mut from_workbook = open_workbook_auto_from_rs(from_cursor)
        .map_err(DiffError::FromWorkbookError)?;
    let mut to_workbook = open_workbook_auto_from_rs(to_cursor)
        .map_err(DiffError::ToWorkbookError)?;

    // Create new workbook for output
    let mut workbook = Workbook::new();

    // Get all sheet names from both workbooks
    let from_sheets: std::collections::HashSet<String> =
        from_workbook.sheet_names().iter().cloned().collect();
    let to_sheets: std::collections::HashSet<String> =
        to_workbook.sheet_names().iter().cloned().collect();

    // Process all sheets (from both workbooks)
    let all_sheets: std::collections::HashSet<String> =
        from_sheets.union(&to_sheets).cloned().collect();

    for sheet_name in all_sheets {
        let mut worksheet = workbook.add_worksheet();
        worksheet.set_name(&sheet_name)
            .map_err(DiffError::WorksheetNameError)?;

        // Get ranges from both workbooks (if they exist)
        let from_range = if from_sheets.contains(&sheet_name) {
            from_workbook.worksheet_range(&sheet_name).ok()
        } else {
            None
        };

        let to_range = if to_sheets.contains(&sheet_name) {
            to_workbook.worksheet_range(&sheet_name).ok()
        } else {
            None
        };

        match (from_range, to_range) {
            (Some(from_range), Some(to_range)) => {
                // Both sheets exist - compare them
                create_diff_sheet(&mut worksheet, &from_range, &to_range)?;
            }
            (Some(from_range), None) => {
                // Sheet only exists in from (was deleted)
                create_removed_sheet(&mut worksheet, &from_range)?;
            }
            (None, Some(to_range)) => {
                // Sheet only exists in to (was added)
                create_added_sheet(&mut worksheet, &to_range)?;
            }
            (None, None) => {
                // This shouldn't happen since we got the name from one of the sets
                continue;
            }
        }
    }

    // Save the workbook
    workbook.save(output_path)?;
    Ok(())
}

/// Create a diff sheet comparing two ranges
fn create_diff_sheet(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    from_range: &Range<Data>,
    to_range: &Range<Data>,
) -> Result<(), DiffError> {
    let from_dims = from_range.get_size();
    let to_dims = to_range.get_size();
    let max_cols = from_dims.1.max(to_dims.1);

    // Use our improved row diff algorithm to get change information
    let row_changes = analyze_row_changes(from_range, to_range, from_dims, to_dims);

    // Create a mapping of row changes for quick lookup
    let mut row_change_map = std::collections::HashMap::new();
    for change in &row_changes {
        match change {
            RowChange::Added { row_num, .. } => {
                row_change_map.insert(*row_num - 1, Some(ADDED_COLOR));
            },
            RowChange::Removed { .. } => {
                // Removed rows don't exist in the "to" version, so we don't map them here
                // They'll be handled separately in the display logic
            },
            RowChange::Modified { row_num, .. } => {
                row_change_map.insert(*row_num - 1, Some(MODIFIED_COLOR));
            },
            RowChange::Moved { to_row, .. } => {
                row_change_map.insert(*to_row - 1, Some(MODIFIED_COLOR));
            },
        }
    }

    // Write the "to" version of the data with appropriate formatting
    for row in 0..to_dims.0 {
        for col in 0..max_cols {
            let to_cell = to_range.get((row, col)).unwrap_or(&Data::Empty);
            let from_cell = from_range.get((row, col)).unwrap_or(&Data::Empty);

            // Determine the appropriate color
            let color = if let Some(&row_color) = row_change_map.get(&row) {
                // Row has changes according to our diff algorithm
                if let Some(row_color_val) = row_color {
                    // For modified/moved rows, check if this specific cell changed
                    if row_color_val == MODIFIED_COLOR && from_cell == to_cell {
                        None // Cell didn't change within a modified row
                    } else {
                        Some(row_color_val)
                    }
                } else {
                    None
                }
            } else if from_cell != to_cell {
                // Cell-level change detection (fallback for cases not caught by row analysis)
                Some(MODIFIED_COLOR)
            } else {
                None // Unchanged
            };

            write_cell_with_color(worksheet, row as u32, col as u16, to_cell, color)
                .map_err(DiffError::CellWriteError)?;
        }
    }

    Ok(())
}

/// Create a sheet showing only removed content
fn create_removed_sheet(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    from_range: &Range<Data>,
) -> Result<(), DiffError> {
    let dims = from_range.get_size();

    for row in 0..dims.0 {
        for col in 0..dims.1 {
            let cell = from_range.get((row, col)).unwrap_or(&Data::Empty);
            write_cell_with_color(worksheet, row as u32, col as u16, cell, Some(REMOVED_COLOR))
                .map_err(DiffError::CellWriteError)?;
        }
    }

    Ok(())
}

/// Create a sheet showing only added content
fn create_added_sheet(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    to_range: &Range<Data>,
) -> Result<(), DiffError> {
    let dims = to_range.get_size();

    for row in 0..dims.0 {
        for col in 0..dims.1 {
            let cell = to_range.get((row, col)).unwrap_or(&Data::Empty);
            write_cell_with_color(worksheet, row as u32, col as u16, cell, Some(ADDED_COLOR))
                .map_err(DiffError::CellWriteError)?;
        }
    }

    Ok(())
}

fn write_value(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    col: u16,
    value: impl IntoExcelData,
    color: Option<u32>,
) -> Result<(), rust_xlsxwriter::XlsxError> {
    if let Some(bg_color) = color {
        let format = Format::new().set_background_color(Color::RGB(bg_color));
        worksheet.write_with_format(row, col, value, &format)?;
    } else {
        worksheet.write(row, col, value)?;
    }

    Ok(())
}

/// Write a cell value with optional background color using rust_xlsxwriter's write_with_format
fn write_cell_with_color(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    col: u16,
    cell: &Data,
    color: Option<u32>,
) -> Result<(), rust_xlsxwriter::XlsxError> {
    match cell {
        Data::Empty => write_value(worksheet, row, col, "", color),
        Data::String(s) => write_value(worksheet, row, col, s, color),
        Data::Float(f) => write_value(worksheet, row, col, *f, color),
        Data::Int(i) => write_value(worksheet, row, col, *i, color),
        Data::Bool(b) => write_value(worksheet, row, col, *b, color),
        Data::Error(e) => write_value(worksheet, row, col, format!("ERROR({e})"), color),
        Data::DateTime(dt) => write_value(worksheet, row, col, dt.as_f64(), color),
        Data::DateTimeIso(dt) => write_value(worksheet, row, col, format!("ISO_DATE({dt})"), color),
        Data::DurationIso(d) => {
            write_value(worksheet, row, col, format!("ISO_DURATION({d})"), color)
        }
    }
}

pub(crate) fn file_diff(
    file: impl AsRef<Path>,
    from_commit: &ObjectId,
    to_commit: &ObjectId,
    git_info: &impl GitFileOps,
) -> Option<String> {
    let file = file.as_ref();
    let Ok(from_bytes) = git_info.file_bytes_at_commit(file, from_commit) else {
        log::debug!("Could not read file at from commit ({from_commit})...");
        return None;
    };
    // Get bytes from both commits
    let to_bytes = git_info.file_bytes_at_commit(file, to_commit).ok()?;

    // Try to handle as Excel file first
    if is_excel_file(file) {
        if let Some(excel_diff) = diff_excel_files(from_bytes.clone(), to_bytes.clone()) {
            return Some(excel_diff);
        }
        log::debug!("Failed to diff as Excel, falling back to text diff");
    }

    // Fall back to text diff
    diff_text_files(from_bytes, to_bytes)
}

pub fn is_excel_file(file: &Path) -> bool {
    if let Some(ext) = file.extension().and_then(|e| e.to_str()) {
        matches!(
            ext.to_lowercase().as_str(),
            "xlsx" | "xlsm" | "xlsb" | "xls"
        )
    } else {
        false
    }
}

fn diff_excel_files(from_bytes: Vec<u8>, to_bytes: Vec<u8>) -> Option<String> {
    // Use Cursor to provide Read + Seek traits
    let from_cursor = Cursor::new(from_bytes);
    let to_cursor = Cursor::new(to_bytes);

    // Try to open both workbooks
    let mut from_workbook = open_workbook_auto_from_rs(from_cursor).ok()?;
    let mut to_workbook = open_workbook_auto_from_rs(to_cursor).ok()?;

    let mut diff_lines = Vec::new();
    diff_lines.push("```diff".to_string());

    // Get worksheet names from both workbooks
    let from_sheets: std::collections::HashSet<String> =
        from_workbook.sheet_names().iter().cloned().collect();
    let to_sheets: std::collections::HashSet<String> =
        to_workbook.sheet_names().iter().cloned().collect();

    // Check for added/removed sheets
    for sheet in &from_sheets {
        if !to_sheets.contains(sheet) {
            diff_lines.push(format!("- Sheet removed: {}", sheet));
        }
    }

    for sheet in &to_sheets {
        if !from_sheets.contains(sheet) {
            diff_lines.push(format!("+ Sheet added: {}", sheet));
        }
    }

    // Compare common sheets
    for sheet_name in from_sheets.intersection(&to_sheets) {
        if let Some(sheet_diff) = diff_excel_sheet(&mut from_workbook, &mut to_workbook, sheet_name)
        {
            diff_lines.push(format!("@@ Sheet: {} @@", sheet_name));
            diff_lines.extend(sheet_diff);
        }
    }

    diff_lines.push("```".to_string());

    if diff_lines.len() > 2 {
        // More than just the ``` markers
        Some(diff_lines.join("\n"))
    } else {
        Some("\nNo differences between Excel file versions.\n".to_string())
    }
}

fn diff_excel_sheet<R>(
    from_workbook: &mut R,
    to_workbook: &mut R,
    sheet_name: &str,
) -> Option<Vec<String>>
where
    R: Reader<Cursor<Vec<u8>>>,
{
    let from_range = from_workbook.worksheet_range(sheet_name).ok()?;
    let to_range = to_workbook.worksheet_range(sheet_name).ok()?;

    let mut changes = Vec::new();
    let mut has_changes = false;

    // Get dimensions
    let from_dims = from_range.get_size();
    let to_dims = to_range.get_size();

    if from_dims != to_dims {
        changes.push(format!(
            "  Sheet dimensions changed: {}x{} -> {}x{}",
            from_dims.1, from_dims.0, to_dims.1, to_dims.0
        ));
        has_changes = true;
    }

    // Analyze changes by row for better formatting
    let row_changes = analyze_row_changes(&from_range, &to_range, from_dims, to_dims);

    if !row_changes.is_empty() {
        format_row_changes(&mut changes, &row_changes);
        has_changes = true;
    }

    if has_changes { Some(changes) } else { None }
}

#[derive(Debug, Clone)]
enum RowChange {
    Added {
        row_num: usize,
        values: Vec<String>,
    },
    Removed {
        row_num: usize,
        values: Vec<String>,
    },
    Modified {
        row_num: usize,
        changes: Vec<CellChange>,
    },
    Moved {
        from_row: usize,
        to_row: usize,
        changes: Vec<CellChange>,
    },
}

#[derive(Debug, Clone)]
struct CellChange {
    col_letter: char,
    old_value: String,
    new_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RowSignature {
    /// First few cells of the row used to identify it
    key_cells: Vec<String>,
    /// Full row content for exact matching
    full_content: Vec<String>,
}

impl RowSignature {
    fn new(values: Vec<String>) -> Self {
        // Use first 3 non-empty cells as the key, or all if fewer than 3
        let key_cells = values
            .iter()
            .filter(|s| !s.is_empty())
            .take(3)
            .cloned()
            .collect();

        Self {
            key_cells,
            full_content: values,
        }
    }

    /// Calculate similarity between two row signatures (0.0 to 1.0)
    fn similarity(&self, other: &RowSignature) -> f64 {
        if self.key_cells.is_empty() && other.key_cells.is_empty() {
            return 1.0;
        }

        if self.key_cells.is_empty() || other.key_cells.is_empty() {
            return 0.0;
        }

        // Count matching key cells
        let mut matches = 0;
        let max_len = self.key_cells.len().max(other.key_cells.len());
        let empty_string = String::new();

        for i in 0..max_len {
            let self_cell = self.key_cells.get(i).unwrap_or(&empty_string);
            let other_cell = other.key_cells.get(i).unwrap_or(&empty_string);

            if !self_cell.is_empty() && !other_cell.is_empty() && self_cell == other_cell {
                matches += 1;
            }
        }

        matches as f64 / max_len as f64
    }

    /// Check if this signature represents an empty row
    fn is_empty(&self) -> bool {
        self.full_content.iter().all(|s| s.is_empty())
    }
}

/// Analyze changes by row to provide better formatting, detecting insertions and moves
fn analyze_row_changes(
    from_range: &calamine::Range<Data>,
    to_range: &calamine::Range<Data>,
    from_dims: (usize, usize),
    to_dims: (usize, usize),
) -> Vec<RowChange> {
    let max_cols = from_dims.1.max(to_dims.1);
    let cols_to_use = max_cols.min(26); // Limit to A-Z for now

    // Create row signatures for both versions
    let from_rows: Vec<RowSignature> = (0..from_dims.0)
        .map(|row| {
            let values = get_row_values(from_range, row, cols_to_use);
            RowSignature::new(values)
        })
        .collect();

    let to_rows: Vec<RowSignature> = (0..to_dims.0)
        .map(|row| {
            let values = get_row_values(to_range, row, cols_to_use);
            RowSignature::new(values)
        })
        .collect();

    // Use improved diff algorithm
    analyze_row_diff(&from_rows, &to_rows, from_range, to_range, cols_to_use)
}

/// Perform intelligent row diffing that detects insertions, deletions, and moves
fn analyze_row_diff(
    from_rows: &[RowSignature],
    to_rows: &[RowSignature],
    from_range: &calamine::Range<Data>,
    to_range: &calamine::Range<Data>,
    cols_to_use: usize,
) -> Vec<RowChange> {
    let mut changes = Vec::new();
    let mut from_used = vec![false; from_rows.len()];
    let mut to_used = vec![false; to_rows.len()];

    // First pass: find exact matches in same position (no changes needed)
    for (idx, (from_row, to_row)) in from_rows.iter().zip(to_rows.iter()).enumerate() {
        if from_row.is_empty() || to_row.is_empty() {
            continue;
        }

        if from_row.full_content == to_row.full_content {
            from_used[idx] = true;
            to_used[idx] = true;
        }
    }

    // Second pass: find moves and modifications (rows with high similarity)
    const SIMILARITY_THRESHOLD: f64 = 0.6; // At least 60% similar to consider a move

    for (from_idx, from_row) in from_rows.iter().enumerate() {
        if from_used[from_idx] || from_row.is_empty() {
            continue;
        }

        let mut best_match = None;
        let mut best_similarity = SIMILARITY_THRESHOLD;

        for (to_idx, to_row) in to_rows.iter().enumerate() {
            if to_used[to_idx] || to_row.is_empty() {
                continue;
            }

            let similarity = from_row.similarity(to_row);
            if similarity > best_similarity {
                best_similarity = similarity;
                best_match = Some(to_idx);
            }
        }

        if let Some(to_idx) = best_match {
            from_used[from_idx] = true;
            to_used[to_idx] = true;

            // Calculate cell changes
            let cell_changes =
                calculate_cell_changes(from_range, to_range, from_idx, to_idx, cols_to_use);

            if from_idx == to_idx {
                // Same position, just modified
                if !cell_changes.is_empty() {
                    changes.push(RowChange::Modified {
                        row_num: to_idx + 1,
                        changes: cell_changes,
                    });
                }
            } else {
                // Different position, it's a move
                changes.push(RowChange::Moved {
                    from_row: from_idx + 1,
                    to_row: to_idx + 1,
                    changes: cell_changes,
                });
            }
        }
    }

    // Third pass: handle deletions (unused from_rows)
    for (from_idx, from_row) in from_rows.iter().enumerate() {
        if !from_used[from_idx] && !from_row.is_empty() {
            changes.push(RowChange::Removed {
                row_num: from_idx + 1,
                values: from_row.full_content.clone(),
            });
        }
    }

    // Fourth pass: handle additions (unused to_rows)
    for (to_idx, to_row) in to_rows.iter().enumerate() {
        if !to_used[to_idx] && !to_row.is_empty() {
            changes.push(RowChange::Added {
                row_num: to_idx + 1,
                values: to_row.full_content.clone(),
            });
        }
    }

    // Sort changes by row number for cleaner output
    changes.sort_by_key(|change| match change {
        RowChange::Added { row_num, .. } => *row_num,
        RowChange::Removed { row_num, .. } => *row_num,
        RowChange::Modified { row_num, .. } => *row_num,
        RowChange::Moved { to_row, .. } => *to_row,
    });

    // Limit output to prevent overwhelming results
    if changes.len() > 10 {
        changes.truncate(10);
    }

    changes
}

fn calculate_cell_changes(
    from_range: &calamine::Range<Data>,
    to_range: &calamine::Range<Data>,
    from_row: usize,
    to_row: usize,
    cols_to_use: usize,
) -> Vec<CellChange> {
    let mut cell_changes = Vec::new();

    for col in 0..cols_to_use {
        let from_cell = from_range.get((from_row, col)).unwrap_or(&Data::Empty);
        let to_cell = to_range.get((to_row, col)).unwrap_or(&Data::Empty);

        if from_cell != to_cell {
            let col_letter = (b'A' + (col % 26) as u8) as char;
            cell_changes.push(CellChange {
                col_letter,
                old_value: format_cell_value(from_cell),
                new_value: format_cell_value(to_cell),
            });
        }
    }

    cell_changes
}

/// Get all values from a row as strings
fn get_row_values(range: &calamine::Range<Data>, row: usize, num_cols: usize) -> Vec<String> {
    let mut values = Vec::new();
    let cols_to_get = num_cols.min(26); // Limit to A-Z

    for col in 0..cols_to_get {
        let cell = range.get((row, col)).unwrap_or(&Data::Empty);
        values.push(format_cell_value(cell));
    }

    values
}

/// Format row changes into readable diff output
fn format_row_changes(changes: &mut Vec<String>, row_changes: &[RowChange]) {
    let mut change_count = 0;

    for row_change in row_changes {
        if change_count >= 10 {
            let remaining = row_changes.len() - change_count;
            changes.push(format!("  ... and {} more row changes", remaining));
            break;
        }

        match row_change {
            RowChange::Added { row_num, values } => {
                let row_content = values.join(" | ");
                changes.push(format!("+ Row {}: {}", row_num, row_content));
            }
            RowChange::Removed { row_num, values } => {
                let row_content = values.join(" | ");
                changes.push(format!("- Row {}: {}", row_num, row_content));
            }
            RowChange::Modified {
                row_num,
                changes: cell_changes,
            } => {
                if cell_changes.len() == 1 {
                    // Single cell change - show it concisely
                    let change = &cell_changes[0];
                    changes.push(format!(
                        "  Row {} {}: {} → {}",
                        row_num, change.col_letter, change.old_value, change.new_value
                    ));
                } else if cell_changes.len() <= 3 {
                    // Few cell changes - show them on one line
                    let change_strings: Vec<String> = cell_changes
                        .iter()
                        .map(|c| format!("{}: {} → {}", c.col_letter, c.old_value, c.new_value))
                        .collect();
                    changes.push(format!(
                        "  Row {} changes: {}",
                        row_num,
                        change_strings.join(", ")
                    ));
                } else {
                    // Many cell changes - show summary
                    changes.push(format!(
                        "  Row {} has {} cell changes",
                        row_num,
                        cell_changes.len()
                    ));
                    for change in cell_changes.iter().take(3) {
                        changes.push(format!(
                            "    {}: {} → {}",
                            change.col_letter, change.old_value, change.new_value
                        ));
                    }
                    if cell_changes.len() > 3 {
                        changes.push(format!("    ... and {} more", cell_changes.len() - 3));
                    }
                }
            }
            RowChange::Moved {
                from_row,
                to_row,
                changes: cell_changes,
            } => {
                if cell_changes.is_empty() {
                    // Pure move with no changes
                    changes.push(format!("  Row {} moved to row {}", from_row, to_row));
                } else if cell_changes.len() == 1 {
                    // Move with single cell change
                    let change = &cell_changes[0];
                    changes.push(format!(
                        "  Row {} moved to row {} with change: {}: {} → {}",
                        from_row, to_row, change.col_letter, change.old_value, change.new_value
                    ));
                } else {
                    // Move with multiple changes
                    changes.push(format!(
                        "  Row {} moved to row {} with {} changes:",
                        from_row,
                        to_row,
                        cell_changes.len()
                    ));
                    for change in cell_changes.iter().take(3) {
                        changes.push(format!(
                            "    {}: {} → {}",
                            change.col_letter, change.old_value, change.new_value
                        ));
                    }
                    if cell_changes.len() > 3 {
                        changes.push(format!("    ... and {} more", cell_changes.len() - 3));
                    }
                }
            }
        }

        change_count += 1;
    }
}

fn format_cell_value(cell: &Data) -> String {
    match cell {
        Data::Empty => "".to_string(),
        Data::String(s) => format!("\"{}\"", s),
        Data::Float(f) => f.to_string(),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::Error(e) => format!("ERROR({:?})", e),
        Data::DateTime(dt) => format!("DATE({})", dt),
        Data::DateTimeIso(dt) => format!("ISO_DATE({})", dt),
        Data::DurationIso(d) => format!("ISO_DURATION({})", d),
    }
}

fn diff_text_files(from_bytes: Vec<u8>, to_bytes: Vec<u8>) -> Option<String> {
    let from_str = String::from_utf8_lossy(&from_bytes);
    let to_str = String::from_utf8_lossy(&to_bytes);
    Some(diff(&from_str, &to_str))
}

/// Generate a markdown-formatted diff between two strings showing only changed hunks with context
fn diff(old_content: &str, new_content: &str) -> String {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    // Check if files are identical
    if old_lines == new_lines {
        return "\nNo difference between file versions.\n".to_string();
    }

    let changeset = lines(old_content, new_content);

    // Group changes into hunks with context
    let hunks = create_hunks(&changeset, 3); // 3 lines of context

    if hunks.is_empty() {
        return "\nNo difference between file versions.\n".to_string();
    }

    let mut result = Vec::new();
    result.push("```diff".to_string());

    for hunk in hunks {
        result.push(format_hunk(&hunk));
    }

    result.push("```".to_string());
    result.join("\n")
}

#[derive(Debug, Clone)]
struct DiffHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
enum DiffLine {
    Context(String, usize, usize), // content, old_line_num, new_line_num
    Addition(String, usize),       // content, new_line_num
    Deletion(String, usize),       // content, old_line_num
}

fn create_hunks(changeset: &[DiffResult<&str>], context_lines: usize) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_hunk_lines = Vec::new();
    let mut old_line = 1;
    let mut new_line = 1;
    let mut last_change_idx = None;

    for (idx, change) in changeset.iter().enumerate() {
        let is_change = matches!(change, DiffResult::Left(_) | DiffResult::Right(_));

        if is_change {
            // If this is a change, include context before it if we haven't started a hunk
            if current_hunk_lines.is_empty() {
                let context_start = idx.saturating_sub(context_lines);
                for i in context_start..idx {
                    if let DiffResult::Both(line, _) = &changeset[i] {
                        let ctx_old = old_line - (idx - i);
                        let ctx_new = new_line - (idx - i);
                        current_hunk_lines.push(DiffLine::Context(
                            line.to_string(),
                            ctx_old,
                            ctx_new,
                        ));
                    }
                }
            }
            last_change_idx = Some(idx);
        }

        // Add the current line to the hunk
        match change {
            DiffResult::Left(line) => {
                current_hunk_lines.push(DiffLine::Deletion(line.to_string(), old_line));
                old_line += 1;
            }
            DiffResult::Right(line) => {
                current_hunk_lines.push(DiffLine::Addition(line.to_string(), new_line));
                new_line += 1;
            }
            DiffResult::Both(line, _) => {
                if !current_hunk_lines.is_empty() {
                    current_hunk_lines.push(DiffLine::Context(
                        line.to_string(),
                        old_line,
                        new_line,
                    ));
                }
                old_line += 1;
                new_line += 1;
            }
        }

        // Check if we should end the current hunk
        if let Some(last_change) = last_change_idx {
            let distance_from_last_change = idx - last_change;
            if distance_from_last_change >= context_lines * 2 && !current_hunk_lines.is_empty() {
                // Trim to exactly context_lines after the last change
                let mut lines_to_keep = current_hunk_lines.len();
                let mut context_after_change = 0;

                // Count backwards from the end to find where to cut off
                for (i, line) in current_hunk_lines.iter().enumerate().rev() {
                    if matches!(line, DiffLine::Context(_, _, _)) {
                        context_after_change += 1;
                        if context_after_change > context_lines {
                            lines_to_keep = i + 1;
                            break;
                        }
                    } else {
                        // Hit a change line, reset counter
                        context_after_change = 0;
                    }
                }

                current_hunk_lines.truncate(lines_to_keep);

                if let Some(hunk) = create_hunk_from_lines(current_hunk_lines.clone()) {
                    hunks.push(hunk);
                }
                current_hunk_lines.clear();
                last_change_idx = None;
            }
        }
    }

    // Handle remaining hunk
    if !current_hunk_lines.is_empty() {
        // Trim final hunk to exactly context_lines after the last change
        let mut lines_to_keep = current_hunk_lines.len();
        let mut context_after_change = 0;

        // Count backwards from the end to find where to cut off
        for (i, line) in current_hunk_lines.iter().enumerate().rev() {
            if matches!(line, DiffLine::Context(_, _, _)) {
                context_after_change += 1;
                if context_after_change > context_lines {
                    lines_to_keep = i + 1;
                    break;
                }
            } else {
                // Hit a change line, reset counter
                context_after_change = 0;
            }
        }

        current_hunk_lines.truncate(lines_to_keep);

        if let Some(hunk) = create_hunk_from_lines(current_hunk_lines) {
            hunks.push(hunk);
        }
    }

    hunks
}

fn create_hunk_from_lines(lines: Vec<DiffLine>) -> Option<DiffHunk> {
    if lines.is_empty() {
        return None;
    }

    let mut old_start = usize::MAX;
    let mut new_start = usize::MAX;
    let mut old_count = 0;
    let mut new_count = 0;

    for line in &lines {
        match line {
            DiffLine::Context(_, old_num, new_num) => {
                old_start = old_start.min(*old_num);
                new_start = new_start.min(*new_num);
                old_count += 1;
                new_count += 1;
            }
            DiffLine::Addition(_, new_num) => {
                new_start = new_start.min(*new_num);
                new_count += 1;
            }
            DiffLine::Deletion(_, old_num) => {
                old_start = old_start.min(*old_num);
                old_count += 1;
            }
        }
    }

    Some(DiffHunk {
        old_start,
        old_count,
        new_start,
        new_count,
        lines,
    })
}

fn format_hunk(hunk: &DiffHunk) -> String {
    let mut result = Vec::new();

    // Add hunk header
    result.push(format!(
        "@@ previous script: lines {}-{} @@",
        hunk.old_start,
        hunk.old_start + hunk.old_count - 1
    ));
    result.push(format!(
        "@@  current script: lines {}-{} @@",
        hunk.new_start,
        hunk.new_start + hunk.new_count - 1
    ));

    // Add hunk content with line numbers
    for line in &hunk.lines {
        match line {
            DiffLine::Context(content, _, new_num) => {
                result.push(format!("  {} {}", new_num, content));
            }
            DiffLine::Addition(content, new_num) => {
                result.push(format!("+ {} {}", new_num, content));
            }
            DiffLine::Deletion(content, old_num) => {
                result.push(format!("- {} {}", old_num, content));
            }
        }
    }

    result.join("\n")
}

// string diff testing in comment.rs

#[derive(thiserror::Error, Debug)]
pub enum DiffError {
    #[error("Failed to read file at from commit: {0}")]
    FromCommitReadError(#[from] crate::git::GitFileOpsError),
    #[error("Failed to read file at to commit: {error}")]
    ToCommitReadError { error: crate::git::GitFileOpsError },
    #[error("Could not open from workbook: {0}")]
    FromWorkbookError(calamine::Error),
    #[error("Could not open to workbook: {0}")]
    ToWorkbookError(calamine::Error),
    #[error("Failed to save workbook: {0}")]
    WorkbookSaveError(#[from] rust_xlsxwriter::XlsxError),
    #[error("Failed to set worksheet name: {0}")]
    WorksheetNameError(rust_xlsxwriter::XlsxError),
    #[error("Failed to write cell data: {0}")]
    CellWriteError(rust_xlsxwriter::XlsxError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{Data, Range};

    #[test]
    fn test_row_insertion_in_middle() {
        // Before: A, B, C
        let mut from_range = Range::new((0, 0), (2, 0)); // 3 rows, 1 column
        from_range.set_value((0, 0), Data::String("A".to_string()));
        from_range.set_value((1, 0), Data::String("B".to_string()));
        from_range.set_value((2, 0), Data::String("C".to_string()));

        // After: A, X, B, C (inserted X between A and B)
        let mut to_range = Range::new((0, 0), (3, 0)); // 4 rows, 1 column
        to_range.set_value((0, 0), Data::String("A".to_string()));
        to_range.set_value((1, 0), Data::String("X".to_string())); // New row inserted
        to_range.set_value((2, 0), Data::String("B".to_string())); // B moved down
        to_range.set_value((3, 0), Data::String("C".to_string())); // C moved down

        let changes = analyze_row_changes(&from_range, &to_range, (3, 1), (4, 1));

        println!("Row insertion test results:");
        for change in &changes {
            match change {
                RowChange::Added { row_num, values } => {
                    println!("+ Row {}: {}", row_num, values.join(" | "));
                }
                RowChange::Moved {
                    from_row,
                    to_row,
                    changes: _,
                } => {
                    println!("  Row {} moved to row {}", from_row, to_row);
                }
                RowChange::Modified {
                    row_num,
                    changes: _,
                } => {
                    println!("  Row {} modified", row_num);
                }
                RowChange::Removed { row_num, values } => {
                    println!("- Row {}: {}", row_num, values.join(" | "));
                }
            }
        }

        // Verify the algorithm correctly detects:
        // 1. Addition of "X" at row 2
        // 2. Move of "B" from row 2 to row 3
        // 3. Move of "C" from row 3 to row 4
        // NOT: Modifications of rows 2 and 3

        let additions: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c, RowChange::Added { .. }))
            .collect();
        let moves: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c, RowChange::Moved { .. }))
            .collect();
        let modifications: Vec<_> = changes
            .iter()
            .filter(|c| matches!(c, RowChange::Modified { .. }))
            .collect();

        assert_eq!(additions.len(), 1, "Should have exactly 1 addition");
        assert_eq!(moves.len(), 2, "Should have exactly 2 moves (B and C)");
        assert_eq!(modifications.len(), 0, "Should have 0 modifications");
    }

    #[test]
    fn test_row_signature_similarity() {
        let sig_same = RowSignature::new(vec!["A".to_string(), "B".to_string()]);
        let sig_identical = RowSignature::new(vec!["A".to_string(), "B".to_string()]);
        let sig_different = RowSignature::new(vec!["X".to_string(), "Y".to_string()]);

        assert_eq!(sig_same.similarity(&sig_identical), 1.0);
        assert_eq!(sig_same.similarity(&sig_different), 0.0);
    }
}
