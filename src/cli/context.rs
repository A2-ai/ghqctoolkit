use anyhow::{Result, anyhow, bail};
use inquire::{Confirm, Text, validator::Validation};
use octocrab::models::{Milestone, issues::Issue};

use std::path::{Path, PathBuf};

use crate::{
    Configuration, DiskCache, GitFileOps, GitHelpers, GitHubReader, GitHubWriter, GitInfo,
    GitRepository, QCApprove, QCIssue, QCReview, QCUnapprove, RepoUser,
    cli::file_parser::{IssueUrlArg, RelevantFileArg},
    cli::interactive::{
        RelevantFileClassType, prompt_add_another_relevant_file, prompt_assignees,
        prompt_checklist, prompt_collaborators, prompt_commits, prompt_existing_milestone,
        prompt_file, prompt_include_previous_qc_diff, prompt_issue, prompt_milestone, prompt_note,
        prompt_relevant_description, prompt_relevant_file_class, prompt_relevant_file_path,
        prompt_relevant_file_source, prompt_single_commit, prompt_want_relevant_files,
        thread_commits,
    },
    comment::QCComment,
    create::{
        collaborator_override_for_policy, normalize_collaborator_entries, resolve_issue_people,
    },
    issue::IssueThread,
    relevant_files::{RelevantFile, RelevantFileClass},
    round::Placement,
};
use gix::ObjectId;

impl QCIssue {
    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        checklist_name: String,
        assignees: Option<Vec<String>>,
        add_collaborator: Vec<String>,
        remove_collaborator: Vec<String>,
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

        let milestone_issues = git_info.get_issues(Some(milestone.number as u64)).await?;
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
        let relevant_files = validate_and_convert_relevant_files(
            previous_qc,
            gating_qc,
            relevant_qc,
            relevant_file,
            git_info,
        )?;

        let authors = git_info.authors(&file)?;
        let configured_author = git_info.configured_author();
        let current_user = git_info.get_current_user().await?;
        let collaborator_additions =
            normalize_collaborator_entries(&add_collaborator).map_err(anyhow::Error::msg)?;
        let collaborator_removals =
            normalize_collaborator_entries(&remove_collaborator).map_err(anyhow::Error::msg)?;
        let should_include_collaborators = configuration.include_collaborators()
            || !collaborator_additions.is_empty()
            || !collaborator_removals.is_empty();
        let (_author, default_collaborators) = resolve_issue_people(
            configured_author.as_ref(),
            current_user.as_deref(),
            &authors,
            collaborator_override_for_policy(should_include_collaborators, None),
        );
        let collaborators = apply_collaborator_overrides(
            default_collaborators,
            collaborator_additions,
            collaborator_removals,
        );
        let (author, collaborators) = resolve_issue_people(
            configured_author.as_ref(),
            current_user.as_deref(),
            &authors,
            Some(collaborators),
        );

        let issue = QCIssue::new_without_git(
            &file,
            milestone.number as u64,
            git_info.commit()?,
            git_info.branch()?,
            author,
            collaborators,
            assignees,
            checklist,
            relevant_files,
        );

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
        let milestone_issues = git_info.get_issues(Some(milestone.number as u64)).await?;

        let file = prompt_file(project_dir, &milestone_issues)?;
        let checklist = prompt_checklist(&configuration)?;
        let assignees = prompt_assignees(&repo_users)?;
        let authors = git_info.authors(&file)?;
        let configured_author = git_info.configured_author();
        let current_user = git_info.get_current_user().await?;
        let (_, default_collaborators) = resolve_issue_people(
            configured_author.as_ref(),
            current_user.as_deref(),
            &authors,
            collaborator_override_for_policy(configuration.include_collaborators(), None),
        );
        let collaborators = if configuration.include_collaborators() {
            prompt_collaborators(&default_collaborators)?
        } else {
            Vec::new()
        };
        let (author, collaborators) = resolve_issue_people(
            configured_author.as_ref(),
            current_user.as_deref(),
            &authors,
            Some(collaborators),
        );

        // Prompt for relevant files
        let relevant_files = if prompt_want_relevant_files()? {
            // Fetch all issues (need for matching file paths to issues)
            let all_issues = git_info.get_issues(None).await?;

            let mut relevant_files = Vec::new();
            loop {
                let relevant_file_path = prompt_relevant_file_path(project_dir, &all_issues)?;

                // Find matching issues (where issue.title == file_path)
                let matching_issues: Vec<_> = all_issues
                    .iter()
                    .filter(|i| i.title == relevant_file_path.display().to_string())
                    .collect();

                let relevant_file = if matching_issues.is_empty() {
                    // No matching issues - must be File type with justification
                    let justification =
                        prompt_relevant_description(true)?.expect("justification required");
                    RelevantFile {
                        file_name: relevant_file_path,
                        class: RelevantFileClass::File { justification },
                    }
                } else {
                    // Has matching issues - let user choose
                    match prompt_relevant_file_source(
                        &relevant_file_path,
                        &matching_issues,
                        milestone.number as u64,
                    )? {
                        Some(issue) => {
                            let class_type = prompt_relevant_file_class()?;
                            let description = prompt_relevant_description(false)?;
                            // Extract issue.id for blocking relationships
                            let issue_id = Some(issue.id.0);
                            RelevantFile {
                                file_name: relevant_file_path,
                                class: match class_type {
                                    RelevantFileClassType::GatingQC => {
                                        RelevantFileClass::GatingQC {
                                            issue_number: issue.number,
                                            issue_id,
                                            description,
                                        }
                                    }
                                    RelevantFileClassType::PreviousQC => {
                                        let include_diff = prompt_include_previous_qc_diff()?;
                                        RelevantFileClass::PreviousQC {
                                            issue_number: issue.number,
                                            issue_id,
                                            description,
                                            include_diff,
                                        }
                                    }
                                    RelevantFileClassType::RelevantQC => {
                                        RelevantFileClass::RelevantQC {
                                            issue_number: issue.number,
                                            description,
                                        }
                                    }
                                },
                            }
                        }
                        None => {
                            // User chose File
                            let justification =
                                prompt_relevant_description(true)?.expect("justification required");
                            RelevantFile {
                                file_name: relevant_file_path,
                                class: RelevantFileClass::File { justification },
                            }
                        }
                    }
                };

                relevant_files.push(relevant_file);

                if !prompt_add_another_relevant_file()? {
                    break;
                }
            }
            relevant_files
        } else {
            Vec::new()
        };

        // Display summary
        println!("\n✨ Creating issue with:");
        println!("   📊 Milestone: {}", milestone_status);
        println!("   📁 File: {}", file.display());
        println!("   📋 Checklist: {}", checklist.name);
        if !assignees.is_empty() {
            println!("   👥 Assignees: {}", assignees.join(", "));
        }
        if !collaborators.is_empty() {
            println!("   🤝 Collaborators: {}", collaborators.join(", "));
        }
        if !relevant_files.is_empty() {
            println!("   🔗 Relevant files: {}", relevant_files.len());
        }
        println!();

        // Create the QCIssue
        let issue = QCIssue::new_without_git(
            &file,
            milestone.number as u64,
            git_info.commit()?,
            git_info.branch()?,
            author,
            collaborators,
            assignees,
            checklist,
            relevant_files,
        );

        Ok(issue)
    }
}

fn apply_collaborator_overrides(
    defaults: Vec<String>,
    additions: Vec<String>,
    removals: Vec<String>,
) -> Vec<String> {
    let removals = removals
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut collaborators = defaults
        .into_iter()
        .filter(|entry| !removals.contains(entry))
        .collect::<Vec<_>>();

    for addition in additions {
        if !collaborators.contains(&addition) {
            collaborators.push(addition);
        }
    }

    collaborators
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
    let mut process_issue_arg = |arg: IssueUrlArg, relevant_file: RelevantFile, flag_name: &str| {
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
    // Note: issue_id is None because we only have the URL in CLI args mode.
    // The ID will be fetched when creating blocking relationships in main.rs.
    for arg in previous_qc {
        let relevant = RelevantFile {
            file_name: PathBuf::from(format!("issue #{}", arg.issue_number)),
            class: RelevantFileClass::PreviousQC {
                issue_number: arg.issue_number,
                issue_id: None,
                description: arg.description.clone(),
                include_diff: arg.include_diff,
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
                issue_id: None,
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

/// Why nothing could be pointed at as "the current commit".
///
/// Two very different causes, and the old single message — "no commit could be located"
/// — was false for the first: the commits exist, this segment's walk just could not
/// reach them.
fn no_current_commit(issue_thread: &IssueThread, file: &Path) -> anyhow::Error {
    match issue_thread.active_segment().placement() {
        Placement::Unplaceable(reason) => {
            // One wording for every surface, per `UnplaceableReason::describe`.
            let reason = reason.describe();
            anyhow!(
                "{}'s current round could not be placed ({reason}); pass --current-commit",
                file.display()
            )
        }
        Placement::Placed => anyhow!("No commit could be located for file: {}", file.display()),
    }
}

/// The commit a defaulted `--current-commit` names.
///
/// The newest commit of the segment the issue is actually in: notifying about a commit
/// an earlier round already closed over would be a lie. On a fully approved, undrifted
/// issue that segment is the empty trailing gap, so it owns nothing — but the approval is
/// exactly the commit under discussion, and it is perfectly well known.
fn defaulted_current_commit(issue_thread: &IssueThread) -> Option<ObjectId> {
    issue_thread
        .latest_commit()
        .map(|commit| commit.hash)
        .or_else(|| issue_thread.last_approved_commit().copied())
}

/// The commit a defaulted `--previous-commit` compares against.
///
/// Without an explicit `--current-commit` the model's own answer stands: what the open
/// round last notified, or the commit it opened at. With one, that answer may be *newer*
/// than the commit named — the round already notified something further along — and the
/// diff would run backwards. The base then becomes the commit immediately older than
/// `current` in the thread's own ordering: the rest of its segment first, then the
/// bounding commit of the segment before it.
///
/// A base equal to `current` is dropped either way: a fresh round that has notified
/// nothing defaults to its own anchor, which with no drift *is* the current commit, and
/// `X...X` renders a comparison link with no diff.
fn defaulted_previous_commit(
    issue_thread: &IssueThread,
    current: &ObjectId,
    current_explicit: bool,
) -> Option<ObjectId> {
    let base = if current_explicit {
        // Deduped, so a boundary commit shared by two segments (D1) is found once, in
        // the newer-owning segment — the same frame the picker shows it in.
        let commits = thread_commits(issue_thread);
        match commits.iter().position(|(_, c)| c.hash == *current) {
            // The next entry is older by construction, so the diff cannot run backwards.
            Some(position) => match commits.get(position + 1) {
                Some((_, older)) => Some(older.hash),
                // Nothing the issue knows of is older than `current`, so the model's
                // base can only be newer unless it sits outside every segment.
                None => issue_thread
                    .next_notification_from()
                    .filter(|base| !commits.iter().any(|(_, c)| c.hash == *base)),
            },
            // A commit no segment owns cannot be ordered against them; the model's
            // answer is the only one available.
            None => issue_thread.next_notification_from(),
        }
    } else {
        issue_thread.next_notification_from()
    };
    base.filter(|base| base != current)
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

        // Create IssueThread to get the commits its segments own
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        // A commit named on the command line is looked up across every segment: the
        // user pointed at a commit, and which round owns it does not make it a
        // different commit.
        let commits = thread_commits(&issue_thread);

        if commits.is_empty() {
            return Err(anyhow!("No commits found for file: {}", file.display()));
        }

        let find = |commit_str: &str| {
            commits
                .iter()
                .find(|(_, c)| c.hash.to_string().contains(commit_str))
                .map(|(_, c)| c.hash)
                .ok_or(anyhow!(
                    "Provided commit does not correspond to any commits which edited this file"
                ))
        };

        let current_explicit = current_commit.is_some();
        let final_current_commit = match current_commit {
            Some(commit_str) => find(&commit_str)?,
            None => match defaulted_current_commit(&issue_thread) {
                Some(commit) => commit,
                None => return Err(no_current_commit(&issue_thread, &file)),
            },
        };

        let final_previous_commit = match previous_commit {
            Some(commit_str) => Some(find(&commit_str)?),
            None => {
                defaulted_previous_commit(&issue_thread, &final_current_commit, current_explicit)
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
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

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
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

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

        // Create IssueThread to get the commits its segments own
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        let commits = thread_commits(&issue_thread);

        if commits.is_empty() {
            bail!("No commits found for file: {}", file_path.display());
        }

        // Select single commit to approve with status annotations. Every commit the
        // issue knows about is offered — an approval may name an older one — but the
        // cursor starts on the active segment's newest, which is what is under review.
        let default_position = issue_thread
            .latest_commit()
            .and_then(|latest| commits.iter().position(|(_, c)| c.hash == latest.hash))
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
        let commits = thread_commits(&issue_thread);

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
                    .find(|(_, c)| c.hash.to_string().contains(&commit_str))
                    .ok_or(anyhow!(
                        "Provided commit does not correspond to any commits which edited this file"
                    ))?
                    .1
                    .hash,
                // The newest commit of the segment under review; with nothing placed
                // there — an empty trailing gap — the newest the issue knows of.
                None => issue_thread
                    .latest_commit()
                    .map(|latest| latest.hash)
                    .unwrap_or(commits[0].1.hash),
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
        println!(
            "   This says a past approval was wrong. If the file simply changed again and needs \
             another QC pass, use `ghqc issue new-round` instead."
        );

        // Select milestone (existing only)
        let milestone = prompt_existing_milestone(milestones)?;

        // Get issues for this milestone
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;
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
                "No closed issues found in milestone '{}' that could be unapproved",
                milestone.title
            );
        }

        // Select issue by title
        let issue = prompt_issue(&closed_issues)?;

        // Prompt for reason
        let reason_input = Text::new("📝 Why is this issue being unapproved?")
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
        println!("\n✨ Unapproving with:");
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
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

        // Select issue by title
        let issue = prompt_issue(&issues)?;

        // Extract file path from issue - we need to determine which file this issue is about
        let file_path = PathBuf::from(&issue.title);

        // Create IssueThread to get QC-tracked commits for status/metadata
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;

        // A review compares the working directory against a commit the issue knows
        // about, wherever it sits: no segment scope applies to picking one.
        let commits = thread_commits(&issue_thread);
        if commits.is_empty() {
            return Err(anyhow!(
                "No commits found for file: {}",
                file_path.display()
            ));
        }

        // Set default position to HEAD commit if it exists in the file's commit history
        let default_position = match git_info.commit() {
            Ok(head_str) => {
                // Look for HEAD commit in the file's commit history
                if let Some(head_position) = commits
                    .iter()
                    .position(|(_, c)| c.hash.to_string().starts_with(&head_str[..8]))
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
        let stash_after_review =
            inquire::Confirm::new("Stash local changes for this file after posting review?")
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
        if !stash_after_review {
            println!("   📦 Auto-stash disabled");
        }
        println!();

        Ok(Self {
            file: file_path,
            issue,
            commit: commit_hash,
            note,
            no_diff,
            stash_after_review,
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
        stash_after_review: bool,
    ) -> Result<Self> {
        let issue = find_issue(&milestone_name, &file, milestones, git_info).await?;

        // Create IssueThread to get commits from the issue's specific branch
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;

        let commits = thread_commits(&issue_thread);
        if commits.is_empty() {
            return Err(anyhow!("No commits found for file: {}", file.display()));
        }

        let default_commit = || {
            Self::get_default_commit(git_info, &issue_thread).ok_or(anyhow!(
                "No commit could be located to review file '{}' against",
                file.display()
            ))
        };
        let final_commit = match commit {
            Some(commit_str) => {
                // Try to find the commit in the file's history first
                match commits
                    .iter()
                    .find(|(_, c)| c.hash.to_string().contains(&commit_str))
                    .map(|(_, c)| c.hash)
                {
                    Some(found) => found,
                    None => {
                        // If not found in file history, try to parse as ObjectId
                        use std::str::FromStr;
                        match gix::ObjectId::from_str(&commit_str) {
                            Ok(parsed) => parsed,
                            Err(_) => {
                                log::warn!(
                                    "Could not parse commit '{}', using fallback logic",
                                    commit_str
                                );
                                // Use same fallback chain as interactive mode
                                default_commit()?
                            }
                        }
                    }
                }
            }
            None => {
                // Use fallback chain to find the best default commit
                default_commit()?
            }
        };

        Ok(Self {
            file,
            issue,
            commit: final_commit,
            note,
            no_diff,
            stash_after_review,
            working_dir: git_info.repository_path.clone(),
        })
    }

    /// Get default commit with robust fallback chain:
    /// 1. HEAD commit from repository
    /// 2. The newest commit of the segment the issue is in
    ///
    /// `None` when neither is available, which leaves the caller to say so rather
    /// than review against a commit nobody chose.
    fn get_default_commit(git_info: &GitInfo, issue_thread: &IssueThread) -> Option<gix::ObjectId> {
        // Try HEAD commit from repository
        if let Ok(head_str) = git_info.commit() {
            if let Ok(head_oid) = std::str::FromStr::from_str(&head_str) {
                return Some(head_oid);
            }
        }

        // Use latest_commit from issue thread as fallback
        issue_thread.latest_commit().map(|commit| commit.hash)
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

    let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::IssueCommit;
    use crate::round::{
        ChecklistSource, Gap, GapContinuity, Round, RoundEvent, RoundOpen, RoundState, Segment,
        UnplaceableReason,
    };
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";
    const D: &str = "ddddddd000000000000000000000000000000004";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    /// `shas` oldest-first for readability, reversed into the newest-first order a
    /// segment stores its commits in.
    fn commits(shas: &[&str]) -> Vec<IssueCommit> {
        shas.iter()
            .rev()
            .map(|sha| IssueCommit {
                hash: oid(sha),
                message: format!("commit {sha}"),
                file_changed: true,
            })
            .collect()
    }

    fn round(index: u32, opened_at: &str, closed_at: Option<&str>, own: &[&str]) -> Round {
        Round {
            index,
            opened_at: oid(opened_at),
            branch: "main".to_string(),
            opened: RoundOpen::IssueCreated,
            checklist: ChecklistSource::IssueBody,
            checklist_name: None,
            state: match closed_at {
                Some(commit) => RoundState::Closed {
                    commit: oid(commit),
                    by: "reviewer".to_string(),
                    at: chrono::Utc::now(),
                    comment_index: 0,
                    comment_id: None,
                    comment_url: None,
                },
                None => RoundState::Open,
            },
            events: Vec::new(),
            retractions: Vec::new(),
            extensions: Vec::new(),
            commits: commits(own),
            placement: Placement::Placed,
        }
    }

    fn gap(own: &[&str]) -> Segment {
        Segment::Gap(Gap {
            branch: "main".to_string(),
            commits: commits(own),
            continuity: GapContinuity::Linear,
            placement: Placement::Placed,
        })
    }

    fn notification(sha: &str) -> RoundEvent {
        RoundEvent::Notification {
            commit: oid(sha),
            by: "author".to_string(),
            at: chrono::Utc::now(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn thread(segments: Vec<Segment>) -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "m1".to_string(),
            open: true,
            blocking_qcs: Vec::new(),
            segments,
            anomalies: Vec::new(),
        }
    }

    /// The canonical approved state: `[Initial QC(closed at B), Gap(empty)]`. The active
    /// segment owns nothing, so `ghqc comment` used to refuse with "no commit could be
    /// located" — about a commit the record names outright.
    #[test]
    fn an_approved_undrifted_issue_defaults_to_its_approval() {
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[]),
        ]);
        assert!(thread.latest_commit().is_none(), "the state under test");

        assert_eq!(defaulted_current_commit(&thread), Some(oid(B)));
    }

    /// Drift after an approval is what a comment is usually about, so the gap's newest
    /// commit still wins over the older approval.
    #[test]
    fn drift_after_an_approval_outranks_the_approval() {
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[C]),
        ]);
        assert_eq!(defaulted_current_commit(&thread), Some(oid(C)));
    }

    /// A round under review is what the comment is about — never the approval an
    /// *earlier* round granted, which the open round supersedes.
    #[test]
    fn an_open_round_defaults_to_its_own_newest_commit() {
        let mut second = round(2, C, None, &[C, D]);
        second.events.push(notification(C));
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[]),
            Segment::Round(second),
        ]);
        assert_eq!(defaulted_current_commit(&thread), Some(oid(D)));
    }

    /// An unplaceable active segment is a different failure from an issue with no known
    /// commits at all, and only one of the two is remedied by naming a commit.
    #[test]
    fn an_unplaceable_active_segment_says_so_and_names_the_flag() {
        let mut only = round(1, A, None, &[]);
        only.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let thread = thread(vec![Segment::Round(only)]);

        assert_eq!(defaulted_current_commit(&thread), None);
        let message = no_current_commit(&thread, Path::new("src/main.rs")).to_string();
        assert_eq!(
            message,
            "src/main.rs's current round could not be placed (its branch is unavailable \
             locally); pass --current-commit"
        );
    }

    /// With no `--current-commit` the model's own base stands: what the open round last
    /// notified.
    #[test]
    fn a_defaulted_current_commit_keeps_the_models_base() {
        let mut second = round(2, C, None, &[C, D]);
        second.events.push(notification(C));
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[]),
            Segment::Round(second),
        ]);

        assert_eq!(
            defaulted_previous_commit(&thread, &oid(D), false),
            Some(oid(C)),
            "the commit the round last notified"
        );
    }

    /// The backwards case: the round has already notified `D`, and the user names the
    /// older `C` explicitly. The model's base would be newer than the commit being
    /// described, so the diff would run backwards; the base must be re-derived from `C`.
    #[test]
    fn an_explicit_older_current_commit_re_bases_the_default() {
        let mut second = round(2, C, None, &[C, D]);
        second.events.push(notification(D));
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[]),
            Segment::Round(second),
        ]);
        assert_eq!(
            thread.next_notification_from(),
            Some(oid(D)),
            "the model's base is newer than the commit named — the bug"
        );

        let base = defaulted_previous_commit(&thread, &oid(C), true)
            .expect("C has a predecessor to compare against");
        assert_ne!(base, oid(C), "never a comparison of a commit with itself");
        assert_eq!(
            base,
            oid(B),
            "the bounding commit of the segment before C's, which is older than C"
        );

        // Older, stated as the flat ordering states it: further down the newest-first list.
        let flat = thread_commits(&thread);
        let position = |sha: ObjectId| flat.iter().position(|(_, c)| c.hash == sha).unwrap();
        assert!(position(base) > position(oid(C)), "the diff runs forwards");
    }

    /// A fresh round that has notified nothing defaults its base to its own anchor, which
    /// with no drift *is* the current commit. `X...X` renders a comparison link with no
    /// diff, so there is nothing to compare and the comment says so.
    #[test]
    fn a_base_equal_to_the_current_commit_is_dropped() {
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[]),
            Segment::Round(round(2, C, None, &[C])),
        ]);
        assert_eq!(thread.next_notification_from(), Some(oid(C)));

        assert_eq!(defaulted_previous_commit(&thread, &oid(C), false), None);
    }

    /// D1: round 2 opened at the commit Initial QC closed on, so both own it. Deduped, it
    /// is found once — in round 2's frame — and its predecessor is the one before it in
    /// the thread, never the other copy of itself.
    #[test]
    fn a_shared_boundary_commit_re_bases_onto_a_genuinely_older_commit() {
        let thread = thread(vec![
            Segment::Round(round(1, A, Some(B), &[A, B])),
            gap(&[]),
            Segment::Round(round(2, B, None, &[B])),
        ]);

        assert_eq!(
            defaulted_previous_commit(&thread, &oid(B), true),
            Some(oid(A)),
        );
    }

    /// The oldest commit the issue knows of has nothing older to compare against, and the
    /// model's base can only be newer. Better no comparison than a backwards one.
    #[test]
    fn the_oldest_commit_gets_no_base_rather_than_a_newer_one() {
        let mut only = round(1, A, None, &[A, B]);
        only.events.push(notification(B));
        let thread = thread(vec![Segment::Round(only)]);

        assert_eq!(defaulted_previous_commit(&thread, &oid(A), true), None);
    }
}
