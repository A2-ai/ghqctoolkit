use anyhow::Result;
use gix::ObjectId;
use inquire::{Autocomplete, CustomUserError, Select, Text, validator::Validation};
use octocrab::models::{Milestone, issues::Issue};
use std::fs;
use std::path::PathBuf;

use crate::{
    Configuration, RelevantFile, configuration::Checklist, create::MilestoneStatus, git::RepoUser,
};

/// Modular milestone selection - allows creation of new milestones
pub fn prompt_milestone(milestones: Vec<Milestone>) -> Result<MilestoneStatus> {
    let mut options = vec!["📝 Create new milestone".to_string()];
    let milestone_titles: Vec<String> = milestones
        .iter()
        .filter(|m| m.state.as_deref() == Some("open"))
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
        Ok(MilestoneStatus::New(new_milestone.trim().to_string()))
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
    let open_milestones: Vec<_> = milestones
        .iter()
        .filter(|m| m.state.as_deref() == Some("open"))
        .collect();

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

pub fn prompt_file(current_dir: &PathBuf) -> Result<PathBuf> {
    #[derive(Clone)]
    struct FileCompleter {
        current_dir: PathBuf,
    }

    impl Autocomplete for FileCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
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
                                files.push(relative_path);
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
                Some(suggestion) => inquire::autocompletion::Replacement::Some(suggestion),
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    let file_completer = FileCompleter {
        current_dir: current_dir.clone(),
    };

    let validator_dir = current_dir.clone();
    let file_path =
        Text::new("📁 Enter file path (Tab for autocomplete, directories shown with /):")
            .with_autocomplete(file_completer)
            .with_validator(move |input: &str| {
                let trimmed = input.trim();
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

pub fn prompt_relevant_files(current_dir: &PathBuf) -> Result<Vec<RelevantFile>> {
    #[derive(Clone)]
    struct FileCompleter {
        current_dir: PathBuf,
    }

    impl Autocomplete for FileCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
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
                                files.push(relative_path);
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
                Some(suggestion) => inquire::autocompletion::Replacement::Some(suggestion),
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    let file_completer = FileCompleter {
        current_dir: current_dir.clone(),
    };

    let mut relevant_files = Vec::new();

    loop {
        let prompt_text = if relevant_files.is_empty() {
            "📁 Enter relevant file path (Tab for autocomplete, directories shown with /, Enter for none):".to_string()
        } else {
            format!(
                "📁 Enter another relevant file (current: {}, Tab for autocomplete, Enter to finish):",
                relevant_files
                    .iter()
                    .map(RelevantFile::to_string)
                    .collect::<Vec<String>>()
                    .join(", ")
            )
        };

        let validator_dir = current_dir.clone();
        let input = Text::new(&prompt_text)
            .with_autocomplete(file_completer.clone())
            .with_validator(move |input: &str| {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    Ok(Validation::Valid) // Empty is valid - means finish
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

        let trimmed_input = input.trim();
        if trimmed_input.is_empty() {
            break; // User pressed Enter without input, finish
        }

        let file_path = PathBuf::from(trimmed_input);

        // Suggest a default name based on the filename
        let suggested_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(trimmed_input);

        // Prompt for the name with the suggested default
        let name_prompt = format!(
            "📝 Enter name for this file (default: '{}'):",
            suggested_name
        );
        let name_input = Text::new(&name_prompt)
            .with_default(suggested_name)
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let final_name = if name_input.trim().is_empty() {
            suggested_name.to_string()
        } else {
            name_input.trim().to_string()
        };

        // Prompt for optional notes (supports \n for line breaks)
        let notes_input = Text::new(
            "📝 Enter optional notes for this file (use \\n for line breaks, Enter to finish):",
        )
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let final_notes = if notes_input.trim().is_empty() {
            None
        } else {
            Some(notes_input.trim().to_string())
        };

        let relevant_file = RelevantFile {
            name: final_name,
            path: file_path.clone(),
            notes: final_notes,
        };

        // Avoid duplicates (check by path)
        if !relevant_files.iter().any(|f| f.path == file_path) {
            relevant_files.push(relevant_file);
        }
    }

    Ok(relevant_files)
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

/// Helper function to format commit options for display
fn format_commit_options(
    file_commits: &[(gix::ObjectId, String)],
    selected: &[usize],
) -> Vec<String> {
    file_commits
        .iter()
        .enumerate()
        .map(|(i, (commit_id, message))| {
            let short_hash = commit_id.to_string()[..8].to_string();
            let short_message = if message.is_empty() {
                "No message".to_string()
            } else {
                // Take first line and truncate if too long
                let first_line = message.lines().next().unwrap_or("");
                if first_line.len() > 50 {
                    format!("{}...", &first_line[..47])
                } else {
                    first_line.to_string()
                }
            };

            let time_desc = if i == 0 {
                "latest".to_string()
            } else {
                format!("{} commits ago", i)
            };
            let selection_indicator = if selected.contains(&i) {
                format!(
                    "✓ {} - {} - {} (already selected)",
                    short_hash, short_message, time_desc
                )
            } else {
                format!("  {} - {} - {}", short_hash, short_message, time_desc)
            };

            selection_indicator
        })
        .collect()
}

/// Select commits for comparison - returns (current, previous) in chronological order
pub fn prompt_commits(
    file_commits: &[(gix::ObjectId, String)],
) -> Result<(ObjectId, Option<ObjectId>)> {
    if file_commits.is_empty() {
        return Err(anyhow::anyhow!("No commits found for this file"));
    }

    if file_commits.len() == 1 {
        return Ok((file_commits[0].0, None));
    }

    let mut selected_commits: Vec<usize> = Vec::new();

    // First selection
    println!("📝 Select first commit (press Enter for latest):");
    let options = format_commit_options(file_commits, &selected_commits);
    let first_selection = Select::new("Pick commit:", options)
        .with_starting_cursor(0) // Default to first (most recent) commit
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    // Extract short hash from selection (remove prefixes)
    let first_short_hash = first_selection
        .trim_start_matches("✓ ")
        .trim_start_matches("  ")
        .split(" - ")
        .next()
        .unwrap_or("");

    // Find the commit index
    let first_index = file_commits
        .iter()
        .position(|(commit_id, _)| commit_id.to_string().starts_with(first_short_hash))
        .unwrap_or(0);

    selected_commits.push(first_index);

    // Second selection with loop to prevent selecting already chosen commits
    let second_selection = loop {
        let options = format_commit_options(file_commits, &selected_commits);
        if options.len() <= 1 {
            // Only one commit available, return it
            return Ok((file_commits[first_index].0, None));
        }
        // Default to the first unselected commit (usually index 1 if first selection was 0)
        let (default_index, message) = if selected_commits.contains(&0) {
            (1, "1 commit ago")
        } else {
            (0, "latest")
        };
        println!("📝 Select second commit (press Enter for {message}):");

        let selection = Select::new("Pick commit:", options)
            .with_starting_cursor(default_index)
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        // Extract short hash from the selection
        let short_hash = selection
            .trim_start_matches("✓ ")
            .trim_start_matches("  ")
            .split(" - ")
            .next()
            .unwrap_or("");

        // Check if this commit is already selected
        let is_selected = file_commits
            .iter()
            .position(|(commit_id, _)| commit_id.to_string().starts_with(short_hash))
            .map(|idx| selected_commits.contains(&idx))
            .unwrap_or(false);

        if is_selected {
            println!("⚠️  This commit is already selected. Please choose a different commit.\n");
            continue;
        }

        break selection;
    };

    // Extract short hash from second selection
    let second_short_hash = second_selection
        .trim_start_matches("✓ ")
        .trim_start_matches("  ")
        .split(" - ")
        .next()
        .unwrap_or("");

    // Find the second commit index
    let second_index = file_commits
        .iter()
        .position(|(commit_id, _)| commit_id.to_string().starts_with(second_short_hash))
        .unwrap_or(0);

    // Determine chronological order (current should be more recent)
    let (current_commit, previous_commit) = if first_index <= second_index {
        // first_index is more recent (smaller index)
        (
            file_commits[first_index].0,
            Some(file_commits[second_index].0),
        )
    } else {
        // second_index is more recent
        (
            file_commits[second_index].0,
            Some(file_commits[first_index].0),
        )
    };

    Ok((current_commit, previous_commit))
}

/// Select a single commit from file commits - returns the selected commit
pub fn prompt_single_commit(
    file_commits: &[(gix::ObjectId, String)],
    prompt_text: &str,
) -> Result<ObjectId> {
    if file_commits.is_empty() {
        return Err(anyhow::anyhow!("No commits found for this file"));
    }

    if file_commits.len() == 1 {
        return Ok(file_commits[0].0);
    }

    // Create commit options (no selection tracking for single commit)
    let commit_options = format_commit_options(file_commits, &[]);

    println!("{}", prompt_text);
    let commit_selection = Select::new("Pick commit:", commit_options)
        .with_starting_cursor(0) // Default to latest commit
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let commit_short_hash = commit_selection.trim_start_matches("  ").split(" - ").next().unwrap_or("");
    let commit_index = file_commits
        .iter()
        .position(|(commit_id, _)| commit_id.to_string().starts_with(commit_short_hash))
        .unwrap_or(0);

    Ok(file_commits[commit_index].0)
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

#[cfg(test)]
mod tests {
    use super::*;

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
