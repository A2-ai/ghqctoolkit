use std::future::Future;

use octocrab::models::Milestone;
use octocrab::models::issues::Issue;

use crate::GitInfo;
use crate::issues::QCIssue;
#[cfg(test)]
use mockall::automock;

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

#[cfg_attr(test, automock)]
pub trait GitHubApi {
    fn get_milestones(&self)
    -> impl Future<Output = Result<Vec<Milestone>, GitHubApiError>> + Send;
    fn get_milestone_issues(
        &self,
        milestone: &Milestone,
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
    fn create_labels_if_needed(
        &self,
        branch: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send;
}

// TODO: implement caching for milestones and issues. Frequent updates and comments must be considered about each,
// so not implementing until comment functionality is included
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
        milestone: &Milestone,
    ) -> impl std::future::Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let milestone_id = milestone.number as u64;

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
        let cache = self.cache.clone();

        async move {
            // Try to get assignees from cache first (using default TTL from DiskCache)
            let cached_assignees: Option<Vec<String>> = if let Some(ref cache) = cache {
                cache.read::<Vec<String>>(&["users"], "assignees")
            } else {
                None
            };

            let assignee_logins = if let Some(logins) = cached_assignees {
                log::trace!("Using cached assignees for {}/{}", owner, repo);
                logins
            } else {
                log::debug!(
                    "Assignees not found or expired in cache for repository {}/{}. Fetching...",
                    owner,
                    repo
                );

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

                // Extract just the login names
                let logins: Vec<String> = all_assignees
                    .into_iter()
                    .filter_map(|assignee| {
                        assignee
                            .get("login")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect();

                // Cache the assignee list with TTL (temporary cache)
                if let Some(ref cache) = cache {
                    if let Err(e) = cache.write(&["users"], "assignees", &logins, true) {
                        log::warn!("Failed to cache assignees: {}", e);
                    }
                }

                logins
            };

            // TODO: Add cache invalidation mechanism for assignee list when repository access changes

            // Parallelize user detail fetching with permanent cache
            let user_futures: Vec<_> = assignee_logins
                .into_iter()
                .map(|username| {
                    let octocrab = octocrab.clone();
                    let cache = cache.clone();
                    async move {
                        // Try to get user details from cache first (permanent cache)
                        let cached_user: Option<RepoUser> = if let Some(ref cache) = cache {
                            cache.read::<RepoUser>(&["users", "details"], &username)
                        } else {
                            None
                        };

                        if let Some(user) = cached_user {
                            log::trace!("Using cached user details for: {}", username);
                            return user;
                        }

                        log::debug!("User details for {username} not found in cache. Fetching...");

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
                                log::warn!(
                                    "Failed to fetch user details for {}: {}, using login only",
                                    username,
                                    e
                                );
                            }
                        }

                        // Cache user details permanently (no TTL)
                        if let Some(ref cache) = cache {
                            if let Err(e) =
                                cache.write(&["users", "details"], &username, &res, false)
                            {
                                log::warn!("Failed to cache user details for {}: {}", username, e);
                            }
                        }

                        res
                    }
                })
                .collect();

            // TODO: Add cache invalidation mechanism for individual user details when user updates their profile

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
        let cache = self.cache.clone();

        async move {
            // Try to get labels from cache first
            let cached_labels: Option<Vec<String>> = if let Some(ref cache) = cache {
                cache.read::<Vec<String>>(&["labels"], "names")
            } else {
                None
            };

            let label_names = if let Some(names) = cached_labels {
                log::trace!("Using cached label names for {}/{}", owner, repo);
                names
            } else {
                log::debug!(
                    "Label names not found or expired in cache for repository {}/{}. Fetching...",
                    owner,
                    repo
                );
                let labels = octocrab
                    .issues(&owner, &repo)
                    .list_labels_for_repo()
                    .send()
                    .await
                    .map_err(GitHubApiError::APIError)?;
                log::debug!("Found {} labels", labels.items.len());

                let names: Vec<String> = labels.items.into_iter().map(|l| l.name).collect();

                // Cache the label names with TTL
                if let Some(ref cache) = cache {
                    if let Err(e) = cache.write(&["labels"], "names", &names, true) {
                        log::warn!("Failed to cache label names: {}", e);
                    }
                }

                names
            };

            let original_count = label_names.len();
            let mut updated_labels = label_names;

            if !updated_labels.iter().any(|name| name == "ghqc") {
                log::debug!("ghqc label does not exist. Creating...");
                octocrab
                    .issues(&owner, &repo)
                    .create_label("ghqc", "FFCB05", "ghqc Issue")
                    .await
                    .map_err(GitHubApiError::APIError)?;

                // Add the new label to our cache
                updated_labels.push("ghqc".to_string());
            }

            if !updated_labels.iter().any(|name| name == &branch) {
                log::debug!("Branch label ({branch}) does not exist. Creating...");
                octocrab
                    .issues(&owner, &repo)
                    .create_label(&branch, "00274C", "QC Branch")
                    .await
                    .map_err(GitHubApiError::APIError)?;

                // Add the new label to our cache
                updated_labels.push(branch.clone());
            }

            // Update cache with new labels if we created any
            if updated_labels.len() != original_count {
                if let Some(ref cache) = cache {
                    if let Err(e) = cache.write(&["labels"], "names", &updated_labels, true) {
                        log::warn!("Failed to update cached label names: {}", e);
                    }
                }
            }

            Ok(())
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum GitHubApiError {
    #[error("GitHub API not loaded")]
    NoApi,
    #[error("GitHub API URL access failed due to: {0}")]
    APIError(octocrab::Error),
}
