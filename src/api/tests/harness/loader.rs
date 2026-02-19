use anyhow::{Context, Result};
use octocrab::models::{Milestone, issues::Issue};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::api::tests::harness::types::UserSource;
use crate::git::RepoUser;
use crate::test_utils::{create_test_issue, create_test_milestone};

use super::types::{BlockingRelationship, Fixtures, IssueSource, MilestoneSource};

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
    pub fn load_fixtures(
        &mut self,
        fixtures: &Fixtures,
        git_state: &super::types::GitState,
    ) -> Result<LoadedFixtures> {
        let mut issues = HashMap::new();
        let mut milestones = HashMap::new();
        let mut users = Vec::new();

        // Load or create issues
        for issue_source in &fixtures.issues {
            let issue = match issue_source {
                IssueSource::Fixture { file } => self.load_issue(file)?,
                IssueSource::Mock {
                    number,
                    title,
                    body,
                    state,
                    milestone,
                } => create_test_issue(
                    &git_state.owner,
                    &git_state.repo,
                    *number,
                    title,
                    body,
                    *milestone,
                    state,
                ),
            };
            issues.insert(issue.number, issue);
        }

        // Load or create milestones
        for milestone_source in &fixtures.milestones {
            let milestone = match milestone_source {
                MilestoneSource::Fixture { file } => self.load_milestone(file)?,
                MilestoneSource::Mock {
                    number,
                    title,
                    description,
                    state,
                } => create_test_milestone(
                    &git_state.owner,
                    &git_state.repo,
                    *number,
                    title,
                    description.as_deref(),
                    state,
                ),
            };
            milestones.insert(milestone.number, milestone);
        }

        // Load users
        for user_source in &fixtures.users {
            match user_source {
                UserSource::Fixture { file } => users.extend(self.load_users(file)?),
                UserSource::Mock { login, name } => users.push(RepoUser {
                    login: login.clone(),
                    name: name.clone(),
                }),
            };
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
