use std::path::Path;

use octocrab::models::issues::Issue;
use octocrab::{models::Milestone, Octocrab};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitInfoTrait {
    async fn load_milestones(&mut self) -> Result<(), GitInfoError>;
    async fn get_milestone_issues(&self, milestone_num: u64) -> Result<Vec<Issue>, GitInfoError>;
}

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub owner: String,
    pub repo: String,
    pub octocrab: Octocrab,
    milestones: Vec<Milestone>,
}

impl GitInfo {
    fn from_path(path: &Path) -> Result<Self, GitInfoError> {
        let repository = gix::open(path).map_err(GitInfoError::RepoOpen)?;
        
        let remote = repository.find_default_remote(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemote)?
            .map_err(GitInfoError::RemoteNotFound)?;

        let remote_url = remote.url(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemoteUrl)?
            .to_string();

        let (owner, repo, base_url) = parse_github_url(&remote_url)?;

        let octocrab = if base_url == "https://github.com" {
            Octocrab::builder().build()
        } else {
            Octocrab::builder()
                .base_uri(&format!("{}/api/v3", base_url))
                .map_err(GitInfoError::APIError)?
                .build()
        }
        .map_err(GitInfoError::APIError)?;

        Ok(GitInfo {
            owner,
            repo,
            octocrab,
            milestones: Vec::new(),
        })
    }

}

impl GitInfoTrait for GitInfo {
    async fn load_milestones(&mut self) -> Result<(), GitInfoError> {
        self.milestones = self.octocrab
            .get(format!("/repos/{}/{}/milestones", &self.owner, &self.repo), None::<&()>)
            .await
            .map_err(GitInfoError::APIError)?;

        Ok(())
    }

    async fn get_milestone_issues(&self, milestone_num: u64) -> Result<Vec<Issue>, GitInfoError> {
        self
            .octocrab
            .issues(&self.owner, &self.repo)
            .list()
            .milestone(milestone_num)
            .send()
            .await
            .map(|issues| issues.items)
            .map_err(GitInfoError::APIError)
    }
}

fn parse_github_url(url: &str) -> Result<(String, String, String), GitInfoError> {
    let url = url.strip_suffix(".git").unwrap_or(url);
    
    if let Some(captures) = url.strip_prefix("https://") {
        let parts: Vec<&str> = captures.split('/').collect();
        if parts.len() >= 3 {
            let host = parts[0];
            let owner = parts[1];
            let repo = parts[2];
            return Ok((owner.to_string(), repo.to_string(), format!("https://{}", host)));
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
                return Ok((owner.to_string(), repo.to_string(), format!("https://{}", host)));
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
    #[error("GitHub API URL access failed due to: {0}")]
    APIError(octocrab::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url_matrix() {
        let test_cases = [
            // GitHub.com HTTPS
            ("https://github.com/owner/repo", Ok(("owner", "repo", "https://github.com"))),
            ("https://github.com/owner/repo.git", Ok(("owner", "repo", "https://github.com"))),
            ("https://github.com/owner/repo/extra/path", Ok(("owner", "repo", "https://github.com"))),
            
            // GitHub.com SSH
            ("git@github.com:owner/repo", Ok(("owner", "repo", "https://github.com"))),
            ("git@github.com:owner/repo.git", Ok(("owner", "repo", "https://github.com"))),
            ("git@github.com:owner/repo/subpath", Ok(("owner", "repo", "https://github.com"))),
            
            // GitHub Enterprise HTTPS
            ("https://github.enterprise.com/owner/repo", Ok(("owner", "repo", "https://github.enterprise.com"))),
            ("https://github.enterprise.com/owner/repo.git", Ok(("owner", "repo", "https://github.enterprise.com"))),
            ("https://ghe.company.internal/owner/repo", Ok(("owner", "repo", "https://ghe.company.internal"))),
            
            // GitHub Enterprise SSH
            ("git@github.enterprise.com:owner/repo", Ok(("owner", "repo", "https://github.enterprise.com"))),
            ("git@github.enterprise.com:owner/repo.git", Ok(("owner", "repo", "https://github.enterprise.com"))),
            ("git@ghe.company.internal:owner/repo", Ok(("owner", "repo", "https://ghe.company.internal"))),
            
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
                        (exp_owner.to_string(), exp_repo.to_string(), exp_base_url.to_string()),
                        "Failed for input: {}", input
                    );
                }
                Err(_) => {
                    assert!(
                        parse_github_url(input).is_err(),
                        "Expected error for input: {}", input
                    );
                }
            }
        }
    }
}