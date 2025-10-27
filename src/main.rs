use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use octocrab::models::Milestone;
use std::path::PathBuf;

use ghqctoolkit::cli::{
    RelevantFileParser, find_issue, interactive_milestone_status, interactive_status,
    milestone_status, prompt_milestone_archive, prompt_milestone_record, single_issue_status,
};
use ghqctoolkit::utils::StdEnvProvider;
use ghqctoolkit::{
    Configuration, DiskCache, GitCommand, GitFileOps, GitHubReader, GitHubWriter, GitInfo,
    GitRepository, GitStatusOps, IssueThread, QCStatus, RelevantFile, compress,
    configuration_status, create_labels_if_needed, determine_config_dir, fetch_milestone_issues,
    get_archive_content, get_milestone_issue_information, get_repo_users, is_excel_file, record,
    render, setup_configuration,
};
use ghqctoolkit::{QCApprove, QCComment, QCIssue, QCUnapprove};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// the git project directory for which to QC on
    #[clap(short = 'd', long, default_value = ".", global = true)]
    directory: PathBuf,

    /// Configuration directory path
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,

    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,
}

#[derive(Subcommand)]
enum Commands {
    /// Issue management commands
    Issue {
        #[command(subcommand)]
        issue_command: IssueCommands,
    },
    /// Milestone management commands
    Milestone {
        #[command(subcommand)]
        milestone_command: MilestoneCommands,
    },
    /// Configuration management commands
    Configuration {
        #[command(subcommand)]
        configuration_command: ConfigurationCommands,
    },
    /// Excel file commands
    Excel {
        #[command(subcommand)]
        excel_command: ExcelCommands,
    },
}

#[derive(Subcommand)]
enum IssueCommands {
    /// Create a new issue for quality control
    Create {
        /// Milestone for the issue (will prompt if not provided)
        #[arg(short, long)]
        milestone: Option<String>,

        /// File path to create issue for (will prompt if not provided)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Name of the checklist to use (will prompt if not provided)
        #[arg(short, long)]
        checklist_name: Option<String>,

        /// Assignees for the issue (usernames)
        #[arg(short, long)]
        assignees: Option<Vec<String>>,

        /// Additional relevant files for the issue (format: "name:path" or just "path")
        #[arg(short = 'r', long, value_parser = RelevantFileParser)]
        relevant_files: Option<Vec<RelevantFile>>,

        /// Description for the milestone (only used when creating a new milestone)
        #[arg(short = 'D', long)]
        description: Option<String>,
    },
    /// Comment on an existing issue, providing updated context
    Comment {
        /// Milestone for the issue (will prompt if not provided)
        #[arg(short, long)]
        milestone: Option<String>,

        /// File path of issue to comment on (will prompt if not provided)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Current commit (defaults to most recent file commit if not in interactive mode)
        #[arg(short, long)]
        current_commit: Option<String>,

        /// Previous commit (defaults to second most recent file commit if not in interactive mode)
        #[arg(short, long)]
        previous_commit: Option<String>,

        /// Optional note to include in the comment
        #[arg(short, long)]
        note: Option<String>,

        /// Do not include commit diff between files even if possible. No effect in interactive mode
        #[arg(long)]
        no_diff: bool,
    },
    /// Approve and close an existing issue
    Approve {
        /// Milestone for the issue (will prompt if not provided)
        #[arg(short, long)]
        milestone: Option<String>,

        /// File path of issue to approve and close (will prompt if not provided)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Approved commit (defaults to most recent file commit if not in interactive mode)
        #[arg(short, long)]
        approved_commit: Option<String>,

        /// Optional note to include in the approval
        #[arg(short, long)]
        note: Option<String>,
    },
    /// Unapprove a closed issue
    Unapprove {
        /// Milestone for the issue (will prompt if not provided)
        #[arg(short, long)]
        milestone: Option<String>,

        /// File path of issue to un-approve and re-open (will prompt if not provided)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Reason to re-open issue (will prompt if not provided)
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// detailed status of the ongoing qc issue
    Status {
        /// Milestone for the issue (will prompt if not provided)
        #[arg(short, long)]
        milestone: Option<String>,

        /// File path of issue to check status for (will prompt if not provided)
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum MilestoneCommands {
    /// Overview of the status of the issues within the milestone(s)
    Status {
        /// Milestone names to check status for
        milestones: Vec<String>,

        /// Check status for all milestones
        #[arg(long)]
        all_milestones: bool,
    },
    /// Generate a record for the milestones within the repository
    Record {
        /// Milestone names to create record for
        milestones: Vec<String>,

        /// Make record for all milestones
        #[arg(long)]
        all_milestones: bool,

        /// File name to save the record pdf as. Will default to <repo>_<milestone names>.pdf
        #[arg(short, long)]
        record_path: Option<PathBuf>,

        /// Only include tables and skip detailed issue content
        #[arg(long)]
        only_tables: bool,
    },
    /// Create an archive of files from milestones
    Archive {
        /// Milestone names to archive
        milestones: Vec<String>,

        /// Archive all milestones
        #[arg(long)]
        all_milestones: bool,

        /// Include unapproved issues in archive
        #[arg(long)]
        include_unapproved: bool,

        /// Flatten archive structure (put all files in root directory)
        #[arg(long)]
        flatten: bool,

        /// File name to save the archive as. Will default to <repo>_<milestone names>.tar.gz
        #[arg(short, long)]
        archive_path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigurationCommands {
    /// Set-up the custom configuration to be used by the tool
    Setup {
        /// git repository url to be cloned
        git: Option<String>,
    },
    /// Status of the configuration repository
    Status,
}

#[derive(Subcommand)]
enum ExcelCommands {
    /// perform a diff on an excel file and save a colorized version locally
    Diff {
        /// what excel file to perform diff on
        file: PathBuf,
        /// what commit to compare from
        #[arg(short, long)]
        from: String,
        /// what commit to compare to
        #[arg(short, long)]
        to: String,
        /// where to save the resulting file. By default, `./ghqc/<file basename>_<from sha>...<to sha>.xlsx`
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = cli.verbose.log_level_filter();
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Off) // Turn off all logs by default
        .filter(Some("ghqctoolkit"), log_level) // Allow logs from your crate
        .filter(Some("octocrab"), log_level) // Allow logs from octocrab
        .init();

    let env = StdEnvProvider;

    match cli.command {
        Commands::Issue { issue_command } => {
            let git_info = GitInfo::from_path(&cli.directory, &env)?;

            match issue_command {
                IssueCommands::Create {
                    milestone,
                    file,
                    checklist_name,
                    assignees,
                    relevant_files,
                    description,
                } => {
                    let config_dir = determine_config_dir(cli.config_dir, &env)?;
                    let mut configuration = Configuration::from_path(&config_dir);
                    configuration.load_checklists();

                    // Fetch milestones first
                    let milestones = git_info.get_milestones().await?;
                    let cache = DiskCache::from_git_info(&git_info).ok();
                    let repo_users = get_repo_users(cache.as_ref(), &git_info).await?;

                    let qc_issue = match (milestone, file, checklist_name) {
                        (Some(milestone_name), Some(file), Some(checklist_name)) => {
                            QCIssue::from_args(
                                milestone_name,
                                file,
                                checklist_name,
                                assignees,
                                relevant_files,
                                description,
                                milestones,
                                &repo_users,
                                configuration,
                                &git_info,
                            )
                            .await?
                        }
                        (None, None, None) => {
                            QCIssue::from_interactive(
                                &cli.directory,
                                milestones,
                                configuration,
                                &git_info,
                                &repo_users,
                            )
                            .await?
                        }
                        _ => {
                            bail!(
                                "Either provide all three arguments (--milestone, --file, --checklist-name) or none to enter interactive mode"
                            );
                        }
                    };

                    create_labels_if_needed(cache.as_ref(), Some(qc_issue.branch()), &git_info)
                        .await?;

                    let issue_url = git_info.post_issue(&qc_issue).await?;

                    println!("✅ Issue created successfully!");
                    println!("{}", issue_url);
                }
                IssueCommands::Comment {
                    milestone,
                    file,
                    current_commit,
                    previous_commit,
                    note,
                    no_diff,
                } => {
                    // Fetch milestones first
                    let milestones = git_info.get_milestones().await?;
                    let cache = DiskCache::from_git_info(&git_info).ok();

                    let comment = match (milestone, file) {
                        (None, None) => {
                            // Interactive mode
                            QCComment::from_interactive(&milestones, cache.as_ref(), &git_info)
                                .await?
                        }
                        (Some(milestone), Some(file)) => {
                            // Non-interactive mode
                            QCComment::from_args(
                                milestone,
                                file,
                                current_commit,
                                previous_commit,
                                note,
                                &milestones,
                                cache.as_ref(),
                                &git_info,
                                no_diff,
                            )
                            .await?
                        }
                        _ => {
                            bail!(
                                "Must provide both --milestone and --file arguments or neither to enter interactive mode"
                            )
                        }
                    };

                    let comment_url = git_info.post_comment(&comment).await?;

                    println!("✅ Comment created!");
                    println!("{}", comment_url);
                }
                IssueCommands::Approve {
                    milestone,
                    file,
                    approved_commit,
                    note,
                } => {
                    let milestones = git_info.get_milestones().await?;
                    let cache = DiskCache::from_git_info(&git_info).ok();
                    let approval = match (milestone, file, &note) {
                        (None, None, None) => {
                            // Interactive Mode
                            QCApprove::from_interactive(&milestones, cache.as_ref(), &git_info)
                                .await?
                        }
                        (Some(milestone), Some(file), _) => {
                            QCApprove::from_args(
                                milestone,
                                file,
                                approved_commit,
                                note,
                                &milestones,
                                cache.as_ref(),
                                &git_info,
                            )
                            .await?
                        }
                        _ => {
                            bail!(
                                "Must provide both --milestone and --file arguments or no arguments to enter interactive mode"
                            )
                        }
                    };

                    let approval_url = git_info.post_approval(&approval).await?;

                    println!("✅ Approval created and issue closed!");
                    println!("{}", approval_url);
                }
                IssueCommands::Unapprove {
                    milestone,
                    file,
                    reason,
                } => {
                    let milestones = git_info.get_milestones().await?;
                    let unapproval = match (milestone, file, &reason) {
                        (None, None, None) => {
                            // Interactive Mode
                            QCUnapprove::from_interactive(&milestones, &git_info).await?
                        }
                        (Some(milestone), Some(file), Some(reason)) => {
                            QCUnapprove::from_args(
                                milestone,
                                file,
                                reason.clone(),
                                &milestones,
                                &git_info,
                            )
                            .await?
                        }
                        _ => {
                            bail!(
                                "Must provide all arguments (--milestone, --file, --reason) or no arguments to enter interactive mode"
                            )
                        }
                    };

                    let unapproval_url = git_info.post_unapproval(&unapproval).await?;

                    println!("🚫 Issue unapproved and reopened!");
                    println!("{}", unapproval_url);
                }
                IssueCommands::Status { milestone, file } => {
                    let milestones = git_info.get_milestones().await?;
                    let cache = DiskCache::from_git_info(&git_info).ok();
                    match (milestone, file) {
                        (Some(milestone), Some(file)) => {
                            use ghqctoolkit::analyze_issue_checklists;

                            let issue =
                                find_issue(&milestone, &file, &milestones, &git_info).await?;
                            let checklist_summaries = analyze_issue_checklists(&issue);
                            let issue_thread =
                                IssueThread::from_issue(&issue, cache.as_ref(), &git_info).await?;
                            let git_status = git_info.status()?;
                            let qc_status = QCStatus::determine_status(&issue_thread)?;
                            let file_commits = issue_thread.file_commits();
                            println!(
                                "{}",
                                single_issue_status(
                                    &issue_thread,
                                    &git_status,
                                    &qc_status,
                                    &file_commits,
                                    &checklist_summaries,
                                )
                            );
                        }
                        (None, None) => {
                            // Interactive mode
                            interactive_status(&milestones, cache.as_ref(), &git_info).await?;
                        }
                        _ => {
                            bail!(
                                "Must provide both --milestone and --file arguments or neither to enter interactive mode"
                            )
                        }
                    }
                }
            }
        }
        Commands::Milestone { milestone_command } => {
            let git_info = GitInfo::from_path(&cli.directory, &env)?;

            match milestone_command {
                MilestoneCommands::Status {
                    milestones,
                    all_milestones,
                } => {
                    let cache = DiskCache::from_git_info(&git_info).ok();
                    let all_milestones_data = git_info.get_milestones().await?;

                    match (milestones.is_empty(), all_milestones) {
                        (true, false) => {
                            // Interactive mode - no milestones specified and not all_milestones
                            interactive_milestone_status(
                                &all_milestones_data,
                                cache.as_ref(),
                                &git_info,
                            )
                            .await?;
                        }
                        (true, true) => {
                            // All milestones requested
                            milestone_status(&all_milestones_data, cache.as_ref(), &git_info)
                                .await?;
                        }
                        (false, false) => {
                            // Specific milestones provided - filter by name
                            let selected_milestones: Vec<Milestone> = all_milestones_data
                                .into_iter()
                                .filter(|m| milestones.contains(&m.title))
                                .collect();

                            if selected_milestones.is_empty() {
                                bail!(
                                    "No matching milestones found for: {}",
                                    milestones.join(", ")
                                );
                            }

                            milestone_status(&selected_milestones, cache.as_ref(), &git_info)
                                .await?;
                        }
                        (false, true) => {
                            bail!("Cannot specify both milestone names and --all-milestones flag");
                        }
                    }
                }
                MilestoneCommands::Record {
                    milestones,
                    all_milestones,
                    record_path,
                    only_tables,
                } => {
                    let config_dir = determine_config_dir(cli.config_dir, &env)?;
                    let configuration = Configuration::from_path(&config_dir);

                    let cache = DiskCache::from_git_info(&git_info).ok();

                    let milestones_data = git_info.get_milestones().await?;

                    let (selected_milestones, interactive_record_path, interactive_only_tables) =
                        match (milestones.is_empty(), all_milestones, record_path.is_none()) {
                            (true, false, true) => {
                                // Interactive mode - no milestones specified, not all_milestones, and no record_path
                                prompt_milestone_record(&milestones_data)?
                            }
                            (true, true, _) => {
                                // All milestones requested
                                (milestones_data, None, only_tables)
                            }
                            (false, false, _) => {
                                // Specific milestones provided - filter by name
                                let selected: Vec<Milestone> = milestones_data
                                    .into_iter()
                                    .filter(|m| milestones.contains(&m.title))
                                    .collect();

                                if selected.is_empty() {
                                    bail!(
                                        "No matching milestones found for: {}",
                                        milestones.join(", ")
                                    );
                                }

                                (selected, None, only_tables)
                            }
                            (false, true, _) => {
                                bail!(
                                    "Cannot specify both milestone names and --all-milestones flag"
                                );
                            }
                            (true, false, false) => {
                                bail!(
                                    "Cannot use interactive mode when record_path is specified. Please specify milestone names or use --all-milestones."
                                );
                            }
                        };

                    let issues = fetch_milestone_issues(&selected_milestones, &git_info).await?;
                    let issue_information =
                        get_milestone_issue_information(&issues, cache.as_ref(), &git_info).await?;

                    let record_str = record(
                        &selected_milestones,
                        &issue_information,
                        &configuration,
                        &git_info,
                        &env,
                        interactive_only_tables,
                    )?;
                    let final_record_path = interactive_record_path.or(record_path);
                    let record_path = if let Some(mut record_path) = final_record_path {
                        record_path.set_extension(".pdf");
                        record_path
                    } else {
                        PathBuf::from(format!(
                            "{}-{}.pdf",
                            git_info.repo(),
                            issues
                                .keys()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join("-")
                                .replace(" ", "-")
                        ))
                    };

                    render(&record_str, &record_path)?;

                    println!(
                        "✅ Record successfully generated at {}",
                        record_path.display()
                    );
                }
                MilestoneCommands::Archive {
                    milestones,
                    all_milestones,
                    include_unapproved,
                    flatten,
                    archive_path,
                } => {
                    let cache = DiskCache::from_git_info(&git_info).ok();

                    let milestones_data = git_info.get_milestones().await?;

                    let (
                        selected_milestones,
                        interactive_archive_path,
                        interactive_include_unapproved,
                        interactive_flatten,
                    ) = match (
                        milestones.is_empty(),
                        all_milestones,
                        archive_path.is_none(),
                    ) {
                        (true, false, true) => {
                            // Interactive mode - no milestones specified, not all_milestones, and no archive_path
                            prompt_milestone_archive(&milestones_data)?
                        }
                        (true, true, _) => {
                            // All milestones requested
                            (milestones_data, None, include_unapproved, flatten)
                        }
                        (false, false, _) => {
                            // Specific milestones provided - filter by name
                            let selected: Vec<Milestone> = milestones_data
                                .into_iter()
                                .filter(|m| milestones.contains(&m.title))
                                .collect();

                            if selected.is_empty() {
                                bail!(
                                    "No matching milestones found for: {}",
                                    milestones.join(", ")
                                );
                            }

                            (selected, None, include_unapproved, flatten)
                        }
                        (false, true, _) => {
                            bail!("Cannot specify both milestone names and --all-milestones flag");
                        }
                        (true, false, false) => {
                            bail!(
                                "Cannot use interactive mode when archive_path is specified. Please specify milestone names or use --all-milestones."
                            );
                        }
                    };

                    let archive_content = get_archive_content(
                        cache.as_ref(),
                        &selected_milestones,
                        interactive_include_unapproved,
                        &git_info,
                    )
                    .await?;

                    let final_archive_path = interactive_archive_path.or(archive_path);
                    let archive_path = if let Some(mut archive_path) = final_archive_path {
                        if !archive_path.to_string_lossy().ends_with(".tar.gz") {
                            archive_path.set_extension("tar.gz");
                        }
                        archive_path
                    } else {
                        let milestone_names: Vec<&str> = selected_milestones
                            .iter()
                            .map(|m| m.title.as_str())
                            .collect();
                        PathBuf::from(format!(
                            "{}-{}.tar.gz",
                            git_info.repo(),
                            milestone_names.join("-").replace(" ", "-")
                        ))
                    };

                    compress(&archive_content, interactive_flatten, &archive_path)?;

                    println!(
                        "✅ Archive successfully created at {}",
                        archive_path.display()
                    );
                }
            }
        }
        Commands::Configuration {
            configuration_command,
        } => match configuration_command {
            ConfigurationCommands::Setup { git } => {
                let url = if let Some(git) = git {
                    gix::url::parse(git.as_str().into())
                        .map_err(|e| anyhow!("provided url {git} is not a valid git url: {e}"))?
                } else {
                    if let Ok(git) = std::env::var("GHQC_CONFIG_HOME") {
                        gix::url::parse(git.as_str().into()).map_err(|e| {
                            anyhow!("GHQC_CONFIG_HOME value {git} is not a valid git url: {e}")
                        })?
                    } else {
                        bail!(
                            "Must provide `git` flag or have the environment variable `GHQC_CONFIG_HOME` set"
                        );
                    }
                };

                let config_dir = determine_config_dir(cli.config_dir, &env)?;

                let git_action = GitCommand;

                setup_configuration(&config_dir, url, git_action)
                    .await
                    .map_err(|e| anyhow!("{e}"))?;

                println!(
                    "✅ Configuration successfully setup at {}",
                    config_dir.display()
                );
            }
            ConfigurationCommands::Status => {
                let config_dir = determine_config_dir(cli.config_dir, &env)?;
                let mut configuration = Configuration::from_path(&config_dir);
                configuration.load_checklists();
                let git_info = GitInfo::from_path(&config_dir, &env).ok();

                println!("{}", configuration_status(&configuration, &git_info))
            }
        },
        Commands::Excel { excel_command } => match excel_command {
            ExcelCommands::Diff {
                file,
                from,
                to,
                output,
            } => {
                use ghqctoolkit::create_excel_diff;

                if !is_excel_file(&file) {
                    bail!("File {} is not an excel file", file.display());
                }

                let git_info = GitInfo::from_path(&cli.directory, &env)?;
                let commits = git_info.commits(&None)?;
                if from.len() < 7 || to.len() < 7 {
                    bail!(
                        "From ({} characters) and to ({} characters) commits must be at least 7 characters long",
                        from.len(),
                        to.len()
                    );
                }

                let Some(from_commit) = commits
                    .iter()
                    .find(|c| c.commit.to_string().starts_with(&from))
                else {
                    bail!("No commits found for from commit: {from}");
                };

                let Some(to_commit) = commits
                    .iter()
                    .find(|c| c.commit.to_string().starts_with(&to))
                else {
                    bail!("No commits found for to commit: {to}");
                };

                let output_path = match output {
                    Some(o) => o,
                    None => {
                        let file_name = file
                            .file_stem()
                            .unwrap_or(file.as_os_str())
                            .to_string_lossy();
                        let from_str = &from_commit.commit.to_string()[..7];
                        let to_str = &to_commit.commit.to_string()[..7];

                        PathBuf::from("ghqc").join(format!("{file_name}_{from_str}..{to_str}.xlsx"))
                    }
                };

                // Create output directory if it doesn't exist
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // Create the Excel diff
                create_excel_diff(
                    &file,
                    &from_commit.commit,
                    &to_commit.commit,
                    &git_info,
                    &output_path,
                )?;

                println!("Excel diff created: {}", output_path.display());
            }
        },
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
