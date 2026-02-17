use anyhow::{Context, Result};
use octocrab::models::{Milestone, issues::Issue};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::git::RepoUser;

use super::types::{BlockingRelationship, Fixtures};

/// Fixture loader with caching
pub struct FixtureLoader {
    base_path: PathBuf,
    issue_cache: HashMap<String, Issue>,
    milestone_cache: HashMap<String, Milestone>,
    user_cache: HashMap<String, Vec<RepoUser>>,
}

/// Loaded fixture data
pub struct LoadedFixtures {
    /// Issues keyed by issue number
    pub issues: HashMap<u64, Issue>,
    /// Milestones keyed by milestone number
    pub milestones: HashMap<i64, Milestone>,
    /// Repository users
    pub users: Vec<RepoUser>,
    /// Blocking relationships from YAML
    pub blocking: Vec<BlockingRelationship>,
}

impl FixtureLoader {
    /// Create a new fixture loader with the given base path
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            issue_cache: HashMap::new(),
            milestone_cache: HashMap::new(),
            user_cache: HashMap::new(),
        }
    }

    /// Load all fixtures referenced in the test case
    pub fn load_fixtures(&mut self, fixtures: &Fixtures) -> Result<LoadedFixtures> {
        let mut issues = HashMap::new();
        let mut milestones = HashMap::new();
        let mut users = Vec::new();

        // Load issues
        for filename in &fixtures.issues {
            let issue = self.load_issue(filename)?;
            issues.insert(issue.number, issue);
        }

        // Load milestones
        for filename in &fixtures.milestones {
            let milestone = self.load_milestone(filename)?;
            milestones.insert(milestone.number, milestone);
        }

        // Load users
        for filename in &fixtures.users {
            let user_list = self.load_users(filename)?;
            users.extend(user_list);
        }

        Ok(LoadedFixtures {
            issues,
            milestones,
            users,
            blocking: fixtures.blocking.clone(),
        })
    }

    /// Load an issue fixture from JSON file
    fn load_issue(&mut self, filename: &str) -> Result<Issue> {
        // Check cache first
        if let Some(issue) = self.issue_cache.get(filename) {
            return Ok(issue.clone());
        }

        // Load from file
        let path = self.base_path.join("issues").join(filename);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read issue fixture: {}", path.display()))?;
        let issue: Issue = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse issue fixture: {}", path.display()))?;

        // Cache and return
        self.issue_cache.insert(filename.to_string(), issue.clone());
        Ok(issue)
    }

    /// Load a milestone fixture from JSON file
    fn load_milestone(&mut self, filename: &str) -> Result<Milestone> {
        // Check cache first
        if let Some(milestone) = self.milestone_cache.get(filename) {
            return Ok(milestone.clone());
        }

        // Load from file
        let path = self.base_path.join("milestones").join(filename);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read milestone fixture: {}", path.display()))?;
        let milestone: Milestone = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse milestone fixture: {}", path.display()))?;

        // Cache and return
        self.milestone_cache
            .insert(filename.to_string(), milestone.clone());
        Ok(milestone)
    }

    /// Load users fixture from JSON file
    fn load_users(&mut self, filename: &str) -> Result<Vec<RepoUser>> {
        // Check cache first
        if let Some(users) = self.user_cache.get(filename) {
            return Ok(users.clone());
        }

        // Load from file
        let path = self.base_path.join("users").join(filename);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read users fixture: {}", path.display()))?;
        let users: Vec<RepoUser> = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse users fixture: {}", path.display()))?;

        // Cache and return
        self.user_cache.insert(filename.to_string(), users.clone());
        Ok(users)
    }
}
