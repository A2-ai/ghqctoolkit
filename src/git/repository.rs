use crate::GitInfo;
use crate::git::action::{GitCli, GitCommand};
#[cfg(test)]
use mockall::automock;
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum GitRepositoryError {
    #[error("Failed to get HEAD reference: {0}")]
    HeadError(gix::reference::find::existing::Error),
    #[error("Repository is in detached HEAD state")]
    DetachedHead,
    #[error("Failed to get HEAD ID: {0}")]
    HeadIdError(gix::reference::head_id::Error),
    #[error("Failed to access repository: {0}")]
    RepositoryError(#[from] crate::git::GitInfoError),
    #[error("Failed to find remote: {0}")]
    RemoteNotFound(String),
    #[error("Failed to connect to remote: {0}")]
    RemoteConnectionError(String),
    #[error("Failed to fetch from remote: {0}")]
    FetchError(String),
}

/// Basic repository information and metadata
#[cfg_attr(test, automock)]
pub trait GitRepository {
    /// Get the current commit hash
    fn commit(&self) -> Result<String, GitRepositoryError>;

    /// Get the current branch name
    fn branch(&self) -> Result<String, GitRepositoryError>;

    /// Get the repository owner/organization name
    fn owner(&self) -> &str;

    /// Get the repository name
    fn repo(&self) -> &str;

    /// Get the repository path on the filesystem
    fn path(&self) -> &Path;

    /// Fetch the repository remote. Return whether changes found
    fn fetch(&self) -> Result<bool, GitRepositoryError>;
}

impl GitRepository for GitInfo {
    fn commit(&self) -> Result<String, GitRepositoryError> {
        let repo = self.repository()?;
        let head = repo.head().map_err(GitRepositoryError::HeadError)?;
        let commit_id = head.id().ok_or(GitRepositoryError::DetachedHead)?;
        let commit_str = commit_id.to_string();
        log::debug!("Current commit: {}", commit_str);
        Ok(commit_str)
    }

    fn branch(&self) -> Result<String, GitRepositoryError> {
        let repo = self.repository()?;
        let head = repo.head().map_err(GitRepositoryError::HeadError)?;

        // Try to get the branch name directly
        if let Some(branch_name) = head.referent_name() {
            let name_str = branch_name.as_bstr().to_string();
            log::debug!("Found branch name from referent: {}", name_str);

            // Extract the branch name from refs/heads/<branch>
            if let Some(stripped) = name_str.strip_prefix("refs/heads/") {
                return Ok(stripped.to_string());
            } else if let Some(stripped) = name_str.strip_prefix("refs/remotes/origin/") {
                return Ok(stripped.to_string());
            } else {
                return Ok(name_str);
            }
        }

        // Fallback: try to get commit and find branch containing it
        let commit_id = head.id().ok_or(GitRepositoryError::DetachedHead)?;
        log::debug!(
            "HEAD is detached, trying to find branch containing commit: {}",
            commit_id
        );

        // Try to find a local branch that points to this commit
        if let Ok(refs) = repo.references() {
            if let Ok(all_refs) = refs.all() {
                for r_res in all_refs {
                    if let Ok(r) = r_res {
                        let name = r.name().as_bstr().to_string();
                        if name.starts_with("refs/heads/") {
                            if let Some(id) = r.target().try_id() {
                                if commit_id == *id {
                                    if let Some(branch_name) = name.strip_prefix("refs/heads/") {
                                        log::debug!("Found matching branch: {}", branch_name);
                                        return Ok(branch_name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If no branch found, return "HEAD" as fallback
        log::debug!("No branch found, using HEAD as fallback");
        Ok("HEAD".to_string())
    }

    fn owner(&self) -> &str {
        &self.owner
    }

    fn repo(&self) -> &str {
        &self.repo
    }

    fn path(&self) -> &Path {
        &self.repository_path
    }

    fn fetch(&self) -> Result<bool, GitRepositoryError> {
        GitCommand
            .fetch(&self.repository_path)
            .map_err(|e| GitRepositoryError::FetchError(e.to_string()))
    }
}
