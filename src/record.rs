use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader},
    path::{Path, absolute},
    process::{Command, Stdio},
    thread,
};

use chrono;
use lazy_static::lazy_static;
use octocrab::models::{Milestone, issues::Issue};
use serde::{Deserialize, Serialize};
use tera::{Context, Result as TeraResult, Tera, Value};

use crate::{
    ChecklistSummary, Configuration, DiskCache, GitHubReader, GitRepository, RepoUser,
    get_issue_comments, get_issue_events, get_repo_users,
    git::{GitComment, GitCommitAnalysis, GitFileOps, GitStatus, GitStatusOps},
    issue::IssueThread,
    qc_status::{QCStatus, analyze_issue_checklists},
    utils::EnvProvider,
};

lazy_static! {
    pub static ref TEMPLATES: Tera = {
        let mut tera = Tera::default();

        tera.add_raw_template("record.qmd", include_str!("templates/record.qmd"))
            .unwrap();

        // Register custom functions
        tera.register_function("render_milestone_table_rows", render_milestone_table_rows);
        tera.register_function("render_issue_summary_table_rows", render_issue_summary_table_rows);

        tera
    };
}

pub async fn record<'a>(
    milestones: &'a [Milestone],
    configuration: &Configuration,
    git_info: &(impl GitHubReader + GitRepository + GitFileOps + GitCommitAnalysis + GitStatusOps),
    env: impl EnvProvider,
    cache: Option<&DiskCache>,
) -> Result<(Vec<&'a str>, String), RecordError> {
    let mut context = Context::new();

    context.insert("repository_name", git_info.repo());
    context.insert(
        "checklist_name",
        &configuration.options.checklist_display_name,
    );

    if let Ok(author) = env.var("USER") {
        context.insert("author", &author);
    }

    let date = if let Ok(custom_date) = env.var("GHQC_RECORD_DATE") {
        custom_date
    } else {
        chrono::Local::now().format("%B %d, %Y").to_string()
    };
    context.insert("date", &date);

    let logo_path = absolute(configuration.logo_path())?;
    if logo_path.exists() {
        context.insert("logo_path", &logo_path);
    }

    // Fetch all milestone issues
    let issue_objects = fetch_milestone_issues(milestones, git_info).await?;

    // Generate milestone dataframe
    let milestone_data = create_milestone_df(milestones, &issue_objects)?;
    context.insert("milestone_data", &milestone_data);

    // Generate milestone sections for individual milestone tables
    let milestone_sections =
        create_milestone_sections(milestones, &issue_objects, cache, git_info).await?;
    context.insert("milestone_sections", &milestone_sections);

    let milestone_names = milestones
        .iter()
        .filter(|m| issue_objects.contains_key(&m.title))
        .map(|m| m.title.as_str())
        .collect::<Vec<_>>();
    context.insert("milestone_names", &milestone_names.join(", "));

    let rendered = TEMPLATES
        .render("record.qmd", &context)
        .map_err(RecordError::Template)?;

    Ok((milestone_names, rendered))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneRow {
    pub name: String,
    pub description: String,
    pub status: String,
    pub issues: String,
}

/// Fetch all issues for milestones and return as HashMap
pub async fn fetch_milestone_issues(
    milestones: &[Milestone],
    git_info: &(impl GitHubReader + GitRepository),
) -> Result<HashMap<String, Vec<Issue>>, RecordError> {
    let mut issue_map = HashMap::new();

    for milestone in milestones {
        let issues = git_info
            .get_milestone_issues(milestone)
            .await
            .map_err(RecordError::GitHubApi)?;
        if issues.is_empty() {
            log::warn!(
                "Milestone '{}' has no ghqc issues, omitting from record",
                milestone.title
            );
        } else {
            issue_map.insert(milestone.title.clone(), issues);
        }
    }

    Ok(issue_map)
}

/// Create milestone dataframe equivalent to R function
pub fn create_milestone_df(
    milestone_objects: &[Milestone],
    issue_objects: &HashMap<String, Vec<Issue>>,
) -> Result<Vec<MilestoneRow>, RecordError> {
    let mut milestone_rows = Vec::new();

    for milestone in milestone_objects {
        let Some(issues) = issue_objects.get(&milestone.title) else {
            continue;
        };

        let mut issues_with_unapproved_statuses = Vec::new();
        let mut issues_with_open_checklists = Vec::new();

        // Analyze each issue for checklist status
        for issue in issues {
            let checklists = analyze_issue_checklists(issue);

            // Check if any checklist has uncompleted items
            let has_open_checklists = checklists.iter().any(|(_, summary)| !summary.is_complete());

            if has_open_checklists {
                issues_with_open_checklists.push(issue.title.clone());
            }

            // For now, we'll mark all open issues as "unapproved"
            // In a full implementation, you'd check the QC status
            if matches!(issue.state, octocrab::models::IssueState::Open) {
                issues_with_unapproved_statuses.push(issue.title.clone());
            }
        }

        // Format issues string with status indicators
        let issues_str = if issues.is_empty() {
            String::new()
        } else {
            format_issues_for_milestone(
                &issues,
                &issues_with_unapproved_statuses,
                &issues_with_open_checklists,
            )
        };

        // Format milestone status
        let status = milestone
            .state
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("Unknown")
            .to_string();

        // Format description with line breaks
        let description = milestone
            .description
            .as_ref()
            .map(|d| insert_breaks(d, 20))
            .unwrap_or_else(|| "NA".to_string());

        // Format milestone name with line breaks
        let name = insert_breaks(&milestone.title, 18);

        milestone_rows.push(MilestoneRow {
            name,
            description,
            status,
            issues: issues_str,
        });
    }

    Ok(milestone_rows)
}

/// Format issues for milestone with status indicators
fn format_issues_for_milestone(
    issues: &[Issue],
    issues_with_unapproved_statuses: &[String],
    issues_with_open_checklists: &[String],
) -> String {
    if issues.is_empty() {
        return String::new();
    }

    let issue_names: Vec<String> = issues
        .iter()
        .map(|issue| {
            let mut issue_name = insert_breaks(&issue.title, 42);

            if issues_with_unapproved_statuses.contains(&issue.title) {
                issue_name = format!("{}\\textcolor{{red}}{{U}}", issue_name);
            }

            if issues_with_open_checklists.contains(&issue.title) {
                issue_name = format!("{}\\textcolor{{red}}{{C}}", issue_name);
            }

            // Escape underscores for LaTeX
            issue_name.replace('_', "\\_")
        })
        .collect();

    // Join with double newlines and add proper LaTeX line breaks
    issue_names.join("\\newline \\newline ")
}

/// Insert line breaks at word boundaries (equivalent to R's insert_breaks)
fn insert_breaks(text: &str, max_width: usize) -> String {
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

/// Tera function to render milestone table rows only
fn render_milestone_table_rows(args: &HashMap<String, Value>) -> TeraResult<Value> {
    let data = args
        .get("data")
        .ok_or_else(|| tera::Error::msg("Missing 'data' argument for milestone table"))?;

    let rows: Vec<MilestoneRow> = serde_json::from_value(data.clone())
        .map_err(|e| tera::Error::msg(format!("Failed to parse milestone data: {}", e)))?;

    let mut table_rows = Vec::new();

    // Add data rows only
    for (i, row) in rows.iter().enumerate() {
        if i < rows.len() - 1 {
            table_rows.push(format!(
                r"{} & {} & {} & {}\\",
                row.name, row.description, row.status, row.issues
            ));
            table_rows.push(r"\addlinespace\addlinespace".to_string());
        } else {
            table_rows.push(format!(
                r"{} & {} & {} & {}\\*",
                row.name, row.description, row.status, row.issues
            ));
        }
    }

    Ok(Value::String(table_rows.join("\n")))
}

/// Tera function to render issue summary table rows only
fn render_issue_summary_table_rows(args: &HashMap<String, Value>) -> TeraResult<Value> {
    let data = args
        .get("data")
        .ok_or_else(|| tera::Error::msg("Missing 'data' argument for issue summary table"))?;

    let rows: Vec<IssueInformation> = serde_json::from_value(data.clone())
        .map_err(|e| tera::Error::msg(format!("Failed to parse issue summary data: {}", e)))?;

    if rows.is_empty() {
        return Ok(Value::String(String::new()));
    }

    let mut table_rows = Vec::new();

    // Add data rows only
    for (i, row) in rows.iter().enumerate() {
        // Extract author name from "Name (login)" format, fallback to full string
        let author_display = row.created_by.split(" (").next().unwrap_or(&row.created_by);

        // Extract qcer name(s) from "Name (login)" format, fallback to full string
        let qcer_display = row
            .qcer
            .iter()
            .map(|qcer| qcer.split(" (").next().unwrap_or(qcer))
            .collect::<Vec<_>>()
            .join(",\\newline ");

        // Extract closer name from "Name (login)" format, fallback to full string
        let closer_display = row
            .closed_by
            .as_ref()
            .map(|closer| closer.split(" (").next().unwrap_or(closer))
            .unwrap_or("NA");

        if i < rows.len() - 1 {
            table_rows.push(format!(
                r"{} & {} & {} & {} & {}\\",
                row.title.replace('_', "\\_"),
                row.qc_status,
                author_display,
                qcer_display,
                closer_display
            ));
            table_rows.push(r"\addlinespace\addlinespace".to_string());
        } else {
            table_rows.push(format!(
                r"{} & {} & {} & {} & {}\\*",
                row.title.replace('_', "\\_"),
                row.qc_status,
                author_display,
                qcer_display,
                closer_display
            ));
        }
    }

    Ok(Value::String(table_rows.join("\n")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueInformation {
    pub title: String,
    pub number: u64,
    pub milestone: String,
    pub created_by: String,
    pub created_at: String,
    pub qcer: Vec<String>,
    pub qc_status: String,
    pub checklist_summary: String,
    pub git_status: String,
    pub initial_qc_commit: String,
    pub latest_qc_commit: String,
    pub issue_url: String,
    pub state: String,
    pub closed_by: Option<String>,
    pub closed_at: Option<String>,
    pub body: String,
    pub comments: Vec<(String, String)>,
    pub events: Vec<String>,
    pub timeline: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneSection {
    pub name: String,
    pub issues: Vec<IssueInformation>,
}

/// Create issue summary data for each milestone
pub async fn create_milestone_sections(
    milestone_objects: &[Milestone],
    issue_objects: &HashMap<String, Vec<Issue>>,
    cache: Option<&crate::cache::DiskCache>,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis + GitStatusOps),
) -> Result<Vec<MilestoneSection>, RecordError> {
    // Get all repository users with cache for efficient lookup
    let repo_users = get_repo_users(cache, git_info).await?;

    // Get git status once and reuse it across all issues
    let git_status = git_info.status()?;

    let mut sections = Vec::new();

    for milestone in milestone_objects {
        let Some(issues) = issue_objects.get(&milestone.title) else {
            continue;
        };

        // Create detailed issue information for each issue (used for both summary and details)
        let issue_futures: Vec<_> = issues
            .iter()
            .map(|issue| {
                let repo_users_clone = repo_users.clone();
                let git_status_clone = git_status.clone();
                async move {
                    create_issue_information(
                        issue,
                        &milestone.title,
                        &repo_users_clone,
                        &git_status_clone,
                        cache,
                        git_info,
                    )
                    .await
                }
            })
            .collect();

        let issue_results = futures::future::join_all(issue_futures).await;
        let issue_information = issue_results.into_iter().collect::<Result<Vec<_>, _>>()?;

        sections.push(MilestoneSection {
            name: milestone.title.clone(),
            issues: issue_information,
        });
    }

    Ok(sections)
}

/// Create detailed issue information from an issue
async fn create_issue_information(
    issue: &Issue,
    milestone_name: &str,
    repo_users: &[RepoUser],
    git_status: &GitStatus,
    cache: Option<&DiskCache>,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
) -> Result<IssueInformation, RecordError> {
    // Get comments and create issue thread
    let comments = get_issue_comments(issue, cache, git_info).await?;
    let issue_thread = IssueThread::from_issue_comments(issue, &comments, git_info).await?;
    let open = matches!(issue.state, octocrab::models::IssueState::Closed);

    // QC Status
    let qc_status = QCStatus::determine_status(&issue_thread)?.to_string();

    // Checklist Summary
    let checklist_summaries = analyze_issue_checklists(issue);
    let checklist_summary =
        ChecklistSummary::sum(checklist_summaries.iter().map(|c| &c.1)).to_string();

    // Git Status for this specific file
    let file_commits = issue_thread.file_commits();
    let git_status_str = git_status.format_for_file(&issue_thread.file, &file_commits);

    // Created by (with name lookup)
    let created_by = repo_users
        .iter()
        .find(|user| user.login == issue.user.login)
        .and_then(|user| user.name.as_ref())
        .map(|name| format!("{} ({})", name, issue.user.login))
        .unwrap_or_else(|| issue.user.login.clone());

    // QCers (with name lookup)
    let qcer = if issue.assignees.is_empty() {
        vec!["NA".to_string()]
    } else {
        issue
            .assignees
            .iter()
            .map(|assignee| {
                repo_users
                    .iter()
                    .find(|user| user.login == assignee.login)
                    .and_then(|user| user.name.as_ref())
                    .map(|name| format!("{} ({})", name, assignee.login))
                    .unwrap_or_else(|| assignee.login.clone())
            })
            .collect()
    };

    // Get issue events (used for both closer detection and event timeline)
    let events = get_issue_events(issue, cache, git_info).await?;

    // Issue closer (with name lookup)
    let closed_by = if !open {
        match get_issue_closer_username(&events) {
            Some(closer_login) => {
                let closer_display = repo_users
                    .iter()
                    .find(|user| user.login == closer_login)
                    .and_then(|user| user.name.as_ref())
                    .map(|name| format!("{} ({})", name, closer_login))
                    .unwrap_or(closer_login);
                Some(closer_display)
            }
            None => None,
        }
    } else {
        None
    };

    // Format dates
    let created_at = issue.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
    let closed_at = issue
        .closed_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());

    // Commit information
    let initial_qc_commit = issue_thread
        .initial_commit()
        .map(|c| format!("{}", c))
        .unwrap_or_else(|| "No initial commit".to_string());
    let latest_qc_commit = issue_thread
        .latest_commit()
        .map(|c| format!("{}", c))
        .unwrap_or_else(|| "No commits".to_string());

    // Process issue body with header translation (min level 4 since under ### Issue Body)
    let body = issue
        .body
        .as_ref()
        .map(|b| format_markdown_with_min_level(b, 4))
        .unwrap_or_else(|| "No description provided.".to_string());

    // Format comments as header-body pairs
    let formatted_comments = format_comments(&comments, repo_users);

    // Format events timeline
    let formatted_events = format_events(&events, repo_users);

    // Create combined timeline from formatted events and comment headers
    let timeline = create_combined_timeline(&formatted_events, &formatted_comments);

    Ok(IssueInformation {
        title: issue.title.clone(),
        number: issue.number,
        milestone: milestone_name.to_string(),
        created_by,
        created_at,
        qcer,
        qc_status,
        checklist_summary,
        git_status: git_status_str,
        initial_qc_commit,
        latest_qc_commit,
        issue_url: issue.html_url.to_string(),
        state: if open { "Open" } else { "Closed" }.to_string(),
        closed_by,
        closed_at,
        body,
        comments: formatted_comments,
        events: formatted_events,
        timeline,
    })
}

/// Create combined timeline from formatted events and comment headers, sorted chronologically
fn create_combined_timeline(
    formatted_events: &[String],
    formatted_comments: &[(String, String)],
) -> Vec<String> {
    let mut timeline_items = Vec::new();

    // Add formatted events (they already have timestamp and description)
    timeline_items.extend(formatted_events.iter().cloned());

    // Add comment headers (the .0 elements which have timestamp and author)
    // Lowercase "Comment" to "comment" for timeline consistency
    timeline_items.extend(
        formatted_comments
            .iter()
            .map(|(header, _)| header.replace(" - Comment by ", " - commented by ")),
    );

    // Sort by the timestamp at the beginning of each string (YYYY-MM-DD HH:MM:SS format)
    timeline_items.sort_by(|a, b| {
        // Extract timestamp from the beginning of each string
        let timestamp_a = a.split(" - ").next().unwrap_or("");
        let timestamp_b = b.split(" - ").next().unwrap_or("");
        timestamp_a.cmp(timestamp_b)
    });

    timeline_items
}

/// Extract the username of who closed the issue from pre-fetched events
fn get_issue_closer_username(events: &[serde_json::Value]) -> Option<String> {
    // Find the last "closed" event
    events
        .iter()
        .rev() // Start from the most recent
        .find(|event| {
            event
                .get("event")
                .and_then(|e| e.as_str())
                .map(|s| s == "closed")
                .unwrap_or(false)
        })
        .and_then(|event| {
            event
                .get("actor")
                .and_then(|actor| actor.get("login"))
                .and_then(|login| login.as_str())
                .map(|s| s.to_string())
        })
}

/// Format events timeline as bullet points
fn format_events(events: &[serde_json::Value], repo_users: &[RepoUser]) -> Vec<String> {
    let mut formatted_events = Vec::new();

    for event in events {
        let event_type = event.get("event").and_then(|e| e.as_str()).unwrap_or("");

        let created_at = event
            .get("created_at")
            .and_then(|dt| dt.as_str())
            .and_then(|dt_str| chrono::DateTime::parse_from_rfc3339(dt_str).ok())
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown time".to_string());

        let actor_login = event
            .get("actor")
            .and_then(|actor| actor.get("login"))
            .and_then(|login| login.as_str())
            .unwrap_or("Unknown user");

        // Look up display name for actor
        let actor_display = repo_users
            .iter()
            .find(|user| user.login == actor_login)
            .and_then(|user| user.name.as_ref())
            .map(|name| format!("{} ({})", name, actor_login))
            .unwrap_or_else(|| actor_login.to_string());

        let formatted_event = match event_type {
            "milestoned" => {
                let milestone_title = event
                    .get("milestone")
                    .and_then(|m| m.get("title"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("Unknown milestone");
                format!(
                    "{} - milestone set to '{}' by {}",
                    created_at, milestone_title, actor_display
                )
            }
            "assigned" => {
                let assignee_login = event
                    .get("assignee")
                    .and_then(|a| a.get("login"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("Unknown user");

                let assignee_display = repo_users
                    .iter()
                    .find(|user| user.login == assignee_login)
                    .and_then(|user| user.name.as_ref())
                    .map(|name| format!("{} ({})", name, assignee_login))
                    .unwrap_or_else(|| assignee_login.to_string());

                format!(
                    "{} - {} assigned by {}",
                    created_at, assignee_display, actor_display
                )
            }
            "labeled" => {
                let label_name = event
                    .get("label")
                    .and_then(|l| l.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("Unknown label");
                format!(
                    "{} - added label '{}' by {}",
                    created_at, label_name, actor_display
                )
            }
            "closed" => {
                format!("{} - closed by {}", created_at, actor_display)
            }
            "reopened" => {
                format!("{} - reopened by {}", created_at, actor_display)
            }
            _ => continue, // Skip other event types
        };

        formatted_events.push(formatted_event);
    }

    formatted_events
}

/// Format comments as header-body pairs
fn format_comments(comments: &[GitComment], repo_users: &[RepoUser]) -> Vec<(String, String)> {
    let mut formatted_comments = Vec::new();

    for comment in comments {
        // Look up display name
        let author_display = repo_users
            .iter()
            .find(|user| user.login == comment.author_login)
            .and_then(|user| user.name.as_ref())
            .map(|name| format!("{} ({})", name, comment.author_login))
            .unwrap_or_else(|| comment.author_login.clone());

        // Format timestamp and header
        let created_at = comment.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let header = format!("{} - Comment by {}", created_at, author_display);

        // Format comment body (min level 4 since it will be under #### header in template)
        let body = format_markdown_with_min_level(&comment.body, 4);

        formatted_comments.push((header, body));
    }

    formatted_comments
}

/// Translate markdown headers to ensure minimum level and wrap long diff lines
fn format_markdown_with_min_level(markdown: &str, min_level: usize) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut result = Vec::new();
    let mut in_diff_block = false;

    for line in lines {
        let trimmed = line.trim_start();

        // Track if we're in a diff code block
        if trimmed.starts_with("```") {
            in_diff_block = trimmed.contains("diff");
            result.push(line.to_string());
            continue;
        }

        if trimmed.starts_with('#') {
            // Count existing header levels
            let header_level = trimmed.chars().take_while(|&c| c == '#').count();
            if header_level <= 6 {
                // Ensure header is at least at min_level
                let new_level = std::cmp::min(std::cmp::max(header_level, min_level), 6);
                let new_header = "#".repeat(new_level);
                let header_text = trimmed.trim_start_matches('#').trim_start();
                result.push(format!("{} {}", new_header, header_text));
            } else {
                // Keep as-is if already max level
                result.push(line.to_string());
            }
        } else if in_diff_block && (line.starts_with('+') || line.starts_with('-')) {
            // Handle diff line wrapping for long lines
            result.extend(wrap_diff_line(line, 80));
        } else {
            result.push(line.to_string());
        }
    }

    result
        .join("\n")
        .replace("---", "`---`")
        .replace("```diff", "``` diff")
}

/// Wrap a diff line if it's too long, preserving the diff marker
fn wrap_diff_line(line: &str, max_width: usize) -> Vec<String> {
    if line.len() <= max_width {
        return vec![line.to_string()];
    }

    let mut wrapped_lines = Vec::new();
    let diff_marker = &line[0..1]; // Get the + or - marker
    let content = &line[1..]; // Get the content without the marker

    // Find good break points (spaces, after certain characters)
    let mut current_pos = 0;
    let available_width = max_width - 1; // Account for diff marker

    while current_pos < content.len() {
        let remaining = &content[current_pos..];

        if remaining.len() <= available_width {
            // Rest of line fits
            if current_pos == 0 {
                wrapped_lines.push(line.to_string());
            } else {
                wrapped_lines.push(format!("{}      {}", diff_marker, remaining));
            }
            break;
        }

        // Find a good break point within the available width
        let mut break_point = available_width;
        let search_slice = &remaining[..available_width.min(remaining.len())];

        // Look for space, comma, semicolon, or other good break characters
        if let Some(pos) = search_slice.rfind(' ') {
            break_point = pos + 1; // Include the space
        } else if let Some(pos) = search_slice.rfind(',') {
            break_point = pos + 1;
        } else if let Some(pos) = search_slice.rfind(';') {
            break_point = pos + 1;
        } else if let Some(pos) = search_slice.rfind('(') {
            break_point = pos + 1;
        } else if let Some(pos) = search_slice.rfind('{') {
            break_point = pos + 1;
        }

        // Extract the line segment
        let segment = &remaining[..break_point];

        if current_pos == 0 {
            // First line keeps original format
            wrapped_lines.push(format!("{}{}", diff_marker, segment));
        } else {
            // Continuation lines get indented with a tab
            wrapped_lines.push(format!("{}      {}", diff_marker, segment.trim_start()));
        }

        current_pos += break_point;
    }

    wrapped_lines
}

/// Render a Quarto document to PDF using the quarto CLI tool
///
/// # Arguments
/// * `report` - The Quarto markdown content to render
/// * `path` - The output path for the rendered PDF (without extension)
///
/// # Returns
/// * `Ok(())` - If rendering succeeded
/// * `Err(RecordError)` - If rendering failed
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use ghqctoolkit::render;
///
/// let report = "---\ntitle: My Report\n---\n# Hello World";
/// render(report, Path::new("output/my-report")).unwrap();
/// // Creates output/my-report.pdf
/// ```
pub fn render(record_str: &str, path: impl AsRef<Path>) -> Result<(), RecordError> {
    let path = path.as_ref();

    // Create staging directory using hash of report content
    let mut hasher = DefaultHasher::new();
    record_str.hash(&mut hasher);
    let hash = hasher.finish();
    let staging_dir = std::env::temp_dir().join(format!("ghqc-render-{:x}", hash));
    std::fs::create_dir_all(&staging_dir)?;

    let cleanup_staging = || {
        if let Err(e) = std::fs::remove_dir_all(&staging_dir) {
            log::warn!(
                "Failed to cleanup staging directory {}: {}",
                staging_dir.display(),
                e
            );
        }
    };

    let result = render_in_staging(&staging_dir, record_str, &path);

    // Always cleanup staging directory
    cleanup_staging();

    result
}

fn render_in_staging(
    staging_dir: &Path,
    report: &str,
    final_pdf_path: &Path,
) -> Result<(), RecordError> {
    let qmd_file = staging_dir.join("record.qmd");
    let staging_pdf_path = staging_dir.join("record.pdf");

    log::debug!("Writing Quarto document to staging: {}", qmd_file.display());
    std::fs::write(&qmd_file, report)?;

    log::debug!(
        "Rendering PDF with Quarto: {} -> {}",
        qmd_file.display(),
        final_pdf_path.display()
    );

    // Execute quarto render command with combined stdout/stderr
    let mut cmd = Command::new("quarto");
    cmd.args(&["render", qmd_file.to_str().unwrap()])
        .current_dir(staging_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()); // Capture stderr separately

    log::debug!("Executing command: {:?}", cmd);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            RecordError::QuartoNotFound
        } else {
            RecordError::Io(e)
        }
    })?;

    // Collect both stdout and stderr
    let stdout = child.stdout.take().expect("Failed to get stdout");
    let stderr = child.stderr.take().expect("Failed to get stderr");

    let stdout_handle = thread::spawn(move || {
        let mut lines = Vec::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                lines.push(line);
            }
        }
        lines
    });

    let stderr_handle = thread::spawn(move || {
        let mut lines = Vec::new();
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                lines.push(line);
            }
        }
        lines
    });

    // Wait for process to complete
    let exit_status = child.wait()?;

    // Get the collected output from both streams
    let stdout_lines = stdout_handle
        .join()
        .unwrap_or_else(|_| vec!["Failed to collect stdout".to_string()]);
    let stderr_lines = stderr_handle
        .join()
        .unwrap_or_else(|_| vec!["Failed to collect stderr".to_string()]);

    let mut combined_lines = Vec::new();
    combined_lines.extend(stdout_lines);
    combined_lines.extend(stderr_lines);
    let combined_output = combined_lines.join("\n");

    // Check if command succeeded
    if !exit_status.success() {
        let exit_code = exit_status.code().unwrap_or(-1);
        return Err(RecordError::QuartoRenderFailed {
            code: exit_code,
            stderr: combined_output,
        });
    }

    // Verify PDF was created in staging
    if !staging_pdf_path.exists() {
        return Err(RecordError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("PDF not created in staging: {}", staging_pdf_path.display()),
        )));
    }

    // Ensure output directory exists
    if let Some(parent) = final_pdf_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Copy PDF from staging to final location
    log::debug!("Copying PDF from staging to: {}", final_pdf_path.display());
    std::fs::copy(&staging_pdf_path, &final_pdf_path)?;

    log::debug!("Successfully rendered PDF: {}", final_pdf_path.display());

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("IO Error: {0}")]
    Io(#[from] io::Error),
    #[error("Template Error: {0}")]
    Template(#[from] tera::Error),
    #[error("GitHub API Error: {0}")]
    GitHubApi(#[from] crate::git::GitHubApiError),
    #[error("Issue Error: {0}")]
    Issue(#[from] crate::issue::IssueError),
    #[error("QC Status Error: {0}")]
    QCStatus(#[from] crate::qc_status::QCStatusError),
    #[error("Git Status Error: {0}")]
    GitStatus(#[from] crate::git::GitStatusError),
    #[error("Quarto render failed with exit code {code}: {stderr}")]
    QuartoRenderFailed { code: i32, stderr: String },
    #[error(
        "Quarto command not found. Please install Quarto: https://quarto.org/docs/get-started/"
    )]
    QuartoNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GitAuthor, RepoUser,
        git::{
            GitComment, GitCommit, GitCommitAnalysis, GitCommitAnalysisError, GitFileOps,
            GitFileOpsError, GitHelpers, GitHubApiError, GitRepository, GitRepositoryError,
            GitStatus, GitStatusError, GitStatusOps,
        },
    };
    use gix::ObjectId;
    use octocrab::models::{Milestone, issues::Issue};
    use std::{collections::HashMap, path::PathBuf, str::FromStr};

    /// Mock implementation for record testing
    pub struct RecordMockGitInfo {
        pub milestones: Vec<Milestone>,
        pub milestone_issues: HashMap<String, Vec<Issue>>,
        pub issue_comments: HashMap<u64, Vec<GitComment>>,
        pub issue_events: HashMap<u64, Vec<serde_json::Value>>,
        pub repo_users: Vec<RepoUser>,
        pub git_status: GitStatus,
        pub owner: String,
        pub repo: String,
        pub current_branch: String,
        pub current_commit: String,
        pub file_commits: HashMap<PathBuf, Vec<(ObjectId, String)>>,
    }

    impl RecordMockGitInfo {
        pub fn new() -> Self {
            Self {
                milestones: Vec::new(),
                milestone_issues: HashMap::new(),
                issue_comments: HashMap::new(),
                issue_events: HashMap::new(),
                repo_users: Vec::new(),
                git_status: GitStatus::Clean,
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                current_branch: "main".to_string(),
                current_commit: "abc123def456789012345678901234567890abcd".to_string(),
                file_commits: HashMap::new(),
            }
        }

        pub fn with_milestones(mut self, milestones: Vec<Milestone>) -> Self {
            self.milestones = milestones;
            self
        }

        pub fn with_milestone_issues(mut self, issues: HashMap<String, Vec<Issue>>) -> Self {
            self.milestone_issues = issues;
            self
        }

        pub fn with_issue_events(mut self, events: HashMap<u64, Vec<serde_json::Value>>) -> Self {
            self.issue_events = events;
            self
        }

        pub fn with_issue_comments(mut self, comments: HashMap<u64, Vec<GitComment>>) -> Self {
            self.issue_comments = comments;
            self
        }

        pub fn with_repo_users(mut self, users: Vec<RepoUser>) -> Self {
            self.repo_users = users;
            self
        }

        pub fn with_git_status(mut self, status: GitStatus) -> Self {
            self.git_status = status;
            self
        }

        pub fn with_file_commits(
            mut self,
            file: PathBuf,
            commits: Vec<(ObjectId, String)>,
        ) -> Self {
            self.file_commits.insert(file, commits);
            self
        }
    }

    impl GitHubReader for RecordMockGitInfo {
        async fn get_milestones(&self) -> Result<Vec<Milestone>, GitHubApiError> {
            Ok(self.milestones.clone())
        }

        async fn get_milestone_issues(
            &self,
            milestone: &Milestone,
        ) -> Result<Vec<Issue>, GitHubApiError> {
            Ok(self
                .milestone_issues
                .get(&milestone.title)
                .cloned()
                .unwrap_or_default())
        }

        async fn get_assignees(&self) -> Result<Vec<String>, GitHubApiError> {
            Ok(self.repo_users.iter().map(|u| u.login.clone()).collect())
        }

        async fn get_user_details(&self, username: &str) -> Result<RepoUser, GitHubApiError> {
            Ok(self
                .repo_users
                .iter()
                .find(|u| u.login == username)
                .cloned()
                .unwrap_or_else(|| RepoUser {
                    login: username.to_string(),
                    name: None,
                }))
        }

        async fn get_labels(&self) -> Result<Vec<String>, GitHubApiError> {
            Ok(vec!["ghqc".to_string(), "urgent".to_string()])
        }

        async fn get_issue_comments(
            &self,
            issue: &Issue,
        ) -> Result<Vec<GitComment>, GitHubApiError> {
            Ok(self
                .issue_comments
                .get(&issue.number)
                .cloned()
                .unwrap_or_default())
        }

        async fn get_issue_events(
            &self,
            issue: &Issue,
        ) -> Result<Vec<serde_json::Value>, GitHubApiError> {
            Ok(self
                .issue_events
                .get(&issue.number)
                .cloned()
                .unwrap_or_default())
        }
    }

    impl GitRepository for RecordMockGitInfo {
        fn commit(&self) -> Result<String, GitRepositoryError> {
            Ok(self.current_commit.clone())
        }

        fn branch(&self) -> Result<String, GitRepositoryError> {
            Ok(self.current_branch.clone())
        }

        fn owner(&self) -> &str {
            &self.owner
        }

        fn repo(&self) -> &str {
            &self.repo
        }
    }

    impl GitStatusOps for RecordMockGitInfo {
        fn status(&self) -> Result<GitStatus, GitStatusError> {
            Ok(self.git_status.clone())
        }
    }

    impl GitFileOps for RecordMockGitInfo {
        fn commits(&self, _branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
            // Convert all file_commits to a unified commit list
            let mut all_commits = Vec::new();
            for (file, commits) in &self.file_commits {
                for (commit, message) in commits {
                    // Check if this commit is already in the list
                    if !all_commits.iter().any(|c: &GitCommit| c.commit == *commit) {
                        all_commits.push(GitCommit {
                            commit: *commit,
                            message: message.clone(),
                            files: vec![file.clone()],
                        });
                    } else {
                        // Add this file to existing commit
                        if let Some(existing) = all_commits.iter_mut().find(|c| c.commit == *commit)
                        {
                            if !existing.files.contains(file) {
                                existing.files.push(file.clone());
                            }
                        }
                    }
                }
            }

            if all_commits.is_empty() {
                all_commits.push(GitCommit {
                    commit: ObjectId::from_str(&self.current_commit).unwrap(),
                    message: "Test commit".to_string(),
                    files: vec![PathBuf::from("test_file.rs")],
                });
            }

            Ok(all_commits)
        }

        fn authors(&self, _file: &std::path::Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(vec![GitAuthor {
                name: "Test Author".to_string(),
                email: "test@example.com".to_string(),
            }])
        }

        fn file_content_at_commit(
            &self,
            _file: &std::path::Path,
            _commit: &ObjectId,
        ) -> Result<String, GitFileOpsError> {
            Ok("test content".to_string())
        }
    }

    impl GitCommitAnalysis for RecordMockGitInfo {
        fn get_all_merge_commits(&self) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(Vec::new())
        }

        fn get_commit_parents(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(Vec::new())
        }

        fn is_ancestor(
            &self,
            _ancestor: &ObjectId,
            _descendant: &ObjectId,
        ) -> Result<bool, GitCommitAnalysisError> {
            Ok(false)
        }

        fn get_branches_containing_commit(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<String>, GitCommitAnalysisError> {
            Ok(vec![self.current_branch.clone()])
        }
    }

    impl GitHelpers for RecordMockGitInfo {
        fn file_content_url(&self, commit: &str, file: &std::path::Path) -> String {
            format!(
                "https://github.com/{}/{}/blob/{}/{}",
                self.owner,
                self.repo,
                commit,
                file.display()
            )
        }

        fn commit_comparison_url(
            &self,
            current_commit: &ObjectId,
            previous_commit: &ObjectId,
        ) -> String {
            format!(
                "https://github.com/{}/{}/compare/{}...{}",
                self.owner, self.repo, previous_commit, current_commit
            )
        }
    }

    // Test helper functions
    fn load_test_milestone(file_name: &str) -> Milestone {
        let path = format!("src/tests/github_api/milestones/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read milestone file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse milestone file {}: {}", path, e))
    }

    fn load_test_issue(file_name: &str) -> Issue {
        let path = format!("src/tests/github_api/issues/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    fn load_test_events(file_name: &str) -> Vec<serde_json::Value> {
        let path = format!("src/tests/github_api/events/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read events file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse events file {}: {}", path, e))
    }

    fn load_test_users() -> Vec<RepoUser> {
        let path = "src/tests/github_api/users/repository_users.json";
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("Failed to read users file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse users file {}: {}", path, e))
    }

    fn load_test_comments(file_name: &str) -> Vec<GitComment> {
        let path = format!("src/tests/github_api/comments/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read comments file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse comments file {}: {}", path, e))
    }

    fn create_test_configuration() -> Configuration {
        Configuration::from_path("src/tests/default_configuration")
    }

    // Mock environment provider for testing
    struct TestEnvProvider {
        vars: HashMap<String, String>,
    }

    impl crate::utils::EnvProvider for TestEnvProvider {
        fn var(&self, key: &str) -> Result<String, std::env::VarError> {
            self.vars
                .get(key)
                .cloned()
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    fn create_test_env() -> TestEnvProvider {
        TestEnvProvider {
            vars: [
                ("USER".to_string(), "testuser".to_string()),
                (
                    "GHQC_RECORD_DATE".to_string(),
                    "January 01, 2024".to_string(),
                ),
            ]
            .iter()
            .cloned()
            .collect(),
        }
    }

    #[tokio::test]
    async fn test_record_complete_v1_milestone() {
        let v1_milestone = load_test_milestone("v1.0.json");
        let milestones = vec![v1_milestone.clone()];

        let main_issue = load_test_issue("main_file_issue.json");
        let test_issue = load_test_issue("test_file_issue.json");

        let main_events = load_test_events("main_file_issue_events.json");
        let test_events = load_test_events("test_file_issue_events.json");
        let repo_users = load_test_users();

        let mut milestone_issues = HashMap::new();
        milestone_issues.insert(
            "v1.0".to_string(),
            vec![main_issue.clone(), test_issue.clone()],
        );

        let mut issue_events = HashMap::new();
        issue_events.insert(main_issue.number, main_events);
        issue_events.insert(test_issue.number, test_events);

        let git_info = RecordMockGitInfo::new()
            .with_milestones(milestones.clone())
            .with_milestone_issues(milestone_issues)
            .with_issue_events(issue_events)
            .with_repo_users(repo_users)
            .with_git_status(GitStatus::Clean)
            .with_file_commits(
                PathBuf::from("src/main.rs"),
                vec![(
                    ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                    "Initial commit".to_string(),
                )],
            )
            .with_file_commits(
                PathBuf::from("src/test.rs"),
                vec![(
                    ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                    "Test commit".to_string(),
                )],
            );

        let config = create_test_configuration();
        let env = create_test_env();

        let result = record(&milestones, &config, &git_info, env, None).await;
        assert!(result.is_ok());

        insta::assert_snapshot!("record_v1_milestone", result.unwrap().1);
    }

    #[tokio::test]
    async fn test_record_multiple_milestones_with_events() {
        let v1_milestone = load_test_milestone("v1.0.json");
        let v2_milestone = load_test_milestone("v2.0.json");
        let milestones = vec![v1_milestone.clone(), v2_milestone.clone()];

        let main_issue = load_test_issue("main_file_issue.json");
        let test_issue = load_test_issue("test_file_issue.json");
        let config_issue = load_test_issue("config_file_issue.json");

        let main_events = load_test_events("main_file_issue_events.json");
        let test_events = load_test_events("test_file_issue_events.json");
        let config_events = load_test_events("config_file_issue_events.json");

        let main_comments = load_test_comments("main_file_issue_comments.json");
        let test_comments = load_test_comments("test_file_issue_comments.json");
        let config_comments = load_test_comments("config_file_issue_comments.json");

        let repo_users = load_test_users();

        let mut milestone_issues = HashMap::new();
        milestone_issues.insert(
            "v1.0".to_string(),
            vec![main_issue.clone(), test_issue.clone()],
        );
        milestone_issues.insert("v2.0".to_string(), vec![config_issue.clone()]);

        let mut issue_events = HashMap::new();
        issue_events.insert(main_issue.number, main_events);
        issue_events.insert(test_issue.number, test_events);
        issue_events.insert(config_issue.number, config_events);

        let mut issue_comments = HashMap::new();
        issue_comments.insert(main_issue.number, main_comments);
        issue_comments.insert(test_issue.number, test_comments);
        issue_comments.insert(config_issue.number, config_comments);

        let git_info = RecordMockGitInfo::new()
            .with_milestones(milestones.clone())
            .with_milestone_issues(milestone_issues)
            .with_issue_events(issue_events)
            .with_issue_comments(issue_comments)
            .with_repo_users(repo_users)
            .with_git_status(GitStatus::Clean)
            .with_file_commits(
                PathBuf::from("src/main.rs"),
                vec![(
                    ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                    "Initial commit".to_string(),
                )],
            )
            .with_file_commits(
                PathBuf::from("src/test.rs"),
                vec![(
                    ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                    "Test commit".to_string(),
                )],
            )
            .with_file_commits(
                PathBuf::from("src/config.rs"),
                vec![(
                    ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                    "Config commit".to_string(),
                )],
            );

        let config = create_test_configuration();
        let env = create_test_env();

        let result = record(&milestones, &config, &git_info, env, None).await;
        assert!(result.is_ok());

        insta::assert_snapshot!("record_multiple_milestones", result.unwrap().1);
    }

    #[tokio::test]
    async fn test_record_closed_issue_with_events() {
        let v1_milestone = load_test_milestone("v1.0.json");
        let milestones = vec![v1_milestone.clone()];

        // Use config issue but mark it as closed and add close events
        let mut closed_issue = load_test_issue("config_file_issue.json");
        closed_issue.state = octocrab::models::IssueState::Closed;
        closed_issue.closed_at = Some(
            chrono::DateTime::parse_from_rfc3339("2011-04-23T14:30:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let closed_events = load_test_events("closed_approved_issue_events.json");
        let repo_users = load_test_users();

        let mut milestone_issues = HashMap::new();
        milestone_issues.insert("v1.0".to_string(), vec![closed_issue.clone()]);

        let mut issue_events = HashMap::new();
        issue_events.insert(closed_issue.number, closed_events);

        let git_info = RecordMockGitInfo::new()
            .with_milestones(milestones.clone())
            .with_milestone_issues(milestone_issues)
            .with_issue_events(issue_events)
            .with_repo_users(repo_users)
            .with_git_status(GitStatus::Behind(vec![
                ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
            ]))
            .with_file_commits(
                PathBuf::from("src/config.rs"),
                vec![(
                    ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                    "Config commit".to_string(),
                )],
            );

        let config = create_test_configuration();
        let env = create_test_env();

        let result = record(&milestones, &config, &git_info, env, None).await;
        assert!(result.is_ok());

        insta::assert_snapshot!("record_closed_issue", result.unwrap().1);
    }

    #[tokio::test]
    async fn test_record_reopened_issue_lifecycle() {
        let v2_milestone = load_test_milestone("v2.0.json");
        let milestones = vec![v2_milestone.clone()];

        let reopened_issue = load_test_issue("test_file_issue.json");
        let reopened_events = load_test_events("reopened_issue_events.json");
        let repo_users = load_test_users();

        let mut milestone_issues = HashMap::new();
        milestone_issues.insert("v2.0".to_string(), vec![reopened_issue.clone()]);

        let mut issue_events = HashMap::new();
        issue_events.insert(reopened_issue.number, reopened_events);

        let git_info = RecordMockGitInfo::new()
            .with_milestones(milestones.clone())
            .with_milestone_issues(milestone_issues)
            .with_issue_events(issue_events)
            .with_repo_users(repo_users)
            .with_git_status(GitStatus::Dirty(vec![
                PathBuf::from("src/test.rs"),
                PathBuf::from("src/lib.rs"),
            ]))
            .with_file_commits(
                PathBuf::from("src/test.rs"),
                vec![(
                    ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                    "Test commit".to_string(),
                )],
            );

        let config = create_test_configuration();
        let env = create_test_env();

        let result = record(&milestones, &config, &git_info, env, None).await;
        assert!(result.is_ok());

        insta::assert_snapshot!("record_reopened_issue", result.unwrap().1);
    }

    #[tokio::test]
    async fn test_record_empty_milestones() {
        let milestones = vec![];
        let git_info = RecordMockGitInfo::new()
            .with_repo_users(load_test_users())
            .with_git_status(GitStatus::Clean);

        let config = create_test_configuration();
        let env = create_test_env();

        let result = record(&milestones, &config, &git_info, env, None).await;
        assert!(result.is_ok());

        insta::assert_snapshot!("record_empty_milestones", result.unwrap().1);
    }
}
