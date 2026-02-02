use anyhow::{Result, bail};
use gix::ObjectId;
use octocrab::models::Milestone;

use crate::cli::interactive::{prompt_existing_milestone, prompt_issue};
use crate::{
    ChecklistSummary, DiskCache, GitHubReader, GitInfo, GitStatus, GitStatusOps, IssueThread,
    QCStatus, analyze_issue_checklists,
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
    let issues = git_info
        .get_issues(octocrab::params::State::All, Some(milestone.number as u64))
        .await?;
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
    let checklist_summary = analyze_issue_checklists(&issue);

    // Create IssueThread from the selected issue
    let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
    let file_commits = issue_thread.file_commits();

    // Get git status for the file
    let git_status = git_info.status()?;

    // Determine QC status
    let qc_status = QCStatus::determine_status(&issue_thread)?;

    // Display the status
    println!(
        "\n{}",
        single_issue_status(
            &issue_thread,
            &git_status,
            &qc_status,
            &file_commits,
            &checklist_summary
        )
    );

    Ok(())
}

pub fn single_issue_status(
    issue_thread: &IssueThread,
    git_status: &GitStatus,
    qc_status: &QCStatus,
    file_commits: &[&ObjectId],
    checklist_summaries: &[(String, ChecklistSummary)],
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
    let git_str = match git_status {
        GitStatus::Clean => {
            log::debug!("Repository git status: clean");
            "File is up to date!".to_string()
        }
        GitStatus::Dirty(files) => {
            log::debug!("Repository git status: dirty");
            if files.contains(&issue_thread.file) {
                "File has local, uncommitted changes".to_string()
            } else {
                "File is up to date!".to_string()
            }
        }
        GitStatus::Ahead(commits) => {
            log::debug!("Repository git status: ahead");
            if file_commits.iter().any(|c| commits.contains(c)) {
                "File has local, committed changes".to_string()
            } else {
                "File is up to date!".to_string()
            }
        }
        GitStatus::Behind(commits) => {
            log::debug!("Repository git status: behind");
            if file_commits.iter().any(|c| commits.contains(c)) {
                "File has remote changes that have not been pulled locally".to_string()
            } else {
                "File is up to date!".to_string()
            }
        }
        GitStatus::Diverged { ahead, behind } => {
            log::debug!("Repository git status: diverged");
            let is_ahead = file_commits.iter().any(|c| ahead.contains(c));
            let is_behind = file_commits.iter().any(|c| behind.contains(c));

            match (is_ahead, is_behind) {
                (true, true) => {
                    "File has diverged and has local, committed and remote, unpulled changes"
                        .to_string()
                }
                (true, false) => "File has local, committed changes".to_string(),
                (false, true) => {
                    "File has remote changes that have not been pulled locally".to_string()
                }
                (false, false) => "File is up to date!".to_string(),
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
        let issues = git_info
            .get_issues(octocrab::params::State::All, Some(milestone.number as u64))
            .await?;

        for issue in issues {
            // Create IssueThread for each issue
            if let Ok(issue_thread) = IssueThread::from_issue(&issue, cache, git_info).await {
                let file_commits = issue_thread.file_commits();

                // Get git status
                let git_status = git_info.status().unwrap_or(GitStatus::Clean);

                // Determine QC status
                if let Ok(qc_status) = QCStatus::determine_status(&issue_thread) {
                    let checklist_summaries = analyze_issue_checklists(&issue);
                    let checklist_summary =
                        ChecklistSummary::sum(checklist_summaries.iter().map(|(_, c)| c));

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
                        git_status: git_status.format_for_file(&issue_thread.file, &file_commits),
                        checklist_summary,
                    };
                    rows.push(row);
                }
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

    // Print header
    println!();
    println!(
        "{:<file_width$} | {:<milestone_width$} | {:<branch_width$} | {:<issue_state_width$} | {:<qc_status_width$} | {:<git_status_width$} | {:<checklist_width$}",
        "File",
        "Milestone",
        "Branch",
        "Issue State",
        "QC Status",
        "Git Status",
        "Checklist",
        file_width = file_width,
        milestone_width = milestone_width,
        branch_width = branch_width,
        issue_state_width = issue_state_width,
        qc_status_width = qc_status_width,
        git_status_width = git_status_width,
        checklist_width = checklist_width,
    );

    // Print separator
    println!(
        "{:-<file_width$}-+-{:-<milestone_width$}-+-{:-<branch_width$}-+-{:-<issue_state_width$}-+-{:-<qc_status_width$}-+-{:-<git_status_width$}-+-{:-<checklist_width$}",
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
    );

    // Print rows
    for row in rows {
        println!(
            "{:<file_width$} | {:<milestone_width$} | {:<branch_width$} | {:<issue_state_width$} | {:<qc_status_width$} | {:<git_status_width$} | {:<checklist_width$}",
            row.file,
            row.milestone,
            row.branch,
            row.issue_state,
            row.qc_status,
            row.git_status,
            row.checklist_summary,
            file_width = file_width,
            milestone_width = milestone_width,
            branch_width = branch_width,
            issue_state_width = issue_state_width,
            qc_status_width = qc_status_width,
            git_status_width = git_status_width,
            checklist_width = checklist_width,
        );
    }
    println!();
}
