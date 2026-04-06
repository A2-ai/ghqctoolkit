use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gix::Url;

#[cfg_attr(test, mockall::automock)]
pub trait GitCli {
    /// Clone a repository from a URL to a local path
    fn clone(&self, url: Url, path: &Path) -> Result<(), GitCliError>;

    fn remote(&self, path: &Path) -> Result<Url, GitCliError>;

    /// Fetch from origin. Returns whether any refs changed.
    /// Sets GIT_TERMINAL_PROMPT=0 to prevent blocking credential prompts.
    fn fetch(&self, path: &Path) -> Result<bool, GitCliError>;

    /// Stash changes for a single file path.
    fn stash_file(
        &self,
        path: &Path,
        file: &Path,
        message: &str,
    ) -> Result<StashFileOutcome, GitCliError>;

    /// Return the set of full commit SHAs (40-char hex) that touch `file` on `branch`.
    /// If `branch` is None, searches from HEAD.
    /// Uses `git log --format=%H [branch] -- <file>`.
    fn file_touching_commits(
        &self,
        repo_path: &Path,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitCliError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StashFileOutcome {
    Stashed,
    NoChanges,
}

/// Error types for git CLI operations
#[derive(thiserror::Error, Debug)]
pub enum GitCliError {
    #[error("Directory exists: {0}")]
    DirectoryExists(PathBuf),
    #[error("Directory does not exist: {0}")]
    NoDirectoryExists(PathBuf),
    #[error("Git command failed to execute: {0}")]
    GitCommandError(#[from] std::io::Error),
    #[error("Git command failed: {0}")]
    GitCommandFailed(String),
    #[error("Failed to determine remote url: {0}")]
    NoRemote(String),
    #[error("Failed to parse git remote URL: {0}")]
    InvalidRemoteUrl(String),
}

impl<T: GitCli + ?Sized> GitCli for &T {
    fn clone(&self, url: Url, path: &Path) -> Result<(), GitCliError> {
        (**self).clone(url, path)
    }

    fn remote(&self, path: &Path) -> Result<Url, GitCliError> {
        (**self).remote(path)
    }

    fn fetch(&self, path: &Path) -> Result<bool, GitCliError> {
        (**self).fetch(path)
    }

    fn stash_file(
        &self,
        path: &Path,
        file: &Path,
        message: &str,
    ) -> Result<StashFileOutcome, GitCliError> {
        (**self).stash_file(path, file, message)
    }

    fn file_touching_commits(
        &self,
        repo_path: &Path,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitCliError> {
        (**self).file_touching_commits(repo_path, branch, file)
    }
}

/// Default implementation of GitCli using the git command line
#[derive(Debug, Clone, Default)]
pub struct GitCommand;

impl GitCli for GitCommand {
    fn clone(&self, url: Url, path: &Path) -> Result<(), GitCliError> {
        log::debug!("Cloning repository from {} to {}", url, path.display());

        if path.exists() {
            log::debug!("Path ({}) already exists", path.display());
            return Err(GitCliError::DirectoryExists(path.to_path_buf()));
        }

        // Use the system git command for cloning - it handles authentication better
        let mut cmd = std::process::Command::new("git");
        cmd.args(&["clone", &url.to_string(), &path.to_string_lossy()]);

        log::debug!(
            "Running git clone command: git clone {} {}",
            url,
            path.display()
        );

        let output = cmd.output().map_err(|e| GitCliError::GitCommandError(e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!("Git clone failed: {}", stderr);
            return Err(GitCliError::GitCommandFailed(stderr.to_string()));
        }

        log::debug!("Successfully cloned repository using git command");
        Ok(())
    }

    fn remote(&self, path: &Path) -> Result<Url, GitCliError> {
        if !path.exists() {
            return Err(GitCliError::NoDirectoryExists(path.to_path_buf()));
        }

        // Use git command to get remote URL
        let mut cmd = std::process::Command::new("git");
        cmd.args(&["-C", &path.to_string_lossy(), "remote", "get-url", "origin"]);

        log::debug!(
            "Running git remote command: git -C {} remote get-url origin",
            path.display()
        );

        let output = cmd.output().map_err(|e| GitCliError::GitCommandError(e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!("Git remote failed: {}", stderr);
            return Err(GitCliError::NoRemote(stderr.to_string()));
        }

        let url_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        log::debug!("Found remote URL: {}", url_str);

        // Parse the URL using gix
        gix::url::parse(url_str.as_bytes().into()).map_err(|e| {
            GitCliError::InvalidRemoteUrl(format!("Failed to parse '{}': {}", url_str, e))
        })
    }

    fn fetch(&self, path: &Path) -> Result<bool, GitCliError> {
        log::debug!("Fetching from origin in {}", path.display());

        let output = std::process::Command::new("git")
            .args(["-C", &path.to_string_lossy(), "fetch", "origin"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()?;

        if output.status.success() {
            // git prints ref update lines to stderr when changes are received
            let changed = !output.stderr.is_empty();
            log::debug!("Fetch completed (changes: {})", changed);
            Ok(changed)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(GitCliError::GitCommandFailed(stderr.trim().to_string()))
        }
    }

    fn file_touching_commits(
        &self,
        repo_path: &Path,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitCliError> {
        log::debug!(
            "Finding commits touching {:?} on {:?} in {}",
            file,
            branch,
            repo_path.display()
        );

        let mut cmd = std::process::Command::new("git");
        // --full-history disables history simplification so merge commits that
        // introduce changes to the file are included (matching the prior
        // tree-diff behaviour which compared every commit against all parents).
        // -m causes diffs for merge commits to be computed against each parent
        // individually, which is required for --full-history to detect per-file
        // changes in merge commits correctly.
        cmd.args([
            "-C",
            &repo_path.to_string_lossy(),
            "log",
            "--format=%H",
            "--full-history",
            "-m",
        ]);
        if let Some(b) = branch {
            cmd.arg(b);
        }
        cmd.args(["--", &file.to_string_lossy()]);

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitCliError::GitCommandFailed(stderr.trim().to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let hashes = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(hashes)
    }

    fn stash_file(
        &self,
        path: &Path,
        file: &Path,
        message: &str,
    ) -> Result<StashFileOutcome, GitCliError> {
        log::debug!("Stashing file {} in {}", file.display(), path.display());

        let output = std::process::Command::new("git")
            .args([
                "-C",
                &path.to_string_lossy(),
                "stash",
                "push",
                "-m",
                message,
                "--",
                &file.to_string_lossy(),
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitCliError::GitCommandFailed(stderr.trim().to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("No local changes to save") {
            Ok(StashFileOutcome::NoChanges)
        } else {
            Ok(StashFileOutcome::Stashed)
        }
    }
}
