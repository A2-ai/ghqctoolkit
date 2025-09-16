use etcetera::BaseStrategy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::git::GitRepository;

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
    pub comments: Vec<String>,
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
    root: PathBuf,
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
}

/// Get the default cache TTL from environment or use 1 hour default
fn default_ttl() -> Duration {
    let ttl_seconds = std::env::var("GHQC_CACHE_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600); // 1 hour default
    Duration::from_secs(ttl_seconds)
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
