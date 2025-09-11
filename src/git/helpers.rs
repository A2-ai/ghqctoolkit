use std::path::Path;

use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitHelpers {
    fn file_content_url(&self, git_ref: &str, file: &Path) -> String;
    fn commit_comparison_url(&self, current_commit: &ObjectId, previous_commit: &ObjectId) -> String;
}

pub fn parse_github_url(url: &str) -> Result<(String, String, String), GitInfoError> {
    let url = url.strip_suffix(".git").unwrap_or(url);

    if let Some(captures) = url.strip_prefix("https://") {
        let parts: Vec<&str> = captures.split('/').collect();
        if parts.len() >= 3 {
            let host = parts[0];
            let owner = parts[1];
            let repo = parts[2];
            return Ok((
                owner.to_string(),
                repo.to_string(),
                format!("https://{}", host),
            ));
        }
    }

    if let Some(captures) = url.strip_prefix("git@") {
        let host_and_path: Vec<&str> = captures.split(':').collect();
        if host_and_path.len() == 2 {
            let host = host_and_path[0];
            let path_parts: Vec<&str> = host_and_path[1].split('/').collect();
            if path_parts.len() >= 2 {
                let owner = path_parts[0];
                let repo = path_parts[1];
                return Ok((
                    owner.to_string(),
                    repo.to_string(),
                    format!("https://{}", host),
                ));
            }
        }
    }

    Err(GitInfoError::InvalidGitHubUrl(url.to_string()))
}

#[derive(thiserror::Error, Debug)]
pub enum GitInfoError {
    #[error("Failed to open git repository")]
    RepoOpen(gix::open::Error),
    #[error("Failed to find remote")]
    RemoteNotFound(gix::remote::find::existing::Error),
    #[error("No remote configured")]
    NoRemote,
    #[error("No remote URL configured")]
    NoRemoteUrl,
    #[error("Invalid GitHub URL: {0}")]
    InvalidGitHubUrl(String),
    #[error("Failed to build API: {0}")]
    ApiBuildError(#[from] octocrab::Error),
    #[error("Authentication error: {0}")]
    AuthError(#[from] super::auth::AuthError),
}

use crate::git::GitInfo;

impl GitHelpers for GitInfo {
    fn file_content_url(&self, git_ref: &str, file: &Path) -> String {
        let file = file.to_string_lossy().replace(" ", "%20");
        format!(
            "{}/{}/{}/blob/{}/{file}",
            self.base_url, self.owner, self.repo, &git_ref
        )
    }

    fn commit_comparison_url(&self, current_commit: &ObjectId, previous_commit: &ObjectId) -> String {
        format!(
            "{}/{}/{}/compare/{}..{}",
            self.base_url, self.owner, self.repo, previous_commit, current_commit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url_matrix() {
        let test_cases = [
            // GitHub.com HTTPS
            (
                "https://github.com/owner/repo",
                Ok(("owner", "repo", "https://github.com")),
            ),
            (
                "https://github.com/owner/repo.git",
                Ok(("owner", "repo", "https://github.com")),
            ),
            (
                "https://github.com/owner/repo/extra/path",
                Ok(("owner", "repo", "https://github.com")),
            ),
            // GitHub.com SSH
            (
                "git@github.com:owner/repo",
                Ok(("owner", "repo", "https://github.com")),
            ),
            (
                "git@github.com:owner/repo.git",
                Ok(("owner", "repo", "https://github.com")),
            ),
            (
                "git@github.com:owner/repo/subpath",
                Ok(("owner", "repo", "https://github.com")),
            ),
            // GitHub Enterprise HTTPS
            (
                "https://github.enterprise.com/owner/repo",
                Ok(("owner", "repo", "https://github.enterprise.com")),
            ),
            (
                "https://github.enterprise.com/owner/repo.git",
                Ok(("owner", "repo", "https://github.enterprise.com")),
            ),
            (
                "https://ghe.company.internal/owner/repo",
                Ok(("owner", "repo", "https://ghe.company.internal")),
            ),
            // GitHub Enterprise SSH
            (
                "git@github.enterprise.com:owner/repo",
                Ok(("owner", "repo", "https://github.enterprise.com")),
            ),
            (
                "git@github.enterprise.com:owner/repo.git",
                Ok(("owner", "repo", "https://github.enterprise.com")),
            ),
            (
                "git@ghe.company.internal:owner/repo",
                Ok(("owner", "repo", "https://ghe.company.internal")),
            ),
            // Invalid cases
            ("https://github.com/owner", Err(())),
            ("git@github.com:owner", Err(())),
            ("not-a-git-url", Err(())),
            ("https://example.com", Err(())),
            ("", Err(())),
        ];

        for (input, expected) in test_cases.iter() {
            match expected {
                Ok((exp_owner, exp_repo, exp_base_url)) => {
                    let result = parse_github_url(input).unwrap();
                    assert_eq!(
                        result,
                        (
                            exp_owner.to_string(),
                            exp_repo.to_string(),
                            exp_base_url.to_string()
                        ),
                        "Failed for input: {}",
                        input
                    );
                }
                Err(_) => {
                    assert!(
                        parse_github_url(input).is_err(),
                        "Expected error for input: {}",
                        input
                    );
                }
            }
        }
    }
}
