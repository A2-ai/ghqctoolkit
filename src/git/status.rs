use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
};

#[cfg(test)]
use mockall::automock;

use crate::GitInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum GitStatus {
    Dirty(Vec<PathBuf>), // local, uncommitted changes - list of dirty files
    Clean,               // up to date with remote
    Behind(usize),       // remote commits not local - count of commits behind
    Ahead(usize),        // local commits not remote - count of commits ahead
    Diverged { ahead: usize, behind: usize }, // local commits not remote AND remote commits not local
}

impl fmt::Display for GitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dirty(files) => {
                write!(
                    f,
                    "❌ Repository has files with uncommitted, local changes: \n\t- {}",
                    files
                        .iter()
                        .map(|x| x.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("\n\t- ")
                )
            }
            Self::Clean => write!(f, "✅ Repository is up to date!"),
            Self::Behind(count) => write!(f, "⏪ Repository is behind by {count} commits"),
            Self::Ahead(count) => write!(f, "⏩ Repository is ahead by {count} commits"),
            Self::Diverged { ahead, behind } => write!(
                f,
                "↔️ Repository is ahead by {ahead} and behind by {behind} commits"
            ),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum GitStatusError {
    #[error("Failed to get HEAD reference: {0}")]
    HeadError(gix::reference::find::existing::Error),
    #[error("Failed to get worktree status: {0}")]
    StatusError(gix::status::Error),
    #[error("Failed to iterate worktree status: {0}")]
    StatusIterError(gix::status::into_iter::Error),
    #[error("Failed to process worktree entry: {0}")]
    StatusEntryError(gix::status::index_worktree::Error),
    #[error("Failed to get remote reference: {0}")]
    RemoteError(gix::reference::find::existing::Error),
    #[error("No remote found for tracking branch")]
    NoRemote,
    #[error("Failed to walk revision history: {0}")]
    RevWalkError(gix::revision::walk::Error),
    #[error("Failed to traverse commits: {0}")]
    TraverseError(gix::revision::walk::iter::Error),
}

/// Repository and file status operations
#[cfg_attr(test, automock)]
pub trait GitStatusOps {
    /// Get overall repository status
    fn status(&self) -> Result<GitStatus, GitStatusError>;

    /// Get status for a specific file
    fn file_status(
        &self,
        file: &Path,
        branch: &Option<String>,
    ) -> Result<GitStatus, GitStatusError>;
}

impl GitStatusOps for GitInfo {
    fn status(&self) -> Result<GitStatus, GitStatusError> {
        log::debug!("Getting git repository status");

        // Check for uncommitted changes (dirty working tree)
        let status_platform = self
            .repository
            .status(gix::progress::Discard)
            .map_err(GitStatusError::StatusError)?;

        let mut dirty_files = Vec::new();
        for entry in status_platform
            .into_index_worktree_iter(std::iter::empty::<gix::bstr::BString>())
            .map_err(GitStatusError::StatusIterError)?
        {
            let entry = entry.map_err(GitStatusError::StatusEntryError)?;
            dirty_files.push(PathBuf::from(entry.rela_path().to_string()));
        }

        if !dirty_files.is_empty() {
            log::debug!("Repository has {} uncommitted changes", dirty_files.len());
            return Ok(GitStatus::Dirty(dirty_files));
        }

        // Get current branch and its upstream tracking branch
        let head = self.repository.head().map_err(GitStatusError::HeadError)?;
        let current_branch_name = if let Some(branch_name) = head.referent_name() {
            let name_str = branch_name.as_bstr().to_string();
            name_str
                .strip_prefix("refs/heads/")
                .unwrap_or(&name_str)
                .to_string()
        } else {
            return Err(GitStatusError::HeadError(
                gix::reference::find::existing::Error::NotFound {
                    name: gix::refs::PartialName::try_from("HEAD").unwrap(),
                },
            ));
        };

        // Try to find the upstream tracking branch
        let upstream_ref_name = format!("refs/remotes/origin/{}", current_branch_name);
        let upstream_ref = match self.repository.find_reference(&upstream_ref_name) {
            Ok(r) => r,
            Err(_) => {
                // Count local commits since no upstream exists
                let local_revwalk = self.repository.rev_walk([head.id().ok_or_else(|| {
                    GitStatusError::HeadError(gix::reference::find::existing::Error::NotFound {
                        name: gix::refs::PartialName::try_from("HEAD").unwrap(),
                    })
                })?]);
                let local_commit_count = local_revwalk
                    .all()
                    .map_err(GitStatusError::RevWalkError)?
                    .count();
                log::debug!(
                    "No upstream branch found for {}, {} commits ahead",
                    current_branch_name,
                    local_commit_count
                );
                return Ok(GitStatus::Ahead(local_commit_count));
            }
        };

        let local_commit_id = head.id().ok_or_else(|| {
            GitStatusError::HeadError(gix::reference::find::existing::Error::NotFound {
                name: gix::refs::PartialName::try_from("HEAD").unwrap(),
            })
        })?;
        let remote_commit_id = upstream_ref.id();

        if local_commit_id == remote_commit_id {
            log::debug!("Local and remote are in sync");
            return Ok(GitStatus::Clean);
        }

        // Check if local is ahead, behind, or diverged from remote
        let local_revwalk = self.repository.rev_walk([local_commit_id]);
        let remote_revwalk = self.repository.rev_walk([remote_commit_id]);

        // Get all local commits
        let local_commits = local_revwalk
            .all()
            .map_err(GitStatusError::RevWalkError)?
            .map(|info| info.map(|i| i.id).map_err(GitStatusError::TraverseError))
            .collect::<Result<HashSet<_>, _>>()?;

        // Get all remote commits
        let remote_commits = remote_revwalk
            .all()
            .map_err(GitStatusError::RevWalkError)?
            .map(|info| info.map(|i| i.id).map_err(GitStatusError::TraverseError))
            .collect::<Result<HashSet<_>, _>>()?;

        let local_only: Vec<_> = local_commits.difference(&remote_commits).collect();
        let remote_only: Vec<_> = remote_commits.difference(&local_commits).collect();

        match (local_only.is_empty(), remote_only.is_empty()) {
            (true, false) => {
                log::debug!("Local is behind remote by {} commits", remote_only.len());
                Ok(GitStatus::Behind(remote_only.len()))
            }
            (false, true) => {
                log::debug!("Local is ahead of remote by {} commits", local_only.len());
                Ok(GitStatus::Ahead(local_only.len()))
            }
            (false, false) => {
                log::debug!(
                    "Local and remote have diverged: {} local commits, {} remote commits",
                    local_only.len(),
                    remote_only.len()
                );
                Ok(GitStatus::Diverged {
                    ahead: local_only.len(),
                    behind: remote_only.len(),
                })
            }
            (true, true) => {
                // This shouldn't happen since we already checked if commits are equal
                log::debug!("Local and remote are in sync (fallback)");
                Ok(GitStatus::Clean)
            }
        }
    }

    fn file_status(
        &self,
        file: &Path,
        branch: &Option<String>,
    ) -> Result<GitStatus, GitStatusError> {
        log::debug!(
            "Getting git status for file: {:?} on branch: {:?}",
            file,
            branch
        );

        // Check if the file has uncommitted changes
        let status_platform = self
            .repository
            .status(gix::progress::Discard)
            .map_err(GitStatusError::StatusError)?;

        let file_path_str = file.to_string_lossy();
        let mut file_is_dirty = false;

        for entry in status_platform
            .into_index_worktree_iter(std::iter::empty::<gix::bstr::BString>())
            .map_err(GitStatusError::StatusIterError)?
        {
            let entry = entry.map_err(GitStatusError::StatusEntryError)?;
            if entry.rela_path().to_string() == file_path_str {
                file_is_dirty = true;
                break;
            }
        }

        if file_is_dirty {
            log::debug!("File {:?} has uncommitted changes", file);
            return Ok(GitStatus::Dirty(vec![file.to_path_buf()]));
        }

        // For now, return clean if no uncommitted changes
        log::debug!("File {:?} is clean", file);
        Ok(GitStatus::Clean)
    }
}
