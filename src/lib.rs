mod configuration;
mod create;
mod git;
mod issues;

#[cfg(feature = "cli")]
pub mod cli;

pub use configuration::Configuration;
pub use create::{MilestoneStatus, create_issue};
pub use git::{GitHubApi, GitInfo};

#[cfg(feature = "cli")]
pub use cli::interactive::{prompt_checklist, prompt_file, prompt_milestone};
