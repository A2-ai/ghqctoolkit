use std::{
    fmt,
    path::{Path, PathBuf},
};

#[cfg(test)]
use mockall::automock;

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

#[cfg_attr(test, automock)]
pub trait LocalGitInfo {
    fn commit(&self) -> Result<String, LocalGitError>;
    fn branch(&self) -> Result<String, LocalGitError>;
    fn file_commits(&self, file: &Path) -> Result<Vec<(gix::ObjectId, String)>, LocalGitError>;
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, LocalGitError>;
    fn file_content_at_commit(&self, file: &Path, commit: &gix::ObjectId) -> Result<String, LocalGitError>;
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
}

use crate::git::GitInfo;

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

    fn file_commits(&self, file: &Path) -> Result<Vec<(gix::ObjectId, String)>, LocalGitError> {
        log::debug!("Finding commits that touched file: {:?}", file);
        let mut commits = Vec::new();

        let head_id = self
            .repository
            .head_id()
            .map_err(LocalGitError::HeadIdError)?;

        let revwalk = self.repository.rev_walk([head_id]);

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
                let commit_message = commit.message_raw()
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
        let commits = self.file_commits(file)?;

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

    fn file_content_at_commit(&self, file: &Path, commit: &gix::ObjectId) -> Result<String, LocalGitError> {
        let file_path = file;
        log::debug!("Getting file content for {:?} at commit {}", file_path, commit);

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
}
