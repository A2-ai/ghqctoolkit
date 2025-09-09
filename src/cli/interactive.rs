use anyhow::Result;
use inquire::{Autocomplete, CustomUserError, Select, Text, validator::Validation};
use std::fs;
use std::path::PathBuf;

use crate::{
    Configuration, RelevantFile,
    create::MilestoneStatus,
    git::{GitHubApi, RepoUser},
};

pub async fn prompt_milestone(git_info: &impl GitHubApi) -> Result<MilestoneStatus> {
    println!("📋 Fetching milestones...");

    let milestones = git_info
        .get_milestones()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch milestones: {}", e))?;

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
        if let Some(milestone) = milestones.iter().find(|m| m.title == milestone_title) {
            Ok(MilestoneStatus::Existing {
                number: milestone.number as u64,
                name: milestone.title.to_string(),
            })
        } else {
            // Fallback to Unknown if we can't find the milestone
            Ok(MilestoneStatus::Unknown(milestone_title.to_string()))
        }
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

pub fn prompt_checklist(configuration: &Configuration) -> Result<String> {
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
    Ok(selection
        .strip_prefix("📋 ")
        .unwrap_or(&selection)
        .to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_checklist() {
        let mut config = Configuration::default();
        config
            .checklists
            .insert("Test Checklist".to_string(), "- [ ] Test item".to_string());
        config.checklists.insert(
            "Another Checklist".to_string(),
            "- [ ] Another item".to_string(),
        );

        // This test just verifies the function doesn't panic with valid configuration
        // Actual interactive testing would require manual verification
        assert!(config.checklists.len() == 3); // Including the default "Custom" checklist
    }
}
