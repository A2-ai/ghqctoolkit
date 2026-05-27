use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{
    DiskCache, GitInfo,
    cache::{CachedCommit, FileChangeRecord},
    git::action::GitCli,
};
use gix::ObjectId;
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

#[derive(Debug, Clone, PartialEq)]
pub struct GitCommit {
    pub commit: ObjectId,
    pub message: String,
}

#[derive(thiserror::Error, Debug)]
pub enum GitFileOpsError {
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
    /// The named branch ref isn't reachable locally — the user likely needs
    /// to fetch / track it before this tool can read its history.
    #[error("Branch '{0}' is not checked out locally")]
    LocalBranchNotFound(String),
    /// A git operation related to branch lookup failed for a reason other than
    /// a missing local ref (merge analysis, ancestry walk, etc.). The string
    /// is a diagnostic message, not a branch name.
    #[error("Branch lookup failed: {0}")]
    BranchLookupFailed(String),
    #[error("Failed to get HEAD ID: {0}")]
    HeadIdError(gix::reference::head_id::Error),
    #[error("Failed to access repository: {0}")]
    RepositoryError(#[from] crate::git::GitInfoError),
    #[error("Directory not found in git tree: {0}")]
    DirectoryNotFound(String),
    #[error("Path is not a directory: {0}")]
    NotADirectory(String),
    #[error("Failed to parse commit SHA: {0}")]
    ParseError(String),
    #[error("Git CLI error: {0}")]
    GitCliError(#[from] crate::git::action::GitCliError),
}

// ──────────────────────────────────────────────────────────────────────────────
// GitCommitOps — commit history and branch operations
// ──────────────────────────────────────────────────────────────────────────────

/// Commit history and branch-level git operations.
#[cfg_attr(test, automock)]
pub trait GitCommitOps {
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

    /// Return the names of all local and remote branches that contain `commit`.
    fn get_branches_containing_commit(
        &self,
        commit: &ObjectId,
    ) -> Result<Vec<String>, GitFileOpsError>;

    /// Find the branch that `target_commit` was merged into, if any.
    ///
    /// Locates the nearest ancestor merge commit on HEAD that incorporated
    /// `target_commit` via `--ancestry-path`, then returns the first branch
    /// that contains that merge commit.
    fn find_merged_into_branch(
        &self,
        target_commit: &ObjectId,
    ) -> Result<Option<String>, GitFileOpsError>;
}

impl GitCommitOps for GitInfo {
    fn commits(
        &self,
        branch: &Option<String>,
        stop_at: Option<ObjectId>,
    ) -> Result<Vec<GitCommit>, GitFileOpsError> {
        log::debug!("Getting all commits for branch: {:?}", branch);
        let branch_str = branch.as_deref();
        let stop_str = stop_at.map(|id| id.to_string());
        let pairs = self
            .command
            .branch_commits(branch_str, stop_str.as_deref())?;
        pairs
            .into_iter()
            .map(|(hash, msg)| {
                let commit = ObjectId::from_str(&hash)
                    .map_err(|_| GitFileOpsError::ParseError(hash.clone()))?;
                Ok(GitCommit {
                    commit,
                    message: msg,
                })
            })
            .collect()
    }

    fn branch_tip(&self, branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
        let repo = self.repository()?;
        if let Some(branch_name) = branch.as_ref() {
            let branch_ref_name = format!("refs/heads/{}", branch_name);
            let branch_ref = repo
                .find_reference(&branch_ref_name)
                .map_err(|_| GitFileOpsError::LocalBranchNotFound(branch_name.clone()))?;
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
        let branch_name = branch.clone();
        self.command
            .file_touching_commits(branch, file)
            .map_err(|e| {
                let msg = e.to_string();
                // git emits `fatal: bad revision '<ref>'` when asked about a
                // ref that doesn't exist locally. In that case we know which
                // branch was missing — surface it as the structured variant
                // so the UI can offer the copy-pasteable fix instead of a
                // raw "lookup failed" message with no suggestion.
                if let Some(name) = branch_name {
                    if msg.contains("bad revision") {
                        return GitFileOpsError::LocalBranchNotFound(name);
                    }
                }
                GitFileOpsError::BranchLookupFailed(msg)
            })
    }

    fn get_branches_containing_commit(
        &self,
        commit: &ObjectId,
    ) -> Result<Vec<String>, GitFileOpsError> {
        log::debug!("Finding branches containing commit {}", commit);
        let sha = commit.to_string();
        let repo_str = self.repository_path.to_string_lossy();

        let local_out = std::process::Command::new("git")
            .args([
                "-C",
                repo_str.as_ref(),
                "branch",
                "--contains",
                &sha,
                "--format=%(refname:short)",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let remote_out = std::process::Command::new("git")
            .args([
                "-C",
                repo_str.as_ref(),
                "branch",
                "-r",
                "--contains",
                &sha,
                "--format=%(refname:short)",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let branches: Vec<String> = local_out
            .lines()
            .chain(remote_out.lines())
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && l != "HEAD" && !l.ends_with("/HEAD"))
            .collect();

        log::debug!(
            "Found {} branches containing commit {}",
            branches.len(),
            commit
        );
        Ok(branches)
    }

    fn find_merged_into_branch(
        &self,
        target_commit: &ObjectId,
    ) -> Result<Option<String>, GitFileOpsError> {
        let repo_str = self.repository_path.to_string_lossy().to_string();
        let target_commit_str = target_commit.to_string();

        // Find the nearest merge commit that incorporated `target_commit`
        let output = std::process::Command::new("git")
            .args([
                "-C",
                &repo_str,
                "log",
                "--merges",
                "--ancestry-path",
                &format!("{}..HEAD", target_commit_str),
                "--format=%H",
            ])
            .output()
            .map_err(|e| GitFileOpsError::BranchLookupFailed(e.to_string()))?;

        if !output.status.success() {
            return Ok(None);
        }

        let merge_sha = String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let merge_sha = match merge_sha {
            Some(s) => s,
            None => return Ok(None),
        };

        // Find which branch contains that merge commit
        let local = std::process::Command::new("git")
            .args([
                "-C",
                &repo_str,
                "branch",
                "--contains",
                &merge_sha,
                "--format=%(refname:short)",
            ])
            .output()
            .map_err(|e| GitFileOpsError::BranchLookupFailed(e.to_string()))?;
        let remote = std::process::Command::new("git")
            .args([
                "-C",
                &repo_str,
                "branch",
                "-r",
                "--contains",
                &merge_sha,
                "--format=%(refname:short)",
            ])
            .output()
            .map_err(|e| GitFileOpsError::BranchLookupFailed(e.to_string()))?;

        let branches: Vec<String> = [local.stdout, remote.stdout]
            .iter()
            .flat_map(|b| {
                String::from_utf8_lossy(b)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty() && l != "HEAD" && !l.ends_with("/HEAD"))
                    .collect::<Vec<_>>()
            })
            .collect();

        Ok(branches.into_iter().next())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// GitFileOps — file content and metadata operations
// ──────────────────────────────────────────────────────────────────────────────

/// File-content and metadata git operations.
#[cfg_attr(test, automock)]
pub trait GitFileOps {
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
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
        let repo = self.repository()?;
        let all_commits = self.commits(&None, None)?;

        // Find commits that touch this file via git log subprocess
        let touching = self.file_touching_commits(None, file).unwrap_or_default();
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
    let old_map: std::collections::HashMap<&str, &Vec<FileChangeRecord>> = old
        .iter()
        .map(|c| (c.hash.as_str(), &c.file_changes))
        .collect();
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
    git_info: &impl GitCommitOps,
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

/// Get commits with robust branch handling
/// 1. Try the specified branch first (via `find_commits` cache)
/// 2. If commit is provided and branch not found, find merged branch using commit analysis
/// 3. Fall back to searching all branches containing the commit
pub fn get_commits_robust(
    git_info: &impl GitCommitOps,
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
        Err(GitFileOpsError::LocalBranchNotFound(_)) if branch.is_some() => {
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
        if let Some(target_branch) = git_info.find_merged_into_branch(commit)? {
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
        let branches_containing_commit = git_info.get_branches_containing_commit(commit)?;

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
        // We know the branch name — surface it as a structured "not checked
        // out locally" so the UI can offer a copy-pasteable fix.
        Err(GitFileOpsError::LocalBranchNotFound(branch_name.clone()))
    } else {
        Err(GitFileOpsError::BranchLookupFailed(
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
    git_info: &impl GitCommitOps,
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

    // MockGitInfo for testing robust branch handling
    struct RobustMockGitInfo {
        file_commits_responses:
            HashMap<Option<String>, Result<Vec<(ObjectId, String)>, GitFileOpsError>>,
    }

    impl RobustMockGitInfo {
        fn new() -> Self {
            Self {
                file_commits_responses: HashMap::new(),
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
    }

    impl GitCommitOps for RobustMockGitInfo {
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
                Some(Err(GitFileOpsError::LocalBranchNotFound(branch_name))) => {
                    Err(GitFileOpsError::LocalBranchNotFound(branch_name.clone()))
                }
                Some(Err(_e)) => Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))),
                None => Ok(Vec::new()),
            }
        }

        fn branch_tip(&self, branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
            match self.file_commits_responses.get(branch) {
                Some(Ok(commits)) => commits.first().map(|(id, _)| *id).ok_or_else(|| {
                    GitFileOpsError::LocalBranchNotFound("empty branch".to_string())
                }),
                _ => Err(GitFileOpsError::LocalBranchNotFound(
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

        fn get_branches_containing_commit(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<String>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn find_merged_into_branch(
            &self,
            _target_commit: &ObjectId,
        ) -> Result<Option<String>, GitFileOpsError> {
            Ok(None)
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

        let result =
            get_commits_robust(&git_info, &branch, Some(&initial_commit), None, None).unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
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
}

#[cfg(test)]
mod integration_tests {
    use std::process::Command;
    use tempfile::TempDir;

    use crate::GitCli;

    fn setup_repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        Command::new("git")
            .args(["init"])
            .current_dir(p)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(p)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(p)
            .output()
            .unwrap();
        // Set default branch to main
        Command::new("git")
            .args(["checkout", "-b", "main"])
            .current_dir(p)
            .output()
            .unwrap();
        dir
    }

    fn commit_file(dir: &std::path::Path, filename: &str, content: &str, message: &str) -> String {
        std::fs::write(dir.join(filename), content).unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(dir)
            .output()
            .unwrap();
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// Verify that `branch_commits` with stop_at correctly includes the feature
    /// branch commit, which exercises the core bug fix (second-parent chains in merges).
    #[test]
    fn test_commits_on_branch_includes_feature_commit_after_merge() {
        use crate::git::action::GitCommand;

        let dir = setup_repo();
        let p = dir.path();

        // Create initial commit on main
        let initial_sha = commit_file(p, "README.md", "initial", "Initial commit on main");

        // Create a commit on main just before branching
        let pre_branch_sha = commit_file(p, "other.txt", "other", "Pre-branch commit");

        // Create feature branch and add a commit
        Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(p)
            .output()
            .unwrap();
        let feature_sha = commit_file(p, "feature.txt", "feature content", "Feature branch commit");

        // Merge feature back to main
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(p)
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "merge",
                "--no-ff",
                "feature",
                "-m",
                "Merge feature into main",
            ])
            .current_dir(p)
            .output()
            .unwrap();

        // Now call branch_commits with stop_at = pre_branch_sha
        // Expected: feature_sha should be in the result (merge includes all parent chains)
        let commits = GitCommand {
            path: p.to_path_buf(),
        }
        .branch_commits(Some("main"), Some(&pre_branch_sha))
        .unwrap();

        let hashes: Vec<&str> = commits.iter().map(|(h, _)| h.as_str()).collect();

        // The feature commit should be present (it's reachable via the merge's second parent)
        assert!(
            hashes.contains(&feature_sha.as_str()),
            "Feature branch commit {} should be present in commits after merge. Got: {:?}",
            feature_sha,
            hashes
        );

        // pre_branch_sha itself should NOT be included (^pre^@ excludes it and its ancestors)
        assert!(
            !hashes.contains(&initial_sha.as_str()),
            "Initial commit (ancestor of stop_at) should not be present"
        );
    }

    /// Verify branch_commits returns commits in the expected order (newest first).
    #[test]
    fn test_commits_on_branch_ordering() {
        use crate::git::action::GitCommand;

        let dir = setup_repo();
        let p = dir.path();

        let sha1 = commit_file(p, "a.txt", "a", "First");
        let sha2 = commit_file(p, "b.txt", "b", "Second");
        let sha3 = commit_file(p, "c.txt", "c", "Third");

        let commits = GitCommand {
            path: p.to_path_buf(),
        }
        .branch_commits(None, None)
        .unwrap();
        let hashes: Vec<&str> = commits.iter().map(|(h, _)| h.as_str()).collect();

        // Newest first
        assert_eq!(hashes[0], sha3.as_str());
        assert_eq!(hashes[1], sha2.as_str());
        assert_eq!(hashes[2], sha1.as_str());
    }
}
