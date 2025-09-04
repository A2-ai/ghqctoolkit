use anyhow::Result;
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{Verbosity, InfoLevel};
use qchub::{create_issue, Configuration, GitInfo};
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
        /// Milestone for the issue
        #[arg(short, long)]
        milestone: String,
        
        /// File path to create issue for
        #[arg(short, long)]
        file: PathBuf,
        
        /// Configuration directory path
        #[arg(short, long)]
        config_dir: Option<PathBuf>,
        
        /// Name of the checklist to use
        #[arg(short = 'l', long)]
        checklist_name: String,
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
        .init();

    let git_info = GitInfo::from_path(&cli.project)?;
    
    match cli.command {
        Commands::CreateIssue {
            milestone,
            file,
            config_dir,
            checklist_name,
        } => {
            let configuration = if let Some(c) = config_dir {
                log::debug!("Loading configuration from: {:?}", c);
                Configuration::from_path(&c)?
            } else {
                log::debug!("Using default configuration");
                Configuration::default()
            };
            
            create_issue(&file, &milestone, &checklist_name, &configuration, &git_info).await?;
        }
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
