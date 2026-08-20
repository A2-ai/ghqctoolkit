use anyhow::{Result, bail};
use gix::ObjectId;
use inquire::{
    Autocomplete, Confirm, CustomUserError, MultiSelect, Select, Text, validator::Validation,
};
use octocrab::models::{Milestone, issues::Issue};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::{fmt, fs};

use crate::GitHubWriter;
use crate::{
    Configuration, ContextPosition, QCContext,
    configuration::Checklist,
    create::normalize_collaborator_entry,
    git::RepoUser,
    issue::IssueCommit,
    issue::IssueThread,
    round::{RoundEvent, Segment},
};

/// Enum representing the type of relevant file class for interactive selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelevantFileClassType {
    GatingQC,
    PreviousQC,
    RelevantQC,
}

pub enum MilestoneStatus {
    Existing(Milestone),
    New(String, Option<String>), // (name, description)
}

impl fmt::Display for MilestoneStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New(name, _) => write!(f, "{name} (new)"),
            Self::Existing(milestone) => {
                write!(f, "{} (existing: #{})", milestone.title, milestone.number)
            }
        }
    }
}

impl MilestoneStatus {
    pub(crate) async fn determine_milestone<'a>(
        &'a self,
        git_info: &impl GitHubWriter,
    ) -> Result<Cow<'a, Milestone>> {
        match self {
            Self::Existing(milestone) => Ok(Cow::Borrowed(milestone)),
            Self::New(milestone_name, description) => {
                let m = git_info
                    .create_milestone(milestone_name, description)
                    .await?;
                log::debug!(
                    "Created milestone '{}' with ID: {}",
                    milestone_name,
                    m.number
                );
                Ok(Cow::Owned(m))
            }
        }
    }
}

/// Modular milestone selection - allows creation of new milestones
pub fn prompt_milestone(milestones: Vec<Milestone>) -> Result<MilestoneStatus> {
    let mut options = vec!["📝 Create new milestone".to_string()];
    let mut open_milestones: Vec<&Milestone> = milestones
        .iter()
        .filter(|m| m.state.as_deref() == Some("open"))
        .collect();
    open_milestones.sort_by(|a, b| b.number.cmp(&a.number));
    let milestone_titles: Vec<String> = open_milestones
        .iter()
        .map(|m| format!("🎯 {}", m.title))
        .collect();

    options.extend(milestone_titles);

    if options.len() == 1 {
        println!("ℹ️  No open milestones found. You'll need to create a new one.");
    }

    let selection = Select::new("Select or create a milestone:", options)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    if selection.starts_with("📝") {
        let existing_names: Vec<String> = milestones.iter().map(|m| m.title.clone()).collect();

        let new_milestone = Text::new("Enter new milestone name:")
            .with_validator(move |input: &str| {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    Ok(Validation::Invalid("Milestone name cannot be empty".into()))
                } else if existing_names.contains(&trimmed.to_string()) {
                    Ok(Validation::Invalid(
                        format!(
                            "Milestone '{}' already exists. Please choose a different name.",
                            trimmed
                        )
                        .into(),
                    ))
                } else {
                    Ok(Validation::Valid)
                }
            })
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let description = Text::new("Enter milestone description (optional):")
            .with_default("")
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let description = if description.trim().is_empty() {
            None
        } else {
            Some(description.trim().to_string())
        };

        Ok(MilestoneStatus::New(
            new_milestone.trim().to_string(),
            description,
        ))
    } else {
        // Find the selected milestone and return its ID
        let milestone_title = selection.strip_prefix("🎯 ").unwrap_or(&selection);
        let milestone = milestones
            .into_iter()
            .find(|m| m.title == milestone_title)
            .expect("selected milestone to exist");
        Ok(MilestoneStatus::Existing(milestone))
    }
}

/// Modular milestone selection - only existing milestones (for comments)
pub fn prompt_existing_milestone(milestones: &[Milestone]) -> Result<Milestone> {
    let mut open_milestones: Vec<_> = milestones
        .iter()
        .filter(|m| m.state.as_deref() == Some("open"))
        .collect();
    open_milestones.sort_by(|a, b| b.number.cmp(&a.number));

    if open_milestones.is_empty() {
        return Err(anyhow::anyhow!(
            "No open milestones found. Please create a milestone first or ensure there are open milestones with issues."
        ));
    }

    let milestone_titles: Vec<String> = open_milestones
        .iter()
        .map(|m| format!("🎯 {}", m.title))
        .collect();

    let selection = Select::new("Select a milestone:", milestone_titles)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let milestone_title = selection.strip_prefix("🎯 ").unwrap_or(&selection);
    if let Some(milestone) = milestones.iter().find(|m| m.title == milestone_title) {
        Ok(milestone.clone())
    } else {
        Err(anyhow::anyhow!("Selected milestone not found"))
    }
}

pub fn prompt_file(current_dir: &PathBuf, issues: &[Issue]) -> Result<PathBuf> {
    // Extract file paths from existing issues to mark as unavailable
    let existing_issue_files: Vec<String> =
        issues.iter().map(|issue| issue.title.clone()).collect();

    #[derive(Clone)]
    struct FileCompleter {
        current_dir: PathBuf,
        existing_issue_files: Vec<String>,
    }

    impl Autocomplete for FileCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
            let input = input.trim();
            let mut suggestions = Vec::new();

            let (base_path, search_term) = if input.contains('/') {
                let mut parts = input.rsplitn(2, '/');
                let filename = parts.next().unwrap_or("");
                let dir_path = parts.next().unwrap_or("");
                (self.current_dir.join(dir_path), filename)
            } else {
                (self.current_dir.clone(), input)
            };

            if let Ok(entries) = fs::read_dir(&base_path) {
                let mut files = Vec::new();
                let mut dirs = Vec::new();

                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        // Skip hidden files/directories
                        if name.starts_with('.') {
                            continue;
                        }

                        if name.to_lowercase().starts_with(&search_term.to_lowercase()) {
                            let relative_path = if input.contains('/') {
                                let dir_part = input.rsplitn(2, '/').nth(1).unwrap_or("");
                                format!("{}/{}", dir_part, name)
                            } else {
                                name.clone()
                            };

                            if entry.path().is_file() {
                                // Check if this file already has an issue
                                if self.existing_issue_files.contains(&relative_path) {
                                    // Mark as unavailable with gray styling
                                    files.push(format!("🚫 {} (already has issue)", relative_path));
                                } else {
                                    files.push(relative_path);
                                }
                            } else if entry.path().is_dir() {
                                // Add trailing slash to indicate directory
                                dirs.push(format!("{}/", relative_path));
                            }
                        }
                    }
                }

                // Sort directories and files separately, then combine
                dirs.sort();
                files.sort();
                suggestions.extend(dirs);
                suggestions.extend(files);
            }

            Ok(suggestions)
        }

        fn get_completion(
            &mut self,
            _input: &str,
            highlighted_suggestion: Option<String>,
        ) -> std::result::Result<inquire::autocompletion::Replacement, CustomUserError> {
            Ok(match highlighted_suggestion {
                Some(suggestion) => {
                    // If the suggestion is marked as unavailable, don't allow completion
                    if suggestion.starts_with("🚫 ") {
                        inquire::autocompletion::Replacement::None
                    } else {
                        inquire::autocompletion::Replacement::Some(suggestion)
                    }
                }
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    let file_completer = FileCompleter {
        current_dir: current_dir.clone(),
        existing_issue_files: existing_issue_files.clone(),
    };

    let existing_files_for_validator = existing_issue_files.clone();
    let validator_dir = current_dir.clone();
    let file_path =
        Text::new("📁 Enter file path (Tab for autocomplete, directories shown with /):")
            .with_autocomplete(file_completer)
            .with_validator(move |input: &str| {
                let trimmed = input.trim();
                // Handle case where user somehow enters the grayed-out format
                if trimmed.starts_with("🚫 ") {
                    return Ok(Validation::Invalid(
                        "This file already has a corresponding issue in the milestone. Please select a different file.".into(),
                    ));
                }
                if trimmed.is_empty() {
                    Ok(Validation::Invalid("File path cannot be empty".into()))
                } else if trimmed.ends_with('/') {
                    Ok(Validation::Invalid(
                        "Cannot select a directory. Please select a file.".into(),
                    ))
                } else {
                    let path = validator_dir.join(trimmed);
                    if path.exists() && path.is_dir() {
                        Ok(Validation::Invalid(
                            "Path must be a file, not a directory".into(),
                        ))
                    } else if existing_files_for_validator.contains(&trimmed.to_string()) {
                        Ok(Validation::Invalid(
                            "This file already has a corresponding issue in the milestone. Please select a different file.".into(),
                        ))
                    } else {
                        Ok(Validation::Valid)
                    }
                }
            })
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    Ok(PathBuf::from(file_path.trim()))
}

pub fn prompt_checklist(configuration: &Configuration) -> Result<Checklist> {
    let mut checklist_names: Vec<String> = configuration.checklists.keys().cloned().collect();
    checklist_names.sort();

    if checklist_names.is_empty() {
        return Err(anyhow::anyhow!("No checklists available in configuration"));
    }

    let formatted_options: Vec<String> = checklist_names
        .iter()
        .map(|name| format!("📋 {}", name))
        .collect();

    let selection = Select::new("Select a checklist:", formatted_options)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    // Remove the emoji prefix
    let sel = selection.strip_prefix("📋 ").unwrap_or(&selection);

    Ok(configuration.checklists[sel].clone())
}

pub fn prompt_assignees(repo_users: &[RepoUser]) -> Result<Vec<String>> {
    #[derive(Clone)]
    struct UserCompleter {
        users: Vec<RepoUser>,
    }

    impl Autocomplete for UserCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
            let input = input.trim();
            let mut suggestions = Vec::new();

            for user in &self.users {
                // Search by login or name
                let matches_login = user.login.to_lowercase().contains(&input.to_lowercase());
                let matches_name = user
                    .name
                    .as_ref()
                    .map(|name| name.to_lowercase().contains(&input.to_lowercase()))
                    .unwrap_or(false);

                if matches_login || matches_name {
                    suggestions.push(user.to_string());
                }
            }

            // Sort suggestions alphabetically
            suggestions.sort();

            Ok(suggestions)
        }

        fn get_completion(
            &mut self,
            _input: &str,
            highlighted_suggestion: Option<String>,
        ) -> std::result::Result<inquire::autocompletion::Replacement, CustomUserError> {
            Ok(match highlighted_suggestion {
                Some(suggestion) => inquire::autocompletion::Replacement::Some(suggestion),
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    if repo_users.is_empty() {
        return Ok(Vec::new());
    }

    let user_completer = UserCompleter {
        users: repo_users.to_vec(),
    };

    // Create owned copy for validator
    let valid_logins: Vec<String> = repo_users.iter().map(|u| u.login.clone()).collect();

    let mut assignees = Vec::new();

    loop {
        let prompt_text = if assignees.is_empty() {
            "👥 Enter assignee username (use Tab for autocomplete, Enter for none):".to_string()
        } else {
            format!(
                "👥 Enter another assignee (current: {}, use Tab for autocomplete, Enter to finish):",
                assignees.join(", ")
            )
        };

        let valid_logins_for_validator = valid_logins.clone();
        let input = Text::new(&prompt_text)
            .with_autocomplete(user_completer.clone())
            .with_validator(move |input: &str| {
                if input.trim().is_empty() {
                    Ok(Validation::Valid) // Empty is valid - means finish
                } else {
                    // Validate that the assignee exists and extract login from display format
                    let login = if let Some(space_pos) = input.find(' ') {
                        &input[..space_pos]
                    } else {
                        input.trim()
                    };

                    if valid_logins_for_validator.iter().any(|u| u == login) {
                        Ok(Validation::Valid)
                    } else {
                        Ok(Validation::Invalid(
                            format!("User '{}' not found in repository", login).into(),
                        ))
                    }
                }
            })
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let trimmed_input = input.trim();
        if trimmed_input.is_empty() {
            break; // User pressed Enter without input, finish
        }

        // Extract login from display format "login (name)" or just "login"
        let login = if let Some(space_pos) = trimmed_input.find(' ') {
            trimmed_input[..space_pos].to_string()
        } else {
            trimmed_input.to_string()
        };

        // Avoid duplicates
        if !assignees.contains(&login) {
            assignees.push(login);
        }
    }

    Ok(assignees)
}

pub fn prompt_collaborators(defaults: &[String]) -> Result<Vec<String>> {
    let mut collaborators = if defaults.is_empty() {
        Vec::new()
    } else {
        MultiSelect::new("🤝 Select collaborators to keep:", defaults.to_vec())
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?
    };

    loop {
        let prompt = if collaborators.is_empty() {
            "🤝 Add collaborator (Name <email>, Enter to finish):"
        } else {
            "🤝 Add another collaborator (Name <email>, Enter to finish):"
        };

        let input = Text::new(prompt)
            .with_validator(|input: &str| {
                if input.trim().is_empty() || normalize_collaborator_entry(input).is_some() {
                    Ok(Validation::Valid)
                } else {
                    Ok(Validation::Invalid(
                        "Collaborator must use the format Name <email>".into(),
                    ))
                }
            })
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let trimmed = input.trim();
        if trimmed.is_empty() {
            break;
        }

        let normalized = normalize_collaborator_entry(trimmed)
            .ok_or_else(|| anyhow::anyhow!("Collaborator must use the format Name <email>"))?;

        if !collaborators.contains(&normalized) {
            collaborators.push(normalized);
        }
    }

    Ok(collaborators)
}

/// Select an issue from a milestone by title with autocomplete
pub fn prompt_issue(issues: &[Issue]) -> Result<Issue> {
    #[derive(Clone)]
    struct IssueCompleter {
        issues: Vec<Issue>,
    }

    impl Autocomplete for IssueCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
            let input = input.trim();
            let mut suggestions = Vec::new();

            for issue in &self.issues {
                // Search by title
                if issue.title.to_lowercase().contains(&input.to_lowercase()) {
                    suggestions.push(issue.title.clone());
                }
            }

            // Sort suggestions alphabetically by title
            suggestions.sort();

            Ok(suggestions)
        }

        fn get_completion(
            &mut self,
            _input: &str,
            highlighted_suggestion: Option<String>,
        ) -> std::result::Result<inquire::autocompletion::Replacement, CustomUserError> {
            Ok(match highlighted_suggestion {
                Some(suggestion) => inquire::autocompletion::Replacement::Some(suggestion),
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    if issues.is_empty() {
        return Err(anyhow::anyhow!("No issues found in the selected milestone"));
    }

    let issue_completer = IssueCompleter {
        issues: issues.to_vec(),
    };

    let issue_input = Text::new("🎫 Enter issue title (use Tab for autocomplete):")
        .with_autocomplete(issue_completer)
        .with_validator(move |input: &str| {
            let trimmed = input.trim();
            if trimmed.is_empty() {
                Ok(Validation::Invalid(
                    "Issue selection cannot be empty".into(),
                ))
            } else {
                Ok(Validation::Valid)
            }
        })
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    // Find the issue by title
    if let Some(issue) = issues.iter().find(|i| i.title == issue_input.trim()) {
        Ok(issue.clone())
    } else {
        Err(anyhow::anyhow!(
            "Issue with title '{}' not found",
            issue_input.trim()
        ))
    }
}

/// Every commit the issue's segments own, newest first, paired with the position of
/// the segment that owns it.
///
/// Segments are oldest-first and each owns its commits newest-first, so walking them
/// in reverse yields one newest-first list. The owning segment travels with the commit
/// because what a commit *is* — a round's anchor, its approval, drift after one — is a
/// fact of that segment and of nothing wider.
///
/// A boundary commit shared by two adjacent segments (**D1**: a round may open at the
/// previous round's closing commit) appears once, credited to the newer-owning segment.
/// The UI's per-segment picker renders both copies deliberately (**D14**) because each
/// row sits under its own segment heading; a single flat list is not that, and a commit
/// listed twice makes two picker indices name the same commit — which is how "compare
/// the latest two file changes" came to mean comparing a commit against itself.
pub fn thread_commits(issue_thread: &IssueThread) -> Vec<(usize, &IssueCommit)> {
    let mut seen = std::collections::HashSet::new();
    issue_thread
        .segments
        .iter()
        .enumerate()
        .rev()
        .flat_map(|(position, segment)| {
            segment
                .commits()
                .iter()
                .map(move |commit| (position, commit))
        })
        .filter(|(_, commit)| seen.insert(commit.hash))
        .collect()
}

/// What to call the segment at `position` in a picker row.
fn segment_label(issue_thread: &IssueThread, position: usize) -> String {
    match &issue_thread.segments[position] {
        Segment::Round(round) => round.name(),
        // A gap is bounded by the round one position back — never two, which is the
        // round-to-round offset.
        Segment::Gap(_) => match position
            .checked_sub(1)
            .and_then(|previous| issue_thread.segments.get(previous))
            .and_then(Segment::as_round)
        {
            Some(previous) => format!("since {}'s approval", previous.name()),
            None => "outside any round".to_string(),
        },
    }
}

/// The legend the pickers print above their rows.
const COMMIT_LEGEND: &str =
    "   🌱 Round anchor  💬 Notified  👀 Reviewed  ✅ Approved  📍 Latest  📝 File changed";

/// Every mark `format_commit_options` may put in front of a row's hash. Listed once so
/// the readers below cannot fall out of step with the writer above.
const COMMIT_MARKS: [char; 6] = ['🌱', '💬', '👀', '✅', '📍', '📝'];

/// The short hash out of a row `format_commit_options` produced.
fn selected_short_hash(row: &str) -> &str {
    row.trim_start_matches("✓ ")
        .trim_start_matches(|c: char| c.is_whitespace() || COMMIT_MARKS.contains(&c))
        .split(" - ")
        .next()
        .unwrap_or("")
        .trim()
}

/// Helper function to format commit options for display
fn format_commit_options(issue_thread: &IssueThread, selected: &[usize]) -> Vec<String> {
    let latest = issue_thread.latest_commit().map(|commit| commit.hash);
    thread_commits(issue_thread)
        .into_iter()
        .enumerate()
        .map(|(i, (position, commit))| {
            let short_hash = commit.hash.to_string()[..8].to_string();
            let short_message = if commit.message.is_empty() {
                "No message".to_string()
            } else {
                // Take first line and truncate if too long
                let first_line = commit.message.lines().next().unwrap_or("");
                if first_line.len() > 50 {
                    format!("{}...", &first_line[..47])
                } else {
                    first_line.to_string()
                }
            };

            // What the owning round says about this commit. Read straight off that
            // round's own events and state — nothing here re-derives round scope.
            let status_indicator = match &issue_thread.segments[position] {
                Segment::Round(round) => {
                    let approved = round.closing_commit() == Some(&commit.hash);
                    let anchor = round.opened_at == commit.hash;
                    // Notified and reviewed are different events with different
                    // meanings, so they get different marks: the legend says 💬
                    // Notified, and a review drawn as 💬 would misname itself.
                    let event = round
                        .events
                        .iter()
                        .rev()
                        .find(|event| *event.commit() == commit.hash);
                    match (approved, anchor, event) {
                        (true, true, _) => "🌱✅",
                        (true, false, _) => "✅",
                        (false, true, _) => "🌱",
                        (false, false, Some(RoundEvent::Review { .. })) => "👀",
                        (false, false, Some(RoundEvent::Notification { .. })) => "💬",
                        _ if latest == Some(commit.hash) => "📍",
                        _ => "  ",
                    }
                }
                // Drift between rounds: nothing announced it, by definition.
                Segment::Gap(_) if latest == Some(commit.hash) => "📍",
                Segment::Gap(_) => "  ",
            };

            // Add file change indicator
            let file_indicator = if commit.file_changed { "📝" } else { "  " };

            let label = segment_label(issue_thread, position);
            if selected.contains(&i) {
                format!(
                    "✓ {}{} {} - {} [{}] (already selected)",
                    status_indicator, file_indicator, short_hash, short_message, label
                )
            } else {
                format!(
                    "  {}{} {} - {} [{}]",
                    status_indicator, file_indicator, short_hash, short_message, label
                )
            }
        })
        .collect()
}

/// Select commits for comparison - returns (current, previous) in chronological order
pub fn prompt_commits(issue_thread: &IssueThread) -> Result<(ObjectId, Option<ObjectId>)> {
    let commits = thread_commits(issue_thread);
    if commits.is_empty() {
        return Err(anyhow::anyhow!("No commits found for this file"));
    }

    if commits.len() == 1 {
        return Ok((commits[0].1.hash, None));
    }

    // Get commits that actually changed the file for smart defaults
    let file_changing_commits: Vec<_> = commits
        .iter()
        .enumerate()
        .filter(|(_, (_, commit))| commit.file_changed)
        .collect();

    // Determine default cursor position (prefer file-changing commits)
    let default_cursor = if !file_changing_commits.is_empty() {
        file_changing_commits[0].0 // First file-changing commit
    } else {
        0 // Fall back to first commit overall
    };

    println!("📋 Commit Status Legend:");
    println!("{COMMIT_LEGEND}");
    println!();

    let mut selected_commits: Vec<usize> = Vec::new();

    // First selection
    println!("📝 Select first commit (press Enter for latest file change):");
    let options = format_commit_options(issue_thread, &selected_commits);
    let first_selection = Select::new("Pick commit:", options)
        .with_starting_cursor(default_cursor)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let first_short_hash = selected_short_hash(&first_selection);

    // Find the commit index
    let first_index = commits
        .iter()
        .position(|(_, commit)| commit.hash.to_string().starts_with(first_short_hash))
        .unwrap_or(0);

    selected_commits.push(first_index);

    // Determine default cursor for second selection (prefer second file-changing commit)
    let second_default_cursor = if file_changing_commits.len() > 1 {
        file_changing_commits[1].0 + 1 // +1 because we insert "Skip" at position 0
    } else {
        0 // Default to skip if no second file-changing commit
    };

    // Second selection
    println!("\n📝 Select second commit for comparison (press Enter for second file change):");
    let mut options_with_skip = format_commit_options(issue_thread, &selected_commits);
    options_with_skip.insert(
        0,
        "  ⏭️  Skip second commit (compare with nothing)".to_string(),
    );

    let second_selection = Select::new("Pick commit:", options_with_skip)
        .with_starting_cursor(second_default_cursor)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let second_commit = if second_selection.contains("⏭️") {
        None
    } else {
        let second_short_hash = selected_short_hash(&second_selection);

        let second_index = commits
            .iter()
            .position(|(_, commit)| commit.hash.to_string().starts_with(second_short_hash))
            .unwrap_or(0);

        Some(commits[second_index].1.hash)
    };

    Ok((commits[first_index].1.hash, second_commit))
}

/// Select a single commit from file commits - returns the selected commit
pub fn prompt_single_commit(
    issue_thread: &IssueThread,
    prompt_text: &str,
    default_position: usize,
) -> Result<ObjectId> {
    let commits = thread_commits(issue_thread);
    if commits.is_empty() {
        return Err(anyhow::anyhow!("No commits found for this file"));
    }

    if commits.len() == 1 {
        log::info!("Only one commit found for this file. Selecting the commit...");
        return Ok(commits[0].1.hash);
    }

    println!("📋 Commit Status Legend:");
    println!("{COMMIT_LEGEND}");
    println!();

    // Create commit options with status indicators
    let commit_options = format_commit_options(issue_thread, &[]);

    println!("{}", prompt_text);
    let commit_selection = Select::new("Pick commit:", commit_options)
        .with_starting_cursor(default_position.min(commits.len() - 1)) // Use provided default position, clamped to valid range
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let commit_short_hash = selected_short_hash(&commit_selection);

    let commit_index = commits
        .iter()
        .position(|(_, commit)| commit.hash.to_string().starts_with(commit_short_hash))
        .unwrap_or(0);

    Ok(commits[commit_index].1.hash)
}

/// Prompt for optional note for a comment
pub fn prompt_note() -> Result<Option<String>> {
    let note_input = Text::new("📝 Enter optional note for this comment (Enter to skip):")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    let trimmed_input = note_input.trim();
    if trimmed_input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed_input.to_string()))
    }
}

/// Interactive milestone selection for record generation
pub fn prompt_milestone_record(
    milestones: &[Milestone],
) -> Result<(Vec<Milestone>, Option<PathBuf>, bool)> {
    println!("📄 Welcome to GHQC Milestone Record Mode!");

    if milestones.is_empty() {
        bail!("No milestones found in repository");
    }

    // First ask if they want to select all or choose specific ones
    let choice = Select::new(
        "📄 How would you like to select milestones for the record?",
        vec!["📋 Select All Milestones", "🎯 Choose Specific Milestones"],
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let selected_milestones: Vec<Milestone> = if choice == "📋 Select All Milestones" {
        milestones.to_vec()
    } else {
        // Multi-select specific milestones
        let milestone_options: Vec<String> = milestones
            .iter()
            .map(|m| format!("{} ({})", m.title, m.number))
            .collect();

        let selected_strings =
            MultiSelect::new("📄 Select milestones for the record:", milestone_options)
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
            .cloned()
            .collect()
    };

    if selected_milestones.is_empty() {
        bail!("No milestones selected");
    }

    // Prompt for only_tables using y/N format
    let only_tables = Confirm::new("📋 Generate only tables without detailed issue content?")
        .with_default(false)
        .with_help_message("N = include detailed issue content, y = only summary tables")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    // Prompt for optional record path
    let record_path_input = Text::new("📁 Enter record file name (Enter for default):")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    let record_path = if record_path_input.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(record_path_input.trim()))
    };

    Ok((selected_milestones, record_path, only_tables))
}

/// Interactive milestone selection for archive generation
pub fn prompt_milestone_archive(
    milestones: &[Milestone],
) -> Result<(Vec<Milestone>, Option<PathBuf>, bool, bool)> {
    use inquire::Confirm;

    println!("📦 Welcome to GHQC Milestone Archive Mode!");

    if milestones.is_empty() {
        bail!("No milestones found in repository");
    }

    // First ask if they want to select all or choose specific ones
    let choice = Select::new(
        "📦 How would you like to select milestones for the archive?",
        vec!["📋 Select All Milestones", "🎯 Choose Specific Milestones"],
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let selected_milestones: Vec<Milestone> = if choice == "📋 Select All Milestones" {
        milestones.to_vec()
    } else {
        // Multi-select specific milestones
        let milestone_options: Vec<String> = milestones
            .iter()
            .map(|m| format!("{} ({})", m.title, m.number))
            .collect();

        let selected_strings =
            MultiSelect::new("📦 Select milestones for the archive:", milestone_options)
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
            .cloned()
            .collect()
    };

    if selected_milestones.is_empty() {
        bail!("No milestones selected");
    }

    // Prompt for include_unapproved using y/N format
    let include_unapproved = Confirm::new("📋 Include unapproved issues?")
        .with_default(false)
        .with_help_message("N = only approved issues, y = all issues")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    // Prompt for flatten using y/N format
    let flatten = Confirm::new("📁 Flatten archive structure?")
        .with_default(false)
        .with_help_message("N = preserve directory structure, y = put all files in root")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    // Prompt for optional archive path
    let archive_path_input = Text::new("📁 Enter archive file name (Enter for default):")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    let archive_path = if archive_path_input.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(archive_path_input.trim()))
    };

    Ok((
        selected_milestones,
        archive_path,
        include_unapproved,
        flatten,
    ))
}

/// Interactive context file selection for record generation
/// Lists PDF files and allows users to select and choose prepend/append position
pub fn prompt_context_files(current_dir: &PathBuf) -> Result<Vec<QCContext>> {
    // Find all .pdf files in the directory
    let context_files_available: Vec<PathBuf> = fs::read_dir(current_dir)
        .map_err(|e| anyhow::anyhow!("Failed to read directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    let ext_lower = ext.to_string_lossy().to_lowercase();
                    if ext_lower == "pdf" {
                        return Some(path);
                    }
                }
            }
            None
        })
        .collect();

    if context_files_available.is_empty() {
        println!("ℹ️  No PDF context documents found in directory");
        return Ok(Vec::new());
    }

    let mut context_files: Vec<QCContext> = Vec::new();
    let mut available_files = context_files_available.clone();

    loop {
        if available_files.is_empty() {
            println!("ℹ️  No more context documents available to add");
            break;
        }

        // Create options list with file names
        let mut options: Vec<String> = available_files
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| format!("📄 {}", n.to_string_lossy()))
            .collect();
        options.insert(0, "✅ Done adding context files".to_string());

        let prompt_text = if context_files.is_empty() {
            "📄 Select a document to include as context (or Done to skip):"
        } else {
            "📄 Select another document (or Done to finish):"
        };

        let selection = Select::new(prompt_text, options)
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        if selection.starts_with("✅") {
            break;
        }

        // Extract filename from selection
        let selected_name = selection.strip_prefix("📄 ").unwrap_or(&selection);

        // Find the matching file path
        let selected_path = available_files
            .iter()
            .find(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy() == selected_name)
                    .unwrap_or(false)
            })
            .cloned();

        if let Some(path) = selected_path {
            // Ask whether to prepend or append
            let position_options = vec![
                "⬆️  Prepend (before main findings)".to_string(),
                "⬇️  Append (after main findings)".to_string(),
            ];

            let position_selection = Select::new(
                &format!("📍 Where should '{}' appear?", selected_name),
                position_options,
            )
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

            let position = if position_selection.starts_with("⬆️") {
                ContextPosition::Prepend
            } else {
                ContextPosition::Append
            };

            context_files.push(QCContext::new(&path, position));

            // Remove from available files
            available_files.retain(|p| p != &path);

            println!(
                "✅ Added '{}' to {} context",
                selected_name,
                if matches!(position, ContextPosition::Prepend) {
                    "prepended"
                } else {
                    "appended"
                }
            );
        }
    }

    // Show summary if any files were selected
    if !context_files.is_empty() {
        println!("\n📋 Context files summary:");
        for ctx in &context_files {
            let pos = match ctx.position() {
                ContextPosition::Prepend => "prepend",
                ContextPosition::Append => "append",
            };
            println!(
                "   {} {} ({})",
                if matches!(ctx.position(), ContextPosition::Prepend) {
                    "⬆️"
                } else {
                    "⬇️"
                },
                ctx.file().display(),
                pos
            );
        }
        println!();
    }

    Ok(context_files)
}

/// Asks if user wants to add relevant files
pub fn prompt_want_relevant_files() -> Result<bool> {
    Confirm::new("Do you want to add any relevant files?")
        .with_default(false)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))
}

/// File selector for relevant files with issue indicators
/// Shows files with their related issue numbers if they exist
pub fn prompt_relevant_file_path(current_dir: &PathBuf, all_issues: &[Issue]) -> Result<PathBuf> {
    // Build a map of file paths to their issue numbers
    let file_to_issues: HashMap<String, Vec<u64>> = {
        let mut map: HashMap<String, Vec<u64>> = HashMap::new();
        for issue in all_issues {
            map.entry(issue.title.clone())
                .or_default()
                .push(issue.number);
        }
        map
    };

    #[derive(Clone)]
    struct RelevantFileCompleter {
        current_dir: PathBuf,
        file_to_issues: HashMap<String, Vec<u64>>,
    }

    impl RelevantFileCompleter {
        fn format_file_with_issues(&self, relative_path: &str) -> String {
            if let Some(issue_numbers) = self.file_to_issues.get(relative_path) {
                if issue_numbers.len() <= 3 {
                    let issues_str = issue_numbers
                        .iter()
                        .map(|n| format!("#{}", n))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{} [{}]", relative_path, issues_str)
                } else {
                    let first_three = issue_numbers[..3]
                        .iter()
                        .map(|n| format!("#{}", n))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "{} [{} +{} more]",
                        relative_path,
                        first_three,
                        issue_numbers.len() - 3
                    )
                }
            } else {
                relative_path.to_string()
            }
        }
    }

    impl Autocomplete for RelevantFileCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
            let input = input.trim();
            let mut suggestions = Vec::new();

            let (base_path, search_term) = if input.contains('/') {
                let mut parts = input.rsplitn(2, '/');
                let filename = parts.next().unwrap_or("");
                let dir_path = parts.next().unwrap_or("");
                (self.current_dir.join(dir_path), filename)
            } else {
                (self.current_dir.clone(), input)
            };

            if let Ok(entries) = fs::read_dir(&base_path) {
                let mut files = Vec::new();
                let mut dirs = Vec::new();

                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        // Skip hidden files/directories
                        if name.starts_with('.') {
                            continue;
                        }

                        if name.to_lowercase().starts_with(&search_term.to_lowercase()) {
                            let relative_path = if input.contains('/') {
                                let dir_part = input.rsplitn(2, '/').nth(1).unwrap_or("");
                                format!("{}/{}", dir_part, name)
                            } else {
                                name.clone()
                            };

                            if entry.path().is_file() {
                                // Format file with issue indicators
                                files.push(self.format_file_with_issues(&relative_path));
                            } else if entry.path().is_dir() {
                                // Add trailing slash to indicate directory
                                dirs.push(format!("{}/", relative_path));
                            }
                        }
                    }
                }

                // Sort directories and files separately, then combine
                dirs.sort();
                files.sort();
                suggestions.extend(dirs);
                suggestions.extend(files);
            }

            Ok(suggestions)
        }

        fn get_completion(
            &mut self,
            _input: &str,
            highlighted_suggestion: Option<String>,
        ) -> std::result::Result<inquire::autocompletion::Replacement, CustomUserError> {
            Ok(match highlighted_suggestion {
                Some(suggestion) => {
                    // Strip issue indicators from completion
                    // Format is "path [#1, #2]" or "path [#1, #2 +N more]"
                    let clean_path = if let Some(bracket_pos) = suggestion.find(" [") {
                        suggestion[..bracket_pos].to_string()
                    } else {
                        suggestion
                    };
                    inquire::autocompletion::Replacement::Some(clean_path)
                }
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    let file_completer = RelevantFileCompleter {
        current_dir: current_dir.clone(),
        file_to_issues,
    };

    let validator_dir = current_dir.clone();
    let file_path = Text::new("📁 Select a relevant file:")
        .with_autocomplete(file_completer)
        .with_validator(move |input: &str| {
            // Strip issue indicators if present
            let trimmed = if let Some(bracket_pos) = input.find(" [") {
                input[..bracket_pos].trim()
            } else {
                input.trim()
            };

            if trimmed.is_empty() {
                Ok(Validation::Invalid("File path cannot be empty".into()))
            } else if trimmed.ends_with('/') {
                Ok(Validation::Invalid(
                    "Cannot select a directory. Please select a file.".into(),
                ))
            } else {
                let path = validator_dir.join(trimmed);
                if path.exists() && path.is_dir() {
                    Ok(Validation::Invalid(
                        "Path must be a file, not a directory".into(),
                    ))
                } else {
                    Ok(Validation::Valid)
                }
            }
        })
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    // Strip issue indicators if present and return clean path
    let clean_path = if let Some(bracket_pos) = file_path.find(" [") {
        file_path[..bracket_pos].trim().to_string()
    } else {
        file_path.trim().to_string()
    };

    Ok(PathBuf::from(clean_path))
}

/// Select issue or File option for a given file path
/// Returns Some(issue) if issue selected, None if File selected
pub fn prompt_relevant_file_source<'a>(
    file_path: &PathBuf,
    matching_issues: &'a [&Issue],
    current_milestone_number: u64,
) -> Result<Option<&'a Issue>> {
    // Sort issues: current milestone first, then by issue number descending
    let mut sorted_issues: Vec<&Issue> = matching_issues.to_vec();
    sorted_issues.sort_by(|a, b| {
        let a_is_current = a
            .milestone
            .as_ref()
            .map(|m| m.number == current_milestone_number as i64)
            .unwrap_or(false);
        let b_is_current = b
            .milestone
            .as_ref()
            .map(|m| m.number == current_milestone_number as i64)
            .unwrap_or(false);

        match (a_is_current, b_is_current) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.number.cmp(&a.number), // Descending by issue number
        }
    });

    // Build options list - File option first, then issues
    let mut options: Vec<String> = vec!["📄 File (not linked to an issue)".to_string()];

    let issue_options: Vec<String> = sorted_issues
        .iter()
        .map(|issue| {
            let milestone_name = issue
                .milestone
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "No milestone".to_string());
            format!("🔗 #{} ({})", issue.number, milestone_name)
        })
        .collect();
    options.extend(issue_options);

    let selection = Select::new(
        &format!("Select the source for '{}':", file_path.display()),
        options,
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    // Check if user selected File option
    if selection.starts_with("📄 ") {
        return Ok(None);
    }

    // Extract issue number from selection (format: "🔗 #123 (Milestone Name)")
    let issue_number_str = selection
        .trim_start_matches("🔗 ")
        .trim_start_matches('#')
        .split_whitespace()
        .next()
        .unwrap_or("");

    if let Ok(issue_number) = issue_number_str.parse::<u64>() {
        if let Some(issue) = sorted_issues.iter().find(|i| i.number == issue_number) {
            return Ok(Some(*issue));
        }
    }

    Ok(None)
}

/// Select relevant file class type (GatingQC, PreviousQC, RelevantQC)
pub fn prompt_relevant_file_class() -> Result<RelevantFileClassType> {
    let options = vec![
        "🚦 Gating QC - This issue must be approved before the current issue".to_string(),
        "📜 Previous QC - A previous version of the QC for this file".to_string(),
        "🔗 Relevant QC - Related QC that provides context".to_string(),
    ];

    let selection = Select::new("Select the relationship type:", options)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    if selection.starts_with("🚦") {
        Ok(RelevantFileClassType::GatingQC)
    } else if selection.starts_with("📜") {
        Ok(RelevantFileClassType::PreviousQC)
    } else {
        Ok(RelevantFileClassType::RelevantQC)
    }
}

/// Prompt whether to include a diff comment for this Previous QC
pub fn prompt_include_previous_qc_diff() -> Result<bool> {
    Confirm::new("Include a diff comment for this Previous QC?")
        .with_default(true)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))
}

/// Prompt for description (optional for issues, required for files)
pub fn prompt_relevant_description(required: bool) -> Result<Option<String>> {
    let prompt_text = if required {
        "📝 Provide a justification for this file:"
    } else {
        "📝 Add a description? (optional, press Enter to skip):"
    };

    if required {
        let input = Text::new(prompt_text)
            .with_validator(|input: &str| {
                if input.trim().is_empty() {
                    Ok(Validation::Invalid("Justification cannot be empty".into()))
                } else {
                    Ok(Validation::Valid)
                }
            })
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        Ok(Some(input.trim().to_string()))
    } else {
        let input = Text::new(prompt_text)
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let trimmed = input.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }
}

/// Asks if user wants to add another relevant file
pub fn prompt_add_another_relevant_file() -> Result<bool> {
    Confirm::new("Add another relevant file?")
        .with_default(false)
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::round::{
        ChecklistSource, Gap, GapContinuity, Placement, Round, RoundOpen, RoundState,
    };
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";
    const D: &str = "ddddddd000000000000000000000000000000004";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    /// `shas` oldest-first for readability, reversed into the newest-first order every
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

    fn round(index: u32, opened_at: &str, closed_at: Option<&str>, own: &[&str]) -> Segment {
        Segment::Round(Round {
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
        })
    }

    fn gap(own: &[&str]) -> Segment {
        Segment::Gap(Gap {
            branch: "main".to_string(),
            commits: commits(own),
            continuity: GapContinuity::Linear,
            placement: Placement::Placed,
        })
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

    /// Segments are oldest-first and each owns its commits newest-first; the flat list
    /// every picker indexes into must be one newest-first run, with each commit credited
    /// to the segment that owns it.
    #[test]
    fn thread_commits_is_newest_first_across_segments() {
        // [Initial QC(A..B, closed at B), Gap(C), Round 2(D, open)]
        let thread = thread(vec![
            round(1, A, Some(B), &[A, B]),
            gap(&[C]),
            round(2, D, None, &[D]),
        ]);

        let flat: Vec<(usize, String)> = thread_commits(&thread)
            .into_iter()
            .map(|(position, commit)| (position, commit.hash.to_string()))
            .collect();
        assert_eq!(
            flat,
            vec![
                (2, D.to_string()),
                (1, C.to_string()),
                (0, B.to_string()),
                (0, A.to_string()),
            ]
        );
    }

    /// D1: a round may open at the previous round's closing commit, so both own it and
    /// the gap between them is empty. Listed twice, indices 0 and 1 of a flat picker
    /// name the *same* commit — pressing Enter twice then compares it against itself,
    /// which renders an `X...X` link with no diff. The newer-owning copy wins.
    #[test]
    fn thread_commits_lists_a_shared_boundary_commit_once() {
        // Initial QC closed at B; round 2 re-QCs the very same commit.
        let thread = thread(vec![
            round(1, A, Some(B), &[A, B]),
            gap(&[]),
            round(2, B, None, &[B]),
        ]);

        let flat: Vec<(usize, String)> = thread_commits(&thread)
            .into_iter()
            .map(|(position, commit)| (position, commit.hash.to_string()))
            .collect();
        assert_eq!(
            flat,
            vec![(2, B.to_string()), (0, A.to_string())],
            "B is one commit, credited to the round that owns it now"
        );
    }

    /// A gap is bounded by the round one position back. Two back is the round-to-round
    /// offset and would name the wrong round — Initial QC where Round 2 belongs.
    #[test]
    fn segment_label_names_the_round_bounding_a_gaps_older_end() {
        let thread = thread(vec![
            round(1, A, Some(B), &[A, B]),
            gap(&[C]),
            round(2, C, Some(D), &[C, D]),
            gap(&[]),
        ]);

        assert_eq!(segment_label(&thread, 0), "Initial QC");
        assert_eq!(segment_label(&thread, 1), "since Initial QC's approval");
        assert_eq!(segment_label(&thread, 2), "Round 2");
        assert_eq!(segment_label(&thread, 3), "since Round 2's approval");
    }

    /// Notified and reviewed are different events; the legend names both, so the rows
    /// must draw both. A review drawn as 💬 labels itself wrongly.
    #[test]
    fn format_commit_options_distinguishes_notified_from_reviewed() {
        let mut segments = vec![round(1, A, None, &[A, B, C])];
        if let Segment::Round(round) = &mut segments[0] {
            round.events.push(RoundEvent::Notification {
                commit: oid(B),
                by: "author".to_string(),
                at: chrono::Utc::now(),
                comment_index: 0,
                comment_id: None,
                comment_url: None,
            });
            round.events.push(RoundEvent::Review {
                commit: oid(C),
                by: "reviewer".to_string(),
                at: chrono::Utc::now(),
                comment_index: 1,
                comment_id: None,
                comment_url: None,
            });
        }
        let thread = thread(segments);

        let rows = format_commit_options(&thread, &[]);
        assert!(rows[0].contains("👀"), "the reviewed commit: {}", rows[0]);
        assert!(!rows[0].contains('💬'), "not notified: {}", rows[0]);
        assert!(rows[1].contains("💬"), "the notified commit: {}", rows[1]);
        assert!(!rows[1].contains('👀'), "not reviewed: {}", rows[1]);
        assert!(COMMIT_LEGEND.contains("💬 Notified"));
        assert!(COMMIT_LEGEND.contains("👀 Reviewed"));
    }

    /// Every mark the rows may carry has to come back off again, or the hash a
    /// selection names cannot be found and the picker silently falls back to index 0.
    #[test]
    fn every_row_yields_back_its_own_short_hash() {
        let thread = thread(vec![
            round(1, A, Some(B), &[A, B]),
            gap(&[C]),
            round(2, D, None, &[D]),
        ]);

        let flat = thread_commits(&thread);
        for (index, row) in format_commit_options(&thread, &[0]).iter().enumerate() {
            let hash = selected_short_hash(row);
            assert!(
                !hash.is_empty() && flat[index].1.hash.to_string().starts_with(hash),
                "row {index} ({row}) did not yield its own hash, got {hash:?}"
            );
        }
    }

    #[test]
    fn test_prompt_checklist() {
        use crate::configuration::Checklist;

        let mut config = Configuration::default();
        config.checklists.insert(
            "Test Checklist".to_string(),
            Checklist::new(
                "Test Checklist".to_string(),
                None,
                "- [ ] Test item".to_string(),
            ),
        );
        config.checklists.insert(
            "Another Checklist".to_string(),
            Checklist::new(
                "Another Checklist".to_string(),
                None,
                "- [ ] Another item".to_string(),
            ),
        );

        // This test just verifies the function doesn't panic with valid configuration
        // Actual interactive testing would require manual verification
        assert!(config.checklists.len() == 3); // Including the default "Custom" checklist
    }
}
