use gix::ObjectId;

use crate::{
    GitCommitAnalysis, GitFileOps, GitStatus,
    issue::{IssueError, IssueThread},
};

#[derive(Debug, Clone)]
pub enum QCStatus {
    Approved,
    ChangesAfterApproval(ObjectId),
    ApprovalRequired,
    AwaitingApproval,
    InProgress,
    ChangesToComment(ObjectId),
}

impl QCStatus {
    pub async fn determine_status(
        issue_thread: &IssueThread,
        git_status: &GitStatus,
        git_info: &(impl GitFileOps + GitCommitAnalysis),
    ) -> Result<Self, QCStatusError> {
        let status = if let Some(approved) = &issue_thread.approved_commit {
            if matches!(
                git_status,
                GitStatus::Ahead(_) | GitStatus::Diverged { .. } | GitStatus::Dirty(_)
            ) {
                // local changes takes precedence over checking if there are changes after approval
                Self::Approved
            } else {
                //
                let file_commits = issue_thread.commits(git_info).await?;
                file_commits
                    .first()
                    .and_then(|(latest_commit, _)| {
                        if latest_commit != approved {
                            Some(Self::ChangesAfterApproval(*latest_commit))
                        } else {
                            None
                        }
                    })
                    .unwrap_or(Self::Approved)
            }
        } else {
            // if not approved and closed
            if !issue_thread.open {
                Self::ApprovalRequired
            } else if matches!(git_status, GitStatus::Behind(_) | GitStatus::Clean) {
                let file_commits = issue_thread.commits(git_info).await?;
                file_commits
                    .first()
                    .map(|(latest_commit, _)| {
                        if latest_commit == issue_thread.latest_commit() {
                            Self::AwaitingApproval
                        } else {
                            Self::ChangesToComment(*latest_commit)
                        }
                    })
                    .unwrap_or(Self::InProgress)
            } else {
                Self::InProgress
            }
        };

        Ok(status)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QCStatusError {
    #[error("Failed to determine commits for issue due to: {0}")]
    IssueError(#[from] IssueError),
}
