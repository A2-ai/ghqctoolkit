use std::{collections::HashMap, io, path::absolute};

use chrono;
use lazy_static::lazy_static;
use octocrab::models::{Milestone, issues::Issue};
use serde::{Deserialize, Serialize};
use tera::{Context, Result as TeraResult, Tera, Value};

use crate::{
    Configuration, GitHubReader, GitRepository, qc_status::{analyze_issue_checklists, QCStatus},
    utils::EnvProvider, issue::IssueThread, git::{GitFileOps, GitCommitAnalysis},
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
    git_info: &(impl GitHubReader + GitRepository + GitFileOps + GitCommitAnalysis),
    env: impl EnvProvider,
    cache: Option<&crate::cache::DiskCache>,
) -> Result<String, RecordError> {
    let mut context = Context::new();

    let milestone_names = milestones
        .iter()
        .map(|m| m.title.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    context.insert("milestone_names", &milestone_names);

    context.insert("repository_name", git_info.repo());

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
    let milestone_sections = create_milestone_sections(milestones, &issue_objects, cache, git_info).await?;
    context.insert("milestone_sections", &milestone_sections);

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
        issue_map.insert(milestone.title.clone(), issues);
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
        let empty_vec = Vec::new();
        let issues = issue_objects.get(&milestone.title).unwrap_or(&empty_vec);

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

    let rows: Vec<IssueSummaryRow> = serde_json::from_value(data.clone())
        .map_err(|e| tera::Error::msg(format!("Failed to parse issue summary data: {}", e)))?;

    if rows.is_empty() {
        return Ok(Value::String(String::new()));
    }

    let mut table_rows = Vec::new();

    // Add data rows only
    for (i, row) in rows.iter().enumerate() {
        if i < rows.len() - 1 {
            table_rows.push(format!(
                r"{} & {} & {} & {} & {}\\",
                row.file_path, row.qc_status, row.author, row.qcer, row.issue_closer
            ));
            table_rows.push(r"\addlinespace\addlinespace".to_string());
        } else {
            table_rows.push(format!(
                r"{} & {} & {} & {} & {}\\*",
                row.file_path, row.qc_status, row.author, row.qcer, row.issue_closer
            ));
        }
    }

    Ok(Value::String(table_rows.join("\n")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSummaryRow {
    pub file_path: String,
    pub qc_status: String,
    pub author: String,
    pub qcer: String,
    pub issue_closer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneSection {
    pub name: String,
    pub issues: Vec<IssueSummaryRow>,
}

/// Create issue summary data for each milestone
pub async fn create_milestone_sections(
    milestone_objects: &[Milestone],
    issue_objects: &HashMap<String, Vec<Issue>>,
    cache: Option<&crate::cache::DiskCache>,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
) -> Result<Vec<MilestoneSection>, RecordError> {
    // Get all repository users with cache for efficient lookup
    let repo_users = crate::cache::get_repo_users(cache, git_info).await?;

    let mut sections = Vec::new();

    for milestone in milestone_objects {
        let empty_vec = Vec::new();
        let issues = issue_objects.get(&milestone.title).unwrap_or(&empty_vec);

        // Create futures for processing each issue
        let issue_futures: Vec<_> = issues
            .iter()
            .map(|issue| {
                let repo_users_clone = repo_users.clone();
                async move {
                    // Extract file path from issue title
                    let file_path = issue.title.replace('_', "\\_");

                    // Extract author from issue body (or use issue creator as fallback)
                    let author =
                        extract_author_from_issue(issue).unwrap_or_else(|| issue.user.login.clone());

                    // Get comments for this issue and determine QC status
                    let comments = crate::cache::get_issue_comments(issue, cache, git_info).await?;
                    let issue_thread = IssueThread::from_issue_comments(issue, &comments, git_info).await?;
                    let file_commits = issue_thread.commits(git_info).await?;
                    let commit_ids: Vec<_> = file_commits.into_iter().map(|(id, _)| id).collect();
                    let qc_status = QCStatus::determine_status(&issue_thread, &commit_ids)?.to_string();

                    // Extract QCers from issue assignees using repo users lookup
                    let qcer = if issue.assignees.is_empty() {
                        "NA".to_string()
                    } else {
                        let qcer_names: Vec<String> = issue.assignees
                            .iter()
                            .map(|assignee| {
                                // Look up user in repo_users by login
                                repo_users_clone
                                    .iter()
                                    .find(|user| user.login == assignee.login)
                                    .and_then(|user| user.name.as_ref())
                                    .map(|name| name.clone())
                                    .unwrap_or_else(|| assignee.login.clone())
                            })
                            .collect();
                        qcer_names.join(",\n")
                    };

                    // Extract issue closer with name lookup from repo users
                    let issue_closer = match get_issue_closer_username(issue, git_info).await {
                        Some(closer_login) => {
                            // Look up the closer in repo_users to get display name
                            repo_users_clone
                                .iter()
                                .find(|user| user.login == closer_login)
                                .and_then(|user| user.name.as_ref())
                                .map(|name| name.clone())
                                .unwrap_or(closer_login) // Fallback to login if name not found
                        }
                        None => "NA".to_string(),
                    };

                    Ok(IssueSummaryRow {
                        file_path,
                        qc_status,
                        author,
                        qcer,
                        issue_closer,
                    })
                }
            })
            .collect();

        // Execute all futures concurrently
        let issue_results = futures::future::join_all(issue_futures).await;
        let issue_rows: Result<Vec<_>, RecordError> = issue_results.into_iter().collect();
        let issue_rows = issue_rows?;

        sections.push(MilestoneSection {
            name: milestone.title.clone(),
            issues: issue_rows,
        });
    }

    Ok(sections)
}

/// Extract the username of who closed the issue from issue events
async fn get_issue_closer_username(
    issue: &Issue,
    git_info: &impl GitHubReader,
) -> Option<String> {
    if !matches!(issue.state, octocrab::models::IssueState::Closed) {
        return None;
    }

    match git_info.get_issue_events(issue).await {
        Ok(events) => {
            // Find the last "closed" event
            events
                .iter()
                .rev() // Start from the most recent
                .find(|event| {
                    event.get("event")
                        .and_then(|e| e.as_str())
                        .map(|s| s == "closed")
                        .unwrap_or(false)
                })
                .and_then(|event| {
                    event.get("actor")
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

/// Extract author information from issue body
fn extract_author_from_issue(issue: &Issue) -> Option<String> {
    issue.body.as_ref().and_then(|body| {
        // Look for author pattern in issue body
        if let Some(start) = body.find("* author: ") {
            let author_line = &body[start + 10..]; // Skip "* author: "
            if let Some(end) = author_line.find('\n') {
                Some(author_line[..end].trim().to_string())
            } else {
                Some(author_line.trim().to_string())
            }
        } else {
            None
        }
    })
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
}
