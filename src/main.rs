use anyhow::Result;
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use std::path::PathBuf;

use qchub::cli::CliContext;
use qchub::{Configuration, GitInfo, RelevantFile, RelevantFileParser, create_issue};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// the git project directory for which to QC on
    #[clap(short, long, default_value = ".", global = true)]
    project: PathBuf,

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

        /// Configuration directory path
        #[arg(short, long)]
        config_dir: Option<PathBuf>,

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
}

#[cfg(feature = "cli")]
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = cli.verbose.log_level_filter();
    env_logger::Builder::new()
        .filter_level(log_level)
        .filter(Some("ureq"), log::LevelFilter::Off)
        .filter(Some("rustls"), log::LevelFilter::Off)
        .filter(Some("os_info"), log::LevelFilter::Off)
        .filter(Some("tracing"), log::LevelFilter::Off)
        .filter(Some("hyper_util"), log::LevelFilter::Off)
        .filter(Some("tower"), log::LevelFilter::Off)
        .filter(Some("mio"), log::LevelFilter::Off)
        .init();

    let git_info = GitInfo::from_path(&cli.project)?;

    match cli.command {
        Commands::Issue { issue_command } => match issue_command {
            IssueCommands::Create {
                milestone,
                file,
                config_dir,
                checklist_name,
                assignees,
                relevant_files,
            } => {
                let configuartion = if let Some(c) = config_dir {
                    Configuration::from_path(&c)?
                } else {
                    log::debug!("Configuration not specified, using default.");
                    Configuration::default()
                };

                let context = match (milestone, file, checklist_name) {
                    (Some(milestone), Some(file), Some(checklist_name)) => {
                        CliContext::from_args(
                            milestone,
                            file,
                            checklist_name,
                            assignees,
                            relevant_files,
                            configuartion,
                            git_info,
                        )
                        .await?
                    }
                    (None, None, None) => {
                        CliContext::from_interactive(&cli.project, configuartion, git_info).await?
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "Either provide all three arguments (--milestone, --file, --checklist-name) or none to enter interactive mode"
                        ));
                    }
                };

                create_issue(
                    &context.file,
                    &context.milestone_status,
                    &context.checklist,
                    context.assignees,
                    &context.configuration,
                    &context.git_info,
                    context.relevant_files,
                )
                .await?;

                println!("✅ Issue created successfully!");
            }
        },
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
