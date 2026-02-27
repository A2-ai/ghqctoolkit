//! Application state for the API server.

use crate::api::cache::StatusCache;
use crate::{CommitCache, Configuration, DiskCache, GitProvider};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Application state shared across all request handlers.
///
/// Generic over the git provider to allow both production (GitInfo)
/// and test (MockGitInfo) implementations.
#[derive(Clone)]
pub struct AppState<G: GitProvider> {
    /// Git repository and GitHub API access.
    git_info: Arc<G>,
    /// Configuration loaded at startup.
    pub configuration: Arc<Configuration>,
    /// Configuration git info to determine status
    configuration_git_info: Option<Arc<G>>,
    /// Disk-based cache for GitHub API responses.
    disk_cache: Option<Arc<DiskCache>>,
    /// In-memory cache for issue status responses.
    pub status_cache: Arc<RwLock<StatusCache>>,
    /// In-memory cache for branch commit lists, shared across requests.
    pub commit_cache: Arc<RwLock<CommitCache>>,
}

impl<G: GitProvider> AppState<G> {
    /// Create a new AppState with the given configuration.
    pub fn new(
        git_info: G,
        configuration: Configuration,
        configuration_git_info: Option<G>,
        disk_cache: Option<DiskCache>,
    ) -> Self {
        Self {
            git_info: Arc::new(git_info),
            configuration: Arc::new(configuration),
            configuration_git_info: configuration_git_info.map(Arc::new),
            disk_cache: disk_cache.map(Arc::new),
            status_cache: Arc::new(RwLock::new(StatusCache::new())),
            commit_cache: Arc::new(RwLock::new(CommitCache::new())),
        }
    }

    pub fn git_info(&self) -> &G {
        &self.git_info
    }

    pub fn disk_cache(&self) -> Option<&DiskCache> {
        self.disk_cache.as_ref().map(|d| &**d)
    }

    pub fn configuration_git_info(&self) -> Option<&G> {
        self.configuration_git_info.as_ref().map(|g| &**g)
    }
}
