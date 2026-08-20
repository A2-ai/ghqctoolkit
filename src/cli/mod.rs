mod archive;
mod auth;
pub mod cache;
mod config_init;
mod context;
mod file_parser;
mod interactive;
mod new_round;
pub mod rename;
mod sitrep;
mod status;

use owo_colors::OwoColorize;

pub(crate) fn section_header(title: &str) -> String {
    const WIDTH: usize = 50;
    let prefix = "── ";
    let suffix = " ";
    let dashes = WIDTH.saturating_sub(prefix.len() + title.len() + suffix.len());
    format!(
        "{}{}{}{}",
        prefix.cyan(),
        title.cyan().bold(),
        suffix,
        "─".repeat(dashes).cyan()
    )
}

// Pruned to what actually crosses a module boundary. The archive module's selection,
// categorization and callout machinery — `ArchiveSelection`, `ArchiveCategory`,
// `ArchiveSummary`, `categorize`, `partition_placeable`, `resolve_selections`,
// `build_archive_files`, `unplaceable_callout`, `UnplaceableSelection`, `has_closed_round`,
// `MilestoneIssueThread` — is reached by module path from inside `crate::cli` and by tests.
// Re-exporting it advertised internals as this crate's CLI API, which is the same
// "reads as API" smell as an exported function only tests call.
pub use archive::{
    MilestoneSelectionFilter, generate_archive_name, get_milestone_issue_threads,
    milestone_archive_files, prompt_archive, reject_unreachable_round_targets,
    report_archive_provenance,
};
pub use auth::{gh_auth_login, gh_auth_logout, gh_auth_status, gh_auth_token};
pub use cache::{CacheCommands, handle_cache};
pub use config_init::{ConfigurationEditCommands, configuration_edit, configuration_init};
pub use context::find_issue;
pub use file_parser::{
    FileCommitPair, FileCommitPairParser, IssueRoundArg, IssueRoundArgParser, IssueUrlArg,
    IssueUrlArgParser, RelevantFileArg, RelevantFileArgParser, round_targets,
};
pub use interactive::{
    prompt_assignees, prompt_checklist, prompt_collaborators, prompt_context_files,
    prompt_existing_milestone, prompt_file, prompt_issue, prompt_milestone,
    prompt_milestone_archive, prompt_milestone_record,
};
pub use new_round::{
    NewRoundArgs, NotificationArg, RepairRoundArgs, new_round, repair_open_round, report_repair,
    report_result,
};
pub use rename::{confirm_rename_noninteractive, interactive_rename};
pub use sitrep::SitRep;
pub use status::{
    MilestoneStatusReport, interactive_milestone_status, interactive_status, milestone_status,
    single_issue_status,
};
