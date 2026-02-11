//! In-memory cache for issue status responses.

use crate::{
    IssueThread, analyze_issue_checklists,
    api::types::{ChecklistSummary, CommitStatusEnum, Issue, IssueCommit, QCStatus, QCStatusEnum},
    parse_blocking_qcs,
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

/// Status cache entries
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub issue: Issue,
    pub qc_status: QCStatus,
    pub branch: String,
    pub commits: Vec<IssueCommit>,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_numbers: Vec<u64>,
}

impl CacheEntry {
    pub fn new(issue: &octoIssue, issue_thread: &IssueThread) -> Self {
        Self {
            issue: issue.clone().into(),
            qc_status: issue_thread.into(),
            branch: issue
                .body
                .as_deref()
                .and_then(crate::parse_branch_from_body)
                .unwrap_or("unknown".to_string()),
            commits: issue_thread.commits.iter().map(IssueCommit::from).collect(),
            checklist_summary: analyze_issue_checklists(issue.body.as_deref()).into(),
            blocking_qc_numbers: issue
                .body
                .as_deref()
                .map(|body| {
                    parse_blocking_qcs(body)
                        .iter()
                        .map(|b| b.issue_number)
                        .collect()
                })
                .unwrap_or_default(),
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

    pub fn remove(&mut self, issue_number: u64) {
        self.entries.remove(&issue_number);
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
            cache_entry.issue = update_issue.clone().into();
            let is_latest_commit = action.update_commits(&mut cache_entry.commits, current_commit);
            action.update_status(&mut cache_entry.qc_status, is_latest_commit, current_commit);
            let checklist_summaries = analyze_issue_checklists(update_issue.body.as_deref());
            cache_entry.checklist_summary = checklist_summaries.into();
        }
    }

    pub fn unapproval(&mut self, key: CacheKey, update_issue: &octoIssue) {
        let mut update_issue = update_issue.clone();
        update_issue.updated_at = key.issue_updated_at;
        if let Some((cache_key, cache_entry)) = self.entries.get_mut(&update_issue.number) {
            *cache_key = key;

            cache_entry.issue = update_issue.clone().into();
            match cache_entry.qc_status.status {
                QCStatusEnum::Approved => {
                    cache_entry.qc_status.status = QCStatusEnum::ChangeRequested
                }
                QCStatusEnum::ChangesAfterApproval => {
                    cache_entry.qc_status.status = QCStatusEnum::ChangesToComment
                }
                _ => (),
            };
            // Convert Approved commit statuses to Notification
            for commit in cache_entry.commits.iter_mut() {
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

            cache_entry.checklist_summary =
                analyze_issue_checklists(update_issue.body.as_deref()).into();
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

    fn update_status(
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
}

impl Default for StatusCache {
    fn default() -> Self {
        Self::new()
    }
}
