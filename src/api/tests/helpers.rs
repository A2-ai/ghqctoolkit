//! Test helpers for API route testing.

use crate::CommentBody;
use crate::git::{
    GitCommitAnalysis, GitCommitAnalysisError, GitFileOps, GitFileOpsError, GitHelpers,
    GitRepository, GitRepositoryError, GitStatusError, GitStatusOps,
};
use crate::{GitAuthor, GitCommit, GitHubApiError, GitHubReader, GitHubWriter, GitStatus};
use gix::ObjectId;
use octocrab::models::issues::Issue;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// Tracks write operation calls for test verification.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteCall {
    CreateMilestone {
        name: String,
        description: Option<String>,
    },
    PostIssue {
        title: String,
    },
    PostComment {
        comment_type: String,
    },
    CloseIssue {
        issue_number: u64,
    },
    OpenIssue {
        issue_number: u64,
    },
}

/// Mock implementation of all git traits for testing.
#[derive(Clone)]
pub struct MockGitInfo {
    // Repository metadata
    owner: String,
    repo: String,
    current_commit: String,
    current_branch: String,

    // Mock data storage
    issues: Arc<Mutex<HashMap<u64, Issue>>>,
    blocked_issues: Arc<Mutex<HashMap<u64, Vec<Issue>>>>,
    milestones: Arc<Mutex<Vec<octocrab::models::Milestone>>>,
    users: Arc<Mutex<Vec<crate::RepoUser>>>,

    // Status
    dirty_files: Arc<Mutex<Vec<PathBuf>>>,
    git_status: GitStatus,

    // Call tracking (for assertions)
    calls: Arc<Mutex<Vec<String>>>,
    write_calls: Arc<Mutex<Vec<WriteCall>>>,
}

impl MockGitInfo {
    /// Create a new mock with default values.
    pub fn builder() -> MockGitInfoBuilder {
        MockGitInfoBuilder::new()
    }

    /// Get all write calls for inspection.
    pub fn write_calls(&self) -> Vec<WriteCall> {
        self.write_calls.lock().unwrap().clone()
    }
}

/// Builder for MockGitInfo.
pub struct MockGitInfoBuilder {
    owner: String,
    repo: String,
    commit: String,
    branch: String,
    issues: HashMap<u64, Issue>,
    blocked_issues: HashMap<u64, Vec<Issue>>,
    milestones: Vec<octocrab::models::Milestone>,
    users: Vec<crate::RepoUser>,
    dirty_files: Vec<PathBuf>,
    git_status: GitStatus,
}

impl MockGitInfoBuilder {
    pub fn new() -> Self {
        Self {
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            commit: "abc123".to_string(),
            branch: "main".to_string(),
            issues: HashMap::new(),
            blocked_issues: HashMap::new(),
            milestones: Vec::new(),
            users: Vec::new(),
            dirty_files: Vec::new(),
            git_status: GitStatus::Clean,
        }
    }

    pub fn with_owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = owner.into();
        self
    }

    pub fn with_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = repo.into();
        self
    }

    pub fn with_commit(mut self, commit: impl Into<String>) -> Self {
        self.commit = commit.into();
        self
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    pub fn with_issue(mut self, number: u64, issue: Issue) -> Self {
        self.issues.insert(number, issue);
        self
    }

    pub fn with_blocked_issues(mut self, issue_number: u64, blocking: Vec<Issue>) -> Self {
        self.blocked_issues.insert(issue_number, blocking);
        self
    }

    pub fn with_milestone(mut self, milestone: octocrab::models::Milestone) -> Self {
        self.milestones.push(milestone);
        self
    }

    pub fn with_users(mut self, users: Vec<crate::RepoUser>) -> Self {
        self.users = users;
        self
    }

    pub fn with_dirty_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.dirty_files.push(file.into());
        self
    }

    pub fn with_status(mut self, status: GitStatus) -> Self {
        self.git_status = status;
        self
    }

    pub fn build(self) -> MockGitInfo {
        MockGitInfo {
            owner: self.owner,
            repo: self.repo,
            current_commit: self.commit,
            current_branch: self.branch,
            issues: Arc::new(Mutex::new(self.issues)),
            blocked_issues: Arc::new(Mutex::new(self.blocked_issues)),
            milestones: Arc::new(Mutex::new(self.milestones)),
            users: Arc::new(Mutex::new(self.users)),
            dirty_files: Arc::new(Mutex::new(self.dirty_files)),
            git_status: self.git_status,
            calls: Arc::new(Mutex::new(Vec::new())),
            write_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Default for MockGitInfoBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// Implement all required traits for MockGitInfo
// (GitProvider is auto-implemented via blanket impl)

impl GitRepository for MockGitInfo {
    fn commit(&self) -> Result<String, GitRepositoryError> {
        Ok(self.current_commit.clone())
    }

    fn branch(&self) -> Result<String, GitRepositoryError> {
        Ok(self.current_branch.clone())
    }

    fn owner(&self) -> &str {
        &self.owner
    }

    fn repo(&self) -> &str {
        &self.repo
    }

    fn path(&self) -> &Path {
        Path::new(".")
    }

    fn fetch(&self) -> Result<bool, GitRepositoryError> {
        Ok(false) // Mock: no changes fetched
    }
}

impl GitHelpers for MockGitInfo {
    fn file_content_url(&self, git_ref: &str, file: &Path) -> String {
        format!(
            "https://github.com/{}/{}/blob/{}/{}",
            self.owner,
            self.repo,
            git_ref,
            file.display()
        )
    }

    fn commit_comparison_url(&self, current: &ObjectId, previous: &ObjectId) -> String {
        format!(
            "https://github.com/{}/{}/compare/{}..{}",
            self.owner, self.repo, previous, current
        )
    }

    fn issue_url(&self, issue_number: u64) -> String {
        format!(
            "https://github.com/{}/{}/issues/{}",
            self.owner, self.repo, issue_number
        )
    }
}

impl GitStatusOps for MockGitInfo {
    fn status(&self) -> Result<GitStatus, GitStatusError> {
        Ok(self.git_status.clone())
    }

    fn dirty(&self) -> Result<Vec<PathBuf>, GitStatusError> {
        Ok(self.dirty_files.lock().unwrap().clone())
    }
}

impl GitFileOps for MockGitInfo {
    fn commits(&self, _branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
        // Return a commit that matches test fixtures and touches all common test files
        // This matches the initial commit from config_file_issue.json
        let commit_hash = ObjectId::from_str("456def789abc012345678901234567890123cdef")
            .unwrap_or_else(|_| ObjectId::empty_tree(gix::hash::Kind::Sha1));

        Ok(vec![GitCommit {
            commit: commit_hash,
            message: "Initial commit".to_string(),
            files: vec![
                PathBuf::from("src/test.rs"),
                PathBuf::from("src/config.rs"),
                PathBuf::from("src/main.rs"),
                PathBuf::from("src/lib.rs"),
            ],
        }])
    }

    fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
        // Return dummy authors for any file
        Ok(vec![GitAuthor {
            name: "Test Author".to_string(),
            email: "test@example.com".to_string(),
        }])
    }

    fn file_bytes_at_commit(
        &self,
        _file: &Path,
        _commit: &ObjectId,
    ) -> Result<Vec<u8>, GitFileOpsError> {
        Ok(vec![])
    }
}

impl GitCommitAnalysis for MockGitInfo {
    fn get_all_merge_commits(&self) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
        Ok(vec![])
    }

    fn get_commit_parents(
        &self,
        _commit: &ObjectId,
    ) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
        Ok(vec![])
    }

    fn is_ancestor(
        &self,
        _ancestor: &ObjectId,
        _descendant: &ObjectId,
    ) -> Result<bool, GitCommitAnalysisError> {
        Ok(false)
    }

    fn get_branches_containing_commit(
        &self,
        _commit: &ObjectId,
    ) -> Result<Vec<String>, GitCommitAnalysisError> {
        Ok(vec![])
    }
}

impl GitHubReader for MockGitInfo {
    async fn get_milestones(&self) -> Result<Vec<octocrab::models::Milestone>, GitHubApiError> {
        Ok(self.milestones.lock().unwrap().clone())
    }

    async fn get_issues(&self, milestone: Option<u64>) -> Result<Vec<Issue>, GitHubApiError> {
        let issues = self.issues.lock().unwrap();
        if let Some(milestone_number) = milestone {
            Ok(issues
                .values()
                .filter(|issue| {
                    issue
                        .milestone
                        .as_ref()
                        .map(|m| m.number == milestone_number as i64)
                        .unwrap_or(false)
                })
                .cloned()
                .collect())
        } else {
            Ok(issues.values().cloned().collect())
        }
    }

    async fn get_issue(&self, issue_number: u64) -> Result<Issue, GitHubApiError> {
        eprintln!("MockGitInfo::get_issue({}) called", issue_number);
        let issues = self.issues.clone();
        let calls = self.calls.clone();

        calls
            .lock()
            .unwrap()
            .push(format!("get_issue({})", issue_number));

        let result = issues
            .lock()
            .unwrap()
            .get(&issue_number)
            .cloned()
            .ok_or_else(|| GitHubApiError::NoApi);

        eprintln!(
            "MockGitInfo::get_issue({}) result: {:?}",
            issue_number,
            result.is_ok()
        );
        result
    }

    async fn get_assignees(&self) -> Result<Vec<String>, GitHubApiError> {
        Ok(self
            .users
            .lock()
            .unwrap()
            .iter()
            .map(|u| u.login.clone())
            .collect())
    }

    async fn get_user_details(&self, username: &str) -> Result<crate::RepoUser, GitHubApiError> {
        // Look up the user from stored fixtures
        // On miss, return Ok with name: None (matches production behavior)
        Ok(self
            .users
            .lock()
            .unwrap()
            .iter()
            .find(|u| u.login == username)
            .cloned()
            .unwrap_or_else(|| crate::RepoUser {
                login: username.to_string(),
                name: None,
            }))
    }

    async fn get_labels(&self) -> Result<Vec<String>, GitHubApiError> {
        Ok(vec![])
    }

    async fn get_issue_comments(
        &self,
        _issue: &Issue,
    ) -> Result<Vec<crate::GitComment>, GitHubApiError> {
        Ok(vec![])
    }

    async fn get_issue_events(
        &self,
        _issue: &Issue,
    ) -> Result<Vec<serde_json::Value>, GitHubApiError> {
        Ok(vec![])
    }

    async fn get_blocked_issues(&self, issue_number: u64) -> Result<Vec<Issue>, GitHubApiError> {
        let blocked = self.blocked_issues.clone();
        let calls = self.calls.clone();

        calls
            .lock()
            .unwrap()
            .push(format!("get_blocked_issues({})", issue_number));

        Ok(blocked
            .lock()
            .unwrap()
            .get(&issue_number)
            .cloned()
            .unwrap_or_default())
    }
}

impl GitHubWriter for MockGitInfo {
    async fn create_milestone(
        &self,
        name: &str,
        desc: &Option<String>,
    ) -> Result<octocrab::models::Milestone, GitHubApiError> {
        use chrono::Utc;

        // Track the call with parameters
        self.write_calls
            .lock()
            .unwrap()
            .push(WriteCall::CreateMilestone {
                name: name.to_string(),
                description: desc.clone(),
            });

        // Generate milestone number based on existing milestones count + 1
        let milestone_number = {
            let milestones = self.milestones.lock().unwrap();
            milestones.len() as i64 + 1
        };

        // Construct milestone using JSON deserialization (octocrab types are #[non_exhaustive])
        let milestone_json = serde_json::json!({
            "url": format!("https://api.github.com/repos/{}/{}/milestones/{}",
                          self.owner, self.repo, milestone_number),
            "html_url": format!("https://github.com/{}/{}/milestone/{}",
                               self.owner, self.repo, milestone_number),
            "labels_url": format!("https://api.github.com/repos/{}/{}/milestones/{}/labels",
                                 self.owner, self.repo, milestone_number),
            "id": milestone_number * 1000,
            "node_id": format!("MDk6TWlsZXN0b25l{}", milestone_number),
            "number": milestone_number,
            "title": name,
            "description": desc,
            "creator": null,
            "open_issues": 0,
            "closed_issues": 0,
            "state": "open",
            "created_at": Utc::now().to_rfc3339(),
            "updated_at": Utc::now().to_rfc3339(),
            "due_on": null,
            "closed_at": null,
        });

        serde_json::from_value(milestone_json).map_err(|e| {
            eprintln!("Failed to create mock milestone: {}", e);
            GitHubApiError::NoApi
        })
    }

    async fn post_issue(&self, issue: &crate::QCIssue) -> Result<Issue, GitHubApiError> {
        // Track the call
        self.write_calls.lock().unwrap().push(WriteCall::PostIssue {
            title: issue.title().to_string(),
        });

        // Generate issue number based on existing issues count + 1
        let issue_number = {
            let issues = self.issues.lock().unwrap();
            issues.len() as u64 + 1
        };

        // Create the issue using shared test helper
        let created_issue = crate::test_utils::create_test_issue(
            &self.owner,
            &self.repo,
            issue_number,
            &issue.title(),
            &issue.body(self),
            None,   // milestone
            "open", // state
        );

        // Store the issue so get_issue can find it later
        self.issues
            .lock()
            .unwrap()
            .insert(issue_number, created_issue.clone());

        Ok(created_issue)
    }

    async fn post_comment<T: CommentBody + Sync + 'static>(
        &self,
        comment: &T,
    ) -> Result<String, GitHubApiError> {
        // Track the call
        self.write_calls
            .lock()
            .unwrap()
            .push(WriteCall::PostComment {
                comment_type: std::any::type_name::<T>().to_string(),
            });

        // Return a mock comment URL
        let issue_number = comment.issue().number;
        Ok(format!(
            "https://github.com/{}/{}/issues/{}#issuecomment-123",
            self.owner, self.repo, issue_number
        ))
    }

    async fn close_issue(&self, issue_number: u64) -> Result<(), GitHubApiError> {
        self.write_calls
            .lock()
            .unwrap()
            .push(WriteCall::CloseIssue { issue_number });
        Ok(())
    }

    async fn open_issue(&self, issue_number: u64) -> Result<(), GitHubApiError> {
        self.write_calls
            .lock()
            .unwrap()
            .push(WriteCall::OpenIssue { issue_number });
        Ok(())
    }

    async fn create_label(&self, _name: &str, _color: &str) -> Result<(), GitHubApiError> {
        Err(GitHubApiError::NoApi)
    }

    async fn block_issue(&self, _blocked: u64, _blocking: u64) -> Result<(), GitHubApiError> {
        Err(GitHubApiError::NoApi)
    }
}

/// Helper to load test issue fixtures from JSON
pub fn load_test_issue(name: &str) -> Issue {
    let json = std::fs::read_to_string(format!("src/tests/github_api/issues/{}.json", name))
        .expect("Failed to load test fixture");

    serde_json::from_str(&json).expect("Failed to parse test fixture")
}

/// Helper to load test milestone fixtures from JSON
pub fn load_test_milestone(name: &str) -> octocrab::models::Milestone {
    let json = std::fs::read_to_string(format!("src/tests/github_api/milestones/{}.json", name))
        .expect("Failed to load milestone fixture");

    serde_json::from_str(&json).expect("Failed to parse milestone fixture")
}
