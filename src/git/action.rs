use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Output;

use gix::Url;

#[cfg_attr(test, mockall::automock)]
pub trait GitCli {
    /// Clone a repository from a URL to a local path
    fn clone(&self, url: Url) -> Result<(), GitCliError>;

    fn remote(&self) -> Result<Url, GitCliError>;

    /// Fetch from the named remote. Returns whether any refs changed.
    /// Sets GIT_TERMINAL_PROMPT=0 to prevent blocking credential prompts.
    fn fetch(&self, remote_name: &str) -> Result<bool, GitCliError>;

    /// Stash changes for a single file path.
    fn stash_file(&self, file: &Path, message: &str) -> Result<StashFileOutcome, GitCliError>;

    /// Return the set of full commit SHAs (40-char hex) that touch `file` on `branch`.
    /// If `branch` is None, searches from HEAD.
    /// Uses `git log --format=%H [branch] -- <file>`.
    fn file_touching_commits(
        &self,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitCliError>;

    fn branch_commits<'a>(
        &self,
        branch: Option<&'a str>,
        stop_at: Option<&'a str>,
    ) -> Result<Vec<(String, String)>, GitCliError>;

    fn path(&self) -> &Path;

    /// Construct an instance rooted at `path`.
    fn new(path: &Path) -> Self;
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
    fn clone(&self, url: Url) -> Result<(), GitCliError> {
        (**self).clone(url)
    }

    fn remote(&self) -> Result<Url, GitCliError> {
        (**self).remote()
    }

    fn fetch(&self, remote_name: &str) -> Result<bool, GitCliError> {
        (**self).fetch(remote_name)
    }

    fn stash_file(&self, file: &Path, message: &str) -> Result<StashFileOutcome, GitCliError> {
        (**self).stash_file(file, message)
    }

    fn file_touching_commits(
        &self,
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitCliError> {
        (**self).file_touching_commits(branch, file)
    }
    fn branch_commits(
        &self,
        branch: Option<&str>,
        stop_at: Option<&str>,
    ) -> Result<Vec<(String, String)>, GitCliError> {
        (**self).branch_commits(branch, stop_at)
    }
    fn path(&self) -> &Path {
        (**self).path()
    }

    fn new(_path: &Path) -> Self {
        // Constructing a reference via the blanket impl is never needed;
        // this exists only to satisfy the trait bound.
        panic!("GitCli::new is not supported on references")
    }
}

/// Default implementation of GitCli using the git command line
#[derive(Debug, Clone, Default)]
pub struct GitCommand {
    pub path: PathBuf,
}
impl GitCli for GitCommand {
    fn clone(&self, url: Url) -> Result<(), GitCliError> {
        log::debug!("Cloning repository from {} to {}", url, self.path.display());

        if self.path.exists() {
            log::debug!("Path ({}) already exists", self.path.display());
            return Err(GitCliError::DirectoryExists(self.path.to_path_buf()));
        }

        // Use the system git command for cloning - it handles authentication better
        let mut cmd = std::process::Command::new("git");
        cmd.args(&["clone", &url.to_string(), &self.path.to_string_lossy()]);

        log::debug!(
            "Running git clone command: git clone {} {}",
            url,
            self.path.display()
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

    fn remote(&self) -> Result<Url, GitCliError> {
        if !self.path.exists() {
            return Err(GitCliError::NoDirectoryExists(self.path.to_path_buf()));
        }

        // Use git command to get remote URL
        let args = vec!["remote", "get-url", "origin"];
        let output = self.run_git(&args)?;

        let url_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        log::debug!("Found remote URL: {}", url_str);

        // Parse the URL using gix
        gix::url::parse(url_str.as_bytes().into()).map_err(|e| {
            GitCliError::InvalidRemoteUrl(format!("Failed to parse '{}': {}", url_str, e))
        })
    }

    fn fetch(&self, remote_name: &str) -> Result<bool, GitCliError> {
        log::debug!("Fetching from {} in {}", remote_name, self.path.display());

        let output = std::process::Command::new("git")
            .args(["-C", &self.path.to_string_lossy(), "fetch", remote_name])
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
        branch: Option<String>,
        file: &Path,
    ) -> Result<HashSet<String>, GitCliError> {
        log::debug!(
            "Finding commits touching {:?} on {:?} in {}",
            file,
            branch,
            self.path.display()
        );

        // Rely on git's default history simplification: a merge that is
        // TREESAME with one parent for this path is dropped, which avoids
        // listing merge commits whose resolution matched a parent verbatim
        // (i.e. no net content change for the file).
        let mut args = vec!["log", "--format=%H"];
        if let Some(b) = &branch {
            args.push(b.as_str());
        }
        args.push("--");
        let file_binding = file.to_string_lossy();
        args.push(file_binding.as_ref());

        let output = self.run_git(&args)?;
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

    fn stash_file(&self, file: &Path, message: &str) -> Result<StashFileOutcome, GitCliError> {
        log::debug!(
            "Stashing file {} in {}",
            file.display(),
            self.path.display()
        );
        let args = [
            "stash",
            "push",
            "-m",
            message,
            "--",
            &file.to_string_lossy(),
        ];
        let output = self.run_git(&args)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("No local changes to save") {
            Ok(StashFileOutcome::NoChanges)
        } else {
            Ok(StashFileOutcome::Stashed)
        }
    }

    /// Return commits reachable from `branch` (or HEAD if None) back to and
    /// including `stop_at` (if given), across all parent chains.
    ///
    /// Uses `git log --format="%H%x1f%s" <branch> ^<stop_at>^@` so that the
    /// range is computed correctly even when `stop_at` is a merge commit —
    /// `^<sha>^@` excludes everything reachable from *any* parent of stop_at,
    /// meaning stop_at itself is included and nothing older is.
    fn branch_commits(
        &self,
        branch: Option<&str>,
        stop_at: Option<&str>,
    ) -> Result<Vec<(String, String)>, GitCliError> {
        let mut args = vec!["log", "--format=%H%x1f%s", branch.unwrap_or("HEAD")];
        let stop = stop_at.map(|s| format!("^{}^@", s)).unwrap_or_default();
        if !stop.is_empty() {
            args.push(stop.as_str());
        }
        let output = self.run_git(&args)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let commits = stdout
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|l| {
                let mut parts = l.splitn(2, '\x1f');
                let hash = parts.next()?.trim().to_string();
                let subject = parts.next().unwrap_or("").trim().to_string();
                if hash.is_empty() {
                    None
                } else {
                    Some((hash, subject))
                }
            })
            .collect();
        Ok(commits)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }
}

impl GitCommand {
    fn run_git(&self, args: &[&str]) -> Result<Output, GitCliError> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .output()
            .map_err(GitCliError::GitCommandError)?;

        if !output.status.success() {
            return Err(GitCliError::GitCommandFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        Ok(output)
    }
}
