//! API response types.

use std::path::PathBuf;

use chrono::{DateTime, Utc};

use gix::ObjectId;
use octocrab::models::IssueState;
use serde::{Deserialize, Serialize};

use crate::{
    GitHubApiError, GitProvider, IssueThread,
    api::{ApiError, cache::CacheEntry},
    create::CreateResult,
    get_git_status,
};

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Milestone information.
#[derive(Debug, Serialize, Deserialize)]
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

/// Kind of a relevant file entry in an issue body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RelevantFileKind {
    /// Gating QC or Previous QC — must be approved before this issue
    BlockingQc,
    /// Relevant QC — informational only
    RelevantQc,
    /// Plain file with no associated issue
    File,
}

/// A single entry from the "## Relevant Files" section of an issue body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantFileInfo {
    pub file_name: String,
    pub kind: RelevantFileKind,
    /// GitHub issue URL — present for BlockingQc and RelevantQc kinds, None for File
    pub issue_url: Option<String>,
}

/// Issue information.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub created_by: String,
    pub branch: Option<String>,
    pub checklist_name: Option<String>,
    pub relevant_files: Vec<RelevantFileInfo>,
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
            labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
            milestone: issue.milestone.map(|m| m.title),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            closed_at: issue.closed_at,
            created_by: issue.user.login.clone(),
            branch: issue
                .body
                .as_deref()
                .and_then(parse_branch_from_body_simple),
            checklist_name: issue.body.as_deref().and_then(parse_checklist_name),
            relevant_files: issue
                .body
                .as_deref()
                .map(parse_relevant_file_infos)
                .unwrap_or_default(),
        }
    }
}

/// QC status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QCStatus {
    pub status: QCStatusEnum,
    pub status_detail: String,
    pub approved_commit: Option<String>,
    pub initial_commit: String,
    pub latest_commit: String,
}

impl From<&IssueThread> for QCStatus {
    fn from(issue: &IssueThread) -> Self {
        let status = crate::QCStatus::determine_status(issue);
        Self {
            status_detail: status.to_string(),
            status: status.into(),
            approved_commit: issue.approved_commit().map(|c| c.hash.to_string()),
            initial_commit: issue.initial_commit().to_string(),
            latest_commit: issue.latest_commit().hash.to_string(),
        }
    }
}

/// QC status enum values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl From<crate::QCStatus> for QCStatusEnum {
    fn from(value: crate::QCStatus) -> Self {
        match value {
            crate::QCStatus::Approved => QCStatusEnum::Approved,
            crate::QCStatus::ChangesAfterApproval(_) => QCStatusEnum::ChangesAfterApproval,
            crate::QCStatus::AwaitingReview => QCStatusEnum::AwaitingReview,
            crate::QCStatus::ChangeRequested => QCStatusEnum::ChangeRequested,
            crate::QCStatus::InProgress => QCStatusEnum::InProgress,
            crate::QCStatus::ApprovalRequired => QCStatusEnum::ApprovalRequired,
            crate::QCStatus::ChangesToComment(_) => QCStatusEnum::ChangesToComment,
        }
    }
}

/// Git status information.
#[derive(Debug, Clone, Serialize)]
pub struct GitStatus {
    pub status: GitStatusEnum,
    pub detail: String,
    pub ahead_commits: Vec<String>,
    pub behind_commits: Vec<String>,
}

impl From<crate::GitState> for GitStatus {
    fn from(status: crate::GitState) -> Self {
        let mut res = GitStatus {
            status: GitStatusEnum::Clean,
            detail: status.to_string(),
            ahead_commits: Vec::new(),
            behind_commits: Vec::new(),
        };

        let convert_commits = |commits: Vec<ObjectId>| -> Vec<String> {
            commits.iter().map(ObjectId::to_string).collect()
        };

        match status {
            crate::GitState::Clean => (),
            crate::GitState::Ahead(ahead_commits) => {
                res.status = GitStatusEnum::Ahead;
                res.ahead_commits = convert_commits(ahead_commits);
            }
            crate::GitState::Behind(behind_commits) => {
                res.status = GitStatusEnum::Behind;
                res.behind_commits = convert_commits(behind_commits);
            }
            crate::GitState::Diverged { ahead, behind } => {
                res.status = GitStatusEnum::Diverged;
                res.ahead_commits = convert_commits(ahead);
                res.behind_commits = convert_commits(behind);
            }
        }

        res
    }
}

/// Git status enum values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitStatusEnum {
    Clean,
    Ahead,
    Behind,
    Diverged,
}

/// Commit information for an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueCommit {
    pub hash: String,
    pub message: String,
    pub statuses: Vec<CommitStatusEnum>,
    pub file_changed: bool,
}

impl From<&crate::IssueCommit> for IssueCommit {
    fn from(commit: &crate::IssueCommit) -> Self {
        Self {
            hash: commit.hash.to_string(),
            message: commit.message.to_string(),
            statuses: commit.statuses.iter().map(CommitStatusEnum::from).collect(),
            file_changed: commit.file_changed,
        }
    }
}

/// Commit status enum values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatusEnum {
    Initial,
    Notification,
    Approved,
    Reviewed,
}

impl From<&crate::CommitStatus> for CommitStatusEnum {
    fn from(status: &crate::CommitStatus) -> Self {
        match status {
            crate::CommitStatus::Initial => CommitStatusEnum::Initial,
            crate::CommitStatus::Notification => CommitStatusEnum::Notification,
            crate::CommitStatus::Approved => CommitStatusEnum::Approved,
            crate::CommitStatus::Reviewed => CommitStatusEnum::Reviewed,
        }
    }
}

/// Checklist completion summary.
#[derive(Debug, Clone, Serialize)]
pub struct ChecklistSummary {
    pub completed: u32,
    pub total: u32,
    pub percentage: f32,
}

impl From<Vec<(String, crate::ChecklistSummary)>> for ChecklistSummary {
    fn from(checklists: Vec<(String, crate::ChecklistSummary)>) -> Self {
        let sum = crate::ChecklistSummary::sum(checklists.iter().map(|(_, c)| c));
        Self {
            completed: sum.completed as u32,
            total: sum.total as u32,
            percentage: if sum.total == 0 {
                0.0
            } else {
                sum.completed as f32 / sum.total as f32
            },
        }
    }
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
#[derive(Debug, Clone, Serialize)]
pub struct BlockingQCError {
    pub issue_number: u64,
    pub error: String,
}

impl From<(u64, GitHubApiError)> for BlockingQCError {
    fn from(value: (u64, GitHubApiError)) -> Self {
        Self {
            issue_number: value.0,
            error: value.1.to_string(),
        }
    }
}

/// Blocking QC status summary.
#[derive(Debug, Serialize, Default)]
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
    pub dirty: bool,
    pub branch: String,
    pub commits: Vec<IssueCommit>,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_status: BlockingQCStatus,
}

impl IssueStatusResponse {
    pub fn from_cache_entry(entry: CacheEntry, dirty_files: &[PathBuf]) -> Self {
        Self {
            dirty: dirty_files.contains(&PathBuf::from(&entry.issue.title)),
            issue: entry.issue,
            qc_status: entry.qc_status,
            branch: entry.branch,
            commits: entry.commits,
            checklist_summary: entry.checklist_summary,
            blocking_qc_status: BlockingQCStatus::default(),
        }
    }
}

/// Blocked issue with status.
#[derive(Debug, Serialize, Deserialize)]
pub struct BlockedIssueStatus {
    pub issue: Issue,
    pub qc_status: QCStatus,
}

/// Response for issue creation.
#[derive(Debug, Serialize)]
pub struct CreateIssueResponse {
    pub issue_url: String,
    pub blocking_created: Vec<u64>,
    pub blocking_errors: Vec<BlockingQCError>,
}

impl From<CreateResult> for CreateIssueResponse {
    fn from(res: CreateResult) -> Self {
        Self {
            issue_url: res.issue_url,
            blocking_created: res.successful_blocking,
            blocking_errors: res
                .blocking_errors
                .into_iter()
                .map(BlockingQCError::from)
                .collect(),
        }
    }
}

/// Error kind for batch issue status.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatusErrorKind {
    FetchFailed,
    ProcessingFailed,
}

/// Error entry for batch issue status.
#[derive(Debug, Serialize)]
pub struct IssueStatusError {
    pub issue_number: u64,
    pub kind: IssueStatusErrorKind,
    pub error: String,
}

/// Envelope response for batch issue status.
#[derive(Debug, Serialize)]
pub struct BatchIssueStatusResponse {
    pub results: Vec<IssueStatusResponse>,
    pub errors: Vec<IssueStatusError>,
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
    pub closed: bool,
}

/// Response for issue unapproval.
#[derive(Debug, Serialize)]
pub struct UnapprovalResponse {
    pub unapproval_url: String,
    pub opened: bool,
}

/// Repository assignee.
#[derive(Debug, Serialize)]
pub struct Assignee {
    pub login: String,
    pub name: Option<String>,
}

/// Full checklist with content.
#[derive(Debug, Serialize, Deserialize)]
pub struct Checklist {
    pub name: String,
    pub content: String,
}

impl From<crate::Checklist> for Checklist {
    fn from(checklist: crate::Checklist) -> Self {
        Self {
            name: checklist.name,
            content: checklist.content,
        }
    }
}

/// Git repository configuration status.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigGitRepository {
    pub owner: String,
    pub repo: String,
    pub status: GitStatusEnum,
    pub dirty_files: Vec<String>,
}

impl ConfigGitRepository {
    pub async fn new<G: GitProvider + Clone + Send + 'static>(
        git_info: &G,
    ) -> Result<Self, ApiError> {
        let owner = git_info.owner().to_string();
        let repo = git_info.repo().to_string();
        let git_info = git_info.clone();

        // Perform blocking git operations in a blocking task
        let (status, dirty_files) = tokio::task::spawn_blocking(move || {
            let status = get_git_status(&git_info)?;
            let api_status: GitStatus = status.state.into();
            let dirty_files = status
                .dirty
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>();

            Ok::<_, ApiError>((api_status, dirty_files))
        })
        .await
        .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))??;

        Ok(Self {
            owner,
            repo,
            status: status.status,
            dirty_files,
        })
    }
}

/// Configuration options.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigurationOptions {
    pub prepended_checklist_note: Option<String>,
    pub checklist_display_name: String,
    pub logo_path: String,
    pub logo_found: bool,
    pub checklist_directory: String,
    pub record_path: String,
}

/// Configuration status response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigurationStatusResponse {
    pub directory: String,
    pub exists: bool,
    pub git_repository: Option<ConfigGitRepository>,
    pub options: ConfigurationOptions,
    pub checklists: Vec<Checklist>,
    pub config_repo_env: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoInfoResponse {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub local_commit: String,
    pub remote_commit: String,
    pub git_status: GitStatusEnum,
    pub git_status_detail: String,
    pub dirty_files: Vec<String>,
    pub current_user: Option<String>,
}

/// Kind of a file tree entry.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeEntryKind {
    File,
    Directory,
}

/// A single entry in a file tree listing.
#[derive(Debug, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub kind: TreeEntryKind,
}

/// Response for a file tree listing at a given path.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileTreeResponse {
    pub path: String,
    pub entries: Vec<TreeEntry>,
}

/// Response for archive generation.
#[derive(Debug, Serialize)]
pub struct ArchiveGenerateResponse {
    pub output_path: String,
}

/// Response for context PDF upload.
#[derive(Debug, Serialize)]
pub struct RecordUploadResponse {
    pub temp_path: String,
}

/// Response for record preview generation.
#[derive(Debug, Serialize)]
pub struct RecordPreviewResponse {
    pub key: String,
}

/// Extract the checklist name from the first h1 heading (e.g. "# Code Review").
fn parse_checklist_name(body: &str) -> Option<String> {
    body.lines()
        .find(|l| l.starts_with("# ") && !l.starts_with("## "))
        .map(|l| l[2..].trim().to_string())
}

/// Minimal branch parser — handles plain text and markdown links.
fn parse_branch_from_body_simple(body: &str) -> Option<String> {
    let pattern = "git branch: ";
    let start = body.find(pattern)?;
    let line = body[start + pattern.len()..].lines().next()?;
    // strip markdown/html links to just the link text
    if let (Some(a), Some(b)) = (line.find('['), line.find("](")) {
        return Some(line[a + 1..b].trim().to_string());
    }
    let plain = line.trim();
    if plain.is_empty() {
        None
    } else {
        Some(plain.to_string())
    }
}

/// Parse all entries from the "## Relevant Files" section.
fn parse_relevant_file_infos(body: &str) -> Vec<RelevantFileInfo> {
    use regex::Regex;
    use std::sync::LazyLock;
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap());
    static BOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*([^*]+)\*\*").unwrap());

    let rf_start = match body.find("## Relevant Files") {
        Some(p) => p,
        None => return vec![],
    };
    let section = &body[rf_start..];
    let end = section[17..]
        .find("\n## ")
        .map(|p| p + 17)
        .unwrap_or(section.len());
    let section = &section[..end];

    let mut result = Vec::new();

    for (sub, kind) in [
        ("### Previous QC", RelevantFileKind::BlockingQc),
        ("### Gating QC", RelevantFileKind::BlockingQc),
        ("### Relevant QC", RelevantFileKind::RelevantQc),
        ("### Relevant File", RelevantFileKind::File),
    ] {
        let sub_start = match section.find(sub) {
            Some(p) => p,
            None => continue,
        };
        let sub_section = &section[sub_start..];
        let sub_end = sub_section[sub.len()..]
            .find("\n### ")
            .map(|p| p + sub.len())
            .unwrap_or(sub_section.len());
        let sub_section = &sub_section[..sub_end];

        if kind == RelevantFileKind::File {
            for cap in BOLD.captures_iter(sub_section) {
                result.push(RelevantFileInfo {
                    file_name: cap[1].to_string(),
                    kind: RelevantFileKind::File,
                    issue_url: None,
                });
            }
        } else {
            for cap in LINK.captures_iter(sub_section) {
                result.push(RelevantFileInfo {
                    file_name: cap[1].to_string(),
                    kind: kind.clone(),
                    issue_url: Some(cap[2].to_string()),
                });
            }
        }
    }
    result
}

impl RepoInfoResponse {
    pub async fn new<G: GitProvider + Clone + Send + 'static>(
        git_info: &G,
    ) -> Result<Self, ApiError> {
        let owner = git_info.owner().to_string();
        let repo = git_info.repo().to_string();

        // Async GitHub call — non-fatal, falls back to None
        let current_user = git_info.get_current_user().await.ok().flatten();

        let git_info = git_info.clone();

        // Perform blocking git operations in a blocking task
        let (branch, local_commit, remote_commit, git_status_enum, git_status_detail, dirty_files) =
            tokio::task::spawn_blocking(move || {
                let git_status = get_git_status(&git_info)?;
                let local_commit = git_info.commit()?;
                let branch = git_info.branch()?;
                let remote_commit = git_status.remote_commit.to_string();
                let api_git_status = GitStatus::from(git_status.state.clone());

                Ok::<_, ApiError>((
                    branch,
                    local_commit,
                    remote_commit,
                    api_git_status.status,
                    git_status.state.to_string(),
                    git_status
                        .dirty
                        .into_iter()
                        .map(|p| p.display().to_string())
                        .collect(),
                ))
            })
            .await
            .map_err(|e| ApiError::Internal(format!("Blocking task failed: {}", e)))??;

        Ok(Self {
            owner,
            repo,
            branch,
            local_commit,
            remote_commit,
            git_status: git_status_enum,
            git_status_detail,
            dirty_files,
            current_user,
        })
    }
}
