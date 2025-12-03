use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf, absolute},
    process::{Command, Stdio},
    thread,
};

use chrono;
use lazy_static::lazy_static;
use octocrab::models::{Milestone, issues::Issue};
use serde::{Deserialize, Serialize};
use tera::{Context, Tera};

use crate::{
    ChecklistSummary, Configuration, DiskCache, GitHubReader, GitRepository, GitStatusOps,
    RepoUser, get_issue_comments, get_issue_events, get_repo_users,
    git::{GitComment, GitCommitAnalysis, GitFileOps, GitStatus},
    issue::IssueThread,
    qc_status::{QCStatus, analyze_issue_checklists},
    utils::EnvProvider,
};

// Re-export submodules
mod images;
mod latex;
mod tables;

// Re-export public items from submodules
pub use latex::{escape_latex, format_markdown};
// Template functions - used by tera templates, not directly by Rust code
pub use images::{HttpImageDownloader, ImageDownloader};
#[allow(unused_imports)]
pub use tables::{
    create_milestone_df, insert_breaks, render_issue_summary_table_rows,
    render_milestone_table_rows,
};

lazy_static! {
    pub static ref TEMPLATES: Tera = {
        let mut tera = Tera::default();

        tera.add_raw_template("record.qmd", include_str!("../templates/record.qmd"))
            .unwrap();

        // Register custom functions from tables module
        tera.register_function("render_milestone_table_rows", tables::render_milestone_table_rows);
        tera.register_function("render_issue_summary_table_rows", tables::render_issue_summary_table_rows);

        tera
    };
}

pub fn record(
    milestones: &[Milestone],
    issues: &HashMap<String, Vec<IssueInformation>>,
    configuration: &Configuration,
    git_info: &impl GitRepository,
    env: &impl EnvProvider,
    only_tables: bool,
) -> Result<String, RecordError> {
    let mut context = Context::new();

    context.insert("repository_name", &escape_latex(git_info.repo()));
    context.insert(
        "checklist_name",
        &escape_latex(&configuration.options.checklist_display_name),
    );

    if let Ok(author) = env.var("USER") {
        context.insert("author", &escape_latex(&author));
    }

    let date = if let Ok(custom_date) = env.var("GHQC_RECORD_DATE") {
        escape_latex(&custom_date)
    } else {
        escape_latex(&chrono::Local::now().format("%B %d, %Y").to_string())
    };
    context.insert("date", &date);

    let logo_path = absolute(configuration.logo_path())?;
    if logo_path.exists() {
        context.insert("logo_path", &logo_path);
    }

    // Generate milestone dataframe
    let milestone_data = create_milestone_df(milestones, &issues)?;
    context.insert("milestone_data", &milestone_data);

    // Generate milestone sections for individual milestone tables
    // Use the original milestone order to ensure deterministic output
    let milestone_sections = milestones
        .iter()
        .filter_map(|milestone| {
            issues
                .get(&milestone.title)
                .map(|issue_list| MilestoneSection {
                    name: milestone.title.clone(),
                    issues: issue_list.clone(),
                })
        })
        .collect::<Vec<_>>();
    context.insert("milestone_sections", &milestone_sections);

    let milestone_names = milestones
        .iter()
        .map(|m| m.title.as_str())
        .collect::<Vec<_>>();
    context.insert(
        "milestone_names",
        &escape_latex(&milestone_names.join(", ")),
    );

    context.insert("only_tables", &only_tables);

    Ok(TEMPLATES
        .render("record.qmd", &context)
        .map_err(RecordError::Template)?)
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

pub async fn get_milestone_issue_information(
    milestone_issues: &HashMap<String, Vec<Issue>>,
    cache: Option<&DiskCache>,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis + GitStatusOps),
    image_downloader: &impl images::ImageDownloader,
) -> Result<HashMap<String, Vec<IssueInformation>>, RecordError> {
    let repo_users = get_repo_users(cache, git_info).await?;
    let git_status = git_info.status()?;

    let mut res = HashMap::new();
    for (milestone_name, issues) in milestone_issues {
        // Create detailed issue information for each issue (used for both summary and details)
        let issue_futures: Vec<_> = issues
            .iter()
            .map(|issue| {
                let repo_users_clone = repo_users.clone();
                let git_status_clone = git_status.clone();
                async move {
                    create_issue_information(
                        issue,
                        milestone_name,
                        &repo_users_clone,
                        &git_status_clone,
                        cache,
                        git_info,
                        image_downloader,
                    )
                    .await
                }
            })
            .collect();

        let issue_results = futures::future::join_all(issue_futures).await;
        let issue_information = issue_results.into_iter().collect::<Result<Vec<_>, _>>()?;

        res.insert(milestone_name.to_string(), issue_information);
    }

    Ok(res)
}

/// Detect if comments contain images but lack HTML for JWT URL extraction
///
/// Returns true if any comment has images in the body but no HTML content,
/// indicating we need to re-fetch from the API to get HTML with JWT URLs.
/// Note: Issue HTML is handled separately since issues are fetched differently than comments.
fn needs_html_for_jwt_urls(comments: &[GitComment]) -> bool {
    comments.iter().any(|comment| {
        let has_images = !images::extract_image_urls_from_markdown(&comment.body).is_empty();
        let lacks_html = comment.html.is_none();
        let needs_refetch = has_images && lacks_html;

        if needs_refetch {
            log::debug!(
                "Comment from {} has images but no HTML",
                comment.created_at.format("%Y-%m-%d %H:%M:%S")
            );
        }

        needs_refetch
    })
}

/// Create detailed issue information from an issue
pub async fn create_issue_information(
    issue: &Issue,
    milestone_name: &str,
    repo_users: &[RepoUser],
    git_status: &GitStatus,
    cache: Option<&DiskCache>,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
    image_downloader: &impl images::ImageDownloader,
) -> Result<IssueInformation, RecordError> {
    // Get comments and check if we need HTML for JWT URLs
    let mut comments = get_issue_comments(issue, cache, git_info).await?;

    // Check if we need HTML for JWT URLs
    if needs_html_for_jwt_urls(&comments) {
        log::info!(
            "Issue #{} contains images but cached comments lack HTML - re-fetching with HTML",
            issue.number
        );

        // Invalidate cache for this issue to force fresh fetch with HTML
        if let Some(cache) = cache {
            let cache_key = format!("issue_{}", issue.number);
            if let Err(e) = cache.invalidate(&["issues", "comments"], &cache_key) {
                log::warn!(
                    "Failed to invalidate cache for issue #{}: {}",
                    issue.number,
                    e
                );
            }
        }

        // Re-fetch comments (will now include HTML since cache is invalidated)
        comments = git_info.get_issue_comments(issue).await?;

        // Verify we got HTML content for JWT URLs
        if needs_html_for_jwt_urls(&comments) {
            return Err(RecordError::HtmlRequiredForJwtUrls {
                issue_number: issue.number,
            });
        }

        log::debug!(
            "Re-fetched {} comments with HTML for issue #{}",
            comments.len(),
            issue.number
        );
    }

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

    // Create IssueImage structs for all images in the issue and comments
    let temp_dir = std::env::temp_dir().join("ghqc-images");
    std::fs::create_dir_all(&temp_dir)?;

    let mut all_issue_images = Vec::new();

    // Create IssueImages from issue body
    if let Some(body_text) = &issue.body {
        let issue_images =
            images::create_issue_images(body_text, issue.body_html.as_deref(), &temp_dir);
        all_issue_images.extend(issue_images);
    }

    // Create IssueImages from each comment
    for comment in &comments {
        let comment_images =
            images::create_issue_images(&comment.body, comment.html.as_deref(), &temp_dir);
        all_issue_images.extend(comment_images);
    }

    log::debug!(
        "Created {} IssueImages for issue #{}",
        all_issue_images.len(),
        issue.number
    );

    // Download all images sequentially
    let download_results: Vec<_> = all_issue_images
        .iter()
        .map(|issue_image| {
            let result = image_downloader.download_issue_image(issue_image);
            (issue_image.clone(), result)
        })
        .collect();

    // Build URL-to-path map from successful downloads and collect failures
    let mut image_url_map = HashMap::new();
    let mut failed_downloads = Vec::new();

    for (issue_image, result) in download_results {
        match result {
            Ok(_) => {
                // Map text URL to downloaded path
                image_url_map.insert(issue_image.text, issue_image.path);
            }
            Err(e) => {
                log::error!("Failed to download image {}: {}", issue_image.html, e);
                failed_downloads.push(format!("{}: {}", issue_image.html, e));
            }
        }
    }

    // Fail loudly if any image downloads failed
    if !failed_downloads.is_empty() {
        return Err(RecordError::MultipleImageDownloadsFailed {
            failures: failed_downloads,
        });
    }

    // Process issue body with header translation (min level 4 since under ### Issue Body)
    let body = issue
        .body
        .as_ref()
        .map(|b| format_markdown(b, 4, &image_url_map))
        .unwrap_or_else(|| "No description provided.".to_string());

    // Format comments as header-body pairs
    let formatted_comments = format_comments(&comments, repo_users, &image_url_map);

    // Format events timeline
    let formatted_events = format_events(&events, repo_users);

    // Create combined timeline from formatted events and comment headers
    let timeline = create_combined_timeline(&formatted_events, &formatted_comments);

    Ok(IssueInformation {
        title: escape_latex(&issue.title),
        number: issue.number,
        milestone: escape_latex(milestone_name),
        created_by: escape_latex(&created_by),
        created_at: escape_latex(&created_at),
        qcer: qcer.into_iter().map(|q| escape_latex(&q)).collect(),
        qc_status: escape_latex(&qc_status),
        checklist_summary: escape_latex(&checklist_summary),
        git_status: escape_latex(&git_status_str),
        initial_qc_commit: escape_latex(&initial_qc_commit),
        latest_qc_commit: escape_latex(&latest_qc_commit),
        issue_url: escape_latex(&issue.html_url.to_string()),
        state: escape_latex(&if open { "Open" } else { "Closed" }.to_string()),
        closed_by: closed_by.map(|c| escape_latex(&c)),
        closed_at: closed_at.map(|c| escape_latex(&c)),
        body, // body already processed with format_markdown_with_min_level which handles LaTeX
        comments: formatted_comments, // comments already processed with format_markdown_with_min_level
        events: formatted_events
            .into_iter()
            .map(|e| escape_latex(&e))
            .collect(),
        timeline: timeline.into_iter().map(|t| escape_latex(&t)).collect(),
    })
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

/// Create combined timeline from formatted events and comment headers, sorted chronologically
pub(crate) fn create_combined_timeline(
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
pub(crate) fn get_issue_closer_username(events: &[serde_json::Value]) -> Option<String> {
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
pub(crate) fn format_events(events: &[serde_json::Value], repo_users: &[RepoUser]) -> Vec<String> {
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

                // Use assigner field instead of actor for assignments
                let assigner_login = event
                    .get("assigner")
                    .and_then(|a| a.get("login"))
                    .and_then(|l| l.as_str())
                    .unwrap_or(actor_login); // Fallback to actor if assigner is not available

                let assignee_display = repo_users
                    .iter()
                    .find(|user| user.login == assignee_login)
                    .and_then(|user| user.name.as_ref())
                    .map(|name| format!("{} ({})", name, assignee_login))
                    .unwrap_or_else(|| assignee_login.to_string());

                // Look up display name for assigner
                let assigner_display = repo_users
                    .iter()
                    .find(|user| user.login == assigner_login)
                    .and_then(|user| user.name.as_ref())
                    .map(|name| format!("{} ({})", name, assigner_login))
                    .unwrap_or_else(|| assigner_login.to_string());

                let formatted_message = format!(
                    "{} - {} assigned by {}",
                    created_at, assignee_display, assigner_display
                );

                formatted_message
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
pub(crate) fn format_comments(
    comments: &[GitComment],
    repo_users: &[RepoUser],
    image_url_map: &HashMap<String, PathBuf>,
) -> Vec<(String, String)> {
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
        let header = format!(
            "{} - Comment by {}",
            escape_latex(&created_at),
            escape_latex(&author_display)
        );

        // Format comment body (min level 4 since it will be under #### header in template)
        let body = format_markdown(&comment.body, 4, image_url_map);

        formatted_comments.push((header, body));
    }

    formatted_comments
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

pub fn render_in_staging(
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
    #[error("Image download failed for URL {url}: {error}")]
    ImageDownloadFailed { url: String, error: String },
    #[error("Multiple image downloads failed: {failures:?}")]
    MultipleImageDownloadsFailed { failures: Vec<String> },
    #[error("Image cleanup failed: {0}")]
    ImageCleanupFailed(String),
    #[error(
        "Unable to fetch HTML content for JWT URL extraction in issue #{issue_number}. Images detected but GitHub API did not provide body_html field."
    )]
    HtmlRequiredForJwtUrls { issue_number: u64 },
}
