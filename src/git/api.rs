use std::future::Future;

use octocrab::models::Milestone;
use octocrab::models::issues::Issue;

use crate::issues::QCIssue;
#[cfg(test)]
use mockall::automock;

#[derive(Debug, Clone)]
pub struct RepoUser {
    pub login: String,
    pub name: Option<String>,
}

impl std::fmt::Display for RepoUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{} ({})", self.login, name),
            None => write!(f, "{}", self.login),
        }
    }
}

#[cfg_attr(test, automock)]
pub trait GitHubApi {
    fn get_milestones(&self)
    -> impl Future<Output = Result<Vec<Milestone>, GitHubApiError>> + Send;
    fn get_milestone_issues(
        &self,
        milestone_id: u64,
    ) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send;
    fn create_milestone(
        &self,
        milestone_name: &str,
    ) -> impl Future<Output = Result<Milestone, GitHubApiError>> + Send;
    fn post_issue(
        &self,
        issue: &QCIssue,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send;
    fn get_users(&self) -> impl Future<Output = Result<Vec<RepoUser>, GitHubApiError>> + Send;
}

#[derive(thiserror::Error, Debug)]
pub enum GitHubApiError {
    #[error("GitHub API not loaded")]
    NoApi,
    #[error("GitHub API URL access failed due to: {0}")]
    APIError(octocrab::Error),
}
