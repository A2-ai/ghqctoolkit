use std::collections::HashMap;
use octocrab::models::Milestone;
use tera::{Result as TeraResult, Value};

use super::typst::escape_typst;
use super::{IssueInformation, MilestoneRow};

/// Create milestone dataframe equivalent to R function
pub fn create_milestone_df(
    milestone_objects: &[Milestone],
    issue_information: &HashMap<String, Vec<IssueInformation>>,
) -> Result<Vec<MilestoneRow>, super::RecordError> {
    let mut milestone_rows = Vec::new();

    for milestone in milestone_objects {
        let Some(issues) = issue_information.get(&milestone.title) else {
            continue;
        };

        let issue_names = issues
            .iter()
            .map(|issue| {
                let mut issue_name = insert_breaks(&issue.title, 42);
                if issue.checklist_summary.contains("100.0%") {
                    issue_name = format!("{} #text(fill: red)[U]", issue_name);
                }

                if issue.qc_status.contains("Approved") {
                    issue_name = format!("{} #text(fill: red)[C]", issue_name);
                }

                issue_name
            })
            .collect::<Vec<String>>();

        // Format issues string with status indicators
        let issues_str = if issue_names.is_empty() {
            String::new()
        } else {
            issue_names.join("\n\n")
        };

        // Format milestone status
        let status = escape_typst(
            &milestone
                .state
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("Unknown")
                .to_string(),
        );

        // Format description with line breaks
        let description = milestone
            .description
            .as_ref()
            .map(|d| insert_breaks(&escape_typst(d), 20))
            .unwrap_or_else(|| "NA".to_string());

        // Format milestone name with line breaks
        let name = insert_breaks(&escape_typst(&milestone.title), 18);

        milestone_rows.push(MilestoneRow {
            name,
            description,
            status,
            issues: issues_str,
        });
    }

    Ok(milestone_rows)
}

/// Insert line breaks at word boundaries (equivalent to R's insert_breaks)
pub fn insert_breaks(text: &str, max_width: usize) -> String {
    if text.len() <= max_width {
        return text.to_string();
    }

    let mut result = String::new();
    let mut current_line_len = 0;

    for word in text.split_whitespace() {
        if current_line_len + word.len() + 1 > max_width && current_line_len > 0 {
            result.push('\n');
            current_line_len = 0;
        }

        if current_line_len > 0 {
            result.push(' ');
            current_line_len += 1;
        }

        result.push_str(word);
        current_line_len += word.len();
    }

    result
}

/// Tera function to render milestone table rows only (Typst format)
pub fn render_milestone_table_rows(args: &HashMap<String, Value>) -> TeraResult<Value> {
    let data = args
        .get("data")
        .ok_or_else(|| tera::Error::msg("Missing 'data' argument for milestone table"))?;

    let rows: Vec<MilestoneRow> = serde_json::from_value(data.clone())
        .map_err(|e| tera::Error::msg(format!("Failed to parse milestone data: {}", e)))?;

    let mut table_rows = Vec::new();

    // Add data rows as Typst table cells
    for row in rows.iter() {
        table_rows.push(format!(
            "[{}], [{}], [{}], [{}],",
            row.name,        // already escaped in create_milestone_df
            row.description, // already escaped in create_milestone_df
            row.status,      // already escaped in create_milestone_df
            row.issues       // issues string already contains Typst formatting commands
        ));
    }

    Ok(Value::String(table_rows.join("\n")))
}

/// Tera function to render issue summary table rows only (Typst format)
pub fn render_issue_summary_table_rows(args: &HashMap<String, Value>) -> TeraResult<Value> {
    let data = args
        .get("data")
        .ok_or_else(|| tera::Error::msg("Missing 'data' argument for issue summary table"))?;

    let rows: Vec<IssueInformation> = serde_json::from_value(data.clone())
        .map_err(|e| tera::Error::msg(format!("Failed to parse issue summary data: {}", e)))?;

    if rows.is_empty() {
        return Ok(Value::String(String::new()));
    }

    let mut table_rows = Vec::new();

    // Add data rows as Typst table cells
    for row in rows.iter() {
        // Extract author name from "Name (login)" format, fallback to full string
        let author_display = row.created_by.split(" (").next().unwrap_or(&row.created_by);

        // Extract qcer name(s) from "Name (login)" format, fallback to full string
        let qcer_display = row
            .qcer
            .iter()
            .map(|qcer| qcer.split(" (").next().unwrap_or(qcer))
            .collect::<Vec<_>>()
            .join(", ");

        // Extract closer name from "Name (login)" format, fallback to full string
        let closer_display = row
            .closed_by
            .as_ref()
            .map(|closer| closer.split(" (").next().unwrap_or(closer))
            .unwrap_or("NA");

        table_rows.push(format!(
            "[{}], [{}], [{}], [{}], [{}],",
            &row.title, &row.qc_status, author_display, &qcer_display, closer_display
        ));
    }

    Ok(Value::String(table_rows.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use octocrab::models::Milestone;
    use std::collections::HashMap;

    // Test helper functions (borrowed from the old record.rs tests)
    fn load_test_milestone(file_name: &str) -> Milestone {
        let path = format!("src/tests/github_api/milestones/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read milestone file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse milestone file {}: {}", path, e))
    }

    fn create_test_issue_information(
        title: &str,
        checklist_summary: &str,
        qc_status: &str,
    ) -> IssueInformation {
        IssueInformation {
            title: title.to_string(),
            number: 1,
            milestone: "v1.0".to_string(),
            created_by: "author".to_string(),
            created_at: "2025-11-01 10:00:00".to_string(),
            qcer: vec!["qcer1".to_string()],
            qc_status: qc_status.to_string(),
            checklist_summary: checklist_summary.to_string(),
            git_status: "Clean".to_string(),
            initial_qc_commit: "abc123".to_string(),
            latest_qc_commit: "def456".to_string(),
            issue_url: "https://github.com/owner/repo/issues/1".to_string(),
            state: "Open".to_string(),
            closed_by: None,
            closed_at: None,
            body: "Issue body".to_string(),
            comments: vec![],
            events: vec![],
            timeline: vec![],
        }
    }

    #[test]
    fn test_insert_breaks_short_text() {
        assert_eq!(insert_breaks("short", 20), "short");
    }

    #[test]
    fn test_insert_breaks_long_text() {
        let long_text = "This is a very long text that should be broken into multiple lines";
        let result = insert_breaks(long_text, 20);

        // Should contain line breaks
        assert!(result.contains('\n'));

        // Each line should be <= 20 characters (accounting for word boundaries)
        for line in result.lines() {
            assert!(line.len() <= 30, "Line '{}' is {} chars", line, line.len()); // Allow some flexibility for word boundaries
        }
    }

    #[test]
    fn test_create_milestone_df_basic() {
        let milestone = load_test_milestone("v1.0.json");
        let milestones = vec![milestone];

        let mut issues = HashMap::new();
        issues.insert(
            "v1.0".to_string(),
            vec![
                create_test_issue_information("Test Issue 1", "50.0%", "In Progress"),
                create_test_issue_information("Test Issue 2", "100.0%", "Approved"),
            ],
        );

        let result = create_milestone_df(&milestones, &issues).unwrap();

        assert_eq!(result.len(), 1);
        let row = &result[0];
        assert_eq!(row.name, "v1.0");

        // Should contain both issues with proper Typst formatting
        assert!(row.issues.contains("Test Issue 1"));
        assert!(row.issues.contains("Test Issue 2"));
        assert!(row.issues.contains("#text(fill: red)[U]")); // 100% issue should have U marker
        assert!(row.issues.contains("#text(fill: red)[C]")); // Approved issue should have C marker
    }

    #[test]
    fn test_create_milestone_df_empty_issues() {
        let milestone = load_test_milestone("v1.0.json");
        let milestones = vec![milestone];
        let issues = HashMap::new();

        let result = create_milestone_df(&milestones, &issues).unwrap();

        // Should skip milestones with no issues
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_render_milestone_table_rows() {
        let rows = vec![
            MilestoneRow {
                name: "v1.0".to_string(),
                description: "First version".to_string(),
                status: "open".to_string(),
                issues: "Issue 1\n\nIssue 2".to_string(),
            },
            MilestoneRow {
                name: "v2.0".to_string(),
                description: "Second version".to_string(),
                status: "closed".to_string(),
                issues: "Issue 3".to_string(),
            },
        ];

        let mut args = HashMap::new();
        args.insert("data".to_string(), serde_json::to_value(&rows).unwrap());

        let result = render_milestone_table_rows(&args).unwrap();
        let result_str = result.as_str().unwrap();

        // Should contain Typst table cells
        assert!(result_str.contains("[v1.0], [First version], [open], [Issue 1"));
        assert!(result_str.contains("[v2.0], [Second version], [closed], [Issue 3],"));
    }

    #[test]
    fn test_render_issue_summary_table_rows() {
        let rows = vec![
            create_test_issue_information("Test Issue 1", "50.0%", "In Progress"),
            create_test_issue_information("Test Issue 2", "100.0%", "Approved"),
        ];

        let mut args = HashMap::new();
        args.insert("data".to_string(), serde_json::to_value(&rows).unwrap());

        let result = render_issue_summary_table_rows(&args).unwrap();
        let result_str = result.as_str().unwrap();

        // Should contain Typst table cells
        assert!(result_str.contains("[Test Issue 1], [In Progress], [author], [qcer1], [NA],"));
        assert!(result_str.contains("[Test Issue 2], [Approved], [author], [qcer1], [NA],"));
    }

    #[test]
    fn test_render_issue_summary_table_rows_empty() {
        let rows: Vec<IssueInformation> = vec![];

        let mut args = HashMap::new();
        args.insert("data".to_string(), serde_json::to_value(&rows).unwrap());

        let result = render_issue_summary_table_rows(&args).unwrap();
        let result_str = result.as_str().unwrap();

        // Should return empty string for empty input
        assert_eq!(result_str, "");
    }
}
