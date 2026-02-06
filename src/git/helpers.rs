use std::path::Path;

use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

#[derive(Debug, Clone, PartialEq)]
pub struct GitRemote {
    pub owner: String,
    pub repo: String,
    pub url: String,
}

impl GitRemote {
    pub fn from_url(url: &str) -> Option<Self> {
        let url = url.strip_suffix(".git").unwrap_or(url);

        if let Some(captures) = url.strip_prefix("https://") {
            let parts: Vec<&str> = captures.split('/').collect();
            if parts.len() >= 3 {
                let host = parts[0];
                let owner = parts[1];
                let repo = parts[2];
                return Some(GitRemote {
                    owner: owner.to_string(),
                    repo: repo.to_string(),
                    url: format!("https://{}", host),
                });
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
                    return Some(GitRemote {
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        url: format!("https://{}", host),
                    });
                }
            }
        }

        None
    }
}

#[cfg_attr(test, automock)]
pub trait GitHelpers {
    fn file_content_url(&self, git_ref: &str, file: &Path) -> String;
    fn commit_comparison_url(
        &self,
        current_commit: &ObjectId,
        previous_commit: &ObjectId,
    ) -> String;
    fn issue_url(&self, issue_number: u64) -> String;
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

    fn commit_comparison_url(
        &self,
        current_commit: &ObjectId,
        previous_commit: &ObjectId,
    ) -> String {
        format!(
            "{}/{}/{}/compare/{}..{}",
            self.base_url, self.owner, self.repo, previous_commit, current_commit,
        )
    }

    fn issue_url(&self, issue_number: u64) -> String {
        format!(
            "{}/{}/{}/issues/{issue_number}",
            self.base_url, self.owner, self.repo
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
                    let result = GitRemote::from_url(input).unwrap();
                    assert_eq!(
                        result,
                        GitRemote {
                            owner: exp_owner.to_string(),
                            repo: exp_repo.to_string(),
                            url: exp_base_url.to_string(),
                        },
                        "Failed for input: {}",
                        input
                    );
                }
                Err(_) => {
                    assert!(
                        GitRemote::from_url(input).is_none(),
                        "Expected None for input: {}",
                        input
                    );
                }
            }
        }
    }
}
