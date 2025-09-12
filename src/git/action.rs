use std::path::{Path, PathBuf};

use crate::git::auth::get_token;
use crate::utils::StdEnvProvider;
use gix::bstr::BString;
use gix::clone::PrepareFetch;
use gix::create::{self, Kind};
use gix::{Url, open};
#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitAction {
    /// Clone a repository from a URL to a local path
    fn clone(&self, url: Url, path: &Path) -> Result<(), GitActionError>;

    fn remote(&self, path: &Path) -> Result<Url, GitActionError>;
}

/// Error types for git actions
#[derive(thiserror::Error, Debug)]
pub enum GitActionError {
    #[error("Directory exists: {0}")]
    DirectoryExists(PathBuf),
    #[error("Directory does not exist: {0}")]
    NoDirectoryExists(PathBuf),
    #[error("Clone failed: {0}")]
    CloneError(#[from] gix::clone::Error),
    #[error("Fetch failed: {0}")]
    FetchError(#[from] gix::clone::fetch::Error),
    #[error("Failed to checkout main worktree: {0}")]
    WorktreeError(#[from] gix::clone::checkout::main_worktree::Error),
    #[error("Failed to determine remote url: {0}")]
    NoRemote(String),
}

/// Default implementation of GitAction using the gix library
#[derive(Debug, Clone, Default)]
pub struct GitActionImpl;

impl GitAction for GitActionImpl {
    fn clone(&self, url: Url, path: &Path) -> Result<(), GitActionError> {
        log::debug!("Cloning repository from {} to {}", url, path.display());

        if path.exists() {
            log::debug!("Path ({}) already exists", path.display());
            return Err(GitActionError::DirectoryExists(path.to_path_buf()));
        }

        let url_str = url.to_string();

        let open_opts = if let Ok(token) = get_token(&url_str, &StdEnvProvider) {
            let header = format!("Authorization: Basic x-access-token:{token}");
            let kv: BString = format!("http.{url_str}.extraHeader={header}").into();

            open::Options::default().config_overrides([kv])
        } else {
            open::Options::default()
        };

        let mut prep = PrepareFetch::new(
            url.clone(),
            path,
            Kind::WithWorktree,
            create::Options::default(),
            open_opts,
        )?;

        let (mut checkout, _) =
            prep.fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)?;

        checkout.main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)?;

        log::debug!("Successfully cloned repository");
        Ok(())
    }

    fn remote(&self, path: &Path) -> Result<Url, GitActionError> {
        let repo =
            gix::open(path).map_err(|_| GitActionError::NoDirectoryExists(path.to_path_buf()))?;
        let remote = repo
            .find_default_remote(gix::remote::Direction::Fetch)
            .ok_or(GitActionError::NoRemote(
                "No default remote found".to_string(),
            ))?
            .map_err(|e| GitActionError::NoRemote(e.to_string()))?;

        let remote_url =
            remote
                .url(gix::remote::Direction::Fetch)
                .ok_or(GitActionError::NoRemote(
                    "No url set for default remote".to_string(),
                ))?;
        Ok(remote_url.clone())
    }
}
