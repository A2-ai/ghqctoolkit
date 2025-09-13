use std::future::Future;

use octocrab::models::Milestone;
use octocrab::models::issues::Issue;

use crate::{GitInfo, QCApprove, QCComment, QCIssue, QCUnapprove};
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
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send;
    fn post_comment(
        &self,
        comment: &QCComment,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send;
    fn post_approval(
        &self,
        approval: &QCApprove,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send;
    fn post_unapproval(
        &self,
        unapproval: &QCUnapprove,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send;
    fn get_assignees(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send;
    fn get_user_details(&self, username: &str) -> impl Future<Output = Result<RepoUser, GitHubApiError>> + Send;
    fn get_labels(&self) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send;
    fn create_label(
        &self,
        name: &str,
        color: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send;
    fn get_issue_comments(
        &self,
        issue: &Issue,
    ) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send;
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
    ) -> impl std::future::Future<Output = Result<String, GitHubApiError>> + Send {
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
                .labels(vec!["ghqc".to_string(), branch])
                .assignees(assignees);

            let issue = builder.send().await.map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully posted issue #{} to {}/{}",
                issue.number,
                owner,
                repo
            );

            Ok(issue.html_url.to_string())
        }
    }

    fn post_comment(
        &self,
        comment: &QCComment,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = comment.issue.number;
        let body_result = comment.body(self);

        async move {
            let body = body_result?;

            log::debug!(
                "Posting comment to issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            let comment = octocrab
                .issues(&owner, &repo)
                .create_comment(issue_number, body)
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully posted comment {} to issue #{} in {}/{}",
                comment.id,
                issue_number,
                owner,
                repo
            );

            Ok(comment.html_url.to_string())
        }
    }

    fn post_approval(
        &self,
        approval: &QCApprove,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = approval.issue.number;
        let body = approval.body(self);

        async move {
            log::debug!(
                "Posting approval comment and closing issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            // Post the comment first
            let comment = octocrab
                .issues(&owner, &repo)
                .create_comment(issue_number, body)
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully posted approval comment {} to issue #{} in {}/{}",
                comment.id,
                issue_number,
                owner,
                repo
            );

            // Then close the issue
            let update_request = serde_json::json!({
                "state": "closed"
            });

            let _: serde_json::Value = octocrab
                .patch(
                    format!("/repos/{}/{}/issues/{}", &owner, &repo, issue_number),
                    Some(&update_request),
                )
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully closed issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            Ok(comment.html_url.to_string())
        }
    }

    fn post_unapproval(
        &self,
        unapproval: &QCUnapprove,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = unapproval.issue.number;
        let body = unapproval.body();

        async move {
            log::debug!(
                "Posting unapproval comment and reopening issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            // Post the comment first
            let comment = octocrab
                .issues(&owner, &repo)
                .create_comment(issue_number, body)
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully posted unapproval comment {} to issue #{} in {}/{}",
                comment.id,
                issue_number,
                owner,
                repo
            );

            // Then reopen the issue
            let update_request = serde_json::json!({
                "state": "open"
            });

            let _: serde_json::Value = octocrab
                .patch(
                    format!("/repos/{}/{}/issues/{}", &owner, &repo, issue_number),
                    Some(&update_request),
                )
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!(
                "Successfully reopened issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            Ok(comment.html_url.to_string())
        }
    }

    fn get_assignees(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
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
        let octocrab = self.octocrab.clone();
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
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        async move {
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

    fn create_label(
        &self,
        name: &str,
        color: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let name = name.to_string();
        let color = color.to_string();

        async move {
            log::debug!("Creating label '{}' with color '{}' for {}/{}", name, color, owner, repo);
            octocrab
                .issues(&owner, &repo)
                .create_label(&name, &color, "")
                .await
                .map_err(GitHubApiError::APIError)?;
            
            log::debug!("Successfully created label '{}'", name);
            Ok(())
        }
    }

    fn get_issue_comments(
        &self,
        issue: &Issue,
    ) -> impl Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
        let octocrab = self.octocrab.clone();
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = issue.number;

        async move {
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

                let comments: Vec<serde_json::Value> = octocrab
                    .get(url, None::<&()>)
                    .await
                    .map_err(GitHubApiError::APIError)?;

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

            // Extract comment bodies with error handling
            let mut comment_bodies = Vec::new();
            let mut error_count = 0;
            let total_comments = all_comments.len();

            for (idx, comment) in all_comments.into_iter().enumerate() {
                let is_last_comment = total_comments > 0 && idx == total_comments - 1;

                match comment.get("body").and_then(|b| b.as_str()) {
                    Some(body) => comment_bodies.push(body.to_string()),
                    None => {
                        error_count += 1;
                        let comment_id = comment
                            .get("id")
                            .and_then(|id| id.as_u64())
                            .unwrap_or(0);

                        if is_last_comment {
                            // Last comment failed - this is critical
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
                            // Non-last comment failed - warn but continue
                            log::warn!(
                                "Failed to extract body from comment {} for issue #{}: missing body field",
                                comment_id,
                                issue_number
                            );
                        }
                    }
                }
            }

            // Only error if ALL comments failed to parse (and we're not already handling last comment failure)
            if !comment_bodies.is_empty() || total_comments == 0 {
                if error_count > 0 {
                    log::info!(
                        "Successfully extracted {} out of {} comment bodies for issue #{}",
                        comment_bodies.len(),
                        total_comments,
                        issue_number
                    );
                }

                Ok(comment_bodies)
            } else {
                Err(GitHubApiError::APIError(octocrab::Error::Other {
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Failed to extract all comment bodies",
                    )),
                    backtrace: std::backtrace::Backtrace::capture(),
                }))
            }
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
    CommentGenerationError(#[from] crate::comment::CommentError),
}
