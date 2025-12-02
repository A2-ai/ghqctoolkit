use octocrab::models::Milestone;
use octocrab::models::issues::Issue;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::future::Future;

use super::{GitHubApiError, RepoUser};
use crate::git::GitInfo;

/// Git comment data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitComment {
    pub body: String,
    pub author_login: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing)]
    pub(crate) html: Option<String>,
}

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitHubReader {
    fn get_milestones(&self)
    -> impl Future<Output = Result<Vec<Milestone>, GitHubApiError>> + Send;
    fn get_milestone_issues(
        &self,
        milestone: &Milestone,
    ) -> impl Future<Output = Result<Vec<Issue>, GitHubApiError>> + Send;
    fn get_assignees(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send;
    fn get_user_details(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<RepoUser, GitHubApiError>> + Send;
    fn get_labels(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send;
    fn get_issue_comments(
        &self,
        issue: &Issue,
    ) -> impl Future<Output = Result<Vec<GitComment>, GitHubApiError>> + Send;
    fn get_issue_events(
        &self,
        issue: &Issue,
    ) -> impl Future<Output = Result<Vec<serde_json::Value>, GitHubApiError>> + Send;
}

impl GitHubReader for GitInfo {
    fn get_milestones(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Milestone>, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
            log::debug!("Fetching milestones for {}/{}", owner, repo);
            let milestones: Vec<Milestone> = octocrab
                .get(
                    format!("/repos/{}/{}/milestones?state=all", &owner, &repo),
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
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let milestone_id = milestone.number as u64;
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
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
                .state(octocrab::params::State::All)
                .labels(&[String::from("ghqc")])
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

    fn get_assignees(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
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

            Ok(logins)
        }
    }

    fn get_user_details(
        &self,
        username: &str,
    ) -> impl std::future::Future<Output = Result<RepoUser, GitHubApiError>> + Send {
        let username = username.to_string();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
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
                    log::warn!(
                        "Failed to fetch user details for {}: {}, using login only",
                        username,
                        e
                    );
                }
            }

            Ok(res)
        }
    }

    fn get_labels(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
            log::debug!("Fetching labels for repository {}/{}", owner, repo);
            let labels = octocrab
                .issues(&owner, &repo)
                .list_labels_for_repo()
                .send()
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!("Found {} labels", labels.items.len());
            let names: Vec<String> = labels.items.into_iter().map(|l| l.name).collect();
            Ok(names)
        }
    }

    fn get_issue_comments(
        &self,
        issue: &Issue,
    ) -> impl Future<Output = Result<Vec<GitComment>, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = issue.number;
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
            log::debug!(
                "Fetching comments for issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            let mut all_comments = Vec::new();
            let mut page = 1;
            let per_page = 100; // Maximum per page

            loop {
                let url = format!(
                    "/repos/{}/{}/issues/{}/comments?per_page={}&page={}",
                    &owner, &repo, issue_number, per_page, page
                );

                // let parameters = [
                //     // ("Accept", "application/vnd.github.v3.raw"),
                //     ("Accept", "application/vnd.github.full+json"),
                // ]
                // .into_iter()
                // .collect::<HashMap<_, _>>();

                let headers = [(
                    ACCEPT,
                    HeaderValue::from_static("application/vnd.github.full+json"),
                )]
                .into_iter()
                .collect::<HeaderMap<_>>();

                let comments: Vec<serde_json::Value> = octocrab
                    .get_with_headers(url, None::<&()>, Some(headers))
                    .await
                    .map_err(GitHubApiError::APIError)?;

                // let comments: Vec<serde_json::Value> = octocrab
                //     .get(url, Some(&parameters))
                //     .await
                //     .map_err(GitHubApiError::APIError)?;

                if comments.is_empty() {
                    break; // No more pages
                }

                log::debug!("Fetched {} comments on page {}", comments.len(), page);
                all_comments.extend(comments);
                page += 1;

                // Safety check to prevent infinite loops
                if page > 100 {
                    log::warn!("Reached maximum page limit (100) for comments");
                    break;
                }
            }

            log::debug!(
                "Total comments fetched for issue #{}: {}",
                issue_number,
                all_comments.len()
            );

            // Extract comment data with error handling
            let mut git_comments = Vec::new();
            let mut error_count = 0;
            let total_comments = all_comments.len();

            for (idx, comment) in all_comments.into_iter().enumerate() {
                let is_last_comment = total_comments > 0 && idx == total_comments - 1;
                let comment_id = comment.get("id").and_then(|id| id.as_u64()).unwrap_or(0);

                // Extract body
                let body = match comment.get("body").and_then(|b| b.as_str()) {
                    Some(body) => body.to_string(),
                    None => {
                        error_count += 1;
                        if is_last_comment {
                            log::error!(
                                "Failed to extract body from last comment {} for issue #{}",
                                comment_id,
                                issue_number
                            );
                            return Err(GitHubApiError::APIError(octocrab::Error::Other {
                                source: Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "Last comment missing body",
                                )),
                                backtrace: std::backtrace::Backtrace::capture(),
                            }));
                        } else {
                            log::warn!(
                                "Failed to extract body from comment {} for issue #{}: missing body field",
                                comment_id,
                                issue_number
                            );
                            continue;
                        }
                    }
                };

                // Extract author login
                let author_login = comment
                    .get("user")
                    .and_then(|u| u.get("login"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Extract created_at timestamp
                let created_at = comment
                    .get("created_at")
                    .and_then(|t| t.as_str())
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|| chrono::Utc::now());

                // Extract HTML body (with JWT URLs) - only available from fresh API calls
                let html = comment.get("body_html").and_then(|h| h.as_str()).map(|h| {
                    log::debug!("Comment HTML available: {} chars", h.len());
                    h.to_string()
                });

                git_comments.push(GitComment {
                    body,
                    author_login,
                    created_at,
                    html,
                });
            }

            if error_count > 0 {
                log::info!(
                    "Successfully extracted {} out of {} comments for issue #{}",
                    git_comments.len(),
                    total_comments,
                    issue_number
                );
            }

            Ok(git_comments)
        }
    }

    fn get_issue_events(
        &self,
        issue: &Issue,
    ) -> impl Future<Output = Result<Vec<serde_json::Value>, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = issue.number;
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
            log::debug!(
                "Fetching events for issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            let mut all_events = Vec::new();
            let mut page = 1;
            let per_page = 100; // Maximum per page

            loop {
                let url = format!(
                    "/repos/{}/{}/issues/{}/events?per_page={}&page={}",
                    &owner, &repo, issue_number, per_page, page
                );

                let events: Vec<serde_json::Value> = octocrab
                    .get(url, None::<&()>)
                    .await
                    .map_err(GitHubApiError::APIError)?;

                if events.is_empty() {
                    break; // No more pages
                }

                log::debug!("Fetched {} events on page {}", events.len(), page);
                all_events.extend(events);
                page += 1;

                // Safety check to prevent infinite loops
                if page > 100 {
                    log::warn!("Reached maximum page limit (100) for events");
                    break;
                }
            }

            log::debug!(
                "Total events fetched for issue #{}: {}",
                issue_number,
                all_events.len()
            );

            Ok(all_events)
        }
    }
}
