//! Application state for the API server.

use crate::api::cache::StatusCache;
use crate::{CommitCache, Configuration, DiskCache, GitCli, GitCommand, GitProvider};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Application state shared across all request handlers.
///
/// Generic over the git provider to allow both production (GitInfo)
/// and test (MockGitInfo) implementations.
#[derive(Clone)]
pub struct AppState<G: GitProvider> {
    /// Git repository and GitHub API access.
    git_info: Arc<G>,
    /// Configuration loaded at startup.
    pub configuration: Arc<RwLock<Configuration>>,
    /// Configuration git info to determine status
    configuration_git_info: Arc<RwLock<Option<G>>>,
    /// Disk-based cache for GitHub API responses.
    disk_cache: Option<Arc<DiskCache>>,
    /// In-memory cache for issue status responses.
    pub status_cache: Arc<RwLock<StatusCache>>,
    /// Configuration git_info update
    pub config_git_info_creator: Arc<dyn Fn(&Path) -> Option<G> + Send + Sync + 'static>,
    /// Git Cli trait
    git_cli: Arc<dyn GitCli + Send + Sync>,
    /// Preview PDF store: UUID key → temp file path
    preview_store: Arc<Mutex<HashMap<String, PathBuf>>>,
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
            configuration: Arc::new(RwLock::new(configuration)),
            configuration_git_info: Arc::new(RwLock::new(configuration_git_info)),
            disk_cache: disk_cache.map(Arc::new),
            status_cache: Arc::new(RwLock::new(StatusCache::new())),
            commit_cache: Arc::new(RwLock::new(CommitCache::new())),
            config_git_info_creator: Arc::new(|_| None),
            git_cli: Arc::new(GitCommand),
            preview_store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_git_cli(mut self, cli: impl GitCli + Send + Sync + 'static) -> Self {
        self.git_cli = Arc::new(cli);
        self
    }

    pub fn with_creator(
        mut self,
        creator: impl Fn(&Path) -> Option<G> + Send + Sync + 'static,
    ) -> Self {
        self.config_git_info_creator = Arc::new(creator);
        self
    }

    pub fn git_info(&self) -> &G {
        &self.git_info
    }

    pub fn disk_cache(&self) -> Option<&DiskCache> {
        self.disk_cache.as_ref().map(|d| &**d)
    }

    pub async fn configuration_git_info(&self) -> Option<G> {
        self.configuration_git_info.read().await.clone()
    }

    pub async fn update_config_git_info(&self, path: &Path) {
        let git_info = (self.config_git_info_creator)(path);
        let mut config_git_info = self.configuration_git_info.write().await;
        *config_git_info = git_info.clone();
    }

    pub fn git_cli(&self) -> &(dyn GitCli + Send + Sync) {
        self.git_cli.as_ref()
    }

    pub async fn preview_store(&self) -> tokio::sync::MutexGuard<'_, HashMap<String, PathBuf>> {
        self.preview_store.lock().await
    }
}
