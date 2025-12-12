use anyhow::{Result, anyhow, bail};
use inquire::{Confirm, Text, validator::Validation};
use octocrab::models::{Milestone, issues::Issue};

use std::path::{Path, PathBuf};

use crate::{
    Configuration, DiskCache, GitHubReader, GitHubWriter, GitHelpers, GitInfo, GitRepository,
    QCApprove, QCIssue, QCReview, QCUnapprove, RepoUser,
    cli::file_parser::{IssueUrlArg, RelevantFileArg},
    cli::interactive::{
        prompt_assignees, prompt_checklist, prompt_commits, prompt_existing_milestone, prompt_file,
        prompt_issue, prompt_milestone, prompt_note, prompt_single_commit,
    },
    comment::QCComment,
    issue::IssueThread,
    relevant_files::{RelevantFile, RelevantFileClass},
};

impl QCIssue {
    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        checklist_name: String,
        assignees: Option<Vec<String>>,
        description: Option<String>,
        previous_qc: Vec<IssueUrlArg>,
        gating_qc: Vec<IssueUrlArg>,
        relevant_qc: Vec<IssueUrlArg>,
        relevant_file: Vec<RelevantFileArg>,
        milestones: Vec<Milestone>,
        repo_users: &[RepoUser],
        configuration: Configuration,
        git_info: &GitInfo,
    ) -> Result<Self> {
        let milestone = if let Some(m) = milestones.into_iter().find(|m| m.title == milestone_name)
        {
            log::debug!("Found existing milestone {}", m.number);
            m
        } else {
            git_info
                .create_milestone(&milestone_name, &description)
                .await?
        };

        let milestone_issues = git_info.get_milestone_issues(&milestone).await?;
        if milestone_issues
            .iter()
            .any(|i| i.title == file.display().to_string())
        {
            bail!("File already has a corresponding issue within the milestone");
        }

        let assignees = if let Some(assignees_vec) = assignees {
            assignees_vec
                .into_iter()
                .filter(|a| {
                    if repo_users.iter().any(|r| &r.login == a) {
                        true
                    } else {
                        log::warn!("Login {a} is not a valid assignee");
                        false
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let checklist = configuration
            .checklists
            .get(&checklist_name)
            .ok_or(anyhow!("No checklist named {checklist_name}"))?
            .clone();

        // Validate and convert issue URL arguments to RelevantFile structs
        let relevant_files =
            validate_and_convert_relevant_files(previous_qc, gating_qc, relevant_qc, relevant_file, git_info)?;

        let issue = QCIssue::new(
            file,
            git_info,
            milestone.number as u64,
            assignees,
            checklist,
            relevant_files,
        )?;

        Ok(issue)
    }

    pub async fn from_interactive(
        project_dir: &PathBuf,
        milestones: Vec<Milestone>,
        configuration: Configuration,
        git_info: &GitInfo,
        repo_users: &[RepoUser],
    ) -> Result<Self> {
        println!("🚀 Welcome to GHQC Interactive Mode!");

        // Interactive prompts
        let milestone_status = prompt_milestone(milestones)?;

        let milestone = milestone_status.determine_milestone(git_info).await?;
        let milestone_issues = git_info.get_milestone_issues(milestone.as_ref()).await?;

        let file = prompt_file(project_dir, &milestone_issues)?;
        let checklist = prompt_checklist(&configuration)?;
        let assignees = prompt_assignees(&repo_users)?;

        // Display summary
        println!("\n✨ Creating issue with:");
        println!("   📊 Milestone: {}", milestone_status);
        println!("   📁 File: {}", file.display());
        println!("   📋 Checklist: {}", checklist.name);
        if !assignees.is_empty() {
            println!("   👥 Assignees: {}", assignees.join(", "));
        }
        println!();

        // Create the QCIssue
        let issue = QCIssue::new(
            file,
            git_info,
            milestone.number as u64,
            assignees,
            checklist,
            Vec::new(), // relevant_files not supported in interactive mode yet
        )?;

        Ok(issue)
    }
}

/// Validates and converts CLI relevant file arguments to RelevantFile structs.
/// Collects all validation errors and returns them together.
fn validate_and_convert_relevant_files(
    previous_qc: Vec<IssueUrlArg>,
    gating_qc: Vec<IssueUrlArg>,
    relevant_qc: Vec<IssueUrlArg>,
    relevant_file: Vec<RelevantFileArg>,
    git_info: &GitInfo,
) -> Result<Vec<RelevantFile>> {
    let mut result = Vec::new();
    let mut errors = Vec::new();

    // Helper to validate issue URL and add to results or errors
    let mut process_issue_arg =
        |arg: IssueUrlArg, relevant_file: RelevantFile, flag_name: &str| {
            let expected_url = git_info.issue_url(arg.issue_number);
            if arg.url != expected_url {
                errors.push(format!(
                    "{}: Issue URL '{}' does not match expected repository URL '{}'",
                    flag_name, arg.url, expected_url
                ));
            } else {
                result.push(relevant_file);
            }
        };

    // Process previous QC issues
    for arg in previous_qc {
        let relevant = RelevantFile {
            file_name: PathBuf::from(format!("issue #{}", arg.issue_number)),
            class: RelevantFileClass::PreviousQC {
                issue_number: arg.issue_number,
                description: arg.description.clone(),
            },
        };
        process_issue_arg(arg, relevant, "--previous-qc");
    }

    // Process gating QC issues
    for arg in gating_qc {
        let relevant = RelevantFile {
            file_name: PathBuf::from(format!("issue #{}", arg.issue_number)),
            class: RelevantFileClass::GatingQC {
                issue_number: arg.issue_number,
                description: arg.description.clone(),
            },
        };
        process_issue_arg(arg, relevant, "--gating-qc");
    }

    // Process relevant QC issues
    for arg in relevant_qc {
        let relevant = RelevantFile {
            file_name: PathBuf::from(format!("issue #{}", arg.issue_number)),
            class: RelevantFileClass::RelevantQC {
                issue_number: arg.issue_number,
                description: arg.description.clone(),
            },
        };
        process_issue_arg(arg, relevant, "--relevant-qc");
    }

    // Process relevant files (validate file exists in repository)
    for arg in relevant_file {
        if !arg.file.exists() {
            errors.push(format!(
                "--relevant-file: File '{}' does not exist",
                arg.file.display()
            ));
        } else {
            result.push(RelevantFile {
                file_name: arg.file,
                class: RelevantFileClass::File {
                    justification: arg.justification,
                },
            });
        }
    }

    // Return all errors if any were found
    if !errors.is_empty() {
        bail!("Validation errors:\n  - {}", errors.join("\n  - "));
    }

    Ok(result)
}

impl QCComment {
    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        current_commit: Option<String>,
        previous_commit: Option<String>,
        note: Option<String>,
        milestones: &[Milestone],
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
        no_diff: bool,
    ) -> Result<Self> {
        let issue = find_issue(&milestone_name, &file, milestones, git_info).await?;

        // Create IssueThread to get commits from the issue's specific branch
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        let commits = &issue_thread.commits;

        if commits.is_empty() {
            return Err(anyhow!("No commits found for file: {}", file.display()));
        }

        let final_current_commit =
            match current_commit {
                Some(commit_str) => commits
                    .iter()
                    .find(|c| c.hash.to_string().contains(&commit_str))
                    .ok_or(anyhow!(
                        "Provided commit does not correspond to any commits which edited this file"
                    ))?
                    .hash,
                None => {
                    // Default to most recent commit for this file (first in chronological order)
                    commits[0].hash
                }
            };

        let final_previous_commit = match previous_commit {
            Some(commit_str) => {
                // Parse the provided commit string into ObjectId
                Some(
                    commits
                        .iter()
                        .find(|c| c.hash.to_string().contains(&commit_str))
                        .ok_or(anyhow!("Provided commit does not correspond to any commits which edited this file"))?
                        .hash
                )
            }
            None => {
                // Default to second most recent commit if it exists
                if commits.len() > 1 {
                    Some(commits[1].hash)
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

    pub async fn from_interactive(
        milestones: &[Milestone],
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
    ) -> Result<Self> {
        println!("💬 Welcome to GHQC Comment Mode!");

        // Select milestone (existing only)
        let milestone = prompt_existing_milestone(milestones)?;

        // Get issues for this milestone
        let issues = git_info.get_milestone_issues(&milestone).await?;

        // Select issue by title
        let issue = prompt_issue(&issues)?;

        // Extract file path from issue - we need to determine which file this issue is about
        let file_path = PathBuf::from(&issue.title);

        // Create IssueThread to get commits from the issue's specific branch
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        // Select commits for comparison with status annotations
        let (current_commit, previous_commit) = prompt_commits(&issue_thread)?;

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

impl QCApprove {
    pub async fn from_interactive(
        milestones: &[Milestone],
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
    ) -> Result<Self> {
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
        let file_path = PathBuf::from(&issue.title);

        // Create IssueThread to get commits from the issue's specific branch
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        let commits = &issue_thread.commits;

        if commits.is_empty() {
            bail!("No commits found for file: {}", file_path.display());
        }

        // Select single commit to approve with status annotations
        // Default to latest_commit position, otherwise use position 0 (most recent file change)
        let latest = issue_thread.latest_commit();
        let default_position = issue_thread
            .commits
            .iter()
            .position(|c| c.hash == latest.hash)
            .unwrap_or(0);

        let approved_commit = prompt_single_commit(
            &issue_thread,
            "📝 Select commit to approve (press Enter for latest):",
            default_position,
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
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
    ) -> Result<Self> {
        let issue = find_issue(&milestone_name, &file, milestones, git_info).await?;
        if issue.state == octocrab::models::IssueState::Closed {
            bail!("")
        }

        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        let commits = &issue_thread.commits;

        if commits.is_empty() {
            bail!(
                "No open issue found for file '{}' in milestone '{milestone_name}'",
                file.display()
            )
        }

        let approved_commit =
            match approve_commit {
                Some(commit_str) => commits
                    .iter()
                    .find(|c| c.hash.to_string().contains(&commit_str))
                    .ok_or(anyhow!(
                        "Provided commit does not correspond to any commits which edited this file"
                    ))?
                    .hash,
                None => commits[0].hash,
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
        let issue = find_issue(&milestone_name, &file, milestones, git_info).await?;
        if issue.state == octocrab::models::IssueState::Closed {
            bail!(
                "No closed issue found for file '{}' in milestone '{milestone_name}'",
                file.display()
            )
        }

        Ok(Self { issue, reason })
    }
}

impl QCReview {
    pub async fn from_interactive(
        milestones: Vec<Milestone>,
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
    ) -> Result<Self> {
        println!("📝 Welcome to GHQC Review Mode!");

        // Select milestone (existing only)
        let milestone = prompt_existing_milestone(&milestones)?;

        // Get issues for this milestone
        let issues = git_info.get_milestone_issues(&milestone).await?;

        // Select issue by title
        let issue = prompt_issue(&issues)?;

        // Extract file path from issue - we need to determine which file this issue is about
        let file_path = PathBuf::from(&issue.title);

        // Create IssueThread to get QC-tracked commits for status/metadata
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;

        if issue_thread.commits.is_empty() {
            return Err(anyhow!(
                "No commits found for file: {}",
                file_path.display()
            ));
        }

        // Set default position to HEAD commit if it exists in the file's commit history
        let default_position = match git_info.commit() {
            Ok(head_str) => {
                // Look for HEAD commit in the file's commit history
                if let Some(head_position) = issue_thread
                    .commits
                    .iter()
                    .position(|c| c.hash.to_string().starts_with(&head_str[..8]))
                {
                    // HEAD is in file's commit history - use it as default selection
                    head_position
                } else {
                    // HEAD is not in the file's commit history - this is an error
                    return Err(anyhow!(
                        "Cannot review: HEAD commit '{}' is not in the known git history for file '{}'.\n\
                        \n\
                        This means you're on a branch that doesn't affect this file, or the file \n\
                        hasn't been modified in your current branch.\n\
                        \n\
                        You may need to:\n\
                        1. Switch to the correct branch for this file\n\
                        2. Ensure this file has been modified in a tracked commit\n\
                        3. Check that you're in the right repository",
                        &head_str[..8],
                        file_path.display()
                    ));
                }
            }
            Err(_) => {
                return Err(anyhow!("Could not determine HEAD commit from repository"));
            }
        };

        let commit_hash = prompt_single_commit(
            &issue_thread,
            "📝 Select commit to compare against working directory:",
            default_position,
        )?;

        let note = prompt_note()?;
        let no_diff = !inquire::Confirm::new("Include diff between commit and working directory?")
            .with_default(true)
            .prompt()?;

        println!();
        println!("📝 QC Review Summary:");
        println!("   📁 File: {}", file_path.display());
        println!("   🏷️  Issue: #{} - {}", issue.number, issue.title);
        println!("   📋 Milestone: {}", milestone.title);
        println!("   🔗 Comparing against commit: {}", commit_hash);
        if let Some(note) = &note {
            println!("   📝 Note: {}", note);
        }
        if no_diff {
            println!("   ⚠️  Diff generation disabled");
        }
        println!();

        Ok(Self {
            file: file_path,
            issue,
            commit: commit_hash,
            note,
            no_diff,
            working_dir: git_info.repository_path.clone(),
        })
    }

    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        commit: Option<String>,
        note: Option<String>,
        milestones: &[Milestone],
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
        no_diff: bool,
    ) -> Result<Self> {
        let issue = find_issue(&milestone_name, &file, milestones, git_info).await?;

        // Create IssueThread to get commits from the issue's specific branch
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;

        if issue_thread.commits.is_empty() {
            return Err(anyhow!("No commits found for file: {}", file.display()));
        }

        let final_commit = match commit {
            Some(commit_str) => {
                // Try to find the commit in the file's history first
                issue_thread
                    .commits
                    .iter()
                    .find(|c| c.hash.to_string().contains(&commit_str))
                    .map(|c| c.hash)
                    .unwrap_or_else(|| {
                        // If not found in file history, try to parse as ObjectId
                        use std::str::FromStr;
                        gix::ObjectId::from_str(&commit_str).unwrap_or_else(|_| {
                            log::warn!(
                                "Could not parse commit '{}', using fallback logic",
                                commit_str
                            );
                            // Use same fallback chain as interactive mode
                            Self::get_default_commit(git_info, &issue_thread)
                        })
                    })
            }
            None => {
                // Use fallback chain to find the best default commit
                Self::get_default_commit(git_info, &issue_thread)
            }
        };

        Ok(Self {
            file,
            issue,
            commit: final_commit,
            note,
            no_diff,
            working_dir: git_info.repository_path.clone(),
        })
    }

    /// Get default commit with robust fallback chain:
    /// 1. HEAD commit from repository
    /// 2. Latest commit from issue thread
    /// 3. Most recent file commit (position 0)
    fn get_default_commit(git_info: &GitInfo, issue_thread: &IssueThread) -> gix::ObjectId {
        // Try HEAD commit from repository
        if let Ok(head_str) = git_info.commit() {
            if let Ok(head_oid) = std::str::FromStr::from_str(&head_str) {
                return head_oid;
            }
        }

        // Use latest_commit from issue thread as fallback
        issue_thread.latest_commit().hash
    }
}

pub async fn find_issue(
    milestone_name: &str,
    file: impl AsRef<Path>,
    milestones: &[Milestone],
    git_info: &impl GitHubReader,
) -> Result<Issue> {
    let milestone = milestones
        .iter()
        .find(|m| m.title == milestone_name)
        .ok_or(anyhow!("Milestone '{}' not found", milestone_name))?;

    let issues = git_info.get_milestone_issues(milestone).await?;

    let file_str = file.as_ref().to_string_lossy();
    let issue = issues
        .into_iter()
        .find(|issue| {
            issue.title.contains(file_str.as_ref())
                && matches!(issue.state, octocrab::models::IssueState::Open)
        })
        .ok_or(anyhow!(
            "No open issue found for file '{file_str}' in milestone '{milestone_name}'"
        ))?;
    Ok(issue)
}
