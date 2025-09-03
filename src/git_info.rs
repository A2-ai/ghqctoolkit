use std::fmt;
use std::path::{Path, PathBuf};

use gix::Repository;
use octocrab::models::issues::Issue;
use octocrab::{Octocrab, models::Milestone};

#[cfg(test)]
use mockall::automock;

use crate::issues::QCIssue;

#[derive(Debug, Clone)]
pub struct GitAuthor {
    pub(crate) name: String,
    pub(crate) email: String,
}

impl fmt::Display for GitAuthor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.email)
    }
}

#[cfg_attr(test, automock)]
pub trait LocalGitInfo {
    fn commit(&self) -> Result<String, GitInfoError>;
    fn branch(&self) -> Result<String, GitInfoError>;
    fn file_commits(&self, file: &Path) -> Result<Vec<gix::ObjectId>, GitInfoError>;
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitInfoError>;
}

#[cfg_attr(test, automock)]
pub trait GitHubApi {
    async fn get_milestones(&self) -> Result<Vec<Milestone>, GitInfoError>;
    async fn get_milestone_issues(&self, milestone_num: u64) -> Result<Vec<Issue>, GitInfoError>;
    async fn post_issue(&self, issue: &QCIssue) -> Result<(), GitInfoError>;
}

#[cfg_attr(test, automock)]
pub trait GitHelpers {
    fn file_content_url(&self, commit: &str, file: &Path) -> String;
}

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) base_url: String,
    pub(crate) repository: Repository,
    pub(crate) octocrab: Octocrab,
}

impl GitInfo {
    pub fn from_path(path: &Path) -> Result<Self, GitInfoError> {
        let repository = gix::open(path).map_err(GitInfoError::RepoOpen)?;

        let remote = repository
            .find_default_remote(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemote)?
            .map_err(GitInfoError::RemoteNotFound)?;

        let remote_url = remote
            .url(gix::remote::Direction::Fetch)
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

        Ok(Self {
            owner,
            repo,
            base_url,
            repository,
            octocrab,
        })
    }
}

impl LocalGitInfo for GitInfo {
    fn commit(&self) -> Result<String, GitInfoError> {
        let head = self.repository.head().map_err(GitInfoError::HeadError)?;
        let commit_id = head.id().ok_or(GitInfoError::DetachedHead)?;
        Ok(commit_id.to_string())
    }

    fn branch(&self) -> Result<String, GitInfoError> {
        let head = self.repository.head().map_err(GitInfoError::HeadError)?;

        let name = head.name();
        let branch_name = name.shorten().to_string();
        // Remove "refs/heads/" prefix if present
        if let Some(branch) = branch_name.strip_prefix("refs/heads/") {
            Ok(branch.to_string())
        } else {
            Ok(branch_name)
        }
    }

    fn file_commits(&self, file: &Path) -> Result<Vec<gix::ObjectId>, GitInfoError> {
        let mut commits = Vec::new();

        let head_id = self
            .repository
            .head_id()
            .map_err(GitInfoError::HeadIdError)?;

        let revwalk = self.repository.rev_walk([head_id]);

        for commit_info in revwalk.all().map_err(GitInfoError::RevWalkError)? {
            let commit_info = commit_info.map_err(GitInfoError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(GitInfoError::FindObjectError)?
                .try_into_commit()
                .map_err(GitInfoError::CommitError)?;

            // Check if this commit touched the file
            if let Ok(tree) = commit.tree() {
                let mut buffer = Vec::new();
                if tree.lookup_entry_by_path(file, &mut buffer).is_ok() {
                    commits.push(commit_id);
                }
            }
        }

        Ok(commits)
    }

    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitInfoError> {
        let commits = self.file_commits(file)?;

        let mut res = Vec::new();

        for commit_id in commits {
            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(GitInfoError::FindObjectError)?
                .try_into_commit()
                .map_err(GitInfoError::CommitError)?;

            let signature = commit.author().map_err(GitInfoError::SignatureError)?;
            res.push(GitAuthor {
                name: signature.name.to_string(),
                email: signature.email.to_string(),
            });
        }

        if res.is_empty() {
            Err(GitInfoError::AuthorNotFound(file.to_path_buf()))
        } else {
            Ok(res)
        }
    }
}

impl GitHubApi for GitInfo {
    async fn get_milestones(&self) -> Result<Vec<Milestone>, GitInfoError> {
        self.octocrab
            .get(
                format!("/repos/{}/{}/milestones", &self.owner, &self.repo),
                None::<&()>,
            )
            .await
            .map_err(GitInfoError::APIError)
    }

    async fn get_milestone_issues(&self, milestone_num: u64) -> Result<Vec<Issue>, GitInfoError> {
        self.octocrab
            .issues(&self.owner, &self.repo)
            .list()
            .milestone(milestone_num)
            .send()
            .await
            .map(|issues| issues.items)
            .map_err(GitInfoError::APIError)
    }

    async fn post_issue(&self, issue: &QCIssue) -> Result<(), GitInfoError> {
        let handler = self
            .octocrab
            .issues(self.owner.to_string(), self.repo.to_string());
        let builder = handler
            .create(issue.title())
            .body(issue.body(self)?)
            .milestone(Some(issue.milestone_id))
            .labels(vec!["ghqc".to_string(), issue.branch.to_string()])
            .assignees(issue.assignees.clone());

        builder.send().await.map_err(GitInfoError::APIError)?;

        Ok(())
    }
}

impl GitHelpers for GitInfo {
    fn file_content_url(&self, commit: &str, file: &Path) -> String {
        let file = file.to_string_lossy().replace(" ", "%20");
        format!(
            "{}/{}/{}/blob/{}/{file}",
            self.base_url,
            self.owner,
            self.repo,
            &commit[..7]
        )
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
    #[error("GitHub API not loaded")]
    NoApi,
    #[error("GitHub API URL access failed due to: {0}")]
    APIError(octocrab::Error),
    #[error("Failed to get HEAD reference: {0}")]
    HeadError(gix::reference::find::existing::Error),
    #[error("Repository is in detached HEAD state")]
    DetachedHead,
    #[error("Failed to get HEAD ID: {0}")]
    HeadIdError(gix::reference::head_id::Error),
    #[error("Failed to walk revision history: {0}")]
    RevWalkError(gix::revision::walk::Error),
    #[error("Failed to traverse commits: {0}")]
    TraverseError(gix::traverse::commit::simple::Error),
    #[error("Failed to find git object: {0}")]
    FindObjectError(gix::object::find::existing::Error),
    #[error("Failed to parse commit: {0}")]
    CommitError(gix::object::try_into::Error),
    #[error("Failed to get signature: {0}")]
    SignatureError(gix::objs::decode::Error),
    #[error("Author not found for file: {0:?}")]
    AuthorNotFound(PathBuf),
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
