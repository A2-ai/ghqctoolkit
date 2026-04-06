use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use gix::ObjectId;
#[cfg(test)]
use mockall::automock;
use crate::{
    DiskCache,
    cache::{CachedCommit, FileChangeRecord},
    GitInfo,
    git::GitCommitAnalysis,
};

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
pub struct GitCommit {
    pub commit: ObjectId,
    pub message: String,
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
    #[error("Failed to convert object to tree: {0}")]
    ObjectToTreeError(gix::object::try_into::Error),
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
    #[error("Directory not found in git tree: {0}")]
    DirectoryNotFound(String),
    #[error("Path is not a directory: {0}")]
    NotADirectory(String),
}

/// File-specific git operations
#[cfg_attr(test, automock)]
pub trait GitFileOps {
    /// Get all commits for a branch/reference (hash + message only, no file diffs).
    /// If `stop_at` is provided, the walk stops (inclusive) once that commit is reached.
    fn commits(
        &self,
        branch: &Option<String>,
        stop_at: Option<ObjectId>,
    ) -> Result<Vec<GitCommit>, GitFileOpsError>;

    /// Return the tip (HEAD) commit ID for a branch without walking history.
    fn branch_tip(&self, branch: &Option<String>) -> Result<ObjectId, GitFileOpsError>;

    /// Return the set of full commit SHAs that touch `file` on `branch` (or HEAD if None).
    fn file_touching_commits(
        &self,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitFileOpsError>;

    /// Get all authors who have modified a file
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError>;

    /// Get file bytes at a specific commit
    /// Return bytes to either use in excel reader or convert to string
    fn file_bytes_at_commit(
        &self,
        file: &Path,
        commit: &ObjectId,
    ) -> Result<Vec<u8>, GitFileOpsError>;

    /// List immediate children of `path` in the HEAD commit tree.
    /// `path` is repo-relative, slash-separated, no leading/trailing slash.
    /// Empty string = repo root.
    /// Returns `(name, is_directory)` pairs sorted: dirs first, then files, alpha within each.
    fn list_tree_entries(&self, path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError>;
}

impl GitFileOps for GitInfo {
    fn commits(
        &self,
        branch: &Option<String>,
        stop_at: Option<ObjectId>,
    ) -> Result<Vec<GitCommit>, GitFileOpsError> {
        log::debug!("Getting all commits for branch: {:?}", branch);
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
            // Use HEAD as default
            repo.head_id().map_err(GitFileOpsError::HeadIdError)?
        };

        let revwalk = repo.rev_walk([start_id]);

        let commit_ids = revwalk
            .all()
            .map_err(GitFileOpsError::RevWalkError)?
            .into_iter()
            .filter_map(|c| c.map(|info| info.id).ok())
            .collect::<Vec<_>>();

        let short_stop = stop_at.map(|s| s.to_string()[0..7].to_string());

        log::debug!(
            "Found {} potential commits on {:?}{}",
            commit_ids.len(),
            branch,
            short_stop
                .as_ref()
                .map(|s| format!(". Looking for {s}"))
                .unwrap_or_default()
        );

        if let Some(stop_commit) = &stop_at {
            if let Some((i, _)) = commit_ids
                .iter()
                .enumerate()
                .find(|(_, c)| c == &stop_commit)
            {
                log::debug!("Found {} at {i}", short_stop.as_ref().unwrap());
            }
        }

        for commit_id in commit_ids {
            let commit_obj = repo
                .find_object(commit_id)
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_commit()
                .map_err(GitFileOpsError::CommitError)?;

            let commit_message = commit_obj
                .message_raw()
                .map(|msg| msg.to_string())
                .unwrap_or_default();

            let is_stop = stop_at.map_or(false, |s| s == commit_id);
            commits.push(GitCommit {
                commit: commit_id,
                message: commit_message,
            });
            if is_stop {
                break;
            }
        }

        log::debug!("Found {} commits for branch: {:?}", commits.len(), branch);
        Ok(commits)
    }

    fn branch_tip(&self, branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
        let repo = self.repository()?;
        if let Some(branch_name) = branch.as_ref() {
            let branch_ref_name = format!("refs/heads/{}", branch_name);
            let branch_ref = repo
                .find_reference(&branch_ref_name)
                .map_err(|_| GitFileOpsError::BranchNotFound(branch_name.clone()))?;
            Ok(branch_ref.id().detach())
        } else {
            repo.head_id()
                .map(|id| id.detach())
                .map_err(GitFileOpsError::HeadIdError)
        }
    }

    fn file_touching_commits(
        &self,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitFileOpsError> {
        use crate::git::action::{GitCli as _, GitCommand};
        GitCommand
            .file_touching_commits(&self.repository_path, branch, file)
            .map_err(|e| GitFileOpsError::BranchNotFound(e.to_string()))
    }

    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
        let repo = self.repository()?;
        let all_commits = self.commits(&None, None)?;

        // Find commits that touch this file via git log subprocess
        let touching = self
            .file_touching_commits(None, file)
            .unwrap_or_default();
        let file_commits: Vec<&GitCommit> = all_commits
            .iter()
            .filter(|c| touching.contains(&c.commit.to_string()))
            .collect();

        let mut res: Vec<GitAuthor> = Vec::new();

        for commit in file_commits {
            let commit_obj = repo
                .find_object(commit.commit)
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_commit()
                .map_err(GitFileOpsError::CommitError)?;

            let signature = commit_obj
                .author()
                .map_err(GitFileOpsError::SignatureError)?;
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

    fn file_bytes_at_commit(
        &self,
        file: &Path,
        commit: &gix::ObjectId,
    ) -> Result<Vec<u8>, GitFileOpsError> {
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

        log::debug!(
            "Successfully read {} bytes from file {:?} at commit {}",
            blob.data.len(),
            file_path,
            commit
        );

        Ok(blob.data.clone())
    }

    fn list_tree_entries(&self, path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
        let repo = self.repository()?;
        let head_id = repo.head_id().map_err(GitFileOpsError::HeadIdError)?;
        let commit_obj = repo
            .find_object(head_id)
            .map_err(GitFileOpsError::ObjectError)?
            .try_into_commit()
            .map_err(GitFileOpsError::CommitError)?;
        let root_tree = commit_obj.tree().map_err(GitFileOpsError::TreeError)?;

        // Navigate directly to the target subtree instead of collecting all files
        let target_tree = if path.is_empty() {
            root_tree
        } else {
            let entry = root_tree
                .lookup_entry_by_path(path)
                .map_err(|_| GitFileOpsError::DirectoryNotFound(path.to_string()))?
                .ok_or_else(|| GitFileOpsError::DirectoryNotFound(path.to_string()))?;

            if !entry.mode().is_tree() {
                return Err(GitFileOpsError::NotADirectory(path.to_string()));
            }

            entry
                .object()
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_tree()
                .map_err(GitFileOpsError::ObjectToTreeError)?
        };

        // Iterate only immediate children of the target tree
        let mut result: Vec<(String, bool)> = Vec::new();
        for entry in target_tree.iter() {
            let entry = entry.map_err(|e| GitFileOpsError::TreeError(e.into()))?;
            let name = entry.filename().to_string();
            let is_dir = entry.mode().is_tree();
            result.push((name, is_dir));
        }

        result.sort_by(
            |(name_a, is_dir_a), (name_b, is_dir_b)| match (is_dir_a, is_dir_b) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => name_a.cmp(name_b),
            },
        );

        Ok(result)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Disk-cache helpers
// ──────────────────────────────────────────────────────────────────────────────

fn commits_to_cached(commits: &[GitCommit]) -> Vec<CachedCommit> {
    commits
        .iter()
        .map(|c| CachedCommit {
            hash: c.commit.to_string(),
            message: c.message.clone(),
            file_changes: Vec::new(),
        })
        .collect()
}

fn cached_to_commits(cached: &[CachedCommit]) -> Vec<GitCommit> {
    cached
        .iter()
        .filter_map(|c| {
            ObjectId::from_str(&c.hash).ok().map(|id| GitCommit {
                commit: id,
                message: c.message.clone(),
            })
        })
        .collect()
}

/// Merge `file_changes` from `old` into `new_cached` where hashes match.
fn merge_file_changes(
    mut new_cached: Vec<CachedCommit>,
    old: Option<Vec<CachedCommit>>,
) -> Vec<CachedCommit> {
    let Some(old) = old else {
        return new_cached;
    };
    let old_map: std::collections::HashMap<&str, &Vec<FileChangeRecord>> =
        old.iter().map(|c| (c.hash.as_str(), &c.file_changes)).collect();
    for commit in &mut new_cached {
        if let Some(fc) = old_map.get(commit.hash.as_str()) {
            commit.file_changes = (*fc).clone();
        }
    }
    new_cached
}

fn disk_cache_key(branch: &Option<String>) -> String {
    branch.as_deref().unwrap_or("HEAD").to_string()
}

fn save_cached(disk_cache: &DiskCache, cache_key: &str, cached: &[CachedCommit]) {
    if let Err(e) = disk_cache.write(&["commits"], cache_key, &cached, false) {
        log::warn!("Failed to write commit cache to disk: {}", e);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────────────

/// Get commits for a branch, backed by disk cache.
///
/// Cache validity requires:
/// 1. The cached HEAD matches the current branch tip (all commits present).
/// 2. `stop_at` is present somewhere in the cached list.
///
/// On a mismatch the full walk is re-run and the cache is updated (preserving any
/// previously stored `file_changes` for commits that still exist in the new history).
/// If `stop_at` is no longer reachable (force-pushed away), the walk runs without it.
pub fn find_commits(
    git_info: &impl GitFileOps,
    branch: &Option<String>,
    stop_at: Option<ObjectId>,
    disk_cache: Option<&DiskCache>,
) -> Result<Vec<GitCommit>, GitFileOpsError> {
    let cache_key = disk_cache_key(branch);

    // ── Try disk cache ──────────────────────────────────────────────────────
    if let Some(cache) = disk_cache {
        let cached: Option<Vec<CachedCommit>> = cache.read(&["commits"], &cache_key);
        if let Some(ref cached_commits) = cached {
            // Validate: current branch tip must equal the first cached entry
            let current_tip = git_info.branch_tip(branch).ok().map(|id| id.to_string());
            let cached_head = cached_commits.first().map(|c| c.hash.as_str());

            let tip_matches = current_tip.as_deref() == cached_head;

            let stop_present = stop_at
                .map(|s| {
                    let s = s.to_string();
                    cached_commits.iter().any(|c| c.hash == s)
                })
                .unwrap_or(true);

            if tip_matches && stop_present {
                log::debug!("Disk cache hit for branch {:?}", branch);
                return Ok(cached_to_commits(cached_commits));
            }

            if tip_matches && !stop_present {
                log::debug!(
                    "Disk cache insufficient for branch {:?} (stop_at not found)",
                    branch
                );
            } else {
                log::debug!(
                    "Disk cache stale for branch {:?} (HEAD changed or force-pushed)",
                    branch
                );
            }
        }

        // ── Full walk ───────────────────────────────────────────────────────
        log::debug!("Full commit walk for branch {:?}", branch);
        let effective_stop = stop_at.filter(|s| {
            // If stop_at no longer exists in the repo, skip it to avoid infinite walk
            git_info.branch_tip(&Some(s.to_string())).is_ok()
                || git_info.commits(branch, Some(*s)).is_ok()
        });
        let commits = git_info.commits(branch, effective_stop)?;
        let old_cached: Option<Vec<CachedCommit>> = cache.read(&["commits"], &cache_key);
        let new_cached = merge_file_changes(commits_to_cached(&commits), old_cached);
        save_cached(cache, &cache_key, &new_cached);
        Ok(commits)
    } else {
        // No disk cache – plain walk
        git_info.commits(branch, stop_at)
    }
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

/// Get commits with robust branch handling
/// 1. Try the specified branch first (via `find_commits` cache)
/// 2. If commit is provided and branch not found, find merged branch using commit analysis
/// 3. Fall back to searching all branches containing the commit
pub fn get_commits_robust(
    git_info: &(impl GitFileOps + GitCommitAnalysis),
    branch: &Option<String>,
    commit: Option<&ObjectId>,
    stop_at: Option<ObjectId>,
    disk_cache: Option<&DiskCache>,
) -> Result<Vec<GitCommit>, GitFileOpsError> {
    // First, try to get commits from the specified branch
    match find_commits(git_info, branch, stop_at, disk_cache) {
        Ok(commits) => {
            log::debug!("Found {} commits for branch {:?}", commits.len(), branch);
            return Ok(commits);
        }
        Err(GitFileOpsError::BranchNotFound(_)) if branch.is_some() => {
            log::debug!(
                "Branch {:?} not found locally, searching for merged commits",
                branch
            );
        }
        Err(e) => {
            return Err(e);
        }
    }

    // If we have a commit, try to find which branch it was merged into
    if let Some(commit) = commit {
        log::debug!("Using commit {} to find merged branch for commits", commit);

        // Try to find which branch this commit was merged into
        if let Some(target_branch) = find_merged_into_branch(git_info, commit)? {
            log::debug!(
                "Found that commit {} was merged into branch {}",
                commit,
                target_branch
            );

            // Try to get commits from the target branch
            match find_commits(git_info, &Some(target_branch.clone()), stop_at, disk_cache) {
                Ok(commits) => {
                    log::debug!(
                        "Found {} commits for merged target branch {}",
                        commits.len(),
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

            // Try each branch until we find one that works
            for branch_name in branches_containing_commit {
                match find_commits(git_info, &Some(branch_name.clone()), stop_at, disk_cache) {
                    Ok(commits) if !commits.is_empty() => {
                        log::debug!(
                            "Found {} commits for branch {} (contains commit)",
                            commits.len(),
                            branch_name
                        );
                        return Ok(commits);
                    }
                    Ok(_) => {
                        log::debug!("Branch {} contains commit but has no commits?", branch_name);
                    }
                    Err(e) => {
                        log::debug!("Failed to get commits from branch {}: {}", branch_name, e);
                    }
                }
            }
        }
    }

    // Final fallback: return error that branch couldn't be found
    if let Some(branch_name) = branch {
        Err(GitFileOpsError::BranchNotFound(format!(
            "Could not find branch '{}' or determine alternative branch from commit",
            branch_name
        )))
    } else {
        Err(GitFileOpsError::BranchNotFound(
            "Could not determine branch from commit".to_string(),
        ))
    }
}

/// For a list of commit hashes, determine which touch `file` on `branch`.
///
/// Checks the disk cache first: if every commit already has a `FileChangeRecord` for `file`,
/// returns without running git. Otherwise runs `git log -- <file>` once, updates all entries in
/// the cache for this branch, and saves back to disk.
pub fn find_or_cache_file_changes(
    commit_hashes: &[String],
    git_info: &impl GitFileOps,
    branch: Option<String>,
    file: &Path,
    disk_cache: Option<&DiskCache>,
) -> Result<HashSet<String>, GitFileOpsError> {
    let file_str = file.to_string_lossy().to_string();
    let cache_key = disk_cache_key(&branch);

    // ── Check disk cache ────────────────────────────────────────────────────
    if let Some(cache) = disk_cache {
        let cached: Option<Vec<CachedCommit>> = cache.read(&["commits"], &cache_key);
        if let Some(mut cached_commits) = cached {
            let all_have_entry = commit_hashes.iter().all(|h| {
                cached_commits
                    .iter()
                    .find(|c| &c.hash == h)
                    .map(|c| c.file_changes.iter().any(|fc| fc.file == file_str))
                    .unwrap_or(false)
            });

            if all_have_entry {
                log::debug!("file_changes cache hit for {:?} on {:?}", file, branch);
                let touching = cached_commits
                    .iter()
                    .filter(|c| {
                        c.file_changes
                            .iter()
                            .any(|fc| fc.file == file_str && fc.changed)
                    })
                    .map(|c| c.hash.clone())
                    .collect();
                return Ok(touching);
            }

            // Cache miss for some commits – run git and update all entries
            let touching_set = git_info.file_touching_commits(branch.clone(), file)?;

            for commit in &mut cached_commits {
                commit.file_changes.retain(|fc| fc.file != file_str);
                commit.file_changes.push(FileChangeRecord {
                    file: file_str.clone(),
                    changed: touching_set.contains(&commit.hash),
                });
            }
            save_cached(cache, &cache_key, &cached_commits);
            return Ok(touching_set);
        }
    }

    // ── No cache or cache not available – just run git ──────────────────────
    git_info.file_touching_commits(branch, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitCommitAnalysis, GitCommitAnalysisError};
    use std::{collections::HashMap, path::PathBuf, str::FromStr};

    fn create_test_commits() -> Vec<(ObjectId, String)> {
        vec![
            (
                ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                "Initial commit".to_string(),
            ),
            (
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                "Second commit".to_string(),
            ),
            (
                ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                "Third commit".to_string(),
            ),
            (
                ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap(),
                "Fourth commit".to_string(),
            ),
            (
                ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap(),
                "Fifth commit".to_string(),
            ),
            (
                ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap(),
                "Sixth commit".to_string(),
            ),
            (
                ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap(),
                "Seventh commit".to_string(),
            ),
        ]
    }

    // Enhanced MockGitInfo for testing robust branch handling
    struct RobustMockGitInfo {
        file_commits_responses:
            HashMap<Option<String>, Result<Vec<(ObjectId, String)>, GitFileOpsError>>,
        merge_commits: Vec<ObjectId>,
        commit_parents: HashMap<ObjectId, Vec<ObjectId>>,
        ancestor_relationships: HashMap<(ObjectId, ObjectId), bool>,
        branches_containing_commits: HashMap<ObjectId, Vec<String>>,
    }

    impl RobustMockGitInfo {
        fn new() -> Self {
            Self {
                file_commits_responses: HashMap::new(),
                merge_commits: Vec::new(),
                commit_parents: HashMap::new(),
                ancestor_relationships: HashMap::new(),
                branches_containing_commits: HashMap::new(),
            }
        }

        fn with_file_commits_result(
            mut self,
            branch: Option<String>,
            result: Result<Vec<(ObjectId, String)>, GitFileOpsError>,
        ) -> Self {
            self.file_commits_responses.insert(branch, result);
            self
        }

        fn with_merge_commits(mut self, commits: Vec<ObjectId>) -> Self {
            self.merge_commits = commits;
            self
        }

        fn with_commit_parents(mut self, commit: ObjectId, parents: Vec<ObjectId>) -> Self {
            self.commit_parents.insert(commit, parents);
            self
        }

        fn with_ancestor_relationship(
            mut self,
            ancestor: ObjectId,
            descendant: ObjectId,
            is_ancestor: bool,
        ) -> Self {
            self.ancestor_relationships
                .insert((ancestor, descendant), is_ancestor);
            self
        }

        fn with_branches_containing_commit(
            mut self,
            commit: ObjectId,
            branches: Vec<String>,
        ) -> Self {
            self.branches_containing_commits.insert(commit, branches);
            self
        }
    }

    impl GitFileOps for RobustMockGitInfo {
        fn commits(
            &self,
            branch: &Option<String>,
            _stop_at: Option<ObjectId>,
        ) -> Result<Vec<GitCommit>, GitFileOpsError> {
            match self.file_commits_responses.get(branch) {
                Some(Ok(commits)) => Ok(commits
                    .iter()
                    .map(|(commit, message)| GitCommit {
                        commit: *commit,
                        message: message.clone(),
                    })
                    .collect()),
                Some(Err(GitFileOpsError::BranchNotFound(branch_name))) => {
                    Err(GitFileOpsError::BranchNotFound(branch_name.clone()))
                }
                Some(Err(_e)) => Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))), // Fallback error for testing
                None => Ok(Vec::new()),
            }
        }

        fn branch_tip(&self, branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
            // Return the first commit of this branch as its tip
            match self.file_commits_responses.get(branch) {
                Some(Ok(commits)) => commits
                    .first()
                    .map(|(id, _)| *id)
                    .ok_or_else(|| GitFileOpsError::BranchNotFound("empty branch".to_string())),
                _ => Err(GitFileOpsError::BranchNotFound(
                    branch.clone().unwrap_or_default(),
                )),
            }
        }

        fn file_touching_commits(
            &self,
            _branch: Option<String>,
            _file: &Path,
        ) -> Result<HashSet<String>, GitFileOpsError> {
            Ok(HashSet::new())
        }

        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    impl GitCommitAnalysis for RobustMockGitInfo {
        fn get_all_merge_commits(&self) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(self.merge_commits.clone())
        }

        fn get_commit_parents(
            &self,
            commit: &ObjectId,
        ) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(self.commit_parents.get(commit).cloned().unwrap_or_default())
        }

        fn is_ancestor(
            &self,
            ancestor: &ObjectId,
            descendant: &ObjectId,
        ) -> Result<bool, GitCommitAnalysisError> {
            Ok(self
                .ancestor_relationships
                .get(&(*ancestor, *descendant))
                .copied()
                .unwrap_or(false))
        }

        fn get_branches_containing_commit(
            &self,
            commit: &ObjectId,
        ) -> Result<Vec<String>, GitCommitAnalysisError> {
            Ok(self
                .branches_containing_commits
                .get(commit)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn test_get_commits_robust_success_on_first_try() {
        let test_commits = create_test_commits();
        let branch = Some("feature-branch".to_string());
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(branch.clone(), Ok(test_commits.clone()));

        let result = get_commits_robust(
            &git_info,
            &branch,
            Some(&initial_commit),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.len(), test_commits.len());
        // Convert result to expected format for comparison
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }

    #[tokio::test]
    async fn test_get_commits_robust_branch_not_found_uses_merge_detection() {
        let test_commits = create_test_commits();
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let merge_commit = ObjectId::from_str("1234567890abcdef123456789012345678901234").unwrap();
        let parent1 = ObjectId::from_str("2345678901234567890123456789012345678901").unwrap();
        let parent2 = ObjectId::from_str("3456789012345678901234567890123456789012").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // Merge detection finds the target branch
            .with_merge_commits(vec![merge_commit])
            .with_commit_parents(merge_commit, vec![parent1, parent2])
            .with_ancestor_relationship(initial_commit, parent2, true) // initial_commit is ancestor of parent2 (merged branch)
            .with_branches_containing_commit(merge_commit, vec!["main".to_string()])
            // Target branch has the commits
            .with_file_commits_result(Some("main".to_string()), Ok(test_commits.clone()));

        let result = get_commits_robust(
            &git_info,
            &Some(branch.to_string()),
            Some(&initial_commit),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }

    #[tokio::test]
    async fn test_get_commits_robust_fallback_to_branches_containing_commit() {
        let test_commits = create_test_commits();
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // No merge commits found
            .with_merge_commits(vec![])
            // But initial commit is found in some branches
            .with_branches_containing_commit(
                initial_commit,
                vec!["main".to_string(), "develop".to_string()],
            )
            // First branch with file commits wins
            .with_file_commits_result(Some("main".to_string()), Ok(test_commits.clone()))
            .with_file_commits_result(Some("develop".to_string()), Ok(Vec::new()));

        let result = get_commits_robust(
            &git_info,
            &Some(branch.to_string()),
            Some(&initial_commit),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }

    #[tokio::test]
    async fn test_get_commits_robust_final_fallback_fails() {
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // No merge commits found
            .with_merge_commits(vec![])
            // No branches contain the commit
            .with_branches_containing_commit(initial_commit, vec![]);

        let result = get_commits_robust(
            &git_info,
            &Some(branch.to_string()),
            Some(&initial_commit),
            None,
            None,
        );

        // Should fail when no branch can be found
        assert!(matches!(result, Err(GitFileOpsError::BranchNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_commits_robust_no_initial_commit_in_issue_body() {
        let branch = "deleted-branch";
        let invalid_commit =
            ObjectId::from_str("0000000000000000000000000000000000000000").unwrap();

        let git_info = RobustMockGitInfo::new().with_file_commits_result(
            Some(branch.to_string()),
            Err(GitFileOpsError::BranchNotFound(branch.to_string())),
        );

        let result = get_commits_robust(
            &git_info,
            &Some(branch.to_string()),
            Some(&invalid_commit),
            None,
            None,
        );

        // Should fail since no branches can be found and all fallbacks fail
        assert!(matches!(result, Err(GitFileOpsError::BranchNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_commits_robust_git_error_propagated() {
        let branch = "test-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new().with_file_commits_result(
            Some(branch.to_string()),
            Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))),
        );

        let result = get_commits_robust(
            &git_info,
            &Some(branch.to_string()),
            Some(&initial_commit),
            None,
            None,
        );

        assert!(matches!(result, Err(GitFileOpsError::AuthorNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_commits_robust_multiple_merge_commits() {
        let test_commits = create_test_commits();
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let merge_commit1 = ObjectId::from_str("1111111111111111111111111111111111111111").unwrap();
        let merge_commit2 = ObjectId::from_str("2222222222222222222222222222222222222222").unwrap();
        let parent1_1 = ObjectId::from_str("3333333333333333333333333333333333333333").unwrap();
        let parent2_1 = ObjectId::from_str("4444444444444444444444444444444444444444").unwrap();
        let parent1_2 = ObjectId::from_str("5555555555555555555555555555555555555555").unwrap();
        let parent2_2 = ObjectId::from_str("6666666666666666666666666666666666666666").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // Multiple merge commits
            .with_merge_commits(vec![merge_commit1, merge_commit2])
            .with_commit_parents(merge_commit1, vec![parent1_1, parent2_1])
            .with_commit_parents(merge_commit2, vec![parent1_2, parent2_2])
            // First merge commit doesn't match
            .with_ancestor_relationship(initial_commit, parent2_1, false)
            // Second merge commit matches
            .with_ancestor_relationship(initial_commit, parent2_2, true)
            .with_branches_containing_commit(merge_commit2, vec!["develop".to_string()])
            // Target branch has the commits
            .with_file_commits_result(Some("develop".to_string()), Ok(test_commits.clone()));

        let result = get_commits_robust(
            &git_info,
            &Some(branch.to_string()),
            Some(&initial_commit),
            None,
            None,
        )
        .unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }
}
