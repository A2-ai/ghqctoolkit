use std::{collections::HashMap, io, path::absolute};

use chrono;
use lazy_static::lazy_static;
use octocrab::models::{Milestone, issues::Issue};
use serde::{Deserialize, Serialize};
use tera::{Context, Result as TeraResult, Tera, Value};

use crate::{
    ChecklistSummary, Configuration, GitHubReader, GitRepository,
    git::{GitCommitAnalysis, GitFileOps, GitStatus, GitStatusOps},
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

pub async fn record(
    milestones: &[Milestone],
    configuration: &Configuration,
    git_info: &(impl GitHubReader + GitRepository + GitFileOps + GitCommitAnalysis + GitStatusOps),
    env: impl EnvProvider,
    cache: Option<&crate::cache::DiskCache>,
) -> Result<String, RecordError> {
    let mut context = Context::new();

    context.insert("repository_name", git_info.repo());
    context.insert(
        "checklist_name",
        &configuration.options.checklist_display_name,
    );

    if let Ok(author) = env.var("USER") {
        context.insert("author", &author);
    }

    let date = chrono::Local::now().format("%B %d, %Y").to_string();
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
        .collect::<Vec<_>>()
        .join(", ");
    context.insert("milestone_names", &milestone_names);

    let rendered = TEMPLATES
        .render("record.qmd", &context)
        .map_err(RecordError::Template)?;

    Ok(rendered)
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
fn render_milestone_table_rows(
    args: &std::collections::HashMap<String, Value>,
) -> TeraResult<Value> {
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
fn render_issue_summary_table_rows(
    args: &std::collections::HashMap<String, Value>,
) -> TeraResult<Value> {
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
            .split(", ")
            .map(|qcer| qcer.split(" (").next().unwrap_or(qcer))
            .collect::<Vec<_>>()
            .join(", ");

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
    pub qcer: String,
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
    pub comments: String,
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
    let repo_users = crate::cache::get_repo_users(cache, git_info).await?;

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
        let issue_information: Result<Vec<_>, RecordError> = issue_results.into_iter().collect();
        let issue_information = issue_information?;

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
    repo_users: &[crate::git::RepoUser],
    git_status: &GitStatus,
    cache: Option<&crate::cache::DiskCache>,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
) -> Result<IssueInformation, RecordError> {
    // Get comments and create issue thread
    let comments = crate::cache::get_issue_comments(issue, cache, git_info).await?;
    let issue_thread = IssueThread::from_issue_comments(issue, &comments, git_info).await?;
    let file_commits = issue_thread.commits(git_info).await?;
    let commit_ids: Vec<_> = file_commits.iter().map(|(id, _)| *id).collect();

    // QC Status
    let qc_status = QCStatus::determine_status(&issue_thread, &commit_ids)?.to_string();

    // Checklist Summary
    let checklist_summaries = analyze_issue_checklists(issue);
    let checklist_summary =
        ChecklistSummary::sum(checklist_summaries.iter().map(|c| &c.1)).to_string();

    // Git Status for this specific file
    let file_commits_option = Some(commit_ids);
    let git_status_str = git_status.format_for_file(&issue_thread, &file_commits_option);

    // Created by (with name lookup)
    let created_by = repo_users
        .iter()
        .find(|user| user.login == issue.user.login)
        .and_then(|user| user.name.as_ref())
        .map(|name| format!("{} ({})", name, issue.user.login))
        .unwrap_or_else(|| issue.user.login.clone());

    // QCers (with name lookup)
    let qcer = if issue.assignees.is_empty() {
        "NA".to_string()
    } else {
        let qcer_names: Vec<String> = issue
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
            .collect();
        qcer_names.join(", ")
    };

    // Issue closer (with name lookup)
    let closed_by = if matches!(issue.state, octocrab::models::IssueState::Closed) {
        match get_issue_closer_username(issue, cache, git_info).await {
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
    let initial_qc_commit = format!("{}", issue_thread.initial_commit);
    let latest_qc_commit = format!("{}", issue_thread.latest_commit());

    // Process issue body with header translation (min level 4 since under ### Issue Body)
    let body = issue
        .body
        .as_ref()
        .map(|b| format_markdown_with_min_level(b, 4))
        .unwrap_or_else(|| "No description provided.".to_string());

    // Format comments with proper header structure
    let formatted_comments = format_comments(&comments, repo_users);

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
        state: match issue.state {
            octocrab::models::IssueState::Open => "Open".to_string(),
            octocrab::models::IssueState::Closed => "Closed".to_string(),
            _ => "Unknown".to_string(),
        },
        closed_by,
        closed_at,
        body,
        comments: formatted_comments,
    })
}

/// Extract the username of who closed the issue from issue events
async fn get_issue_closer_username(
    issue: &Issue,
    cache: Option<&crate::cache::DiskCache>,
    git_info: &impl GitHubReader,
) -> Option<String> {
    if !matches!(issue.state, octocrab::models::IssueState::Closed) {
        return None;
    }

    match crate::cache::get_issue_events(issue, cache, git_info).await {
        Ok(events) => {
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
        Err(e) => {
            log::warn!("Failed to fetch events for issue #{}: {}", issue.number, e);
            None
        }
    }
}

/// Format comments with proper header structure
fn format_comments(
    comments: &[crate::git::GitComment],
    repo_users: &[crate::git::RepoUser],
) -> String {
    if comments.is_empty() {
        return "No comments found.".to_string();
    }

    let mut formatted_comments = Vec::new();

    for comment in comments {
        // Look up display name
        let author_display = repo_users
            .iter()
            .find(|user| user.login == comment.author_login)
            .and_then(|user| user.name.as_ref())
            .map(|name| format!("{} ({})", name, comment.author_login))
            .unwrap_or_else(|| comment.author_login.clone());

        // Format timestamp
        let created_at = comment.created_at.format("%Y-%m-%d %H:%M:%S").to_string();

        // Format comment body (min level 5 since under #### Comment header)
        let body = format_markdown_with_min_level(&comment.body, 5);

        // Format the comment with level 4 header
        formatted_comments.push(format!(
            "#### Comment by {} at {}\n\n{}",
            author_display, created_at, body
        ));
    }

    formatted_comments.join("\n\n")
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

    result.join("\n").replace("---", "`---`").replace("```diff", "``` diff")
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
}
