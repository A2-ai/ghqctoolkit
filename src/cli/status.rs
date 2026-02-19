use std::path::PathBuf;

use anyhow::{Result, bail};
use gix::ObjectId;
use octocrab::models::Milestone;

use crate::cli::interactive::{prompt_existing_milestone, prompt_issue};
use crate::{
    BlockingQCStatus, ChecklistSummary, DiskCache, GitHubReader, GitInfo, GitStatus, GitStatusOps,
    IssueThread, QCStatus, analyze_issue_checklists, get_blocking_qc_status,
    git::fetch_and_status,
};

pub async fn interactive_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    println!("📊 Welcome to GHQC Status Mode!");

    // Select milestone (existing only)
    let milestone = prompt_existing_milestone(milestones)?;

    // Get issues for this milestone
    let issues = git_info.get_issues(Some(milestone.number as u64)).await?;
    log::debug!(
        "Found {} total issues in milestone '{}'",
        issues.len(),
        milestone.title
    );

    if issues.is_empty() {
        bail!("No issues found in milestone '{}'", milestone.title);
    }

    // Select issue by title
    let issue = prompt_issue(&issues)?;
    let checklist_summary = analyze_issue_checklists(issue.body.as_deref());

    // Create IssueThread from the selected issue
    let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
    let file_commits = issue_thread.file_commits();

    // Get git status for the file
    let git_status = fetch_and_status(git_info)?;
    let dirty_files = git_info.dirty()?;

    // Determine QC status
    let qc_status = QCStatus::determine_status(&issue_thread);
    let blocking_qc_status =
        get_blocking_qc_status(&issue_thread.blocking_qcs, git_info, cache).await;

    // Display the status
    println!(
        "\n{}",
        single_issue_status(
            &issue_thread,
            &git_status,
            &qc_status,
            &dirty_files,
            &file_commits,
            &checklist_summary,
            &blocking_qc_status,
        )
    );

    Ok(())
}

pub fn single_issue_status(
    issue_thread: &IssueThread,
    git_status: &GitStatus,
    qc_status: &QCStatus,
    dirty_files: &[PathBuf],
    file_commits: &[&ObjectId],
    checklist_summaries: &[(String, ChecklistSummary)],
    blocking_qc_status: &BlockingQCStatus,
) -> String {
    let mut res = vec![
        format!("- File:        {}", issue_thread.file.display()),
        format!("- Branch:      {}", issue_thread.branch),
    ];
    res.push(format!(
        "- Issue State: {}",
        if issue_thread.open { "open" } else { "closed" }
    ));

    let qc_str = match qc_status {
        QCStatus::Approved => format!("Approved"),
        QCStatus::ChangesAfterApproval(_) => {
            format!("Approved. File has changed since approval")
        }
        QCStatus::AwaitingReview => format!("Awaiting review. Latest commit notified"),
        QCStatus::ChangeRequested => format!("Changes requested. Latest commit reviewed"),
        QCStatus::InProgress => format!("Awaiting approval"),
        QCStatus::ApprovalRequired => format!("Issue closed without approval"),
        QCStatus::ChangesToComment(commit) => format!(
            "File change in '{}' not commented",
            commit.to_string()[..7].to_string()
        ),
    };
    let is_dirty = dirty_files.contains(&issue_thread.file);

    let git_str = match git_status {
        GitStatus::Clean => {
            log::debug!("Repository git status: clean");
            if is_dirty {
                "File has local, uncommitted changes"
            } else {
                "File is up to date!"
            }
        }
        GitStatus::Ahead(commits) => {
            log::debug!("Repository git status: ahead");
            match (file_commits.iter().any(|c| commits.contains(c)), is_dirty) {
                (true, true) => "File has local, committed changes and uncommitted changes",
                (true, false) => "File has local, committed changes",
                (false, true) => "File has uncommitted changes",
                (false, false) => "File is up to date!",
            }
        }
        GitStatus::Behind(commits) => {
            log::debug!("Repository git status: behind");
            match (file_commits.iter().any(|c| commits.contains(c)), is_dirty) {
                (true, true) => {
                    "File has remote changes that have not been pulled locally and uncommitted changes. Stash the changes and pull"
                }
                (true, false) => "File has remote changes that have not been pulled locally",
                (false, true) => "File has uncommitted changes",
                (false, false) => "File is up to date!",
            }
        }
        GitStatus::Diverged { ahead, behind } => {
            log::debug!("Repository git status: diverged");
            let is_ahead = file_commits.iter().any(|c| ahead.contains(c));
            let is_behind = file_commits.iter().any(|c| behind.contains(c));

            match (is_ahead, is_behind, is_dirty) {
                (true, true, true) => {
                    "File has diverged and has local, committed and uncommitted changes and remote, unpulled changes"
                }
                (true, true, false) => {
                    "File has diverged and has local, committed and remote, unpulled changes"
                }
                (true, false, true) => "File has local, committed changes and uncommitted changes",
                (true, false, false) => "File has local, committed changes",
                (false, true, true) => {
                    "File has remote changes that have not been pulled locally and uncommitted changes. Stash the changes and pull"
                }
                (false, true, false) => "File has remote changes that have not been pulled locally",
                (false, false, true) => "File has uncommitted changes",
                (false, false, false) => "File is up to date!",
            }
        }
    };
    let indiv_checklist = checklist_summaries
        .iter()
        .map(|(name, sum)| format!("{name}: {sum}"))
        .collect::<Vec<_>>();
    let checklist_sum = ChecklistSummary::sum(checklist_summaries.iter().map(|(_, c)| c));

    res.push(format!("- QC Status:   {qc_str}"));
    res.push(format!("- Git Status:  {git_str}"));
    res.push(format!(
        "- Checklist Summary: {checklist_sum}\n  - {}",
        indiv_checklist.join("\n  - ")
    ));
    res.push(format!("- {}", blocking_qc_status));

    res.join("\n")
}

#[derive(Debug, Clone)]
pub struct MilestoneStatusRow {
    pub file: String,
    pub milestone: String,
    pub branch: String,
    pub issue_state: String,
    pub qc_status: String,
    pub git_status: String,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_status: BlockingQCStatus,
}

pub async fn interactive_milestone_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    println!("📊 Welcome to GHQC Milestone Status Mode!");

    if milestones.is_empty() {
        bail!("No milestones found in repository");
    }

    use inquire::{MultiSelect, Select};

    // First ask if they want to select all or choose specific ones
    let choice = Select::new(
        "📊 How would you like to select milestones?",
        vec!["📋 Select All Milestones", "🎯 Choose Specific Milestones"],
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let selected_milestones: Vec<&Milestone> = if choice == "📋 Select All Milestones" {
        milestones.iter().collect()
    } else {
        // Multi-select specific milestones
        let milestone_options: Vec<String> = milestones
            .iter()
            .map(|m| format!("{} ({})", m.title, m.number))
            .collect();

        let selected_strings =
            MultiSelect::new("📊 Select milestones to check:", milestone_options)
                .with_validator(|selection: &[inquire::list_option::ListOption<&String>]| {
                    if selection.is_empty() {
                        Ok(inquire::validator::Validation::Invalid(
                            "Please select at least one milestone".into(),
                        ))
                    } else {
                        Ok(inquire::validator::Validation::Valid)
                    }
                })
                .prompt()
                .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        // Filter milestones based on selected strings
        milestones
            .iter()
            .filter(|m| {
                let milestone_display = format!("{} ({})", m.title, m.number);
                selected_strings.contains(&milestone_display)
            })
            .collect()
    };

    if selected_milestones.is_empty() {
        bail!("No milestones selected");
    }

    // Get status for all selected milestones
    let status_rows = get_milestone_status_rows(&selected_milestones, cache, git_info).await?;

    // Display results
    display_milestone_status_table(&status_rows);

    Ok(())
}

pub async fn milestone_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    if milestones.is_empty() {
        bail!("No milestones provided");
    }

    // Convert to &[&Milestone] for the function call
    let milestone_refs: Vec<&Milestone> = milestones.iter().collect();

    // Get status for all milestones
    let status_rows = get_milestone_status_rows(&milestone_refs, cache, git_info).await?;

    // Display results
    display_milestone_status_table(&status_rows);

    Ok(())
}

async fn get_milestone_status_rows(
    milestones: &[&Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<Vec<MilestoneStatusRow>> {
    let mut rows = Vec::new();

    for milestone in milestones {
        // Get all issues for this milestone
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

        for issue in issues {
            // Create IssueThread for each issue
            if let Ok(issue_thread) = IssueThread::from_issue(&issue, cache, git_info).await {
                let file_commits = issue_thread.file_commits();

                // Get git status
                let git_status = fetch_and_status(git_info).unwrap_or(GitStatus::Clean);
                let dirty_files = git_info.dirty().unwrap_or_default();

                // Determine QC status
                let qc_status = QCStatus::determine_status(&issue_thread);
                let checklist_summaries = analyze_issue_checklists(issue.body.as_deref());
                let checklist_summary =
                    ChecklistSummary::sum(checklist_summaries.iter().map(|(_, c)| c));

                let mut git_status_str = git_status.format_for_file(&file_commits);
                if dirty_files.contains(&issue_thread.file) {
                    git_status_str.push_str(" (file has uncommitted local changes)");
                }

                let row = MilestoneStatusRow {
                    file: issue_thread.file.display().to_string(),
                    milestone: milestone.title.clone(),
                    branch: issue_thread.branch.clone(),
                    issue_state: if issue_thread.open {
                        "open".to_string()
                    } else {
                        "closed".to_string()
                    },
                    qc_status: qc_status.to_string(),
                    git_status: git_status_str,
                    checklist_summary,
                    blocking_qc_status: get_blocking_qc_status(
                        &issue_thread.blocking_qcs,
                        git_info,
                        cache,
                    )
                    .await,
                };
                rows.push(row);
            }
        }
    }

    // Sort by milestone name, then by file name
    rows.sort_by(|a, b| {
        a.milestone
            .cmp(&b.milestone)
            .then_with(|| a.file.cmp(&b.file))
    });

    Ok(rows)
}

fn display_milestone_status_table(rows: &[MilestoneStatusRow]) {
    if rows.is_empty() {
        println!("No issues found in selected milestones.");
        return;
    }

    // Calculate column widths
    let file_width = rows.iter().map(|r| r.file.len()).max().unwrap_or(4).max(4);
    let milestone_width = rows
        .iter()
        .map(|r| r.milestone.len())
        .max()
        .unwrap_or(9)
        .max(9);
    let branch_width = rows
        .iter()
        .map(|r| r.branch.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let issue_state_width = rows
        .iter()
        .map(|r| r.issue_state.len())
        .max()
        .unwrap_or(11)
        .max(11);
    let qc_status_width = rows
        .iter()
        .map(|r| r.qc_status.len())
        .max()
        .unwrap_or(9)
        .max(9);
    let git_status_width = rows
        .iter()
        .map(|r| r.git_status.len())
        .max()
        .unwrap_or(10)
        .max(10);
    let checklist_width = rows
        .iter()
        .map(|r| r.checklist_summary.to_string().len())
        .max()
        .unwrap_or(9)
        .max(9);
    let blocking_qc_width = rows
        .iter()
        .map(|r| r.blocking_qc_status.as_summary_string().len())
        .max()
        .unwrap_or(12)
        .max(12);

    // Print header
    println!();
    println!(
        "{:<file_width$} | {:<milestone_width$} | {:<branch_width$} | {:<issue_state_width$} | {:<qc_status_width$} | {:<git_status_width$} | {:<checklist_width$} | {:<blocking_qc_width$}",
        "File",
        "Milestone",
        "Branch",
        "Issue State",
        "QC Status",
        "Git Status",
        "Checklist",
        "Blocking QCs",
        file_width = file_width,
        milestone_width = milestone_width,
        branch_width = branch_width,
        issue_state_width = issue_state_width,
        qc_status_width = qc_status_width,
        git_status_width = git_status_width,
        checklist_width = checklist_width,
        blocking_qc_width = blocking_qc_width,
    );

    // Print separator
    println!(
        "{:-<file_width$}-+-{:-<milestone_width$}-+-{:-<branch_width$}-+-{:-<issue_state_width$}-+-{:-<qc_status_width$}-+-{:-<git_status_width$}-+-{:-<checklist_width$}-+-{:-<blocking_qc_width$}",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        file_width = file_width,
        milestone_width = milestone_width,
        branch_width = branch_width,
        issue_state_width = issue_state_width,
        qc_status_width = qc_status_width,
        git_status_width = git_status_width,
        checklist_width = checklist_width,
        blocking_qc_width = blocking_qc_width,
    );

    // Print rows
    for row in rows {
        let checklist_str = row.checklist_summary.to_string();
        let blocking_qc_str = row.blocking_qc_status.as_summary_string();
        println!(
            "{:<file_width$} | {:<milestone_width$} | {:<branch_width$} | {:<issue_state_width$} | {:<qc_status_width$} | {:<git_status_width$} | {:<checklist_width$} | {:<blocking_qc_width$}",
            row.file,
            row.milestone,
            row.branch,
            row.issue_state,
            row.qc_status,
            row.git_status,
            checklist_str,
            blocking_qc_str,
            file_width = file_width,
            milestone_width = milestone_width,
            branch_width = branch_width,
            issue_state_width = issue_state_width,
            qc_status_width = qc_status_width,
            git_status_width = git_status_width,
            checklist_width = checklist_width,
            blocking_qc_width = blocking_qc_width,
        );
    }
    println!();
}
