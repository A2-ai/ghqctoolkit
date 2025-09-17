use gix::ObjectId;

use crate::issue::{IssueError, IssueThread};

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
    pub fn determine_status(
        issue_thread: &IssueThread,
        file_commits: &[ObjectId],
    ) -> Result<Self, QCStatusError> {
        let status = if let Some(approved) = &issue_thread.approved_commit {
            file_commits
                .first()
                .and_then(|latest_commit| {
                    if latest_commit != approved {
                        Some(Self::ChangesAfterApproval(*latest_commit))
                    } else {
                        None
                    }
                })
                .unwrap_or(Self::Approved)
        } else {
            // if not approved and closed
            if !issue_thread.open {
                Self::ApprovalRequired
            } else {
                file_commits
                    .first()
                    .map(|latest_commit| {
                        if latest_commit == issue_thread.latest_commit() {
                            Self::AwaitingApproval
                        } else {
                            Self::ChangesToComment(*latest_commit)
                        }
                    })
                    .unwrap_or(Self::InProgress)
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
