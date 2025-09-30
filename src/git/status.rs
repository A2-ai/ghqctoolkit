use std::{
    fmt,
    path::{Path, PathBuf},
};

use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

use crate::GitInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum GitStatus {
    Dirty(Vec<PathBuf>),   // local, uncommitted changes - list of dirty files
    Clean,                 // up to date with remote
    Behind(Vec<ObjectId>), // remote commits not local - count of commits behind
    Ahead(Vec<ObjectId>),  // local commits not remote - count of commits ahead
    Diverged {
        ahead: Vec<ObjectId>,
        behind: Vec<ObjectId>,
    }, // local commits not remote AND remote commits not local
}

impl fmt::Display for GitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dirty(files) => write!(
                f,
                "Repository has files with uncommitted, local changes: \n\t- {}",
                files
                    .iter()
                    .map(|x| x.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("\n\t- ")
            ),
            Self::Clean => write!(f, "Repository is up to date!"),
            Self::Behind(commits) => write!(f, "Repository is behind by {} commits", commits.len()),
            Self::Ahead(commits) => write!(f, "Repository is ahead by {} commits", commits.len()),
            Self::Diverged { ahead, behind } => write!(
                f,
                "Repository is ahead by {} and behind by {} commits",
                ahead.len(),
                behind.len()
            ),
        }
    }
}

impl GitStatus {
    /// Format git status for a specific file and issue thread
    pub fn format_for_file(
        &self,
        issue_file: impl AsRef<Path>,
        file_commits: &[&ObjectId],
    ) -> String {
        match self {
            GitStatus::Clean => "Up to date".to_string(),
            GitStatus::Dirty(files) => {
                if files.contains(&issue_file.as_ref().to_path_buf()) {
                    "Local changes".to_string()
                } else {
                    "Up to date".to_string()
                }
            }
            GitStatus::Ahead(commits) => {
                if file_commits.iter().any(|c| commits.contains(c)) {
                    "Local commits".to_string()
                } else {
                    "Up to date".to_string()
                }
            }
            GitStatus::Behind(commits) => {
                if file_commits.iter().any(|c| commits.contains(c)) {
                    "Remote changes".to_string()
                } else {
                    "Up to date".to_string()
                }
            }
            GitStatus::Diverged { ahead, behind } => {
                let is_ahead = file_commits.iter().any(|c| ahead.contains(c));
                let is_behind = file_commits.iter().any(|c| behind.contains(c));
                match (is_ahead, is_behind) {
                    (true, true) => "Diverged".to_string(),
                    (true, false) => "Local commits".to_string(),
                    (false, true) => "Remote changes".to_string(),
                    (false, false) => "Up to date".to_string(),
                }
            }
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
    #[error("Failed to access repository: {0}")]
    RepositoryError(#[from] crate::git::GitInfoError),
}

/// Repository and file status operations
#[cfg_attr(test, automock)]
pub trait GitStatusOps {
    /// Get overall repository status
    fn status(&self) -> Result<GitStatus, GitStatusError>;
}

impl GitStatusOps for GitInfo {
    fn status(&self) -> Result<GitStatus, GitStatusError> {
        log::debug!("Getting git repository status");
        let repo = self.repository()?;

        // Check for uncommitted changes (dirty working tree)
        let status_platform = repo
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
        let head = repo.head().map_err(GitStatusError::HeadError)?;
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
        let upstream_ref = match repo.find_reference(&upstream_ref_name) {
            Ok(r) => r,
            Err(_) => {
                // Count local commits since no upstream exists
                let local_revwalk = repo.rev_walk([head.id().ok_or_else(|| {
                    GitStatusError::HeadError(gix::reference::find::existing::Error::NotFound {
                        name: gix::refs::PartialName::try_from("HEAD").unwrap(),
                    })
                })?]);
                let local_commits: Vec<ObjectId> = local_revwalk
                    .all()
                    .map_err(GitStatusError::RevWalkError)?
                    .map(|info| info.map(|i| i.id).map_err(GitStatusError::TraverseError))
                    .collect::<Result<Vec<_>, _>>()?;
                log::debug!(
                    "No upstream branch found for {}, {} commits ahead",
                    current_branch_name,
                    local_commits.len()
                );
                return Ok(GitStatus::Ahead(local_commits));
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
        let local_revwalk = repo.rev_walk([local_commit_id]);
        let remote_revwalk = repo.rev_walk([remote_commit_id]);

        // Get all local commits (preserving chronological order)
        let local_commits = local_revwalk
            .all()
            .map_err(GitStatusError::RevWalkError)?
            .map(|info| info.map(|i| i.id).map_err(GitStatusError::TraverseError))
            .collect::<Result<Vec<_>, _>>()?;

        // Get all remote commits (preserving chronological order)
        let remote_commits = remote_revwalk
            .all()
            .map_err(GitStatusError::RevWalkError)?
            .map(|info| info.map(|i| i.id).map_err(GitStatusError::TraverseError))
            .collect::<Result<Vec<_>, _>>()?;

        // Find commits that exist only in local (ahead commits) - preserve order
        let local_only: Vec<ObjectId> = local_commits
            .iter()
            .filter(|commit| !remote_commits.contains(commit))
            .cloned()
            .collect();

        // Find commits that exist only in remote (behind commits) - preserve order
        let remote_only: Vec<ObjectId> = remote_commits
            .iter()
            .filter(|commit| !local_commits.contains(commit))
            .cloned()
            .collect();

        match (local_only.is_empty(), remote_only.is_empty()) {
            (true, false) => {
                log::debug!("Local is behind remote by {} commits", remote_only.len());
                Ok(GitStatus::Behind(remote_only))
            }
            (false, true) => {
                log::debug!("Local is ahead of remote by {} commits", local_only.len());
                Ok(GitStatus::Ahead(local_only))
            }
            (false, false) => {
                log::debug!(
                    "Local and remote have diverged: {} local commits, {} remote commits",
                    local_only.len(),
                    remote_only.len()
                );
                Ok(GitStatus::Diverged {
                    ahead: local_only,
                    behind: remote_only,
                })
            }
            (true, true) => {
                // This shouldn't happen since we already checked if commits are equal
                log::debug!("Local and remote are in sync (fallback)");
                Ok(GitStatus::Clean)
            }
        }
    }
}
