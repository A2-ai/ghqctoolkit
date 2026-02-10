//! Application state for the API server.

use crate::Configuration;
use crate::DiskCache;
use crate::GitInfo;
use crate::api::cache::StatusCache;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Application state shared across all request handlers.
#[derive(Clone)]
pub struct AppState {
    /// Git repository and GitHub API access.
    git_info: Arc<GitInfo>,
    /// Configuration loaded at startup.
    pub configuration: Arc<Configuration>,
    /// Disk-based cache for GitHub API responses.
    disk_cache: Option<Arc<DiskCache>>,
    /// In-memory cache for issue status responses.
    pub status_cache: Arc<RwLock<StatusCache>>,
}

impl AppState {
    /// Create a new AppState with the given configuration.
    pub fn new(
        git_info: GitInfo,
        configuration: Configuration,
        disk_cache: Option<DiskCache>,
    ) -> Self {
        Self {
            git_info: Arc::new(git_info),
            configuration: Arc::new(configuration),
            disk_cache: disk_cache.map(Arc::new),
            status_cache: Arc::new(RwLock::new(StatusCache::new())),
        }
    }

    pub fn git_info(&self) -> &GitInfo {
        &self.git_info
    }

    pub fn disk_cache(&self) -> Option<&DiskCache> {
        self.disk_cache.as_ref().map(|d| &**d)
    }
}
