use anyhow::{Result, bail};
use gix::ObjectId;
use octocrab::models::Milestone;

use crate::cli::interactive::{prompt_existing_milestone, prompt_issue};
use crate::{DiskCache, GitHubReader, GitInfo, GitStatus, GitStatusOps, IssueThread, QCStatus};

pub async fn interactive_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    println!("📊 Welcome to GHQC Status Mode!");

    // Select milestone (existing only)
    let milestone = prompt_existing_milestone(milestones)?;

    // Get issues for this milestone
    let issues = git_info.get_milestone_issues(&milestone).await?;
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

    // Create IssueThread from the selected issue
    let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
    let file_commits = issue_thread
        .commits(git_info)
        .await
        .ok()
        .map(|v| v.into_iter().map(|(c, _)| c).collect::<Vec<_>>());

    // Get git status for the file
    let git_status = git_info.status()?;

    // Determine QC status
    let qc_status = QCStatus::determine_status(&issue_thread, &git_status, git_info).await?;

    // Display the status
    println!(
        "\n{}",
        single_issue_status(&issue_thread, &git_status, &qc_status, &file_commits)
    );

    Ok(())
}

pub fn single_issue_status(
    issue_thread: &IssueThread,
    git_status: &GitStatus,
    qc_status: &QCStatus,
    file_commits: &Option<Vec<ObjectId>>,
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
        QCStatus::AwaitingApproval => format!("Awaiting approval. Latest commit notified"),
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
            if let Some(file_commits) = file_commits {
                log::debug!(
                    "file commits: {:#?}\nahead commits: {:#?}",
                    file_commits,
                    commits
                );
                if file_commits.iter().any(|c| commits.contains(c)) {
                    "File has local, committed changes".to_string()
                } else {
                    "File is up to date!".to_string()
                }
            } else {
                format!(
                    "Repository is ahead of the remote by {} commits",
                    commits.len()
                )
            }
        }
        GitStatus::Behind(commits) => {
            log::debug!("Repository git status: behind");
            if let Some(file_commits) = file_commits {
                if file_commits.iter().any(|c| commits.contains(c)) {
                    "File has remote changes that have not been pulled locally".to_string()
                } else {
                    "File is up to date!".to_string()
                }
            } else {
                format!(
                    "Repository is behind the remote by {} commits",
                    commits.len()
                )
            }
        }
        GitStatus::Diverged { ahead, behind } => {
            log::debug!("Repository git status: diverged");
            if let Some(file_commits) = file_commits {
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
            } else {
                format!(
                    "Repository has diverged and is ahead by {} and behind by {} commits",
                    ahead.len(),
                    behind.len()
                )
            }
        }
    };
    res.push(format!("- QC Status:   {qc_str}"));
    res.push(format!("- Git Status:  {git_str}"));

    res.join("\n")
}
