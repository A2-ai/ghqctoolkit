use std::path::{Path, PathBuf};

use gix::Url;
#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitCli {
    /// Clone a repository from a URL to a local path
    fn clone(&self, url: Url, path: &Path) -> Result<(), GitCliError>;

    fn remote(&self, path: &Path) -> Result<Url, GitCliError>;
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
}
