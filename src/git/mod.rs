use gix::Repository;
use octocrab::models::issues::Issue;
use octocrab::{Octocrab, models::Milestone};
use std::path::Path;

pub(crate) mod api;
pub(crate) mod auth;
pub(crate) mod helpers;
pub(crate) mod local;

pub use api::{GitHubApi, GitHubApiError, RepoUser};
pub use auth::create_authenticated_client;
pub use helpers::{GitHelpers, GitInfoError, parse_github_url};
pub use local::{LocalGitError, LocalGitInfo};

use crate::git::local::GitAuthor;
use crate::issues::QCIssue;

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
        log::debug!("Initializing GitInfo from path: {:?}", path);

        let repository = gix::open(path).map_err(GitInfoError::RepoOpen)?;
        log::debug!("Opened git repository");

        let remote = repository
            .find_default_remote(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemote)?
            .map_err(GitInfoError::RemoteNotFound)?;

        let remote_url = remote
            .url(gix::remote::Direction::Fetch)
            .ok_or(GitInfoError::NoRemoteUrl)?
            .to_string();
        log::debug!("Found remote URL: {}", remote_url);

        let (owner, repo, base_url) = parse_github_url(&remote_url)?;
        log::debug!(
            "Parsed GitHub info - Owner: {}, Repo: {}, Base URL: {}",
            owner,
            repo,
            base_url
        );

        let octocrab = create_authenticated_client(&base_url)?;

        log::debug!("Successfully initialized GitInfo for {}/{}", owner, repo);

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
    fn commit(&self) -> Result<String, LocalGitError> {
        let head = self.repository.head().map_err(LocalGitError::HeadError)?;
        let commit_id = head.id().ok_or(LocalGitError::DetachedHead)?;
        let commit_str = commit_id.to_string();
        log::debug!("Current commit: {}", commit_str);
        Ok(commit_str)
    }

    fn branch(&self) -> Result<String, LocalGitError> {
        let head = self.repository.head().map_err(LocalGitError::HeadError)?;

        // Try to get the branch name directly
        if let Some(branch_name) = head.referent_name() {
            let name_str = branch_name.as_bstr().to_string();
            log::debug!("Raw branch reference: {}", name_str);

            // Remove "refs/heads/" prefix if present
            let final_branch = if let Some(branch) = name_str.strip_prefix("refs/heads/") {
                branch.to_string()
            } else {
                name_str
            };
            log::debug!("Current branch: {}", final_branch);
            Ok(final_branch)
        } else {
            // Fallback: we might be in detached HEAD state
            log::debug!("No branch reference found, likely detached HEAD");
            Ok("HEAD".to_string())
        }
    }

    fn file_commits(&self, file: &Path) -> Result<Vec<gix::ObjectId>, LocalGitError> {
        log::debug!("Finding commits that touched file: {:?}", file);
        let mut commits = Vec::new();

        let head_id = self
            .repository
            .head_id()
            .map_err(LocalGitError::HeadIdError)?;

        let revwalk = self.repository.rev_walk([head_id]);

        for commit_info in revwalk.all().map_err(LocalGitError::RevWalkError)? {
            let commit_info = commit_info.map_err(LocalGitError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(LocalGitError::FindObjectError)?
                .try_into_commit()
                .map_err(LocalGitError::CommitError)?;

            // Check if this commit touched the file
            if let Ok(tree) = commit.tree() {
                let mut buffer = Vec::new();
                if tree.lookup_entry_by_path(file, &mut buffer).is_ok() {
                    commits.push(commit_id);
                }
            }
        }

        log::debug!(
            "Found {} commits that touched file: {:?}",
            commits.len(),
            file
        );
        Ok(commits)
    }

    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, LocalGitError> {
        let commits = self.file_commits(file)?;

        let mut res: Vec<GitAuthor> = Vec::new();

        for commit_id in commits {
            let commit = self
                .repository
                .find_object(commit_id)
                .map_err(LocalGitError::FindObjectError)?
                .try_into_commit()
                .map_err(LocalGitError::CommitError)?;

            let signature = commit.author().map_err(LocalGitError::SignatureError)?;
            if !res.iter().any(|author| author.email == signature.email) {
                res.push(GitAuthor {
                    name: signature.name.to_string(),
                    email: signature.email.to_string(),
                });
            }
        }

        if res.is_empty() {
            log::warn!("No authors found for file: {:?}", file);
            Err(LocalGitError::AuthorNotFound(file.to_path_buf()))
        } else {
            log::debug!("Found {} unique authors for file: {:?}", res.len(), file);
            Ok(res)
        }
    }
}

impl GitHubApi for GitInfo {
    fn get_milestones(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Milestone>, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        async move {
            log::debug!("Fetching milestones for {}/{}", owner, repo);
            let milestones: Vec<Milestone> = octocrab
                .get(
                    format!("/repos/{}/{}/milestones", &owner, &repo),
                    None::<&()>,
                )
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!("Successfully fetched {} milestones", milestones.len());
            Ok(milestones)
        }
    }

    fn get_milestone_issues(
        &self,
        milestone_id: u64,
    ) -> impl std::future::Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        async move {
            log::debug!(
                "Fetching issues for milestone {} in {}/{}",
                milestone_id,
                owner,
                repo
            );
            let issues = octocrab
                .issues(&owner, &repo)
                .list()
                .milestone(milestone_id)
                .send()
                .await
                .map(|issues| issues.items)
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully fetched {} issues for milestone {}",
                issues.len(),
                milestone_id
            );

            Ok(issues)
        }
    }

    fn create_milestone(
        &self,
        milestone_name: &str,
    ) -> impl std::future::Future<Output = Result<Milestone, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let milestone_name = milestone_name.to_string();

        async move {
            log::debug!(
                "Creating milestone '{}' for {}/{}",
                milestone_name,
                owner,
                repo
            );
            let milestone_request = serde_json::json!({
                "title": milestone_name,
                "state": "open"
            });

            let milestone: Milestone = octocrab
                .post(
                    format!("/repos/{}/{}/milestones", &owner, &repo),
                    Some(&milestone_request),
                )
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully created milestone '{}' with ID: {}",
                milestone_name,
                milestone.number
            );

            Ok(milestone)
        }
    }

    fn post_issue(
        &self,
        issue: &QCIssue,
    ) -> impl std::future::Future<Output = Result<(), GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let title = issue.title();
        let body = issue.body(self);
        let milestone_id = issue.milestone_id;
        let branch = issue.branch.clone();
        let assignees = issue.assignees.clone();

        async move {
            log::debug!("Posting issue '{}' to {}/{}", title, owner, repo);

            let handler = octocrab.issues(owner.clone(), repo.clone());
            let builder = handler
                .create(title.clone())
                .body(body)
                .milestone(Some(milestone_id))
                .labels(vec!["QC".to_string(), branch])
                .assignees(assignees);

            let issue = builder.send().await.map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully posted issue #'{}' to {}/{}",
                issue.number,
                owner,
                repo
            );

            Ok(())
        }
    }

    fn get_users(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<RepoUser>, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        async move {
            log::debug!("Fetching assignees for repository {}/{}", owner, repo);

            let mut all_assignees = Vec::new();
            let mut page = 1;
            let per_page = 100; // Maximum per page

            loop {
                let url = format!(
                    "/repos/{}/{}/assignees?per_page={}&page={}",
                    &owner, &repo, per_page, page
                );

                let assignees: Vec<serde_json::Value> = octocrab
                    .get(url, None::<&()>)
                    .await
                    .map_err(GitHubApiError::APIError)?;

                if assignees.is_empty() {
                    break; // No more pages
                }

                log::debug!("Fetched {} assignees on page {}", assignees.len(), page);
                all_assignees.extend(assignees);
                page += 1;

                // Safety check to prevent infinite loops
                if page > 100 {
                    log::warn!("Reached maximum page limit (100) for assignees");
                    break;
                }
            }

            log::debug!("Total assignees fetched: {}", all_assignees.len());

            // Parallelize user detail fetching
            let user_futures: Vec<_> = all_assignees
                .into_iter()
                .filter_map(|assignee| {
                    assignee.get("login")
                        .and_then(|v| v.as_str())
                        .map(|username| {
                            let octocrab = octocrab.clone();
                            let username = username.to_string();
                            async move {
                                log::debug!("Fetching user details for: {}", username);

                                let mut res = RepoUser {
                                    login: username.to_string(),
                                    name: None,
                                };

                                let user: Result<serde_json::Value, _> = octocrab
                                    .get(format!("/users/{}", username), None::<&()>)
                                    .await;

                                match user {
                                    Ok(user_data) => {
                                        res.name = user_data
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());
                                    }
                                    Err(e) => {
                                        log::warn!("Failed to fetch user details for {}: {}, using login only", username, e);
                                    }
                                }

                                res
                            }
                        })
                })
                .collect();

            // Execute all futures concurrently
            let users: Vec<RepoUser> = futures::future::join_all(user_futures).await;

            log::debug!(
                "Successfully fetched {} assignees with user details",
                users.len()
            );
            Ok(users)
        }
    }

    fn create_labels_if_needed(
        &self,
        branch: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let branch = branch.to_string();

        async move {
            log::debug!("Fetching labels...");
            let labels = octocrab
                .issues(&owner, &repo)
                .list_labels_for_repo()
                .send()
                .await
                .map_err(GitHubApiError::APIError)?;
            log::debug!("Found {} labels", labels.items.len());

            if !labels.items.iter().any(|l| l.name == "QC") {
                log::debug!("QC label does not exist. Creating...");
                octocrab
                    .issues(&owner, &repo)
                    .create_label("QC", "FFCB05", "QC Issue")
                    .await
                    .map_err(GitHubApiError::APIError)?;
            }

            if !labels.items.iter().any(|l| l.name == branch) {
                log::debug!("Branch label ({branch}) does not exist. Creating...");
                octocrab
                    .issues(&owner, &repo)
                    .create_label(&branch, "00274C", "QC Branch")
                    .await
                    .map_err(GitHubApiError::APIError)?;
            }

            Ok(())
        }
    }
}

impl GitHelpers for GitInfo {
    fn file_content_url(&self, git_ref: &str, file: &Path) -> String {
        let file = file.to_string_lossy().replace(" ", "%20");
        format!(
            "{}/{}/{}/blob/{}/{file}",
            self.base_url, self.owner, self.repo, &git_ref
        )
    }
}
