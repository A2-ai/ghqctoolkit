mod read;
mod write;

pub use read::{GitComment, GitHubReader};
pub use write::GitHubWriter;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(thiserror::Error, Debug)]
pub enum GitHubApiError {
    #[error("GitHub API not loaded")]
    NoApi,
    #[error("GitHub API URL access failed due to: {0}")]
    APIError(octocrab::Error),
    #[error("Failed to generate comment body: {0}")]
    CommentGenerationError(#[from] crate::git::GitFileOpsError),
    #[error("Failed to create GitHub client: {0}")]
    ClientCreation(#[from] crate::git::AuthError),
}
