use std::{
    fmt,
    path::{Path, PathBuf},
};

use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

use crate::{GitInfo, git::GitCommitAnalysis};

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

#[derive(thiserror::Error, Debug)]
pub enum GitFileOpsError {
    #[error("Failed to walk revision history: {0}")]
    RevWalkError(gix::revision::walk::Error),
    #[error("Failed to traverse commits: {0}")]
    TraverseError(gix::revision::walk::iter::Error),
    #[error("Failed to find git object: {0}")]
    ObjectError(gix::object::find::existing::Error),
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
    #[error("Failed to decode file content: {0:?}")]
    EncodingError(PathBuf),
    #[error("Branch not found: {0}")]
    BranchNotFound(String),
    #[error("Failed to get HEAD ID: {0}")]
    HeadIdError(gix::reference::head_id::Error),
    #[error("Failed to access repository: {0}")]
    RepositoryError(#[from] crate::git::GitInfoError),
}

/// File-specific git operations
#[cfg_attr(test, automock)]
pub trait GitFileOps {
    /// Get commit history for a specific file
    fn file_commits(
        &self,
        file: &Path,
        branch: &Option<String>,
    ) -> Result<Vec<(ObjectId, String)>, GitFileOpsError>;

    /// Get all authors who have modified a file
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError>;

    /// Get file content at a specific commit
    fn file_content_at_commit(
        &self,
        file: &Path,
        commit: &ObjectId,
    ) -> Result<String, GitFileOpsError>;
}

impl GitFileOps for GitInfo {
    fn file_commits(
        &self,
        file: &Path,
        branch: &Option<String>,
    ) -> Result<Vec<(gix::ObjectId, String)>, GitFileOpsError> {
        log::debug!(
            "Finding commits that touched file: {:?} on branch: {:?}",
            file,
            branch
        );
        let repo = self.repository()?;
        let mut commits = Vec::new();

        let start_id = if let Some(branch_name) = branch.as_ref() {
            // Look up the specific branch
            let branch_ref_name = format!("refs/heads/{}", branch_name);
            let branch_ref = repo
                .find_reference(&branch_ref_name)
                .map_err(|_| GitFileOpsError::BranchNotFound(branch_name.clone()))?;
            branch_ref.id()
        } else {
            // Use HEAD as before
            repo.head_id().map_err(GitFileOpsError::HeadIdError)?
        };

        let revwalk = repo.rev_walk([start_id]);

        for commit_info in revwalk.all().map_err(GitFileOpsError::RevWalkError)? {
            let commit_info = commit_info.map_err(GitFileOpsError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit = repo
                .find_object(commit_id)
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_commit()
                .map_err(GitFileOpsError::CommitError)?;

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
                let current_tree = commit.tree().map_err(GitFileOpsError::TreeError)?;

                for parent_id in commit.parent_ids() {
                    let parent_commit = repo
                        .find_object(parent_id)
                        .map_err(GitFileOpsError::ObjectError)?
                        .try_into_commit()
                        .map_err(GitFileOpsError::CommitError)?;

                    let parent_tree = parent_commit.tree().map_err(GitFileOpsError::TreeError)?;

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

    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
        let repo = self.repository()?;
        let commits = self.file_commits(file, &None)?;

        let mut res: Vec<GitAuthor> = Vec::new();

        for (commit_id, _) in commits {
            let commit = repo
                .find_object(commit_id)
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_commit()
                .map_err(GitFileOpsError::CommitError)?;

            let signature = commit.author().map_err(GitFileOpsError::SignatureError)?;
            if !res.iter().any(|author| author.email == signature.email) {
                res.push(GitAuthor {
                    name: signature.name.to_string(),
                    email: signature.email.to_string(),
                });
            }
        }

        if res.is_empty() {
            log::warn!("No authors found for file: {:?}", file);
            Err(GitFileOpsError::AuthorNotFound(file.to_path_buf()))
        } else {
            log::debug!("Found {} unique authors for file: {:?}", res.len(), file);
            Ok(res)
        }
    }

    fn file_content_at_commit(
        &self,
        file: &Path,
        commit: &gix::ObjectId,
    ) -> Result<String, GitFileOpsError> {
        let file_path = file;
        log::debug!(
            "Getting file content for {:?} at commit {}",
            file_path,
            commit
        );

        let repo = self.repository()?;

        // Get the commit object
        let commit_obj = repo
            .find_object(*commit)
            .map_err(GitFileOpsError::ObjectError)?
            .try_into_commit()
            .map_err(GitFileOpsError::CommitError)?;

        // Get the tree for this commit
        let tree = commit_obj.tree().map_err(GitFileOpsError::TreeError)?;

        // Look up the file in the tree
        let entry = tree
            .lookup_entry_by_path(file_path)
            .map_err(|_| GitFileOpsError::FileNotFoundAtCommit(file_path.to_path_buf()))?
            .ok_or_else(|| GitFileOpsError::FileNotFoundAtCommit(file_path.to_path_buf()))?;

        // Get the blob object for the file
        let blob = repo
            .find_object(entry.oid())
            .map_err(GitFileOpsError::ObjectError)?
            .try_into_blob()
            .map_err(GitFileOpsError::BlobError)?;

        // Convert blob data to string
        let content = std::str::from_utf8(&blob.data)
            .map_err(|_| GitFileOpsError::EncodingError(file_path.to_path_buf()))?;

        log::debug!(
            "Successfully read {} bytes from file {:?} at commit {}",
            content.len(),
            file_path,
            commit
        );

        Ok(content.to_string())
    }
}

/// Get file commits with robust branch handling
/// 1. Try the specified branch first
/// 2. If branch not found, parse initial commit from issue body and find merged branch
/// 3. Fall back to searching all branches containing the initial commit
pub fn get_file_commits_robust(
    git_info: &(impl GitFileOps + GitCommitAnalysis),
    file: &Path,
    branch: &str,
    commit: &ObjectId,
) -> Result<Vec<(gix::ObjectId, String)>, GitFileOpsError> {
    // First, try to get commits from the specified branch
    match git_info.file_commits(file, &Some(branch.to_string())) {
        Ok(commits) => {
            log::debug!(
                "Found {} commits for file {:?} on branch {}",
                commits.len(),
                file,
                branch
            );
            return Ok(commits);
        }
        Err(GitFileOpsError::BranchNotFound(_)) => {
            log::debug!(
                "Branch {} not found locally, searching for merged commits for file {:?}",
                branch,
                file
            );
        }
        Err(e) => {
            return Err(e);
        }
    }

    log::debug!(
        "Using commit {} to find merged branch for file {:?}",
        commit,
        file
    );

    // Try to find which branch this commit was merged into
    if let Some(target_branch) = find_merged_into_branch(git_info, commit)? {
        log::debug!(
            "Found that commit {} was merged into branch {}",
            commit,
            target_branch
        );

        // Try to get commits from the target branch
        match git_info.file_commits(file, &Some(target_branch.clone())) {
            Ok(commits) => {
                log::debug!(
                    "Found {} commits for file {:?} on merged target branch {}",
                    commits.len(),
                    file,
                    target_branch
                );
                return Ok(commits);
            }
            Err(e) => {
                log::warn!(
                    "Failed to get commits from target branch {}: {}",
                    target_branch,
                    e
                );
            }
        }
    }

    // Fallback: Get commits from branches containing the commit
    let branches_containing_commit =
        git_info
            .get_branches_containing_commit(commit)
            .map_err(|e| {
                GitFileOpsError::BranchNotFound(format!(
                    "Failed to get branches containing commit: {}",
                    e
                ))
            })?;

    if !branches_containing_commit.is_empty() {
        log::debug!(
            "Found {} branches containing commit {}: {:?}",
            branches_containing_commit.len(),
            commit,
            branches_containing_commit
        );

        // Try each branch until we find one with file commits
        for branch_name in branches_containing_commit {
            match git_info.file_commits(file, &Some(branch_name.clone())) {
                Ok(commits) if !commits.is_empty() => {
                    log::debug!(
                        "Found {} commits for file {:?} on branch {} (contains commit)",
                        commits.len(),
                        file,
                        branch_name
                    );
                    return Ok(commits);
                }
                Ok(_) => {
                    log::debug!("Branch {} contains commit but no file changes", branch_name);
                }
                Err(e) => {
                    log::debug!("Failed to get commits from branch {}: {}", branch_name, e);
                }
            }
        }
    }

    // Final fallback: Get all file commits (no branch restriction)
    log::warn!(
        "Could not find specific branch for file {:?}, falling back to all commits",
        file
    );

    let all_commits = git_info.file_commits(file, &None)?;

    log::debug!(
        "Found {} commits for file {:?} across all branches",
        all_commits.len(),
        file
    );

    Ok(all_commits)
}

/// Find which branch a commit was merged into using merge commit analysis
/// Based on the R algorithm: looks for merge commits where the target commit
/// is an ancestor of the second parent (merged-in branch)
fn find_merged_into_branch(
    git_info: &(impl GitFileOps + GitCommitAnalysis),
    target_commit: &gix::ObjectId,
) -> Result<Option<String>, GitFileOpsError> {
    let merge_commits = git_info.get_all_merge_commits().map_err(|e| {
        GitFileOpsError::BranchNotFound(format!("Failed to get merge commits: {}", e))
    })?;

    for merge_commit in merge_commits {
        let parents = git_info.get_commit_parents(&merge_commit).map_err(|e| {
            GitFileOpsError::BranchNotFound(format!("Failed to get commit parents: {}", e))
        })?;

        if parents.len() >= 2 {
            let _parent1 = parents[0]; // Branch that received the merge
            let parent2 = parents[1]; // Branch that was merged in

            // Check if target_commit is ancestor of parent2 (the merged-in branch)
            if git_info.is_ancestor(target_commit, &parent2).map_err(|e| {
                GitFileOpsError::BranchNotFound(format!("Failed to check ancestry: {}", e))
            })? {
                // Find branches that contain the merge commit
                let candidate_branches = git_info
                    .get_branches_containing_commit(&merge_commit)
                    .map_err(|e| {
                        GitFileOpsError::BranchNotFound(format!(
                            "Failed to get branches containing commit: {}",
                            e
                        ))
                    })?;

                // Filter to branches where parent1 is in their ancestry
                for branch in candidate_branches {
                    // Skip remote HEAD references
                    if branch.contains("HEAD") {
                        continue;
                    }

                    // We found a candidate branch, return it
                    // (In a more sophisticated implementation, we might validate further)
                    return Ok(Some(branch));
                }
            }
        }
    }

    Ok(None)
}
