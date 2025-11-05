use std::{
    io::Cursor,
    path::Path,
};

use calamine::{Data, Reader, open_workbook_auto_from_rs};
use diff::{Result as DiffResult, lines};

/// Generate a diff between two file versions
///
/// This function handles both Excel and text files, automatically detecting
/// the file type and using the appropriate diff engine.
pub fn file_diff(from_bytes: Vec<u8>, to_bytes: Vec<u8>, file: &Path) -> Option<String> {
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

/// Check if a file is an Excel file based on its extension
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

/// Generate a diff between two Excel files
pub fn diff_excel_files(from_bytes: Vec<u8>, to_bytes: Vec<u8>) -> Option<String> {
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
}

#[derive(Debug, Clone)]
struct CellChange {
    col_letter: char,
    old_value: String,
    new_value: String,
}

/// Analyze changes by row to provide better formatting
fn analyze_row_changes(
    from_range: &calamine::Range<Data>,
    to_range: &calamine::Range<Data>,
    from_dims: (usize, usize),
    to_dims: (usize, usize),
) -> Vec<RowChange> {
    let mut row_changes = Vec::new();
    let max_rows = from_dims.0.max(to_dims.0);
    let max_cols = from_dims.1.max(to_dims.1);

    // Limit the number of rows we analyze to prevent overwhelming output
    let rows_to_analyze = max_rows.min(20);

    for row in 0..rows_to_analyze {
        let row_num = row + 1; // 1-indexed for display

        // Check if this is a completely new row (beyond from_dims)
        if row >= from_dims.0 && row < to_dims.0 {
            let values = get_row_values(to_range, row, to_dims.1);
            if !values.iter().all(|v| v.is_empty()) {
                row_changes.push(RowChange::Added { row_num, values });
            }
            continue;
        }

        // Check if this is a completely removed row (beyond to_dims)
        if row >= to_dims.0 && row < from_dims.0 {
            let values = get_row_values(from_range, row, from_dims.1);
            if !values.iter().all(|v| v.is_empty()) {
                row_changes.push(RowChange::Removed { row_num, values });
            }
            continue;
        }

        // Compare existing rows cell by cell
        let mut cell_changes = Vec::new();
        let cols_to_check = max_cols.min(26); // Limit to A-Z for now

        for col in 0..cols_to_check {
            let from_cell = from_range.get((row, col)).unwrap_or(&Data::Empty);
            let to_cell = to_range.get((row, col)).unwrap_or(&Data::Empty);

            if from_cell != to_cell {
                let col_letter = (b'A' + (col % 26) as u8) as char;
                cell_changes.push(CellChange {
                    col_letter,
                    old_value: format_cell_value(from_cell),
                    new_value: format_cell_value(to_cell),
                });
            }
        }

        if !cell_changes.is_empty() {
            row_changes.push(RowChange::Modified {
                row_num,
                changes: cell_changes,
            });
        }

        // Stop early if we have too many changes to prevent overwhelming output
        if row_changes.len() >= 10 {
            break;
        }
    }

    row_changes
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
                    let change_strs: Vec<String> = cell_changes
                        .iter()
                        .map(|c| format!("{}: {} → {}", c.col_letter, c.old_value, c.new_value))
                        .collect();
                    changes.push(format!(
                        "  Row {} changes: {}",
                        row_num,
                        change_strs.join(", ")
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

/// Generate a diff between two text files
pub fn diff_text_files(from_bytes: Vec<u8>, to_bytes: Vec<u8>) -> Option<String> {
    let from_str = String::from_utf8_lossy(&from_bytes);
    let to_str = String::from_utf8_lossy(&to_bytes);
    Some(diff(&from_str, &to_str))
}

/// Generate a markdown-formatted diff between two strings showing only changed hunks with context
pub fn diff(old_content: &str, new_content: &str) -> String {
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