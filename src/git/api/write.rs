use std::future::Future;

use octocrab::models::Milestone;

use super::GitHubApiError;
use crate::QCIssue;
use crate::comment_system::CommentBody;
use crate::git::GitInfo;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitHubWriter {
    fn create_milestone(
        &self,
        milestone_name: &str,
        description: &Option<String>,
    ) -> impl Future<Output = Result<Milestone, GitHubApiError>> + Send;
    fn post_issue(
        &self,
        issue: &QCIssue,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send;

    // Unified comment posting system
    fn post_comment<T: CommentBody + 'static>(
        &self,
        comment: &T,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send;

    // Explicit issue state management
    fn close_issue(
        &self,
        issue_number: u64,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send;

    fn open_issue(
        &self,
        issue_number: u64,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send;

    fn create_label(
        &self,
        name: &str,
        color: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send;
}

impl GitHubWriter for GitInfo {
    fn create_milestone(
        &self,
        milestone_name: &str,
        description: &Option<String>,
    ) -> impl std::future::Future<Output = Result<Milestone, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let milestone_name = milestone_name.to_string();
        let description = description.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
            log::debug!(
                "Creating milestone '{}' for {}/{}",
                milestone_name,
                owner,
                repo
            );
            let milestone_request = serde_json::json!({
                "title": milestone_name,
                "state": "open",
                "description": description,
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
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let title = issue.title();
        let body = issue.body(self);
        let milestone_id = issue.milestone_id;
        let branch = issue.branch.clone();
        let assignees = issue.assignees.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
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

    fn post_comment<T: CommentBody>(
        &self,
        comment: &T,
    ) -> impl Future<Output = Result<String, GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = comment.issue().number;
        let body = comment.generate_body(self);
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;

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

    fn close_issue(
        &self,
        issue_number: u64,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;

            log::debug!("Closing issue #{} in {}/{}", issue_number, owner, repo);

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

            Ok(())
        }
    }

    fn open_issue(
        &self,
        issue_number: u64,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;

            log::debug!("Opening issue #{} in {}/{}", issue_number, owner, repo);

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
                "Successfully opened issue #{} in {}/{}",
                issue_number,
                owner,
                repo
            );

            Ok(())
        }
    }

    fn create_label(
        &self,
        name: &str,
        color: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let name = name.to_string();
        let color = color.to_string();
        let base_url = self.base_url.clone();
        let auth_token = self.auth_token.clone();

        async move {
            let octocrab = crate::git::auth::create_authenticated_client(&base_url, auth_token)
                .map_err(GitHubApiError::ClientCreation)?;
            log::debug!(
                "Creating label '{}' with color '{}' for {}/{}",
                name,
                color,
                owner,
                repo
            );
            octocrab
                .issues(&owner, &repo)
                .create_label(&name, &color, "")
                .await
                .map_err(GitHubApiError::APIError)?;

            log::debug!("Successfully created label '{}'", name);
            Ok(())
        }
    }
}
