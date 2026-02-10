//! API response types.

use chrono::{DateTime, Utc};
use octocrab::models::IssueState;
use serde::Serialize;

use crate::api::ApiError;

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Milestone information.
#[derive(Debug, Serialize)]
pub struct Milestone {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub description: Option<String>,
    pub open_issues: u64,
    pub closed_issues: u64,
}

impl From<octocrab::models::Milestone> for Milestone {
    fn from(milestone: octocrab::models::Milestone) -> Self {
        Self {
            number: milestone.number as u64,
            title: milestone.title.to_string(),
            state: milestone.state.as_deref().unwrap_or("unknown").to_string(),
            description: milestone.description.clone(),
            open_issues: milestone.open_issues.unwrap_or_default() as u64,
            closed_issues: milestone.closed_issues.unwrap_or_default() as u64,
        }
    }
}

/// Issue information.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

impl From<octocrab::models::issues::Issue> for Issue {
    fn from(issue: octocrab::models::issues::Issue) -> Self {
        Issue {
            number: issue.number as u64,
            title: issue.title,
            state: match issue.state {
                IssueState::Closed => "closed",
                IssueState::Open => "open",
                _ => "unknown",
            }
            .to_string(),
            html_url: issue.html_url.to_string(),
            assignees: issue.assignees.iter().map(|a| a.login.clone()).collect(),
            labels: issue
                .labels
                .iter()
                .filter_map(|l| l.description.clone())
                .collect(),
            milestone: issue.milestone.map(|m| m.title),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            closed_at: issue.closed_at,
        }
    }
}

/// QC status information.
#[derive(Debug, Clone, Serialize)]
pub struct QCStatus {
    pub status: QCStatusEnum,
    pub status_detail: String,
    pub approved_commit: Option<String>,
    pub initial_commit: String,
    pub latest_commit: String,
}

/// QC status enum values.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QCStatusEnum {
    Approved,
    ChangesAfterApproval,
    AwaitingReview,
    ChangeRequested,
    InProgress,
    ApprovalRequired,
    ChangesToComment,
}

/// Git status information.
#[derive(Debug, Clone, Serialize)]
pub struct GitStatus {
    pub status: GitStatusEnum,
    pub detail: String,
    pub ahead_commits: Vec<String>,
    pub behind_commits: Vec<String>,
}

/// Git status enum values.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitStatusEnum {
    Clean,
    Ahead,
    Behind,
    Diverged,
}

/// Commit information for an issue.
#[derive(Debug, Clone, Serialize)]
pub struct IssueCommit {
    pub hash: String,
    pub message: String,
    pub statuses: Vec<CommitStatusEnum>,
    pub file_changed: bool,
}

/// Commit status enum values.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatusEnum {
    Initial,
    Notification,
    Approved,
    Reviewed,
}

/// Checklist completion summary.
#[derive(Debug, Clone, Serialize)]
pub struct ChecklistSummary {
    pub completed: u32,
    pub total: u32,
    pub percentage: f32,
}

/// Blocking QC item (approved).
#[derive(Debug, Serialize)]
pub struct BlockingQCItem {
    pub issue_number: u64,
    pub file_name: String,
}

/// Blocking QC item with status (not approved).
#[derive(Debug, Serialize)]
pub struct BlockingQCItemWithStatus {
    pub issue_number: u64,
    pub file_name: String,
    pub status: String,
}

/// Blocking QC error.
#[derive(Debug, Serialize)]
pub struct BlockingQCError {
    pub issue_number: u64,
    pub error: String,
}

/// Blocking QC status summary.
#[derive(Debug, Serialize)]
pub struct BlockingQCStatus {
    pub total: u32,
    pub approved_count: u32,
    pub summary: String,
    pub approved: Vec<BlockingQCItem>,
    pub not_approved: Vec<BlockingQCItemWithStatus>,
    pub errors: Vec<BlockingQCError>,
}

/// Full issue status response.
#[derive(Debug, Serialize)]
pub struct IssueStatusResponse {
    pub issue: Issue,
    pub qc_status: QCStatus,
    pub git_status: GitStatus,
    pub dirty_files: Vec<String>,
    pub commits: Vec<IssueCommit>,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_status: BlockingQCStatus,
}

/// Response for issue creation.
#[derive(Debug, Serialize)]
pub struct CreateIssueResponse {
    pub issue_url: String,
    pub issue_number: u64,
    pub blocking_created: Vec<u64>,
    pub blocking_errors: Vec<BlockingQCError>,
}

/// Response for comment creation.
#[derive(Debug, Serialize)]
pub struct CommentResponse {
    pub comment_url: String,
}

/// Response for issue approval.
#[derive(Debug, Serialize)]
pub struct ApprovalResponse {
    pub approval_url: String,
    pub skipped_unapproved: Vec<u64>,
    pub skipped_errors: Vec<BlockingQCError>,
}

/// Impact node for unapproval cascade.
#[derive(Debug, Serialize)]
pub struct ImpactNode {
    pub issue_number: u64,
    pub file_name: String,
    pub milestone: String,
    pub relationship: String,
    pub children: Vec<ImpactNode>,
    pub fetch_error: Option<String>,
}

/// Impacted issues from unapproval.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImpactedIssues {
    None,
    ApiUnavailable,
    Some { nodes: Vec<ImpactNode> },
}

/// Response for issue unapproval.
#[derive(Debug, Serialize)]
pub struct UnapprovalResponse {
    pub unapproval_url: String,
    pub impacted_issues: ImpactedIssues,
}

/// Repository assignee.
#[derive(Debug, Serialize)]
pub struct Assignee {
    pub login: String,
    pub name: Option<String>,
}

/// Full checklist with content.
#[derive(Debug, Serialize)]
pub struct Checklist {
    pub name: String,
    pub content: String,
}

impl From<crate::Checklist> for Checklist {
    fn from(checklist: crate::Checklist) -> Self {
        let content = format!(
            "{}{}",
            match &checklist.note {
                Some(n) => format!("{n}\n\n"),
                None => String::new(),
            },
            checklist.content
        );
        Self {
            name: checklist.name.to_string(),
            content,
        }
    }
}

/// Checklist summary information (name and item count).
#[derive(Debug, Serialize)]
pub struct ChecklistInfo {
    pub name: String,
    pub item_count: u32,
}

/// Git repository configuration status.
#[derive(Debug, Serialize)]
pub struct ConfigGitRepository {
    pub owner: String,
    pub repo: String,
    pub status: GitStatusEnum,
    pub dirty_files: Vec<String>,
}

/// Configuration options.
#[derive(Debug, Serialize)]
pub struct ConfigurationOptions {
    pub prepended_checklist_note: Option<String>,
    pub checklist_display_name: String,
    pub logo_path: String,
    pub logo_found: bool,
    pub checklist_directory: String,
    pub record_path: String,
}

/// Configuration status response.
#[derive(Debug, Serialize)]
pub struct ConfigurationStatusResponse {
    pub directory: String,
    pub git_repository: Option<ConfigGitRepository>,
    pub options: ConfigurationOptions,
    pub checklists: Vec<ChecklistInfo>,
}
