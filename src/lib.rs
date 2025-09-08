mod configuration;
mod create;
mod git;
mod issues;

#[cfg(feature = "cli")]
pub mod cli;

pub use configuration::Configuration;
pub use create::{create_issue, MilestoneStatus};
pub use git::{GitInfo, GitHubApi};

#[cfg(feature = "cli")]
pub use cli::interactive::{prompt_milestone, prompt_file, prompt_checklist};

