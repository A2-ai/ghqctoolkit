use std::{fmt, path::PathBuf};

use crate::GitInfo;
use crate::git::repository::{GitRepository, GitRepositoryError};
use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

#[derive(Debug, Clone, PartialEq)]
pub enum GitState {
    Clean,                 // up to date with remote
    Behind(Vec<ObjectId>), // remote commits not local - count of commits behind
    Ahead(Vec<ObjectId>),  // local commits not remote - count of commits ahead
    Diverged {
        ahead: Vec<ObjectId>,
        behind: Vec<ObjectId>,
    }, // local commits not remote AND remote commits not local
}

impl fmt::Display for GitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

impl GitState {
    /// Format git status for a specific file and issue thread
    pub fn format_for_file(&self, file_commits: &[&ObjectId]) -> String {
        match self {
            GitState::Clean => "Up to date".to_string(),
            GitState::Ahead(commits) => {
                if file_commits.iter().any(|c| commits.contains(c)) {
                    "Local commits".to_string()
                } else {
                    "Up to date".to_string()
                }
            }
            GitState::Behind(commits) => {
                if file_commits.iter().any(|c| commits.contains(c)) {
                    "Remote changes".to_string()
                } else {
                    "Up to date".to_string()
                }
            }
            GitState::Diverged { ahead, behind } => {
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

#[derive(Debug, Clone, PartialEq)]
pub struct GitStatus {
    pub remote_commit: ObjectId,
    pub state: GitState,
    pub dirty: Vec<PathBuf>,
}

impl fmt::Display for GitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.state)
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
    #[error("Failed to fetch from remote: {0}")]
    FetchError(#[from] GitRepositoryError),
}

/// Repository and file status operations
#[cfg_attr(test, automock)]
pub trait GitStatusOps {
    /// Get overall repository status
    fn state(&self) -> Result<(ObjectId, GitState), GitStatusError>;
    fn dirty(&self) -> Result<Vec<PathBuf>, GitStatusError>;
}

impl GitStatusOps for GitInfo {
    fn state(&self) -> Result<(ObjectId, GitState), GitStatusError> {
        log::debug!("Getting git repository status");
        let repo = self.repository()?;

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
                let local_commit_id = head.id().ok_or_else(|| {
                    GitStatusError::HeadError(gix::reference::find::existing::Error::NotFound {
                        name: gix::refs::PartialName::try_from("HEAD").unwrap(),
                    })
                })?;
                return Ok((local_commit_id.into(), GitState::Ahead(local_commits)));
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
            return Ok((remote_commit_id.into(), GitState::Clean));
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
                Ok((remote_commit_id.into(), GitState::Behind(remote_only)))
            }
            (false, true) => {
                log::debug!("Local is ahead of remote by {} commits", local_only.len());
                Ok((remote_commit_id.into(), GitState::Ahead(local_only)))
            }
            (false, false) => {
                log::debug!(
                    "Local and remote have diverged: {} local commits, {} remote commits",
                    local_only.len(),
                    remote_only.len()
                );
                Ok((
                    remote_commit_id.into(),
                    GitState::Diverged {
                        ahead: local_only,
                        behind: remote_only,
                    },
                ))
            }
            (true, true) => {
                // This shouldn't happen since we already checked if commits are equal
                log::debug!("Local and remote are in sync (fallback)");
                Ok((remote_commit_id.into(), GitState::Clean))
            }
        }
    }

    fn dirty(&self) -> Result<Vec<PathBuf>, GitStatusError> {
        let repo = self.repository()?;

        let mut dirty_files: std::collections::BTreeSet<PathBuf> =
            std::collections::BTreeSet::new();

        // Unstaged changes: index vs worktree
        let status_platform = repo
            .status(gix::progress::Discard)
            .map_err(GitStatusError::StatusError)?;

        for entry in status_platform
            .into_index_worktree_iter(std::iter::empty::<gix::bstr::BString>())
            .map_err(GitStatusError::StatusIterError)?
        {
            let entry = entry.map_err(GitStatusError::StatusEntryError)?;
            dirty_files.insert(PathBuf::from(entry.rela_path().to_string()));
        }

        // Staged changes: HEAD tree vs index
        // Any file whose index entry differs from HEAD is staged and therefore dirty.
        'staged: {
            let head_commit = match repo.head_commit() {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Could not read HEAD commit for staged check: {e}");
                    break 'staged;
                }
            };
            let head_tree_id = match head_commit.tree_id() {
                Ok(id) => id,
                Err(e) => {
                    log::warn!("Could not read HEAD tree for staged check: {e}");
                    break 'staged;
                }
            };
            let head_index = match repo.index_from_tree(&head_tree_id) {
                Ok(idx) => idx,
                Err(e) => {
                    log::warn!("Could not build HEAD index for staged check: {e}");
                    break 'staged;
                }
            };
            let work_index = match repo.open_index() {
                Ok(idx) => idx,
                Err(e) => {
                    log::warn!("Could not open working index for staged check: {e}");
                    break 'staged;
                }
            };

            // Build path→id map for HEAD tree entries
            let head_backing = head_index.path_backing();
            let head_map: std::collections::HashMap<String, gix::ObjectId> = head_index
                .entries()
                .iter()
                .map(|e| {
                    (
                        String::from_utf8_lossy(&*e.path_in(head_backing)).into_owned(),
                        e.id,
                    )
                })
                .collect();

            let work_backing = work_index.path_backing();
            let mut work_paths: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            // Files in the working index that are new or modified relative to HEAD
            for entry in work_index.entries() {
                let path = String::from_utf8_lossy(&*entry.path_in(work_backing)).into_owned();
                work_paths.insert(path.clone());
                match head_map.get(&path) {
                    Some(head_id) if *head_id == entry.id => {} // unchanged
                    _ => {
                        dirty_files.insert(PathBuf::from(&path));
                    }
                }
            }

            // Files in HEAD that are absent from the working index → staged deletion
            for path in head_map.keys() {
                if !work_paths.contains(path) {
                    dirty_files.insert(PathBuf::from(path));
                }
            }
        }

        log::debug!(
            "Repository has {} dirty files (staged or unstaged)",
            dirty_files.len()
        );
        Ok(dirty_files.into_iter().collect())
    }
}

/// Check whether a file path is tracked in the current git index of `repo_path`.
///
/// Uses `git ls-files <path>` — empty stdout means the file is untracked/deleted.
fn is_file_tracked(repo_path: &std::path::Path, file: &std::path::Path) -> bool {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "ls-files",
            "--error-unmatch",
            &file.to_string_lossy(),
        ])
        .output();
    match output {
        Ok(o) => o.status.success(),
        Err(e) => {
            log::warn!(
                "[rename] is_file_tracked {:?}: failed to spawn git: {}",
                file,
                e
            );
            false
        }
    }
}

/// Find the most recent rename destination for `old_path` using git log with
/// `--follow --diff-filter=R`.
///
/// Returns `Some(new_path)` if git can detect a rename, `None` otherwise.
fn find_rename_target(repo_path: &std::path::Path, old_path: &std::path::Path) -> Option<PathBuf> {
    // git log --follow --diff-filter=R --name-status --format="" HEAD -- <old_path>
    // Outputs lines like: R100\told_name\tnew_name  (with blank lines between entries)
    // Strategy: combining --diff-filter=R with a pathspec only matches when the path is the
    // *destination* of a rename, not the source. Instead we do it in two steps:
    //   1. Find the most recent commit that touched old_path (the rename commit).
    //   2. Inspect that specific commit for R-type diffs and look for old_path as the source.
    // Strategy: combining --diff-filter=R with a pathspec only matches when the path is the
    // *destination* of a rename, not the source. Instead we do it in two steps:
    //   1. Find the most recent commit that touched old_path (the rename commit).
    //   2. Inspect that specific commit for R-type diffs and look for old_path as the source.
    let commit_output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "log",
            "-1",
            "--format=%H",
            "--",
            &old_path.to_string_lossy(),
        ])
        .output()
        .ok()?;

    if !commit_output.status.success() {
        log::warn!(
            "[rename] find_rename_target {:?}: git log -1 failed",
            old_path
        );
        return None;
    }

    let commit_hash = String::from_utf8_lossy(&commit_output.stdout)
        .trim()
        .to_string();
    if commit_hash.is_empty() {
        return None;
    }

    let show_output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "show",
            "--diff-filter=R",
            "--name-status",
            "--format=",
            &commit_hash,
        ])
        .output()
        .ok()?;

    if !show_output.status.success() {
        log::warn!(
            "[rename] find_rename_target {:?}: git show {} failed",
            old_path,
            commit_hash
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&show_output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('R') {
            continue;
        }
        // Tab-separated: R<score>\t<old>\t<new>
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() == 3 && PathBuf::from(parts[1]) == old_path {
            let new_path = PathBuf::from(parts[2]);
            if is_file_tracked(repo_path, &new_path) {
                log::debug!("[rename] {:?} → {:?}", old_path, new_path);
                return Some(new_path);
            }
        }
    }

    None
}

/// For each path in `issue_paths` that no longer exists in the git index, attempt
/// to find a rename using `git log --follow --diff-filter=R`.
///
/// Returns one `FileRenameEvent` (without commit hash) per detected rename.
/// The commit hash field is left empty here — callers that need it should fetch
/// it separately, or it will be populated when the rename is confirmed.
pub fn detect_renames(
    repo_path: &std::path::Path,
    issue_paths: &[PathBuf],
) -> Vec<(PathBuf, PathBuf)> {
    let mut renames = Vec::new();
    for old_path in issue_paths {
        if is_file_tracked(repo_path, old_path) {
            continue; // file still exists — no rename needed
        }
        if let Some(new_path) = find_rename_target(repo_path, old_path) {
            renames.push((old_path.clone(), new_path));
        }
    }
    renames
}

/// Get the short (8-char) HEAD commit hash for the given repo path.
pub fn head_commit_hash(repo_path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "rev-parse",
            "--short",
            "HEAD",
        ])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Fetch from remote and then get repository status
///
/// This function ensures the status check is performed against the actual remote state,
/// not a stale local tracking branch. It first performs a git fetch to update the
/// refs/remotes/origin/* tracking branches, then checks the status.
///
/// If the fetch fails (e.g., no network access, no usable origin), it falls back to
/// checking the local status against the last-fetched remote state. This prevents hard
/// failures for offline workflows while still providing fresh remote data when available.
///
/// # Arguments
/// * `git_info` - A reference to an object implementing both GitRepository and GitStatusOps
///
/// # Returns
/// * `Ok(GitStatus)` - The current status and dirty files for the repository
/// * `Err(GitStatusError)` - If status check fails (fetch errors are logged and ignored)
pub fn get_git_status<T>(git_info: &T) -> Result<GitStatus, GitStatusError>
where
    T: GitRepository + GitStatusOps,
{
    log::debug!("Fetching from remote before status check");
    match git_info.fetch() {
        Ok(changes_found) => {
            log::debug!(
                "Fetch complete (changes found: {}), checking status",
                changes_found
            );
        }
        Err(e) => {
            log::warn!(
                "Failed to fetch from remote: {}. Falling back to local status check against last-fetched remote state.",
                e
            );
        }
    }
    let (remote_commit, state) = git_info.state()?;
    let dirty = git_info.dirty()?;

    Ok(GitStatus {
        remote_commit,
        state,
        dirty,
    })
}
