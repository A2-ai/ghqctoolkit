use etcetera::BaseStrategy;
use octocrab::models::issues::Issue;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::git::{GitComment, GitHubApiError, GitHubReader, GitHubWriter, GitRepository, RepoUser};

/// Cache entry with optional TTL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry<T> {
    pub data: T,
    pub created_at: u64,
    pub ttl_seconds: Option<u64>,
}

/// Cached comments with the issue's last updated timestamp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedComments {
    pub comments: Vec<GitComment>,
    pub issue_updated_at: chrono::DateTime<chrono::Utc>,
}

/// Cached events with the issue's last updated timestamp
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEvents {
    pub events: Vec<serde_json::Value>,
    pub issue_updated_at: chrono::DateTime<chrono::Utc>,
}

impl<T> CacheEntry<T> {
    pub fn new(data: T, ttl: Option<Duration>) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            data,
            created_at,
            ttl_seconds: ttl.map(|d| d.as_secs()),
        }
    }

    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.ttl_seconds {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            now > self.created_at + ttl
        } else {
            false // No TTL means never expires
        }
    }
}

/// Simple disk-based cache for GitHub API responses
#[derive(Debug, Clone)]
pub struct DiskCache {
    pub(crate) root: PathBuf,
    owner: String,
    repo: String,
    ttl: Duration,
}

impl DiskCache {
    /// Create a new DiskCache instance from GitInfo
    pub fn from_git_info(
        git_info: &impl GitRepository,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(git_info.owner().to_string(), git_info.repo().to_string())
    }

    /// Create a new DiskCache instance using the system cache directory
    pub fn new(owner: String, repo: String) -> Result<Self, Box<dyn std::error::Error>> {
        let strategy = etcetera::choose_base_strategy()?;
        let root = strategy.cache_dir().join("ghqc");
        let ttl = default_ttl();

        Ok(Self {
            root,
            owner,
            repo,
            ttl,
        })
    }

    /// Generate a path for a specific cache file based on the directory path and key
    pub fn path(&self, path: &[&str], key: &str) -> PathBuf {
        let mut full_path = self.root.join(&self.owner).join(&self.repo);

        // Add directory parts
        for part in path {
            full_path = full_path.join(part);
        }

        // Add filename with .json extension
        full_path.join(format!("{}.json", key))
    }

    /// Read and deserialize cached data if valid (not expired)
    pub fn read<T>(&self, path: &[&str], key: &str) -> Option<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let file_path = self.path(path, key);
        if !file_path.exists() {
            return None;
        }

        let content = fs::read_to_string(&file_path).ok()?;
        let entry: CacheEntry<T> = serde_json::from_str(&content).ok()?;

        if entry.is_expired() {
            // Clean up expired cache
            let _ = fs::remove_file(&file_path);
            return None;
        }

        Some(entry.data)
    }

    /// Write and serialize data to cache with optional TTL
    pub fn write<T>(
        &self,
        path: &[&str],
        key: &str,
        data: &T,
        use_ttl: bool,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        T: Serialize,
    {
        let file_path = self.path(path, key);

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let ttl = if use_ttl { Some(self.ttl) } else { None };

        let entry = CacheEntry::new(data, ttl);
        let content = serde_json::to_string_pretty(&entry)?;
        fs::write(&file_path, content)?;

        Ok(())
    }

    /// Invalidate a specific cache entry by removing the file
    ///
    /// This is useful when we need to force a fresh fetch from the API,
    /// such as when we need HTML content for JWT URL extraction.
    pub fn invalidate(&self, path: &[&str], key: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file_path = self.path(path, key);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
            log::debug!("Invalidated cache entry: {}", file_path.display());
        } else {
            log::debug!(
                "Cache entry not found for invalidation: {}",
                file_path.display()
            );
        }
        Ok(())
    }
}

/// Get the default cache TTL from environment or use 1 hour default
fn default_ttl() -> Duration {
    let ttl_seconds = std::env::var("GHQC_CACHE_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600); // 1 hour default
    Duration::from_secs(ttl_seconds)
}

/// Get repository users with caching for efficiency
pub async fn get_repo_users(
    cache: Option<&DiskCache>,
    git_info: &impl GitHubReader,
) -> Result<Vec<RepoUser>, GitHubApiError> {
    // Try to get assignees from cache first
    let cached_assignees: Option<Vec<String>> = if let Some(cache) = cache {
        cache.read::<Vec<String>>(&["users"], "assignees")
    } else {
        None
    };

    let assignee_logins = if let Some(logins) = cached_assignees {
        log::debug!("Using cached assignees");
        logins
    } else {
        log::debug!("Assignees not found or expired in cache. Fetching...");
        let logins = git_info.get_assignees().await?;

        // Cache the assignee list with TTL
        if let Some(cache) = cache {
            if let Err(e) = cache.write(&["users"], "assignees", &logins, true) {
                log::warn!("Failed to cache assignees: {}", e);
            }
        }

        logins
    };

    // Parallelize user detail fetching with permanent cache
    let user_futures: Vec<_> = assignee_logins
        .into_iter()
        .map(|username| {
            async move {
                // Try to get user details from cache first (permanent cache)
                let cached_user = if let Some(cache) = cache {
                    cache.read::<RepoUser>(&["users", "details"], &username)
                } else {
                    None
                };

                if let Some(user) = cached_user {
                    log::trace!("Using cached user details for: {}", username);
                    return Ok(user);
                }

                log::debug!(
                    "User details for {} not found in cache. Fetching...",
                    username
                );
                let user = git_info.get_user_details(&username).await?;

                // Cache user details permanently (no TTL)
                if let Some(cache) = cache {
                    if let Err(e) = cache.write(&["users", "details"], &username, &user, false) {
                        log::warn!("Failed to cache user details for {}: {}", username, e);
                    }
                }

                Ok(user)
            }
        })
        .collect();

    // Execute all futures concurrently
    let results: Vec<Result<RepoUser, GitHubApiError>> =
        futures::future::join_all(user_futures).await;

    let users = results.into_iter().collect::<Result<Vec<_>, _>>()?;

    log::debug!(
        "Successfully fetched {} assignees with user details",
        users.len()
    );

    Ok(users)
}

/// Create required labels if they don't exist, with caching
pub async fn create_labels_if_needed(
    cache: Option<&DiskCache>,
    branch: Option<&str>,
    git_info: &(impl GitHubReader + GitHubWriter),
) -> Result<(), GitHubApiError> {
    // Try to get labels from cache first
    let cached_labels: Option<Vec<String>> = if let Some(cache) = cache {
        cache.read::<Vec<String>>(&["labels"], "names")
    } else {
        None
    };

    let label_names = if let Some(names) = cached_labels {
        log::debug!("Using cached label names");
        names
    } else {
        log::debug!("Label names not found or expired in cache. Fetching...");
        let names = git_info.get_labels().await?;

        // Cache the label names with TTL
        if let Some(cache) = cache {
            if let Err(e) = cache.write(&["labels"], "names", &names, true) {
                log::warn!("Failed to cache label names: {}", e);
            }
        }

        names
    };

    let original_count = label_names.len();
    let mut updated_labels = label_names;

    // Ensure "ghqc" label exists
    if !updated_labels.iter().any(|name| name == "ghqc") {
        log::debug!("ghqc label does not exist. Creating...");
        git_info.create_label("ghqc", "FFCB05").await?;
        updated_labels.push("ghqc".to_string());
    }

    // Ensure branch label exists
    if let Some(branch) = branch {
        if !updated_labels.iter().any(|name| name == branch) {
            log::debug!("Branch label ({}) does not exist. Creating...", branch);
            git_info.create_label(branch, "00274C").await?;
            updated_labels.push(branch.to_string());
        }
    }

    // Update cache with new labels if we created any
    if updated_labels.len() != original_count {
        if let Some(cache) = cache {
            if let Err(e) = cache.write(&["labels"], "names", &updated_labels, true) {
                log::warn!("Failed to update cached label names: {}", e);
            }
        }
    }

    Ok(())
}

/// Get issue comments with caching based on issue update timestamp
pub async fn get_issue_comments(
    issue: &Issue,
    cache: Option<&DiskCache>,
    git_info: &impl GitHubReader,
) -> Result<Vec<GitComment>, GitHubApiError> {
    // Create cache key from issue number
    let cache_key = format!("issue_{}", issue.number);

    // Try to get cached comments first
    let cached_comments: Option<CachedComments> = if let Some(cache) = cache {
        cache.read::<CachedComments>(&["issues", "comments"], &cache_key)
    } else {
        None
    };

    // Check if cached comments are still valid by comparing timestamps
    if let Some(cached) = cached_comments {
        if cached.issue_updated_at >= issue.updated_at {
            log::debug!(
                "Using cached comments for issue #{} (cache timestamp: {}, issue timestamp: {})",
                issue.number,
                cached.issue_updated_at,
                issue.updated_at
            );
            return Ok(cached.comments);
        } else {
            log::debug!(
                "Cached comments for issue #{} are stale (cache: {}, issue: {})",
                issue.number,
                cached.issue_updated_at,
                issue.updated_at
            );
        }
    }

    // Fetch fresh comments from API
    log::debug!("Fetching fresh comments for issue #{}", issue.number);
    let comments = git_info.get_issue_comments(issue).await?;

    // Cache the comments with the current issue timestamp (permanently)
    if let Some(cache) = cache {
        let cached_comments = CachedComments {
            comments: comments.clone(),
            issue_updated_at: issue.updated_at,
        };

        if let Err(e) = cache.write(&["issues", "comments"], &cache_key, &cached_comments, false) {
            log::warn!(
                "Failed to cache comments for issue #{}: {}",
                issue.number,
                e
            );
        }
    }

    Ok(comments)
}

/// Get issue events with caching based on issue update timestamp
pub async fn get_issue_events(
    issue: &Issue,
    cache: Option<&DiskCache>,
    git_info: &impl GitHubReader,
) -> Result<Vec<serde_json::Value>, GitHubApiError> {
    // Create cache key from issue number
    let cache_key = format!("issue_{}", issue.number);

    // Try to get cached events first
    let cached_events: Option<CachedEvents> = if let Some(cache) = cache {
        cache.read::<CachedEvents>(&["issues", "events"], &cache_key)
    } else {
        None
    };

    // Check if cached events are still valid by comparing timestamps
    if let Some(cached) = cached_events {
        if cached.issue_updated_at >= issue.updated_at {
            log::debug!(
                "Using cached events for issue #{} (cache timestamp: {}, issue timestamp: {})",
                issue.number,
                cached.issue_updated_at,
                issue.updated_at
            );
            return Ok(cached.events);
        } else {
            log::debug!(
                "Cached events for issue #{} are stale (cache: {}, issue: {})",
                issue.number,
                cached.issue_updated_at,
                issue.updated_at
            );
        }
    }

    // Fetch fresh events from API
    log::debug!("Fetching fresh events for issue #{}", issue.number);
    let events = git_info.get_issue_events(issue).await?;

    // Cache the events with the current issue timestamp (permanently)
    if let Some(cache) = cache {
        let cached_events = CachedEvents {
            events: events.clone(),
            issue_updated_at: issue.updated_at,
        };

        if let Err(e) = cache.write(&["issues", "events"], &cache_key, &cached_events, false) {
            log::warn!("Failed to cache events for issue #{}: {}", issue.number, e);
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_path_generation() {
        let cache = DiskCache {
            root: PathBuf::from("/tmp/cache"),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            ttl: Duration::from_secs(3600),
        };

        let path = cache.path(&[], "milestones");
        assert_eq!(path, PathBuf::from("/tmp/cache/owner/repo/milestones.json"));
    }

    #[test]
    fn test_path_with_complex_names() {
        let cache = DiskCache {
            root: PathBuf::from("/cache"),
            owner: "my-org".to_string(),
            repo: "my-repo_name".to_string(),
            ttl: Duration::from_secs(1800),
        };

        let path = cache.path(&["users"], "user_list");
        assert_eq!(
            path,
            PathBuf::from("/cache/my-org/my-repo_name/users/user_list.json")
        );
    }

    #[test]
    fn test_new_creates_cache_in_system_dir() {
        let cache = DiskCache::new("test-owner".to_string(), "test-repo".to_string()).unwrap();

        // Just verify it doesn't panic and creates reasonable paths
        assert!(cache.root.to_string_lossy().contains("ghqc"));

        let path = cache.path(&[], "test");
        assert!(path.to_string_lossy().contains("test-owner"));
        assert!(path.to_string_lossy().contains("test-repo"));
        assert!(path.to_string_lossy().ends_with("test.json"));
    }

    #[test]
    fn test_cache_read_write_with_ttl() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let cache = DiskCache {
            root: temp_dir.path().to_path_buf(),
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            ttl: Duration::from_secs(3600),
        };

        let test_data = vec!["user1".to_string(), "user2".to_string()];

        // Write data with TTL
        cache.write(&[], "test_users", &test_data, true).unwrap();

        // Read it back
        let cached_data: Option<Vec<String>> = cache.read(&[], "test_users");
        assert_eq!(cached_data, Some(test_data));

        // Test non-existent key
        let missing_data: Option<Vec<String>> = cache.read(&[], "missing");
        assert_eq!(missing_data, None);
    }

    #[test]
    fn test_cache_permanent_storage() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let cache = DiskCache {
            root: temp_dir.path().to_path_buf(),
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            ttl: Duration::from_secs(7200),
        };

        let user_data = ("test_user".to_string(), Some("Test User".to_string()));

        // Write data without TTL (permanent)
        cache.write(&[], "user_test", &user_data, false).unwrap();

        // Read it back
        let cached_user: Option<(String, Option<String>)> = cache.read(&[], "user_test");
        assert_eq!(cached_user, Some(user_data));
    }

    #[test]
    fn test_hierarchical_paths() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let cache = DiskCache {
            root: temp_dir.path().to_path_buf(),
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            ttl: Duration::from_secs(3600),
        };

        // Test nested path structure
        let assignees = vec!["user1".to_string(), "user2".to_string()];
        cache
            .write(&["users"], "assignees", &assignees, true)
            .unwrap();

        let user_details = serde_json::json!({"login": "user1", "name": "User One"});
        cache
            .write(&["users", "details"], "user1", &user_details, false)
            .unwrap();

        // Verify paths are created correctly
        let assignees_path = cache.path(&["users"], "assignees");
        assert!(assignees_path.ends_with("assignees.json"));
        assert!(assignees_path.parent().unwrap().ends_with("users"));

        let user_path = cache.path(&["users", "details"], "user1");
        assert!(user_path.ends_with("user1.json"));
        assert!(user_path.parent().unwrap().ends_with("details"));
        assert!(
            user_path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .ends_with("users")
        );

        // Verify reading works
        let cached_assignees: Option<Vec<String>> = cache.read(&["users"], "assignees");
        assert_eq!(cached_assignees, Some(assignees));

        let cached_user: Option<serde_json::Value> = cache.read(&["users", "details"], "user1");
        assert_eq!(cached_user, Some(user_details));
    }
}
