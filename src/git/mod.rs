use gix::Repository;
use octocrab::Octocrab;
use std::path::Path;

pub(crate) mod action;
pub(crate) mod api;
pub(crate) mod auth;
pub(crate) mod helpers;
pub(crate) mod local;

pub use action::{GitAction, GitActionError, GitActionImpl};
pub use api::{GitHubApi, RepoUser};
pub use auth::create_authenticated_client;
pub use helpers::{GitHelpers, GitInfoError, parse_github_url};
pub use local::{LocalGitError, LocalGitInfo};

use crate::utils::EnvProvider;

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

        let (owner, repo, base_url) = parse_github_url(&remote_url)?;
        log::debug!(
            "Parsed GitHub info - Owner: {}, Repo: {}, Base URL: {}",
            owner,
            repo,
            base_url
        );

        let octocrab = create_authenticated_client(&base_url, env)?;

        log::debug!("Successfully initialized GitInfo for {}/{}", owner, repo);

        Ok(Self {
            owner,
            repo,
            base_url,
            repository,
            octocrab,
        })
    }

}
