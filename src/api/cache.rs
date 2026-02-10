//! In-memory cache for issue status responses.

use crate::{
    QCComment,
    api::{
        ApiError,
        types::{ChecklistSummary, CommitStatusEnum, Issue, IssueCommit, QCStatus, QCStatusEnum},
    },
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Cache validation key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub issue_updated_at: DateTime<Utc>,
    pub branch: String,
    pub head_commit: String,
}

/// Cache entry variants.
#[derive(Debug, Clone)]
pub enum CacheEntry {
    /// Full status data - created when issue is requested for its status.
    Complete {
        issue: Issue,
        qc_status: QCStatus,
        commits: Vec<IssueCommit>,
        checklist_summary: ChecklistSummary,
        blocking_qc_numbers: Vec<u64>,
    },
    /// Minimal data - created when issue is fetched as a blocking QC.
    Partial {
        qc_status: QCStatus,
        file_name: String,
    },
}

/// In-memory cache for issue status responses.
#[derive(Debug)]
pub struct StatusCache {
    entries: HashMap<u64, (CacheKey, CacheEntry)>,
}

impl StatusCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get a cache entry if it exists and the key matches.
    pub fn get(&self, issue_number: u64, key: &CacheKey) -> Option<&CacheEntry> {
        self.entries.get(&issue_number).and_then(
            |(cached_key, entry)| {
                if cached_key == key { Some(entry) } else { None }
            },
        )
    }

    /// Insert or update a cache entry.
    pub fn insert(&mut self, issue_number: u64, key: CacheKey, entry: CacheEntry) {
        self.entries.insert(issue_number, (key, entry));
    }

    pub fn update_comment(&mut self, comment: &QCComment, key: CacheKey) -> Result<(), ApiError> {
        if let Some((cache_key, cache_entry)) = self.entries.get_mut(&comment.issue.number) {
            *cache_key = key;
            match cache_entry {
                CacheEntry::Complete {
                    issue,
                    qc_status,
                    commits,
                    ..
                } => {
                    let current_commit_str = comment.current_commit.to_string();
                    *issue = comment.issue.clone().into();

                    let is_latest_commit = if let Some(commit) =
                        commits.iter_mut().find(|c| c.hash == current_commit_str)
                    {
                        if !commit.statuses.contains(&CommitStatusEnum::Notification) {
                            commit.statuses.push(CommitStatusEnum::Notification);
                        }
                        // Check if commenting on the latest commit (first in list, newest first)
                        commits
                            .first()
                            .map(|c| c.hash == current_commit_str)
                            .unwrap_or(false)
                    } else {
                        // Commit not found - insert at front (newest first)
                        commits.insert(
                            0,
                            IssueCommit {
                                hash: current_commit_str.clone(),
                                message: "New commit".to_string(),
                                statuses: vec![CommitStatusEnum::Notification],
                                file_changed: true,
                            },
                        );
                        // Check if this is a new commit (not in list)
                        !commits.iter().any(|c| c.hash == current_commit_str)
                    };

                    // Update qc_status based on current state
                    if qc_status.status == QCStatusEnum::Approved {
                        // If approved and commenting on a newer commit, it's changes after approval
                        if is_latest_commit {
                            qc_status.status = QCStatusEnum::ChangesAfterApproval;
                            qc_status.status_detail =
                                "Approved; subsequent file changes".to_string();
                            qc_status.latest_commit = current_commit_str;
                        }
                        // If not, keep status the same
                    } else {
                        // For non-approved states, only update if commenting on latest
                        if is_latest_commit {
                            qc_status.status = QCStatusEnum::AwaitingReview;
                            qc_status.status_detail = "Awaiting review".to_string();
                            qc_status.latest_commit = current_commit_str;
                        }
                    }
                }
                CacheEntry::Partial { qc_status, .. } => {
                    // Approximation for partial entries
                    let current_commit_str = comment.current_commit.to_string();
                    if qc_status.status == QCStatusEnum::Approved {
                        if qc_status.approved_commit.as_ref() != Some(&current_commit_str) {
                            qc_status.status = QCStatusEnum::ChangesAfterApproval;
                            qc_status.status_detail =
                                "Approved; subsequent file changes".to_string();
                            qc_status.latest_commit = current_commit_str;
                        }
                    } else {
                        qc_status.status = QCStatusEnum::AwaitingReview;
                        qc_status.status_detail = "Awaiting review".to_string();
                        qc_status.latest_commit = current_commit_str;
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for StatusCache {
    fn default() -> Self {
        Self::new()
    }
}
