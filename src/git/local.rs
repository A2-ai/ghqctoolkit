use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
};

#[cfg(test)]
use mockall::automock;

use crate::GitInfo;

#[derive(Debug, Clone)]
pub struct GitAuthor {
    pub(crate) name: String,
    pub(crate) email: String,
}

impl fmt::Display for GitAuthor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.email)
    }
}

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

#[cfg_attr(test, automock)]
pub trait LocalGitInfo {
    fn commit(&self) -> Result<String, LocalGitError>;
    fn branch(&self) -> Result<String, LocalGitError>;
    fn file_commits(
        &self,
        file: &Path,
        branch: &Option<String>,
    ) -> Result<Vec<(gix::ObjectId, String)>, LocalGitError>;
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, LocalGitError>;
    fn file_content_at_commit(
        &self,
        file: &Path,
        commit: &gix::ObjectId,
    ) -> Result<String, LocalGitError>;
    fn status(&self) -> Result<GitStatus, LocalGitError>;
    fn file_status(&self, file: &Path, branch: &Option<String>)
    -> Result<GitStatus, LocalGitError>;
    fn owner(&self) -> &str;
    fn repo(&self) -> &str;
}

#[derive(thiserror::Error, Debug)]
pub enum LocalGitError {
    #[error("Failed to get HEAD reference: {0}")]
    HeadError(gix::reference::find::existing::Error),
    #[error("Repository is in detached HEAD state")]
    DetachedHead,
    #[error("Failed to get HEAD ID: {0}")]
    HeadIdError(gix::reference::head_id::Error),
    #[error("Failed to walk revision history: {0}")]
    RevWalkError(gix::revision::walk::Error),
    #[error("Failed to traverse commits: {0}")]
    TraverseError(gix::revision::walk::iter::Error),
    #[error("Failed to find git object: {0}")]
    FindObjectError(gix::object::find::existing::Error),
    #[error("Failed to parse commit: {0}")]
    CommitError(gix::object::try_into::Error),
    #[error("Failed to get commit tree: {0}")]
    TreeError(gix::object::commit::Error),
    #[error("Failed to get signature: {0}")]
    SignatureError(gix::objs::decode::Error),
    #[error("Author not found for file: {0:?}")]
    AuthorNotFound(PathBuf),
    #[error("File not found at commit: {0:?}")]
    FileNotFoundAtCommit(PathBuf),
    #[error("Failed to read file content: {0}")]
    BlobError(gix::object::try_into::Error),
    #[error("Failed to get remote reference: {0}")]
    RemoteError(gix::reference::find::existing::Error),
    #[error("No remote found for tracking branch")]
    NoRemote,
    #[error("Failed to get worktree status: {0}")]
    StatusError(gix::status::Error),
    #[error("Failed to iterate worktree status: {0}")]
    StatusIterError(gix::status::into_iter::Error),
    #[error("Failed to process worktree entry: {0}")]
    StatusEntryError(gix::status::index_worktree::Error),
    #[error("Branch not found: {0}")]
    BranchNotFound(String),
}

impl LocalGitInfo for GitInfo {
    fn commit(&self) -> Result<String, LocalGitError> {
        let head = self.repository.head().map_err(LocalGitError::HeadError)?;
        let commit_id = head.id().ok_or(LocalGitError::DetachedHead)?;
        let commit_str = commit_id.to_string();
        log::debug!("Current commit: {}", commit_str);
        Ok(commit_str)
    }

    fn branch(&self) -> Result<String, LocalGitError> {
        let head = self.repository.head().map_err(LocalGitError::HeadError)?;

        // Try to get the branch name directly
        if let Some(branch_name) = head.referent_name() {
            let name_str = branch_name.as_bstr().to_string();
            log::debug!("Raw branch reference: {}", name_str);

            // Remove "refs/heads/" prefix if present
            let final_branch = if let Some(branch) = name_str.strip_prefix("refs/heads/") {
                branch.to_string()
            } else {
                name_str
            };
            log::debug!("Current branch: {}", final_branch);
            Ok(final_branch)
        } else {
            // Fallback: we might be in detached HEAD state
            log::debug!("No branch reference found, likely detached HEAD");
            Ok("HEAD".to_string())
        }
    }

    fn file_commits(
        &self,
        file: &Path,
        branch: &Option<String>,
    ) -> Result<Vec<(gix::ObjectId, String)>, LocalGitError> {
        log::debug!(
            "Finding commits that touched file: {:?} on branch: {:?}",
            file,
            branch
        );
        let mut commits = Vec::new();

        let start_id = if let Some(branch_name) = branch.as_ref() {
            // Look up the specific branch
            let branch_ref_name = format!("refs/heads/{}", branch_name);
            let branch_ref = self
                .repository
                .find_reference(&branch_ref_name)
                .map_err(|_| LocalGitError::BranchNotFound(branch_name.clone()))?;
            branch_ref.id()
        } else {
            // Use HEAD as before
            self.repository
                .head_id()
                .map_err(LocalGitError::HeadIdError)?
        };

        let revwalk = self.repository.rev_walk([start_id]);

        for commit_info in revwalk.all().map_err(LocalGitError::RevWalkError)? {
            let commit_info = commit_info.map_err(LocalGitError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(LocalGitError::FindObjectError)?
                .try_into_commit()
                .map_err(LocalGitError::CommitError)?;

            // Check if this commit actually modified the file by comparing with parents
            let mut file_was_modified = false;

            if commit.parent_ids().count() == 0 {
                // This is the initial commit - check if file exists in this commit
                if let Ok(tree) = commit.tree() {
                    if let Ok(Some(_)) = tree.lookup_entry_by_path(file) {
                        file_was_modified = true;
                    }
                }
            } else {
                // Compare this commit's tree with each parent to see if the file changed
                let current_tree = commit.tree().map_err(LocalGitError::TreeError)?;

                for parent_id in commit.parent_ids() {
                    let parent_commit = self
                        .repository
                        .find_object(parent_id)
                        .map_err(LocalGitError::FindObjectError)?
                        .try_into_commit()
                        .map_err(LocalGitError::CommitError)?;

                    let parent_tree = parent_commit.tree().map_err(LocalGitError::TreeError)?;

                    // Check if file exists in current and parent trees
                    let file_in_current = current_tree.lookup_entry_by_path(file);
                    let file_in_parent = parent_tree.lookup_entry_by_path(file);

                    match (file_in_current, file_in_parent) {
                        (Ok(Some(current_entry)), Ok(Some(parent_entry))) => {
                            // File exists in both - check if content changed
                            if current_entry.oid() != parent_entry.oid() {
                                file_was_modified = true;
                                break;
                            }
                        }
                        (Ok(Some(_)), Ok(None)) | (Ok(Some(_)), Err(_)) => {
                            // File added in this commit
                            file_was_modified = true;
                            break;
                        }
                        (Ok(None), Ok(Some(_))) | (Err(_), Ok(Some(_))) => {
                            // File deleted in this commit
                            file_was_modified = true;
                            break;
                        }
                        _ => {
                            // File doesn't exist in either or other cases - no change for this file
                            continue;
                        }
                    }
                }
            }

            if file_was_modified {
                // Get commit message, fallback to empty string if not available
                let commit_message = commit
                    .message_raw()
                    .map(|msg| msg.to_string())
                    .unwrap_or(String::new());

                commits.push((commit_id, commit_message));
            }
        }

        log::debug!(
            "Found {} commits that touched file: {:?}",
            commits.len(),
            file
        );
        Ok(commits)
    }

    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, LocalGitError> {
        let commits = self.file_commits(file, &None)?;

        let mut res: Vec<GitAuthor> = Vec::new();

        for (commit_id, _) in commits {
            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(LocalGitError::FindObjectError)?
                .try_into_commit()
                .map_err(LocalGitError::CommitError)?;

            let signature = commit.author().map_err(LocalGitError::SignatureError)?;
            if !res.iter().any(|author| author.email == signature.email) {
                res.push(GitAuthor {
                    name: signature.name.to_string(),
                    email: signature.email.to_string(),
                });
            }
        }

        if res.is_empty() {
            log::warn!("No authors found for file: {:?}", file);
            Err(LocalGitError::AuthorNotFound(file.to_path_buf()))
        } else {
            log::debug!("Found {} unique authors for file: {:?}", res.len(), file);
            Ok(res)
        }
    }

    fn file_content_at_commit(
        &self,
        file: &Path,
        commit: &gix::ObjectId,
    ) -> Result<String, LocalGitError> {
        let file_path = file;
        log::debug!(
            "Getting file content for {:?} at commit {}",
            file_path,
            commit
        );

        // Get the commit object
        let commit_obj = self
            .repository
            .find_object(*commit)
            .map_err(LocalGitError::FindObjectError)?
            .try_into_commit()
            .map_err(LocalGitError::CommitError)?;

        // Get the tree for this commit
        let tree = commit_obj.tree().map_err(LocalGitError::TreeError)?;

        // Look up the file in the tree
        let entry = tree
            .lookup_entry_by_path(file_path)
            .map_err(|_| LocalGitError::FileNotFoundAtCommit(file_path.to_path_buf()))?
            .ok_or_else(|| LocalGitError::FileNotFoundAtCommit(file_path.to_path_buf()))?;

        // Get the blob object for the file
        let blob = self
            .repository
            .find_object(entry.oid())
            .map_err(LocalGitError::FindObjectError)?
            .try_into_blob()
            .map_err(LocalGitError::BlobError)?;

        // Convert blob data to string
        let content = std::str::from_utf8(&blob.data)
            .map_err(|_| LocalGitError::FileNotFoundAtCommit(file_path.to_path_buf()))?;

        log::debug!(
            "Successfully read {} bytes from file {:?} at commit {}",
            content.len(),
            file_path,
            commit
        );

        Ok(content.to_string())
    }

    fn status(&self) -> Result<GitStatus, LocalGitError> {
        log::debug!("Getting git repository status");

        // Check for uncommitted changes (dirty working tree)
        let status_platform = self
            .repository
            .status(gix::progress::Discard)
            .map_err(LocalGitError::StatusError)?;

        let mut dirty_files = Vec::new();
        for entry in status_platform
            .into_index_worktree_iter(std::iter::empty::<gix::bstr::BString>())
            .map_err(LocalGitError::StatusIterError)?
        {
            let entry = entry.map_err(LocalGitError::StatusEntryError)?;
            dirty_files.push(PathBuf::from(entry.rela_path().to_string()));
        }

        if !dirty_files.is_empty() {
            log::debug!("Repository has {} uncommitted changes", dirty_files.len());
            return Ok(GitStatus::Dirty(dirty_files));
        }

        // Get current branch and its upstream tracking branch
        let head = self.repository.head().map_err(LocalGitError::HeadError)?;
        let current_branch_name = if let Some(branch_name) = head.referent_name() {
            let name_str = branch_name.as_bstr().to_string();
            name_str
                .strip_prefix("refs/heads/")
                .unwrap_or(&name_str)
                .to_string()
        } else {
            return Err(LocalGitError::DetachedHead);
        };

        // Try to find the upstream tracking branch
        let upstream_ref_name = format!("refs/remotes/origin/{}", current_branch_name);
        let upstream_ref = match self.repository.find_reference(&upstream_ref_name) {
            Ok(r) => r,
            Err(_) => {
                // Count local commits since no upstream exists
                let local_revwalk = self
                    .repository
                    .rev_walk([head.id().ok_or(LocalGitError::DetachedHead)?]);
                let local_commit_count = local_revwalk
                    .all()
                    .map_err(LocalGitError::RevWalkError)?
                    .count();
                log::debug!(
                    "No upstream branch found for {}, {} commits ahead",
                    current_branch_name,
                    local_commit_count
                );
                return Ok(GitStatus::Ahead(local_commit_count));
            }
        };

        let local_commit_id = head.id().ok_or(LocalGitError::DetachedHead)?;
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
            .map_err(LocalGitError::RevWalkError)?
            .map(|info| info.map(|i| i.id).map_err(LocalGitError::TraverseError))
            .collect::<Result<HashSet<_>, _>>()?;

        // Get all remote commits
        let remote_commits = remote_revwalk
            .all()
            .map_err(LocalGitError::RevWalkError)?
            .map(|info| info.map(|i| i.id).map_err(LocalGitError::TraverseError))
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
    ) -> Result<GitStatus, LocalGitError> {
        log::debug!(
            "Getting git status for file: {:?} on branch: {:?}",
            file,
            branch
        );

        // Check if the file has uncommitted changes
        let status_platform = self
            .repository
            .status(gix::progress::Discard)
            .map_err(LocalGitError::StatusError)?;

        let file_path_str = file.to_string_lossy();
        let mut file_is_dirty = false;

        for entry in status_platform
            .into_index_worktree_iter(std::iter::empty::<gix::bstr::BString>())
            .map_err(LocalGitError::StatusIterError)?
        {
            let entry = entry.map_err(LocalGitError::StatusEntryError)?;
            if entry.rela_path().to_string() == file_path_str {
                file_is_dirty = true;
                break;
            }
        }

        if file_is_dirty {
            log::debug!("File {:?} has uncommitted changes", file);
            return Ok(GitStatus::Dirty(vec![file.to_path_buf()]));
        }

        // Get the branch to check - either specified branch or current branch
        let (local_commit_id, current_branch_name) = if let Some(branch_name) = branch.as_ref() {
            // Use the specified branch
            let branch_ref_name = format!("refs/heads/{}", branch_name);
            let branch_ref = self
                .repository
                .find_reference(&branch_ref_name)
                .map_err(|_| LocalGitError::BranchNotFound(branch_name.clone()))?;
            (branch_ref.id(), branch_name.clone())
        } else {
            // Use current branch
            let head = self.repository.head().map_err(LocalGitError::HeadError)?;
            let current_branch_name = if let Some(branch_name) = head.referent_name() {
                let name_str = branch_name.as_bstr().to_string();
                name_str
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&name_str)
                    .to_string()
            } else {
                return Err(LocalGitError::DetachedHead);
            };
            (
                head.id().ok_or(LocalGitError::DetachedHead)?,
                current_branch_name,
            )
        };

        // Try to find the upstream tracking branch
        let upstream_ref_name = format!("refs/remotes/origin/{}", current_branch_name);
        let upstream_ref = match self.repository.find_reference(&upstream_ref_name) {
            Ok(r) => r,
            Err(_) => {
                // Count commits that touched this file since no upstream exists
                let file_commits = self.file_commits(file, branch)?;
                log::debug!(
                    "No upstream branch found for {}, file has {} commits",
                    current_branch_name,
                    file_commits.len()
                );
                return Ok(GitStatus::Ahead(file_commits.len()));
            }
        };

        let remote_commit_id = upstream_ref.id();

        if local_commit_id == remote_commit_id {
            log::debug!("File {:?} is clean (local and remote in sync)", file);
            return Ok(GitStatus::Clean);
        }

        // Get commits that touched this file in both local and remote branches
        let local_file_commits = self.file_commits(file, branch)?;
        let local_file_commit_ids: std::collections::HashSet<_> =
            local_file_commits.iter().map(|(id, _)| *id).collect();

        // For the remote branch, we need to check commits from the remote commit
        // We'll use the same logic but walk from the remote commit
        let remote_revwalk = self.repository.rev_walk([remote_commit_id]);
        let mut remote_file_commits = Vec::new();

        for commit_info in remote_revwalk.all().map_err(LocalGitError::RevWalkError)? {
            let commit_info = commit_info.map_err(LocalGitError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(LocalGitError::FindObjectError)?
                .try_into_commit()
                .map_err(LocalGitError::CommitError)?;

            // Check if this commit modified the file (same logic as file_commits)
            let mut file_was_modified = false;

            if commit.parent_ids().count() == 0 {
                // This is the initial commit - check if file exists in this commit
                if let Ok(tree) = commit.tree() {
                    if let Ok(Some(_)) = tree.lookup_entry_by_path(file) {
                        file_was_modified = true;
                    }
                }
            } else {
                // Compare this commit's tree with each parent to see if the file changed
                let current_tree = commit.tree().map_err(LocalGitError::TreeError)?;

                for parent_id in commit.parent_ids() {
                    let parent_commit = self
                        .repository
                        .find_object(parent_id)
                        .map_err(LocalGitError::FindObjectError)?
                        .try_into_commit()
                        .map_err(LocalGitError::CommitError)?;

                    let parent_tree = parent_commit.tree().map_err(LocalGitError::TreeError)?;

                    // Check if file exists in current and parent trees
                    let file_in_current = current_tree.lookup_entry_by_path(file);
                    let file_in_parent = parent_tree.lookup_entry_by_path(file);

                    match (file_in_current, file_in_parent) {
                        (Ok(Some(current_entry)), Ok(Some(parent_entry))) => {
                            // File exists in both - check if content changed
                            if current_entry.oid() != parent_entry.oid() {
                                file_was_modified = true;
                                break;
                            }
                        }
                        (Ok(Some(_)), Ok(None)) | (Ok(Some(_)), Err(_)) => {
                            // File added in this commit
                            file_was_modified = true;
                            break;
                        }
                        (Ok(None), Ok(Some(_))) | (Err(_), Ok(Some(_))) => {
                            // File deleted in this commit
                            file_was_modified = true;
                            break;
                        }
                        _ => {
                            // File doesn't exist in either or other cases - no change for this file
                            continue;
                        }
                    }
                }
            }

            if file_was_modified {
                remote_file_commits.push(commit_id);
            }
        }

        let remote_file_commit_ids: std::collections::HashSet<_> =
            remote_file_commits.iter().cloned().collect();

        let local_only: Vec<_> = local_file_commit_ids
            .difference(&remote_file_commit_ids)
            .collect();
        let remote_only: Vec<_> = remote_file_commit_ids
            .difference(&local_file_commit_ids)
            .collect();

        match (local_only.is_empty(), remote_only.is_empty()) {
            (true, false) => {
                log::debug!(
                    "File {:?} is behind remote by {} commits",
                    file,
                    remote_only.len()
                );
                Ok(GitStatus::Behind(remote_only.len()))
            }
            (false, true) => {
                log::debug!(
                    "File {:?} is ahead of remote by {} commits",
                    file,
                    local_only.len()
                );
                Ok(GitStatus::Ahead(local_only.len()))
            }
            (false, false) => {
                log::debug!(
                    "File {:?} has diverged: {} local commits, {} remote commits",
                    file,
                    local_only.len(),
                    remote_only.len()
                );
                Ok(GitStatus::Diverged {
                    ahead: local_only.len(),
                    behind: remote_only.len(),
                })
            }
            (true, true) => {
                log::debug!("File {:?} is clean (no unique commits)", file);
                Ok(GitStatus::Clean)
            }
        }
    }

    fn owner(&self) -> &str {
        &self.owner
    }

    fn repo(&self) -> &str {
        &self.repo
    }
}
