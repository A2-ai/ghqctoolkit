use gix::Repository;
use std::path::{Path, PathBuf};

mod action;
mod api;
mod auth;
mod commit_analysis;
mod file_ops;
mod helpers;
mod provider;
mod repository;
mod status;

pub use action::{GitCli, GitCliError, GitCommand};
pub use api::{GitComment, GitHubApiError, GitHubReader, GitHubWriter, RepoUser};
pub use auth::AuthError;
pub use commit_analysis::{GitCommitAnalysis, GitCommitAnalysisError};
pub use file_ops::{
    GitAuthor, GitCommit, GitFileOps, GitFileOpsError, find_file_commits, get_commits_robust,
};

#[cfg(test)]
pub use file_ops::MockGitFileOps;
pub use helpers::GitHelpers;
pub use provider::GitProvider;
pub use repository::{GitRepository, GitRepositoryError};
pub use status::{GitStatus, GitStatusError, GitStatusOps};

use crate::utils::EnvProvider;

#[derive(thiserror::Error, Debug)]
pub enum GitInfoError {
    #[error("Failed to open git repository")]
    RepoOpen(gix::open::Error),
    #[error("Failed to find remote")]
    RemoteNotFound(gix::remote::find::existing::Error),
    #[error("No remote configured")]
    NoRemote,
    #[error("No remote URL configured")]
    NoRemoteUrl,
    #[error("Invalid GitHub URL")]
    InvalidGitHubUrl,
    #[error("Failed to build API: {0}")]
    ApiBuildError(#[from] octocrab::Error),
    #[error("Authentication error: {0}")]
    AuthError(#[from] auth::AuthError),
}

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) base_url: String,
    pub(crate) repository_path: PathBuf,
    pub(crate) auth_token: Option<String>,
}

impl GitInfo {
    pub fn from_path(path: &Path, env: &impl EnvProvider) -> Result<Self, GitInfoError> {
        log::debug!("Initializing GitInfo from path: {:?}", path);

        let repository = gix::open(path).map_err(GitInfoError::RepoOpen)?;
        log::debug!("Opened git repository");

        let remote = repository
            .find_default_remote(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemote)?
            .map_err(GitInfoError::RemoteNotFound)?;

        let remote_url = remote
            .url(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemoteUrl)?
            .to_string();
        log::debug!("Found remote URL: {}", remote_url);

        let remote_info =
            helpers::GitRemote::from_url(&remote_url).ok_or(GitInfoError::InvalidGitHubUrl)?;
        log::debug!(
            "Parsed GitHub info - Owner: {}, Repo: {}, Base URL: {}",
            remote_info.owner,
            remote_info.repo,
            remote_info.url
        );

        // Get auth token but don't create Octocrab client yet
        let auth_token = auth::get_token(&remote_info.url, env);
        if auth_token.is_some() {
            log::debug!("Found authentication token");
        } else {
            log::debug!("No authentication token found");
        }

        log::debug!(
            "Successfully initialized GitInfo for {}/{}",
            remote_info.owner,
            remote_info.repo
        );

        Ok(Self {
            owner: remote_info.owner,
            repo: remote_info.repo,
            base_url: remote_info.url,
            repository_path: path.to_path_buf(),
            auth_token,
        })
    }

    /// Get a repository instance (recreated for thread safety)
    pub fn repository(&self) -> Result<Repository, GitInfoError> {
        gix::open(&self.repository_path).map_err(GitInfoError::RepoOpen)
    }
}
