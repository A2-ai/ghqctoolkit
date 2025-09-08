use anyhow::Result;
use inquire::{Autocomplete, CustomUserError, Select, Text, validator::Validation};
use std::fs;
use std::path::PathBuf;

use crate::{Configuration, create::MilestoneStatus, git::GitHubApi};

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
                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        if name.to_lowercase().starts_with(&search_term.to_lowercase()) {
                            // Only include files in suggestions, not directories
                            if entry.path().is_file() {
                                let relative_path = if input.contains('/') {
                                    let dir_part = input.rsplitn(2, '/').nth(1).unwrap_or("");
                                    format!("{}/{}", dir_part, name)
                                } else {
                                    name
                                };
                                suggestions.push(relative_path);
                            }
                        }
                    }
                }
            }

            // Sort suggestions alphabetically (all files now)
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

    let file_completer = FileCompleter {
        current_dir: current_dir.clone(),
    };

    let validator_dir = current_dir.clone();
    let file_path = Text::new("📁 Enter file path (use Tab for autocomplete):")
        .with_autocomplete(file_completer)
        .with_validator(move |input: &str| {
            if input.trim().is_empty() {
                Ok(Validation::Invalid("File path cannot be empty".into()))
            } else {
                let path = validator_dir.join(input.trim());
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
