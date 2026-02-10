//! In-memory cache for issue status responses.

use crate::{
    analyze_issue_checklists,
    api::types::{ChecklistSummary, CommitStatusEnum, Issue, IssueCommit, QCStatus, QCStatusEnum},
    parse_branch_from_body,
};
use chrono::{DateTime, Utc};
use octocrab::models::issues::Issue as octoIssue;
use std::collections::HashMap;

/// Cache validation key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub issue_updated_at: DateTime<Utc>,
    pub branch: String,
    pub head_commit: String,
}

impl CacheKey {
    pub fn from_issue(issue: &octoIssue, head_commit: String) -> Self {
        Self {
            issue_updated_at: issue.updated_at.clone(),
            branch: issue
                .body
                .as_deref()
                .and_then(parse_branch_from_body)
                .unwrap_or("unknown".to_string()),
            head_commit,
        }
    }
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

impl CacheEntry {
    pub(crate) fn status(&self) -> &QCStatus {
        match self {
            CacheEntry::Complete { qc_status, .. } | CacheEntry::Partial { qc_status, .. } => {
                qc_status
            }
        }
    }
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

    pub fn update(
        &mut self,
        key: CacheKey,
        update_issue: &octoIssue,
        current_commit: &str,
        action: UpdateAction,
    ) {
        let mut update_issue = update_issue.clone();
        update_issue.updated_at = key.issue_updated_at;
        if let Some((cache_key, cache_entry)) = self.entries.get_mut(&update_issue.number) {
            *cache_key = key;
            match cache_entry {
                CacheEntry::Complete {
                    issue,
                    qc_status,
                    commits,
                    checklist_summary,
                    ..
                } => {
                    *issue = update_issue.clone().into();
                    let is_latest_commit = action.update_commits(commits, current_commit);
                    action.update_status_for_complete_entry(
                        qc_status,
                        is_latest_commit,
                        current_commit,
                    );
                    let checklist_summaries =
                        analyze_issue_checklists(update_issue.body.as_deref());
                    *checklist_summary = checklist_summaries.into();
                }
                CacheEntry::Partial { qc_status, .. } => {
                    action.update_status_for_partial_entry(qc_status, current_commit);
                }
            }
        }
    }

    pub fn unapproval(&mut self, key: CacheKey, update_issue: &octoIssue) {
        let mut update_issue = update_issue.clone();
        update_issue.updated_at = key.issue_updated_at;
        if let Some((cache_key, cache_entry)) = self.entries.get_mut(&update_issue.number) {
            *cache_key = key;
            match cache_entry {
                CacheEntry::Complete {
                    issue,
                    qc_status,
                    commits,
                    checklist_summary,
                    ..
                } => {
                    *issue = update_issue.clone().into();
                    match qc_status.status {
                        QCStatusEnum::Approved => qc_status.status = QCStatusEnum::ChangeRequested,
                        QCStatusEnum::ChangesAfterApproval => {
                            qc_status.status = QCStatusEnum::ChangesToComment
                        }
                        _ => (),
                    };
                    // Convert Approved commit statuses to Notification
                    for commit in commits.iter_mut() {
                        if let Some(pos) = commit
                            .statuses
                            .iter()
                            .position(|s| *s == CommitStatusEnum::Approved)
                        {
                            if !commit.statuses.contains(&CommitStatusEnum::Notification) {
                                commit.statuses[pos] = CommitStatusEnum::Notification;
                            } else {
                                commit.statuses.remove(pos);
                            }
                        }
                    }

                    *checklist_summary =
                        analyze_issue_checklists(update_issue.body.as_deref()).into();
                }
                CacheEntry::Partial { qc_status, .. } => {
                    match qc_status.status {
                        QCStatusEnum::Approved => qc_status.status = QCStatusEnum::ChangeRequested,
                        QCStatusEnum::ChangesAfterApproval => {
                            qc_status.status = QCStatusEnum::ChangesToComment
                        }
                        _ => (),
                    };
                }
            }
        }
    }
}

pub enum UpdateAction {
    Notification,
    Review,
    Approve,
}

impl UpdateAction {
    /// updates commits and returns if it updated the latest commit
    fn update_commits(&self, commits: &mut Vec<IssueCommit>, current_commit: &str) -> bool {
        if let Some(commit) = commits.iter_mut().find(|c| c.hash == current_commit) {
            if !commit.statuses.contains(&self.commit_status()) {
                commit.statuses.push(self.commit_status());
            }
            commits
                .first()
                .map(|c| c.hash == current_commit)
                .unwrap_or(false)
        } else {
            commits.insert(
                0,
                IssueCommit {
                    hash: current_commit.to_string(),
                    message: "New commit".to_string(),
                    statuses: vec![self.commit_status()],
                    file_changed: true,
                },
            );
            true
        }
    }

    fn commit_status(&self) -> CommitStatusEnum {
        match self {
            Self::Notification => CommitStatusEnum::Notification,
            Self::Review => CommitStatusEnum::Reviewed,
            Self::Approve => CommitStatusEnum::Approved,
        }
    }

    fn update_status_for_complete_entry(
        &self,
        qc_status: &mut QCStatus,
        is_latest_commit: bool,
        current_commit: &str,
    ) {
        match self {
            UpdateAction::Notification => {
                if is_latest_commit {
                    if qc_status.status == QCStatusEnum::Approved {
                        qc_status.status = QCStatusEnum::ChangesAfterApproval;
                        qc_status.status_detail = "Approved; subsequent file changes".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    } else {
                        qc_status.status = QCStatusEnum::AwaitingReview;
                        qc_status.status_detail = "Awaiting review".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    }
                }
            }
            UpdateAction::Review => {
                if is_latest_commit {
                    if qc_status.status == QCStatusEnum::Approved {
                        qc_status.status = QCStatusEnum::ChangesAfterApproval;
                        qc_status.status_detail = "Approved; subsequent file changes".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    } else {
                        qc_status.status = QCStatusEnum::ChangeRequested;
                        qc_status.status_detail = "Change Requested".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    }
                }
            }
            UpdateAction::Approve => {
                if is_latest_commit {
                    qc_status.status = QCStatusEnum::Approved;
                    qc_status.status_detail = "Approved".to_string();
                    qc_status.latest_commit = current_commit.to_string();
                } else {
                    qc_status.status = QCStatusEnum::ChangesAfterApproval;
                    qc_status.status_detail = "Approved; subsequent file changes".to_string();
                }
            }
        }
    }

    fn update_status_for_partial_entry(&self, qc_status: &mut QCStatus, current_commit: &str) {
        // Assuming current_commit is latest
        match self {
            UpdateAction::Notification => {
                if qc_status.status == QCStatusEnum::Approved {
                    if qc_status.approved_commit.as_deref() != Some(current_commit) {
                        qc_status.status = QCStatusEnum::ChangesAfterApproval;
                        qc_status.status_detail = "Approved; subsequent file changes".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    }
                } else {
                    qc_status.status = QCStatusEnum::AwaitingReview;
                    qc_status.status_detail = "Awaiting review".to_string();
                    qc_status.latest_commit = current_commit.to_string();
                }
            }
            UpdateAction::Review => {
                if qc_status.status == QCStatusEnum::Approved {
                    if qc_status.approved_commit.as_deref() != Some(current_commit) {
                        qc_status.status = QCStatusEnum::ChangesAfterApproval;
                        qc_status.status_detail = "Approved; subsequent file changes".to_string();
                        qc_status.latest_commit = current_commit.to_string();
                    }
                } else {
                    qc_status.status = QCStatusEnum::ChangeRequested;
                    qc_status.status_detail = "Change Requested".to_string();
                    qc_status.latest_commit = current_commit.to_string()
                }
            }
            UpdateAction::Approve => {
                qc_status.status = QCStatusEnum::Approved;
                qc_status.status_detail = "Approved".to_string();
                qc_status.approved_commit = Some(current_commit.to_string());
                qc_status.latest_commit = current_commit.to_string();
            }
        }
    }
}

impl Default for StatusCache {
    fn default() -> Self {
        Self::new()
    }
}
