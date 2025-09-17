use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use octocrab::models::Milestone;
use std::path::PathBuf;

use ghqctoolkit::cli::{RelevantFileParser, find_issue, interactive_milestone_status, interactive_status, milestone_status, single_issue_status};
use ghqctoolkit::utils::StdEnvProvider;
use ghqctoolkit::{
    Configuration, DiskCache, GitActionImpl, GitHubReader, GitHubWriter, GitInfo, GitStatusOps,
    IssueThread, QCStatus, RelevantFile, configuration_status, create_labels_if_needed,
    determine_config_info, get_repo_users, setup_configuration,
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
    /// Milestone status commands
    Milestone {
        #[command(subcommand)]
        milestone_command: MilestoneCommands,
    },
    /// Configuration management commands
    Configuration {
        #[command(subcommand)]
        configuration_command: ConfigurationCommands,
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
    },
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
    Status {
        /// Milestone names to check status for
        milestones: Vec<String>,

        /// Check status for all milestones
        #[arg(long)]
        all_milestones: bool,
    },
}

#[derive(Subcommand)]
enum ConfigurationCommands {
    Setup {
        /// git repository url to be cloned to config_dir
        git: Option<String>,
    },
    Status,
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
                } => {
                    let config_dir = determine_config_info(cli.config_dir, &env)?;
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

                    create_labels_if_needed(cache.as_ref(), qc_issue.branch(), &git_info).await?;

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
                            let issue =
                                find_issue(&milestone, &file, &milestones, &git_info).await?;
                            let issue_thread =
                                IssueThread::from_issue(&issue, cache.as_ref(), &git_info).await?;
                            let file_commits = issue_thread
                                .commits(&git_info)
                                .await
                                .ok()
                                .map(|v| v.into_iter().map(|(c, _)| c).collect::<Vec<_>>());
                            let git_status = git_info.status()?;
                            let qc_status =
                                QCStatus::determine_status(&issue_thread, &git_status, &git_info)
                                    .await?;
                            println!(
                                "{}",
                                single_issue_status(
                                    &issue_thread,
                                    &git_status,
                                    &qc_status,
                                    &file_commits
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
                MilestoneCommands::Status { milestones, all_milestones } => {
                    let cache = DiskCache::from_git_info(&git_info).ok();
                    let all_milestones_data = git_info.get_milestones().await?;

                    match (milestones.is_empty(), all_milestones) {
                        (true, false) => {
                            // Interactive mode - no milestones specified and not all_milestones
                            interactive_milestone_status(&all_milestones_data, cache.as_ref(), &git_info).await?;
                        }
                        (true, true) => {
                            // All milestones requested
                            milestone_status(&all_milestones_data, cache.as_ref(), &git_info).await?;
                        }
                        (false, false) => {
                            // Specific milestones provided - filter by name
                            let selected_milestones: Vec<Milestone> = all_milestones_data
                                .into_iter()
                                .filter(|m| milestones.contains(&m.title))
                                .collect();

                            if selected_milestones.is_empty() {
                                bail!("No matching milestones found for: {}", milestones.join(", "));
                            }

                            milestone_status(&selected_milestones, cache.as_ref(), &git_info).await?;
                        }
                        (false, true) => {
                            bail!("Cannot specify both milestone names and --all-milestones flag");
                        }
                    }
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

                let config_dir = determine_config_info(cli.config_dir, &StdEnvProvider::default())?;

                let git_action = GitActionImpl;

                setup_configuration(&config_dir, url, git_action)
                    .await
                    .map_err(|e| anyhow!("{e}"))?;

                println!(
                    "✅ Configuration successfully setup at {}",
                    config_dir.display()
                );
            }
            ConfigurationCommands::Status => {
                let env = StdEnvProvider;
                let config_dir = determine_config_info(cli.config_dir, &env)?;
                let mut configuration = Configuration::from_path(&config_dir);
                configuration.load_checklists();
                let git_info = GitInfo::from_path(&config_dir, &env).ok();

                println!("{}", configuration_status(&configuration, &git_info))
            }
        },
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
