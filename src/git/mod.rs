use gix::Repository;
use octocrab::Octocrab;
use std::path::Path;

mod action;
mod api;
mod auth;
mod commit_analysis;
mod file_ops;
mod helpers;
mod repository;
mod status;

pub use action::{GitAction, GitActionError, GitActionImpl};
pub use api::{GitHubApiError, GitHubReader, GitHubWriter, RepoUser};
pub use auth::{AuthError, create_authenticated_client};
pub use commit_analysis::{GitCommitAnalysis, GitCommitAnalysisError};
pub use file_ops::{GitAuthor, GitFileOps, GitFileOpsError, get_file_commits_robust};
pub use helpers::GitHelpers;
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
    pub(crate) repository: Repository,
    pub(crate) octocrab: Octocrab,
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

        let octocrab = create_authenticated_client(&remote_info.url, env)?;

        log::debug!(
            "Successfully initialized GitInfo for {}/{}",
            remote_info.owner,
            remote_info.repo
        );

        Ok(Self {
            owner: remote_info.owner,
            repo: remote_info.repo,
            base_url: remote_info.url,
            repository,
            octocrab,
        })
    }
}
