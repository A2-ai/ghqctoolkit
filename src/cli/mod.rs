mod archive;
mod auth;
mod context;
mod file_parser;
mod interactive;
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

pub use archive::{
    MilestoneSelectionFilter, generate_archive_name, get_milestone_issue_threads, prompt_archive,
};
pub use auth::{gh_auth_login, gh_auth_logout, gh_auth_status, gh_auth_token};
pub use context::find_issue;
pub use file_parser::{
    FileCommitPair, FileCommitPairParser, IssueUrlArg, IssueUrlArgParser, RelevantFileArg,
    RelevantFileArgParser,
};
pub use interactive::{
    prompt_assignees, prompt_checklist, prompt_collaborators, prompt_context_files,
    prompt_existing_milestone, prompt_file, prompt_issue, prompt_milestone,
    prompt_milestone_archive, prompt_milestone_record,
};
pub use sitrep::SitRep;
pub use status::{
    interactive_milestone_status, interactive_status, milestone_status, single_issue_status,
};
