use std::future::Future;

use octocrab::models::Milestone;

use super::GitHubApiError;
use crate::git::GitInfo;
use crate::{QCApprove, QCComment, QCIssue, QCUnapprove};

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
pub trait GitHubWriter {
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
    ) -> impl std::future::Future<Output = Result<Milestone, GitHubApiError>> + Send {
        let octocrab = self.create_client().map_err(GitHubApiError::ClientCreation);
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let milestone_name = milestone_name.to_string();

        async move {
            let octocrab = octocrab?;
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
        let octocrab = self.create_client().map_err(GitHubApiError::ClientCreation);
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let title = issue.title();
        let body = issue.body(self);
        let milestone_id = issue.milestone_id;
        let branch = issue.branch.clone();
        let assignees = issue.assignees.clone();

        async move {
            let octocrab = octocrab?;
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
        let octocrab = self.create_client().map_err(GitHubApiError::ClientCreation);
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = comment.issue.number;
        let body_result = comment.body(self);

        async move {
            let octocrab = octocrab?;
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
        let octocrab = self.create_client().map_err(GitHubApiError::ClientCreation);
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = approval.issue.number;
        let body = approval.body(self);

        async move {
            let octocrab = octocrab?;
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
        let octocrab = self.create_client().map_err(GitHubApiError::ClientCreation);
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let issue_number = unapproval.issue.number;
        let body = unapproval.body();

        async move {
            let octocrab = octocrab?;
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

    fn create_label(
        &self,
        name: &str,
        color: &str,
    ) -> impl Future<Output = Result<(), GitHubApiError>> + Send {
        let octocrab = self.create_client().map_err(GitHubApiError::ClientCreation);
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let name = name.to_string();
        let color = color.to_string();

        async move {
            let octocrab = octocrab?;
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
