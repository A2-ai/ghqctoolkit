use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use futures::future;
use gix::ObjectId;
use inquire::{
    Autocomplete, Confirm, CustomUserError, MultiSelect, Select, Text, list_option::ListOption,
    validator::Validation,
};
use octocrab::models::Milestone;

use crate::{
    CommitCache, DiskCache, GitCommitAnalysis, GitFileOps, GitHubReader, GitRepository,
    IssueThread, archive::ArchiveFile, get_issue_comments, git::GitCommit,
};

pub async fn prompt_archive(
    milestones: &[Milestone],
    current_dir: &PathBuf,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis + GitRepository),
    cache: Option<&DiskCache>,
) -> Result<(Vec<ArchiveFile>, PathBuf)> {
    println!("📦 Welcome to GHQC Milestone Archive Mode!");

    let milestone_selection_method = Select::new(
        "📦 How would you like to select milestones for the archive?",
        vec![
            "📋 Select All Milestones",
            "🎯 Choose Specific Milestones",
            "🚫 Select No Milestones",
        ],
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {e}"))?;

    let milestones = match milestone_selection_method {
        "📋 Select All Milestones" => {
            let filtered_milestones = prompt_open_milestones()?.filter_milestones(milestones);
            if filtered_milestones.is_empty() {
                bail!(
                    "No milestones available with the selected filter. Try including open milestones or check if you have any milestones in your repository."
                );
            }
            filtered_milestones
        }
        "🎯 Choose Specific Milestones" => {
            let filtered_milestones = prompt_open_milestones()?.filter_milestones(milestones);

            if filtered_milestones.is_empty() {
                bail!(
                    "No milestones available with the selected filter. Try including open milestones or check if you have any milestones in your repository."
                );
            }

            let milestone_options = filtered_milestones
                .into_iter()
                .map(|m| m.title.to_string())
                .collect::<Vec<_>>();

            let selected_strings =
                MultiSelect::new("📦 Select milestones for the archive:", milestone_options)
                    .with_validator(|selection: &[ListOption<&String>]| {
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

            milestones
                .iter()
                .filter(|m| selected_strings.contains(&m.title))
                .collect()
        }
        "🚫 Select No Milestones" => Vec::new(),
        _ => unreachable!("Milestone Selection Methods can only be 1 of 3 values"),
    };

    let (issue_threads, select_additional_files) = if milestone_selection_method
        != "🚫 Select No Milestones"
    {
        let approved_issues_only = Confirm::new("✅ Include approved issues only?")
            .with_default(true)
            .with_help_message(
                "n = all issues within selected milestones, Y = only approved issues",
            )
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        let mut issue_threads = get_milestone_issue_threads(&milestones, git_info, cache).await?;

        if approved_issues_only {
            issue_threads = issue_threads
                .into_iter()
                .filter(|i| i.approved_commit().is_some())
                .collect()
        };

        if !issue_threads
            .iter()
            .any(|i| git_info.branch().map(|b| b == i.branch).unwrap_or(true))
        {
            println!(
                "⚠️ No issues in selected milestones match local branch. Selecting additional files may not have commits of interested"
            );
        }

        let select_additional_files = Confirm::new("📄 Select additional files?")
            .with_default(false)
            .with_help_message("N = milestone files only, y = select additional files and commits")
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        (issue_threads, select_additional_files)
    } else {
        (Vec::new(), true)
    };

    let additional_files = if select_additional_files {
        let commits = git_info.commits(&None)?;
        let milestone_selected_files = issue_threads
            .iter()
            .map(|i| i.file.as_path())
            .collect::<Vec<_>>();
        prompt_archive_files(current_dir, &milestone_selected_files, &commits)?
    } else {
        Vec::new()
    };

    let flatten = Confirm::new("📁 Flatten archive directory structure?")
        .with_default(false)
        .with_help_message(
            "N = retain repository structure, y = strip folder structure from selected files",
        )
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let mut archive_files = Vec::new();
    for issue_thread in issue_threads {
        archive_files.push(ArchiveFile::from_issue_thread(&issue_thread, flatten)?);
    }
    for (file, commit) in additional_files {
        archive_files.push(ArchiveFile::from_file(file, commit, flatten));
    }

    // Generate default archive name based on milestones
    let default_archive_name = generate_archive_name(&milestones, git_info);
    let default_archive_path = PathBuf::from("archive").join(&default_archive_name);

    // Prompt user for archive path with default
    let archive_path_input = Text::new("📁 Enter archive path:")
        .with_default(&default_archive_path.to_string_lossy())
        .with_help_message("Press Enter to use the default path shown above")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    let final_archive_path = PathBuf::from(archive_path_input.trim());

    Ok((archive_files, final_archive_path))
}

fn prompt_open_milestones() -> Result<MilestoneSelectionFilter> {
    let include_open_milestones = Confirm::new("📦 Include open milestones?")
        .with_default(false)
        .with_help_message("N = include only closed milestones, y = include all milestones")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {e}"))?;

    if include_open_milestones {
        Ok(MilestoneSelectionFilter::All)
    } else {
        Ok(MilestoneSelectionFilter::ClosedOnly)
    }
}

pub enum MilestoneSelectionFilter {
    OpenOnly,
    ClosedOnly,
    All,
}

impl MilestoneSelectionFilter {
    pub fn filter_milestones<'a>(&self, milestones: &'a [Milestone]) -> Vec<&'a Milestone> {
        match self {
            Self::OpenOnly => milestones
                .iter()
                .filter(|m| m.state.as_deref() == Some("open"))
                .collect(),
            Self::ClosedOnly => milestones
                .iter()
                .filter(|m| m.state.as_deref() == Some("closed"))
                .collect(),
            Self::All => milestones.iter().collect(),
        }
    }
}

pub async fn get_milestone_issue_threads(
    milestones: &[&Milestone],
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
    cache: Option<&DiskCache>,
) -> Result<Vec<IssueThread>> {
    let futures = milestones
        .iter()
        .map(|&m| async move {
            let issues = git_info.get_issues(Some(m.number as u64)).await?;
            Ok::<_, anyhow::Error>(issues)
        })
        .collect::<Vec<_>>();
    let milestone_results = future::try_join_all(futures).await?;

    let mut seen_files: HashMap<String, Vec<String>> = HashMap::new();
    for issue in milestone_results.iter().flatten() {
        let entry = seen_files.entry(issue.title.to_string()).or_default();
        if let Some(milestone) = &issue.milestone {
            entry.push(milestone.title.to_string());
        }
    }
    let has_conflict = seen_files
        .iter()
        .filter(|(_, milestones)| milestones.len() > 1)
        .collect::<HashMap<_, _>>();

    if !has_conflict.is_empty() {
        bail!(
            "Files are listed multiple times in selected milestones:\n\t- {}",
            has_conflict
                .iter()
                .map(|(file, milestones)| format!("{file}: {}", milestones.join(", ")))
                .collect::<Vec<_>>()
                .join("\n\t- ")
        )
    }

    // Fetch all comments in parallel first
    let comment_futures = milestone_results
        .iter()
        .flatten()
        .map(|issue| async move { (issue, get_issue_comments(issue, cache, git_info).await) })
        .collect::<Vec<_>>();
    let comment_results = future::join_all(comment_futures).await;

    // Build IssueThreads sequentially so commit_cache can be shared
    let mut commit_cache = CommitCache::new();
    let mut issue_thread_results = Vec::new();
    for (issue, comments_result) in comment_results {
        let comments = comments_result?;
        let issue_thread =
            IssueThread::from_issue_comments(issue, &comments, git_info, &mut commit_cache)?;
        issue_thread_results.push(issue_thread);
    }

    Ok(issue_thread_results)
}

/// Interactive file selection for archive with conflict detection and commit selection
fn prompt_archive_files(
    current_dir: &PathBuf,
    selected_files: &[&Path],
    commits: &[GitCommit],
) -> Result<Vec<(PathBuf, ObjectId)>> {
    #[derive(Clone)]
    struct ArchiveFileCompleter {
        current_dir: PathBuf,
        excluded_files: HashSet<String>,
    }

    impl Autocomplete for ArchiveFileCompleter {
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
                                // Check if this file is excluded
                                if self.excluded_files.contains(&relative_path) {
                                    // Mark as unavailable with styling
                                    files
                                        .push(format!("🚫 {} (already in archive)", relative_path));
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

    // Build set of excluded files from existing issue threads
    let mut excluded_files: HashSet<String> = selected_files
        .iter()
        .map(|file| file.to_string_lossy().to_string())
        .collect();

    let mut selected_files: Vec<(PathBuf, ObjectId)> = Vec::new();

    loop {
        let prompt_text = if selected_files.is_empty() {
            "📁 Enter file path for archive (Tab for autocomplete, Enter for none):".to_string()
        } else {
            format!(
                "📁 Enter another file for archive (current: {}, Tab for autocomplete, Enter to finish):",
                selected_files
                    .iter()
                    .map(|(path, _)| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let file_completer = ArchiveFileCompleter {
            current_dir: current_dir.clone(),
            excluded_files: excluded_files.clone(),
        };

        let validator_dir = current_dir.clone();
        let validator_excluded = excluded_files.clone();
        let input = Text::new(&prompt_text)
            .with_autocomplete(file_completer)
            .with_validator(move |input: &str| {
                let trimmed = input.trim();
                // Handle case where user somehow enters the grayed-out format
                if trimmed.starts_with("🚫 ") {
                    return Ok(Validation::Invalid(
                        "This file is already in the archive. Please select a different file."
                            .into(),
                    ));
                }
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
                    } else if validator_excluded.contains(&trimmed.to_string()) {
                        Ok(Validation::Invalid(
                            "This file is already in the archive. Please select a different file."
                                .into(),
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

        // Filter commits that actually change this file
        let file_changing_commits: Vec<_> = commits
            .iter()
            .filter(|commit| commit.files.iter().any(|f| f == &file_path))
            .collect();

        if file_changing_commits.is_empty() {
            println!(
                "⚠️ No commits found that change file: {}",
                file_path.display()
            );
            continue;
        }

        // Present commit options
        let commit_options: Vec<String> = file_changing_commits
            .iter()
            .map(|commit| {
                let short_hash = commit.commit.to_string()[..8].to_string();
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
                format!("{} - {}", short_hash, short_message)
            })
            .collect();

        let commit_selection = Select::new(
            &format!("📝 Select commit for file {}:", file_path.display()),
            commit_options,
        )
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        // Extract the commit hash from the selection
        let selected_hash_str = commit_selection.split(" - ").next().unwrap_or("");
        let selected_commit = file_changing_commits
            .iter()
            .find(|commit| commit.commit.to_string().starts_with(selected_hash_str))
            .ok_or_else(|| anyhow::anyhow!("Selected commit not found"))?;

        // Add to selected files and excluded set
        selected_files.push((file_path.clone(), selected_commit.commit));
        excluded_files.insert(trimmed_input.to_string());
    }

    Ok(selected_files)
}

/// Generate archive name based on milestones and repository name
pub fn generate_archive_name(milestones: &[&Milestone], git_info: &impl GitRepository) -> String {
    // Get repository name from git_info
    let repo_name = git_info.repo();

    let archive_name = if milestones.is_empty() {
        // No milestones: archive/<repo name>.tar.gz
        format!("{}.tar.gz", repo_name)
    } else {
        // With milestones: archive/<repo name>-<milestone1-milestone2>.tar.gz
        let milestone_names: Vec<String> = milestones
            .iter()
            .map(|m| {
                // Sanitize milestone names for filename usage
                m.title
                    .chars()
                    .map(|c| match c {
                        // Replace problematic characters with dashes
                        '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '-',
                        c => c,
                    })
                    .collect::<String>()
                    // Remove consecutive dashes
                    .split('-')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("-")
            })
            .collect();

        format!("{}-{}.tar.gz", repo_name, milestone_names.join("-"))
    };

    archive_name
}
