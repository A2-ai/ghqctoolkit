use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use std::path::PathBuf;

use ghqctoolkit::QCComment;
use ghqctoolkit::cli::{CreateContext, RelevantFileParser};
use ghqctoolkit::utils::StdEnvProvider;
use ghqctoolkit::{
    Configuration, GitActionImpl, GitHubApi, GitInfo, RelevantFile, create_issue,
    determine_config_info, setup_configuration,
};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// the git project directory for which to QC on
    #[clap(short = 'd', long, default_value = ".", global = true)]
    directory: PathBuf,

    /// Configuration directory path
    #[arg(short, long)]
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
        #[arg(short = 'l', long)]
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

        /// File path to create issue for (will prompt if not provided)
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
}

#[derive(Subcommand)]
enum ConfigurationCommands {
    Setup {
        /// git repository url to be cloned to config_dir
        git: Option<String>,
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
                } => {
                    let config_dir = determine_config_info(cli.config_dir, &env)?;
                    let mut configuration = Configuration::from_path(&config_dir);
                    configuration.load_checklists();

                    // Fetch milestones first
                    let milestones = git_info.get_milestones().await?;

                    let context = match (milestone, file, checklist_name) {
                        (Some(milestone_name), Some(file), Some(checklist_name)) => {
                            CreateContext::from_args(
                                milestone_name,
                                &milestones,
                                file,
                                checklist_name,
                                assignees,
                                relevant_files,
                                configuration,
                                git_info,
                            )
                            .await?
                        }
                        (None, None, None) => {
                            CreateContext::from_interactive(
                                &cli.directory,
                                &milestones,
                                configuration,
                                git_info,
                            )
                            .await?
                        }
                        _ => {
                            bail!(
                                "Either provide all three arguments (--milestone, --file, --checklist-name) or none to enter interactive mode"
                            );
                        }
                    };

                    let issue_url = create_issue(
                        &context.file,
                        &context.milestone_status,
                        &context.checklist,
                        context.assignees,
                        &context.git_info,
                        context.relevant_files,
                    )
                    .await?;

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

                    let comment = match (milestone, file) {
                        (None, None) => {
                            // Interactive mode
                            QCComment::from_interactive(&milestones, &git_info).await?
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
        },
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
