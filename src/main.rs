use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use octocrab::models::Milestone;
use std::path::PathBuf;

use ghqctoolkit::cli::{
    FileCommitPair, FileCommitPairParser, IssueUrlArg, IssueUrlArgParser, MilestoneSelectionFilter,
    RelevantFileArg, RelevantFileArgParser, find_issue, generate_archive_name,
    get_milestone_issue_threads, interactive_milestone_status, interactive_status,
    milestone_status, prompt_archive, prompt_context_files, prompt_milestone_record,
    single_issue_status,
};
use ghqctoolkit::utils::StdEnvProvider;
use ghqctoolkit::{
    ArchiveFile, ArchiveMetadata, Configuration, ContextPosition, DiskCache, GitCommand,
    GitFileOps, GitHubReader, GitHubWriter, GitInfo, GitRepository, IssueThread, QCContext,
    QCStatus, UreqDownloader, analyze_issue_checklists, approve_with_validation, archive,
    configuration_status, create_labels_if_needed, create_staging_dir, determine_config_dir,
    fetch_milestone_issues, get_blocking_qc_status, get_git_status,
    get_milestone_issue_information, get_repo_users, record, render, setup_configuration,
    unapprove_with_impact,
};
use ghqctoolkit::{QCApprove, QCComment, QCIssue, QCReview, QCUnapprove};

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
    #[cfg(all(feature = "api", not(feature = "ui")))]
    /// Start the API server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3103")]
        port: u16,
    },
    #[cfg(feature = "ui")]
    /// Start the embedded UI server and open the browser
    Ui {
        /// Port to listen on
        #[arg(short, long, default_value = "3103")]
        port: u16,
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

        /// Description for the milestone (only used when creating a new milestone)
        #[arg(short = 'D', long)]
        description: Option<String>,

        /// Previous QC issues (issues which are previous QCs of this file or a similar file)
        /// Format: <issue_url>[::description]
        /// Example: https://github.com/owner/repo/issues/123::Previous version of this file
        #[arg(long, value_parser = IssueUrlArgParser)]
        previous_qc: Vec<IssueUrlArg>,

        /// Gating QC issues (issues which must be approved before approving this issue)
        /// Format: <issue_url>[::description]
        /// Example: https://github.com/owner/repo/issues/456::Upstream dependency
        #[arg(long, value_parser = IssueUrlArgParser)]
        gating_qc: Vec<IssueUrlArg>,

        /// Related QC issues (issues related to the file but don't have a direct impact on results)
        /// Format: <issue_url>[::description]
        /// Example: https://github.com/owner/repo/issues/789::Related analysis
        #[arg(long, value_parser = IssueUrlArgParser)]
        relevant_qc: Vec<IssueUrlArg>,

        /// Relevant files (files relevant to the QC but don't require QC themselves)
        /// Format: file_path::justification (justification is required)
        /// Example: data/config.yaml::Configuration used by this script
        #[arg(long, value_parser = RelevantFileArgParser)]
        relevant_file: Vec<RelevantFileArg>,
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

        /// Force approval even if Blocking QCs are not approved
        #[arg(long)]
        force: bool,
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
    /// Review current working directory changes against a commit
    Review {
        /// Milestone for the issue (will prompt if not provided)
        #[arg(short, long)]
        milestone: Option<String>,

        /// File path to review (will prompt if not provided)
        #[arg(short, long)]
        file: Option<PathBuf>,

        /// Commit to compare against (defaults to HEAD if not specified)
        #[arg(short, long)]
        commit: Option<String>,

        /// Optional note to include in the review
        #[arg(short, long)]
        note: Option<String>,

        /// Do not include diff between commit and working directory
        #[arg(long)]
        no_diff: bool,
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

        /// PDF documents to prepend before the main findings.
        /// Files are rendered in the order listed.
        #[arg(long)]
        prepended_context: Vec<PathBuf>,

        /// PDF documents to append after the main findings.
        /// Files are rendered in the order listed.
        #[arg(long)]
        appended_context: Vec<PathBuf>,
    },
    /// Create an archive of files from milestones
    Archive {
        /// Milestone names to archive
        milestones: Vec<String>,

        /// Archive all closed milestones
        #[arg(long, conflicts_with = "all_milestones")]
        all_closed_milestones: bool,

        /// Archive all milestones (including open)
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

        /// Additional files to include with specific commits (format: file:commit)
        #[arg(long, value_parser = FileCommitPairParser)]
        additional_file: Vec<FileCommitPair>,
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
                    description,
                    previous_qc,
                    gating_qc,
                    relevant_qc,
                    relevant_file,
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
                                description,
                                previous_qc,
                                gating_qc,
                                relevant_qc,
                                relevant_file,
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

                    let create_result = qc_issue.post_with_blocking(&git_info).await?;
                    println!("{create_result}");
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
                    force,
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
                                milestone.clone(),
                                file.clone(),
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

                    // Use approval with validation
                    let result =
                        approve_with_validation(&approval, &git_info, cache.as_ref(), force)
                            .await?;

                    println!("{}", result);
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

                    // Use unapproval with impact tree display
                    let result = unapprove_with_impact(&unapproval, &git_info).await?;

                    println!("{}", result);
                }
                IssueCommands::Review {
                    milestone,
                    file,
                    commit,
                    note,
                    no_diff,
                } => {
                    let milestones = git_info.get_milestones().await?;
                    let cache = DiskCache::from_git_info(&git_info).ok();

                    let review = match (milestone, file) {
                        (None, None) => {
                            QCReview::from_interactive(milestones, cache.as_ref(), &git_info)
                                .await?
                        }
                        (Some(m), Some(f)) => {
                            QCReview::from_args(
                                m,
                                f,
                                commit,
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
                                "Must provide both milestone and file arguments, or neither to enter interactive mode"
                            )
                        }
                    };

                    // Post the review comment
                    let review_url = git_info.post_comment(&review).await?;

                    println!("📝 Review comment created!");
                    println!("{}", review_url);
                }
                IssueCommands::Status { milestone, file } => {
                    let milestones = git_info.get_milestones().await?;
                    let cache = DiskCache::from_git_info(&git_info).ok();
                    match (milestone, file) {
                        (Some(milestone), Some(file)) => {
                            let issue =
                                find_issue(&milestone, &file, &milestones, &git_info).await?;
                            let checklist_summaries =
                                analyze_issue_checklists(issue.body.as_deref());
                            let issue_thread =
                                IssueThread::from_issue(&issue, cache.as_ref(), &git_info).await?;
                            let git_status = get_git_status(&git_info)?;
                            let qc_status = QCStatus::determine_status(&issue_thread);
                            let file_commits = issue_thread.file_commits();
                            let blocking_qc_status = get_blocking_qc_status(
                                &issue_thread.blocking_qcs,
                                &git_info,
                                cache.as_ref(),
                            )
                            .await;
                            println!(
                                "{}",
                                single_issue_status(
                                    &issue_thread,
                                    &git_status.state,
                                    &qc_status,
                                    &git_status.dirty,
                                    &file_commits,
                                    &checklist_summaries,
                                    &blocking_qc_status
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
                    prepended_context,
                    appended_context,
                } => {
                    let config_dir = determine_config_dir(cli.config_dir, &env)?;
                    let configuration = Configuration::from_path(&config_dir);

                    let cache = DiskCache::from_git_info(&git_info).ok();

                    let milestones_data = git_info.get_milestones().await?;

                    // Determine if we're in interactive mode (no CLI args provided)
                    let is_interactive_mode = milestones.is_empty()
                        && !all_milestones
                        && record_path.is_none()
                        && prepended_context.is_empty()
                        && appended_context.is_empty();

                    let (selected_milestones, interactive_record_path, interactive_only_tables) =
                        match (milestones.is_empty(), all_milestones, record_path.is_none()) {
                            (true, false, true) if is_interactive_mode => {
                                // Interactive mode - no milestones specified, not all_milestones, and no record_path
                                prompt_milestone_record(&milestones_data)?
                            }
                            (true, false, true) => {
                                // Context files provided but no milestones - need milestones
                                bail!(
                                    "Please specify milestone names or use --all-milestones when using context files."
                                );
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

                    // Build context files from CLI args or interactive prompt
                    let context_files: Vec<QCContext> = if is_interactive_mode {
                        // Interactive mode - prompt for context files
                        prompt_context_files(&cli.directory)?
                    } else {
                        // CLI mode - build from prepended_context and appended_context args
                        let mut contexts = Vec::new();
                        for path in prepended_context {
                            contexts.push(QCContext::new(&path, ContextPosition::Prepend));
                        }
                        for path in appended_context {
                            contexts.push(QCContext::new(&path, ContextPosition::Append));
                        }
                        contexts
                    };

                    let issues = fetch_milestone_issues(&selected_milestones, &git_info).await?;

                    // Create staging directory for images, logo, and template
                    let staging_dir = create_staging_dir()?;

                    let http_downloader = UreqDownloader::new();
                    let issue_information = get_milestone_issue_information(
                        &issues,
                        cache.as_ref(),
                        &git_info,
                        &http_downloader,
                        &staging_dir,
                    )
                    .await?;

                    let record_str = record(
                        &selected_milestones,
                        &issue_information,
                        &configuration,
                        &git_info,
                        &env,
                        interactive_only_tables,
                        &staging_dir,
                    )?;
                    let final_record_path = interactive_record_path.or(record_path);
                    let record_path = if let Some(mut record_path) = final_record_path {
                        record_path.set_extension(".pdf");
                        // Make path relative to the directory argument
                        if record_path.is_relative() {
                            cli.directory.join(record_path)
                        } else {
                            record_path
                        }
                    } else {
                        // Default record path in the directory argument location
                        cli.directory.join(format!(
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

                    render(
                        &record_str,
                        &record_path,
                        &staging_dir,
                        &context_files,
                        cache.as_ref(),
                        &http_downloader,
                    )?;

                    println!(
                        "✅ Record successfully generated at {}",
                        record_path.display()
                    );
                }
                MilestoneCommands::Archive {
                    milestones,
                    all_closed_milestones,
                    all_milestones,
                    include_unapproved,
                    flatten,
                    archive_path,
                    additional_file,
                } => {
                    let selected_archive_files = if !additional_file.is_empty() {
                        let commits = git_info.commits(&None)?;
                        additional_file
                            .iter()
                            .map(|file| file.into_archive_file(&commits, flatten))
                            .collect::<Result<Vec<_>>>()?
                    } else {
                        Vec::new()
                    };

                    let cache = DiskCache::from_git_info(&git_info).ok();

                    let milestones_data = git_info.get_milestones().await?;

                    // Determine milestone selection first
                    let (mut archive_files, archive_path) = match (
                        milestones.is_empty(),
                        all_closed_milestones,
                        all_milestones,
                    ) {
                        (true, false, false)
                            if archive_path.is_none() && additional_file.is_empty() =>
                        {
                            // Interactive mode - no milestones, no archive_path, no file_commit
                            prompt_archive(
                                &milestones_data,
                                &cli.directory,
                                &git_info,
                                cache.as_ref(),
                            )
                            .await?
                        }
                        (true, false, false) => {
                            if additional_file.is_empty() {
                                bail!(
                                    "Must specify milestones and/or file commits to generate an archive"
                                );
                            }
                            // No milestones but have file_commit or archive_path - just use empty milestone files
                            let archive_path = archive_path.unwrap_or(
                                PathBuf::from("archive")
                                    .join(generate_archive_name(&[], &git_info)),
                            );
                            (Vec::new(), archive_path)
                        }
                        (true, false, true) => {
                            // All milestones requested
                            let selected_milestones =
                                MilestoneSelectionFilter::All.filter_milestones(&milestones_data);
                            let artifact_files = get_milestone_issue_threads(
                                &selected_milestones,
                                &git_info,
                                cache.as_ref(),
                            )
                            .await?
                            .into_iter()
                            .filter(|i| include_unapproved || i.approved_commit().is_some())
                            .map(|i| ArchiveFile::from_issue_thread(&i, flatten))
                            .collect::<std::result::Result<Vec<ArchiveFile>, _>>()?;

                            let archive_path = archive_path.unwrap_or(
                                PathBuf::from("archive")
                                    .join(generate_archive_name(&selected_milestones, &git_info)),
                            );
                            (artifact_files, archive_path)
                        }
                        (true, true, false) => {
                            // All closed milestones requested
                            let selected_milestones = MilestoneSelectionFilter::ClosedOnly
                                .filter_milestones(&milestones_data);

                            if selected_milestones.is_empty() {
                                bail!("No closed milestones found in repository");
                            }

                            let artifact_files = get_milestone_issue_threads(
                                &selected_milestones,
                                &git_info,
                                cache.as_ref(),
                            )
                            .await?
                            .into_iter()
                            .filter(|i| include_unapproved || i.approved_commit().is_some())
                            .map(|i| ArchiveFile::from_issue_thread(&i, flatten))
                            .collect::<std::result::Result<Vec<ArchiveFile>, _>>()?;

                            let archive_path = archive_path.unwrap_or(
                                PathBuf::from("archive")
                                    .join(generate_archive_name(&selected_milestones, &git_info)),
                            );
                            (artifact_files, archive_path)
                        }
                        (false, false, false) => {
                            // Specific milestones provided
                            let selected_milestones: Vec<_> = milestones_data
                                .iter()
                                .filter(|m| milestones.contains(&m.title))
                                .collect();

                            if selected_milestones.is_empty() {
                                bail!(
                                    "No matching milestones found for: {}",
                                    milestones.join(", ")
                                );
                            }

                            let artifact_files = get_milestone_issue_threads(
                                &selected_milestones,
                                &git_info,
                                cache.as_ref(),
                            )
                            .await?
                            .into_iter()
                            .filter(|i| include_unapproved || i.approved_commit().is_some())
                            .map(|i| ArchiveFile::from_issue_thread(&i, flatten))
                            .collect::<std::result::Result<Vec<ArchiveFile>, _>>()?;

                            let archive_path = archive_path.unwrap_or(
                                PathBuf::from("archive")
                                    .join(generate_archive_name(&selected_milestones, &git_info)),
                            );
                            (artifact_files, archive_path)
                        }
                        (false, true, true) => {
                            bail!("Cannot specify both milestone names and --all-milestones flag");
                        }
                        (false, true, false) => {
                            bail!(
                                "Cannot specify both milestone names and --all-closed-milestones flag"
                            );
                        }
                        (false, false, true) => {
                            bail!("Cannot specify both milestone names and --all-milestones flag");
                        }
                        (true, true, true) => {
                            bail!(
                                "Cannot specify both --all-closed-milestones and --all-milestones flags"
                            );
                        }
                    };

                    archive_files.extend(selected_archive_files);
                    let archive_path = if archive_path.is_absolute() {
                        archive_path
                    } else {
                        cli.directory.join(&archive_path)
                    };

                    // Create the actual archive using ArchiveFile approach
                    let metadata = ArchiveMetadata::new(archive_files, &env)?;
                    archive(metadata, &git_info, &archive_path)?;

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

                let config_dir = determine_config_dir(cli.config_dir, &StdEnvProvider::default())?;

                let git_action = GitCommand;

                setup_configuration(&config_dir, url, &git_action)
                    .await
                    .map_err(|e| anyhow!("{e}"))?;

                println!(
                    "✅ Configuration successfully setup at {}",
                    config_dir.display()
                );
            }
            ConfigurationCommands::Status => {
                let env = StdEnvProvider;
                let config_dir = determine_config_dir(cli.config_dir, &env)?;
                let mut configuration = Configuration::from_path(&config_dir);
                configuration.load_checklists();
                let git_info = GitInfo::from_path(&config_dir, &env).ok();

                println!("{}", configuration_status(&configuration, &git_info))
            }
        },
        #[cfg(all(feature = "api", not(feature = "ui")))]
        Commands::Serve { port } => {
            use ghqctoolkit::api::{AppState, create_router};

            let config_dir = determine_config_dir(cli.config_dir, &env)?;
            let mut configuration = Configuration::from_path(&config_dir);
            configuration.load_checklists();
            let configuration_git_info = match GitInfo::from_path(&configuration.path, &env) {
                Ok(g) => Some(g),
                Err(e) => {
                    log::warn!(
                        "Failed to determine configuration git info: {e}. Continuing without git status checks"
                    );
                    None
                }
            };

            let git_info = GitInfo::from_path(&cli.directory, &env)?;
            let disk_cache = DiskCache::from_git_info(&git_info).ok();

            let state = AppState::new(git_info, configuration, configuration_git_info, disk_cache)
                .with_creator(|path| GitInfo::from_path(path, &StdEnvProvider).ok());
            let app = create_router(state);

            let addr = format!("0.0.0.0:{}", port);
            println!("Starting API server on http://{}", addr);

            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, app).await?;
        }
        #[cfg(feature = "ui")]
        Commands::Ui { port } => {
            use ghqctoolkit::api::AppState;

            let config_dir = determine_config_dir(cli.config_dir, &env)?;
            let mut configuration = Configuration::from_path(&config_dir);
            configuration.load_checklists();
            let configuration_git_info = match GitInfo::from_path(&configuration.path, &env) {
                Ok(g) => Some(g),
                Err(e) => {
                    log::warn!(
                        "Failed to determine configuration git info: {e}. Continuing without git status checks"
                    );
                    None
                }
            };

            let git_info = GitInfo::from_path(&cli.directory, &env)?;
            let disk_cache = DiskCache::from_git_info(&git_info).ok();

            let state = AppState::new(git_info, configuration, configuration_git_info, disk_cache)
                .with_creator(|path| GitInfo::from_path(path, &StdEnvProvider).ok());
            ghqctoolkit::ui::run(port, state).await?;
        }
    }

    Ok(())
}

#[cfg(not(feature = "cli"))]
fn main() {
    println!("CLI feature not enabled. Build with --features cli to use the CLI.");
}
