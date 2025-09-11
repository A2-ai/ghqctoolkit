use anyhow::{Result, anyhow, bail};
use inquire::Confirm;
use octocrab::models::{Milestone, issues::Issue};

use std::path::PathBuf;

use crate::{
    Configuration, GitHubApi, GitInfo, MilestoneStatus, QCApprove, QCUnapprove, RelevantFile,
    RepoUser,
    cli::interactive::{
        prompt_assignees, prompt_checklist, prompt_commits, prompt_existing_milestone, prompt_file,
        prompt_issue, prompt_milestone, prompt_note, prompt_relevant_files, prompt_single_commit,
    },
    comment::QCComment,
    configuration::Checklist,
    create::validate_assignees,
    git::LocalGitInfo,
};

pub struct CreateContext {
    pub file: PathBuf,
    pub milestone_status: MilestoneStatus,
    pub checklist: Checklist,
    pub assignees: Vec<String>,
    pub relevant_files: Vec<RelevantFile>,
    pub configuration: Configuration,
    pub git_info: GitInfo,
}

impl<'a> CreateContext {
    pub async fn from_interactive(
        project_dir: &PathBuf,
        milestones: Vec<Milestone>,
        configuration: Configuration,
        git_info: GitInfo,
    ) -> Result<Self> {
        println!("🚀 Welcome to GHQC Interactive Mode!");
        // Fetch users once for validation and interactive prompts
        let repo_users: Vec<RepoUser> = git_info.get_users().await?;

        // Interactive prompts
        let milestone_status = prompt_milestone(milestones)?;
        let file = prompt_file(project_dir)?;
        let checklist = prompt_checklist(&configuration)?;
        let assignees = prompt_assignees(&repo_users)?;
        let relevant_files = prompt_relevant_files(project_dir)?;

        // Display summary
        println!("\n✨ Creating issue with:");
        println!("   📊 Milestone: {}", milestone_status);
        println!("   📁 File: {}", file.display());
        println!("   📋 Checklist: {}", checklist.name());
        if !assignees.is_empty() {
            println!("   👥 Assignees: {}", assignees.join(", "));
        }
        if !relevant_files.is_empty() {
            println!(
                "   🔗 Relevant files: {}",
                relevant_files
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        println!();

        Ok(Self {
            file,
            milestone_status,
            checklist,
            assignees,
            relevant_files,
            configuration,
            git_info,
        })
    }

    pub async fn from_args(
        milestone_name: String,
        milestones: Vec<Milestone>,
        file: PathBuf,
        checklist_name: String,
        assignees: Option<Vec<String>>,
        relevant_files: Option<Vec<RelevantFile>>,
        configuration: Configuration,
        git_info: GitInfo,
    ) -> Result<Self> {
        let milestone_status =
            if let Some(m) = milestones.into_iter().find(|m| m.title == milestone_name) {
                log::debug!("Found existing milestone {}", m.number);
                MilestoneStatus::Existing(m)
            } else {
                MilestoneStatus::New(milestone_name)
            };

        let final_assignees = assignees.unwrap_or_default();
        let final_relevant_files = relevant_files.unwrap_or_default();

        // Fetch users for validation
        let repo_users: Vec<RepoUser> = git_info.get_users().await?;

        // Validate assignees if provided
        validate_assignees(&final_assignees, &repo_users)?;

        // Get selected checklist
        let checklist = configuration
            .checklists
            .get(&checklist_name)
            .ok_or(anyhow!("No checklist named {checklist_name}"))?
            .clone();

        Ok(Self {
            file,
            milestone_status,
            checklist,
            assignees: final_assignees,
            relevant_files: final_relevant_files,
            configuration,
            git_info,
        })
    }
}

impl QCComment {
    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        current_commit: Option<String>,
        previous_commit: Option<String>,
        note: Option<String>,
        milestones: &[Milestone],
        git_info: &GitInfo,
        no_diff: bool,
    ) -> Result<Self> {
        // Find the milestone
        let milestone = milestones
            .iter()
            .find(|m| m.title == milestone_name)
            .ok_or_else(|| anyhow!("Milestone '{}' not found", milestone_name))?;

        // Get issues for this milestone
        let issues = git_info.get_milestone_issues(milestone).await?;

        // Find issue that matches the file path
        let file_str = file.display().to_string();
        let issue = issues
            .into_iter()
            .find(|issue| issue.title.contains(&file_str))
            .ok_or_else(|| {
                anyhow!(
                    "No issue found for file '{}' in milestone '{}'",
                    file_str,
                    milestone_name
                )
            })?;

        // Get file commits to determine defaults if needed
        let file_commits = git_info.file_commits(&file)?;

        if file_commits.is_empty() {
            return Err(anyhow!("No commits found for file: {}", file.display()));
        }

        let final_current_commit =
            match current_commit {
                Some(commit_str) => file_commits
                    .iter()
                    .find(|(c, _)| c.to_string().contains(&commit_str))
                    .ok_or(anyhow!(
                        "Provided commit does not correspond to any commits which edited this file"
                    ))?
                    .0,
                None => {
                    // Default to most recent commit for this file (first in chronological order)
                    file_commits[0].0
                }
            };

        let final_previous_commit = match previous_commit {
            Some(commit_str) => {
                // Parse the provided commit string into ObjectId
                Some(
                    file_commits
                        .into_iter()
                        .find(|(c, _)| c.to_string().contains(&commit_str))
                        .ok_or(anyhow!("Provided commit does not correspond to any commits which edited this file"))?.0
                )
            }
            None => {
                // Default to second most recent commit if it exists
                if file_commits.len() > 1 {
                    Some(file_commits[1].0)
                } else {
                    None // Only one commit exists for this file
                }
            }
        };

        Ok(Self {
            issue: issue,
            file,
            current_commit: final_current_commit,
            previous_commit: final_previous_commit,
            note,
            no_diff,
        })
    }

    pub async fn from_interactive(milestones: &[Milestone], git_info: &GitInfo) -> Result<Self> {
        println!("💬 Welcome to GHQC Comment Mode!");

        // Select milestone (existing only)
        let milestone = prompt_existing_milestone(milestones)?;

        // Get issues for this milestone
        let issues = git_info.get_milestone_issues(&milestone).await?;

        // Select issue by title
        let issue = prompt_issue(&issues)?;

        // Extract file path from issue - we need to determine which file this issue is about
        let file_path = extract_file_path_from_issue(&issue)?;

        // Get commits for this file
        let file_commits = git_info.file_commits(&file_path)?;

        // Select commits for comparison
        let (current_commit, previous_commit) = prompt_commits(&file_commits)?;

        // Prompt for optional note
        let note = prompt_note()?;

        // Ask if user wants diff in comment (default is yes/include diff)
        let include_diff = Confirm::new("📊 Include commit diff in comment?")
            .with_default(true)
            .prompt()
            .map_err(|e| anyhow!("Prompt cancelled: {}", e))?;

        // Display summary
        println!("\n✨ Creating comment with:");
        println!("   🎯 Milestone: {}", milestone.title);
        println!("   🎫 Issue: #{} - {}", issue.number, issue.title);
        println!("   📁 File: {}", file_path.display());
        println!("   📝 Current commit: {}", current_commit);
        if let Some(prev) = &previous_commit {
            println!("   📝 Previous commit: {}", prev);
        } else {
            println!("   📝 Previous commit: None (first commit for this file)");
        }
        if let Some(ref n) = note {
            println!("   💬 Note: {}", n);
        }
        println!(
            "   📊 Include diff: {}",
            if include_diff { "Yes" } else { "No" }
        );
        println!();

        Ok(Self {
            issue,
            file: file_path,
            current_commit,
            previous_commit,
            note,
            no_diff: !include_diff,
        })
    }
}

/// Extract file path from issue title or body
fn extract_file_path_from_issue(issue: &Issue) -> Result<PathBuf> {
    // Look for file paths in the title first
    if let Some(path) = find_file_path_in_text(&issue.title) {
        return Ok(PathBuf::from(path));
    }

    // Look in the body if available
    if let Some(body) = &issue.body {
        if let Some(path) = find_file_path_in_text(body) {
            return Ok(PathBuf::from(path));
        }
    }

    Err(anyhow!(
        "Could not determine file path from issue #{} - {}",
        issue.number,
        issue.title
    ))
}

/// Simple heuristic to find file paths in text
fn find_file_path_in_text(text: &str) -> Option<String> {
    // Look for common file patterns: src/something.rs, path/to/file.ext, etc.
    let words: Vec<&str> = text.split_whitespace().collect();

    for word in words {
        // Remove markdown backticks if present
        let clean_word = word.trim_matches('`');

        // Check if it looks like a file path
        if clean_word.contains('/') && clean_word.contains('.') {
            // Basic validation - should have an extension
            if let Some(extension) = clean_word.split('.').last() {
                if extension.len() <= 10 && extension.chars().all(|c| c.is_alphanumeric()) {
                    return Some(clean_word.to_string());
                }
            }
        }
    }

    None
}

impl QCApprove {
    pub async fn from_interactive(milestones: &[Milestone], git_info: &GitInfo) -> Result<Self> {
        println!("✅ Welcome to GHQC Approve Mode!");

        // Select milestone (existing only)
        let milestone = prompt_existing_milestone(milestones)?;

        // Get issues for this milestone
        let issues = git_info.get_milestone_issues(&milestone).await?;

        // Filter to only show open issues (since we can only approve open issues)
        let open_issues: Vec<_> = issues
            .into_iter()
            .filter(|issue| matches!(issue.state, octocrab::models::IssueState::Open))
            .collect();

        if open_issues.is_empty() {
            bail!(
                "No open issues found in milestone '{}' to approve",
                milestone.title
            );
        }

        // Select issue by title
        let issue = prompt_issue(&open_issues)?;

        // Extract file path from issue - we need to determine which file this issue is about
        let file_path = extract_file_path_from_issue(&issue)?;

        // Get commits for this file
        let file_commits = git_info.file_commits(&file_path)?;

        if file_commits.is_empty() {
            bail!("No commits found for file: {}", file_path.display());
        }

        // Select single commit to approve
        let approved_commit = prompt_single_commit(
            &file_commits,
            "📝 Select commit to approve (press Enter for latest):",
        )?;

        // Prompt for optional note
        let note = prompt_note()?;

        // Display summary
        println!("\n✨ Creating approval with:");
        println!("   🎯 Milestone: {}", milestone.title);
        println!("   🎫 Issue: #{} - {}", issue.number, issue.title);
        println!("   📁 File: {}", file_path.display());
        println!("   📝 Commit: {}", approved_commit);
        if let Some(ref n) = note {
            println!("   💬 Note: {}", n);
        }
        println!();

        Ok(Self {
            file: file_path,
            commit: approved_commit,
            issue,
            note,
        })
    }

    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        approve_commit: Option<String>,
        note: Option<String>,
        milestones: &[Milestone],
        git_info: &GitInfo,
    ) -> Result<Self> {
        let milestone = milestones
            .iter()
            .find(|m| m.title == milestone_name)
            .ok_or(anyhow!("Milestone '{}' not found", milestone_name))?;

        let issues = git_info.get_milestone_issues(milestone).await?;

        let file_str = file.to_string_lossy();
        let issue = issues
            .into_iter()
            .find(|issue| {
                issue.title.contains(file_str.as_ref())
                    && matches!(issue.state, octocrab::models::IssueState::Open)
            })
            .ok_or(anyhow!(
                "No open issue found for file '{file_str}' in milestone '{milestone_name}'"
            ))?;

        let file_commits = git_info.file_commits(&file)?;

        if file_commits.is_empty() {
            bail!("There are no commits for the selected file");
        }

        let approved_commit =
            match approve_commit {
                Some(commit_str) => file_commits
                    .iter()
                    .find(|(c, _)| c.to_string().contains(&commit_str))
                    .ok_or(anyhow!(
                        "Provided commit does not correspond to any commits which edited this file"
                    ))?
                    .0,
                None => file_commits[0].0,
            };

        Ok(Self {
            file,
            commit: approved_commit,
            issue,
            note,
        })
    }
}

impl QCUnapprove {
    pub async fn from_interactive(milestones: &[Milestone], git_info: &GitInfo) -> Result<Self> {
        println!("🚫 Welcome to GHQC Unapprove Mode!");

        // Select milestone (existing only)
        let milestone = prompt_existing_milestone(milestones)?;

        // Get issues for this milestone
        let issues = git_info.get_milestone_issues(&milestone).await?;
        log::debug!(
            "Found {} total issues in milestone '{}'",
            issues.len(),
            milestone.title
        );

        // Filter to only show closed issues (since we can only unapprove closed issues)
        let closed_issues: Vec<_> = issues
            .into_iter()
            .filter(|issue| {
                let is_closed = matches!(issue.state, octocrab::models::IssueState::Closed);
                log::debug!(
                    "Issue #{}: '{}' (state: {:?}) -> closed: {}",
                    issue.number,
                    issue.title,
                    issue.state,
                    is_closed
                );
                is_closed
            })
            .collect();

        log::debug!(
            "Found {} closed issues after filtering",
            closed_issues.len()
        );

        if closed_issues.is_empty() {
            bail!(
                "No closed issues found in milestone '{}' to unapprove",
                milestone.title
            );
        }

        // Select issue by title
        let issue = prompt_issue(&closed_issues)?;

        // Prompt for reason
        use inquire::{Text, validator::Validation};
        let reason_input = Text::new("📝 Enter reason for unapproval:")
            .with_validator(|input: &str| {
                if input.trim().is_empty() {
                    Ok(Validation::Invalid("Reason cannot be empty".into()))
                } else {
                    Ok(Validation::Valid)
                }
            })
            .prompt()
            .map_err(|e| anyhow!("Input cancelled: {}", e))?;

        let reason = reason_input.trim().to_string();

        // Display summary
        println!("\n✨ Creating unapproval with:");
        println!("   🎯 Milestone: {}", milestone.title);
        println!("   🎫 Issue: #{} - {}", issue.number, issue.title);
        println!("   🚫 Reason: {}", reason);
        println!();

        Ok(Self { issue, reason })
    }

    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        reason: String,
        milestones: &[Milestone],
        git_info: &GitInfo,
    ) -> Result<Self> {
        let milestone = milestones
            .iter()
            .find(|m| m.title == milestone_name)
            .ok_or(anyhow!("Milestone '{}' not found", milestone_name))?;

        let issues = git_info.get_milestone_issues(milestone).await?;
        log::debug!(
            "Found {} total issues in milestone '{}'",
            issues.len(),
            milestone_name
        );

        let file_str = file.to_string_lossy();
        let issue = issues
            .into_iter()
            .find(|issue| {
                let title_matches = issue.title.contains(file_str.as_ref());
                let is_closed = matches!(issue.state, octocrab::models::IssueState::Closed);
                log::debug!(
                    "Issue #{}: '{}' (state: {:?}) -> title_match: {}, closed: {}",
                    issue.number,
                    issue.title,
                    issue.state,
                    title_matches,
                    is_closed
                );
                title_matches && is_closed
            })
            .ok_or(anyhow!(
                "No closed issue found for file '{file_str}' in milestone '{milestone_name}'"
            ))?;

        Ok(Self { issue, reason })
    }
}
