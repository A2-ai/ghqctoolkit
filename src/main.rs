use anyhow::Result;
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use qchub::{
    Configuration, GitHubApi, GitInfo, MilestoneStatus, RepoUser, create_issue, prompt_assignees,
    prompt_checklist, prompt_file, prompt_milestone, validate_assignees,
};
use std::path::PathBuf;

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
    CreateIssue {
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
    },
}

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
        Commands::CreateIssue {
            milestone,
            file,
            config_dir,
            checklist_name,
            assignees,
        } => {
            // Fetch users once for validation and interactive prompts
            let repo_users: Vec<RepoUser> = git_info.get_users().await?;

            // Check if we should enter interactive mode or validate all args are provided
            let interactive_mode =
                milestone.is_none() && file.is_none() && checklist_name.is_none();
            let all_provided = milestone.is_some() && file.is_some() && checklist_name.is_some();

            if !interactive_mode && !all_provided {
                return Err(anyhow::anyhow!(
                    "Either provide all three arguments (--milestone, --file, --checklist-name) or none to enter interactive mode"
                ));
            }

            // Load configuration
            let configuration = if let Some(c) = config_dir {
                log::debug!("Loading configuration from: {:?}", c);
                let mut config = Configuration::from_path(&c)?;
                config.load_checklists()?;
                config
            } else {
                log::debug!("Using default configuration");
                Configuration::default()
            };

            let (final_milestone_status, final_file, final_checklist, final_assignees) =
                if interactive_mode {
                    println!("🚀 Welcome to QCHub Interactive Mode!");
                    let milestone_status = prompt_milestone(&git_info).await?;
                    let file = prompt_file(&cli.project)?;
                    let checklist = prompt_checklist(&configuration)?;
                    let assignees = prompt_assignees(&repo_users)?;

                    println!("\n✨ Creating issue with:");
                    println!("   📊 Milestone: {}", milestone_status);
                    println!("   📁 File: {}", file.display());
                    println!("   📋 Checklist: {}", checklist);
                    if !assignees.is_empty() {
                        println!("   👥 Assignees: {}", assignees.join(", "));
                    }
                    println!();

                    (milestone_status, file, checklist, assignees)
                } else {
                    let final_assignees = assignees.unwrap_or_default();

                    // Validate assignees if provided
                    validate_assignees(&final_assignees, &repo_users)?;

                    (
                        MilestoneStatus::Unknown(milestone.unwrap()),
                        file.unwrap(),
                        checklist_name.unwrap(),
                        final_assignees,
                    )
                };

            create_issue(
                &final_file,
                &final_milestone_status,
                &final_checklist,
                final_assignees,
                &configuration,
                &git_info,
            )
            .await?;

            if interactive_mode {
                println!("✅ Issue created successfully!");
            }
        }
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
