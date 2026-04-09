//! Issue endpoints.

use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues, format_error_list};
use crate::api::state::AppState;
use crate::api::types::{
    BatchIssueStatusResponse, BlockedIssueStatus, BlockingQCError, BlockingQCItem,
    BlockingQCItemWithStatus, BlockingQCStatus, CreateIssueRequest, CreateIssueResponse, Issue,
    IssueStatusError, IssueStatusErrorKind, IssueStatusResponse, QCStatusEnum,
};
use crate::comment_system::CommentBody;
use crate::create::QCIssueError;
use crate::git::{GitFileOps, GitHelpers, GitHubApiError};
use crate::{
    FileRenameEvent, GitProvider, QCEntry, batch_post_qc_entries, create_labels_if_needed,
    file_history_section, get_repo_users, head_commit_hash, parse_file_history, splice_file_history,
};
use octocrab::models::issues::Issue as OctocrabIssue;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct IssueStatusQuery {
    /// Comma-separated list of issue numbers
    pub issues: String,
}

/// POST /api/milestones/{number}/issues
pub async fn create_issues<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(milestone_number): Path<u64>,
    Json(requests): Json<Vec<CreateIssueRequest>>,
) -> Result<(StatusCode, Json<Vec<CreateIssueResponse>>), ApiError> {
    // Validate milestone exists
    let milestones = state.git_info().get_milestones().await?;
    if !milestones
        .iter()
        .any(|m| m.number == milestone_number as i64)
    {
        return Err(ApiError::NotFound(format!(
            "Milestone {} not found",
            milestone_number
        )));
    }

    // Get existing issues in milestone
    let milestone_issues = state
        .git_info()
        .get_issues(Some(milestone_number))
        .await
        .map_err(|e| {
            ApiError::GitHubApi(format!(
                "Failed to fetch existing issues in milestone {}: {}",
                milestone_number, e
            ))
        })?;

    let entries = requests
        .into_iter()
        .map(QCEntry::try_from)
        .collect::<Result<Vec<QCEntry>, _>>()
        .map_err(ApiError::BadRequest)?;
    let include_collaborators = state.configuration.read().await.include_collaborators();
    let entries = if include_collaborators {
        entries
    } else {
        entries
            .into_iter()
            .map(|mut entry| {
                entry.collaborators = Some(Vec::new());
                entry
            })
            .collect()
    };

    // Check for duplicate filenames within the request
    let mut seen_files = HashSet::new();
    let mut duplicate_files = Vec::new();
    for entry in &entries {
        if !seen_files.insert(&entry.title) {
            duplicate_files.push(entry.title.to_string_lossy().to_string());
        }
    }
    if !duplicate_files.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Duplicate files in request:\n  - {}",
            duplicate_files.join("\n  - ")
        )));
    }

    // Validate assignees exist in repository
    let repo_users = get_repo_users(state.disk_cache(), state.git_info())
        .await?
        .into_iter()
        .map(|r| r.login)
        .collect::<HashSet<_>>();

    let unknown_assignees = entries
        .iter()
        .flat_map(|e| e.assignees.iter())
        .filter(|a| !repo_users.contains(*a))
        .collect::<HashSet<_>>();
    let mut unknown_assignees = unknown_assignees.into_iter().cloned().collect::<Vec<_>>();
    unknown_assignees.sort();
    if !unknown_assignees.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Unknown assignees: {}",
            unknown_assignees.join(", ")
        )));
    }

    // Check if any files already have issues in this milestone
    let duplicate_issues = entries
        .iter()
        .filter(|e| {
            milestone_issues
                .iter()
                .any(|i| PathBuf::from(&i.title) == e.title)
        })
        .map(|e| e.title.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if !duplicate_issues.is_empty() {
        return Err(ApiError::Conflict(format!(
            "Issues already exist in milestone for files:\n  - {}",
            duplicate_issues.join("\n  - ")
        )));
    }

    // Check if labels exist and create if not
    if let Err(e) = create_labels_if_needed(
        state.disk_cache(),
        state.git_info().branch().ok().as_deref(),
        state.git_info(),
    )
    .await
    {
        log::warn!("Failed to create issue labels: {e}. Continuing without...");
    }

    let current_user = state.git_info().get_current_user().await.ok().flatten();

    let res = batch_post_qc_entries(
        &entries,
        state.git_info(),
        milestone_number,
        current_user.as_deref(),
    )
    .await
    .map_err(|e| match e {
        QCIssueError::DependencyResolution { errors } => ApiError::BadRequest(format!(
            "Failed to resolve issue creation order:\n  -{}",
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n  -")
        )),
        QCIssueError::GitHubApiError(e) => ApiError::GitHubApi(e.to_string()),
        _ => ApiError::Internal(e.to_string()),
    })?;

    Ok((
        StatusCode::CREATED,
        Json(res.into_iter().map(CreateIssueResponse::from).collect()),
    ))
}

/// GET /api/issues/status?issues=1,2,3
pub async fn batch_get_issue_status<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Query(query): Query<IssueStatusQuery>,
) -> Result<(StatusCode, Json<BatchIssueStatusResponse>), ApiError> {
    // Parse comma-separated issue numbers — bad input is a caller mistake, return early.
    let parts: Vec<&str> = query.issues.split(',').map(|s| s.trim()).collect();
    let mut issue_numbers = Vec::new();
    let mut invalid_parts = Vec::new();

    for part in parts {
        match part.parse::<u64>() {
            Ok(num) => issue_numbers.push(num),
            Err(_) => invalid_parts.push(part),
        }
    }

    if !invalid_parts.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "Invalid issue numbers: {}",
            invalid_parts.join(", ")
        )));
    }

    if issue_numbers.is_empty() {
        return Err(ApiError::BadRequest(
            "No issue numbers provided".to_string(),
        ));
    }

    let mut fetched_issues = FetchedIssues::fetch_issues(&issue_numbers, state.git_info()).await;

    // Don't return early on fetch errors — accumulate them as FetchFailed entries.
    fetched_issues.fetch_blocking_qcs(state.git_info()).await;

    // Only create threads for successfully fetched issues.
    let created_threads = CreatedThreads::create_threads(&fetched_issues.issues, &state).await;
    fetched_issues.errors.extend(
        created_threads
            .thread_errors
            .into_iter()
            .map(|(n, e)| (n, e.to_string())),
    );

    let mut errors: Vec<IssueStatusError> = Vec::new();
    let mut responses: Vec<IssueStatusResponse> = Vec::new();

    // Preserve request ordering.
    for issue_number in &issue_numbers {
        if let Some(response) = created_threads.responses.get(issue_number) {
            let mut response = response.clone();
            determine_blocking_qc_status(
                &mut response.blocking_qc_status,
                created_threads
                    .blocking_qc_numbers
                    .get(issue_number)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                &created_threads.responses,
                &fetched_issues.errors,
            );
            responses.push(response);
        } else {
            // Distinguish between fetch failures and processing failures.
            let (kind, error) = if fetched_issues
                .issues
                .iter()
                .any(|i| i.number == *issue_number)
            {
                // Issue was fetched but thread/cache creation failed → processing error.
                let msg = fetched_issues
                    .errors
                    .get(issue_number)
                    .cloned()
                    .unwrap_or_else(|| "Failed to determine issue status".to_string());
                (IssueStatusErrorKind::ProcessingFailed, msg)
            } else {
                // Issue was never fetched successfully → fetch error.
                let msg = fetched_issues
                    .errors
                    .get(issue_number)
                    .cloned()
                    .unwrap_or_else(|| "Failed to fetch issue".to_string());
                (IssueStatusErrorKind::FetchFailed, msg)
            };
            errors.push(IssueStatusError {
                issue_number: *issue_number,
                kind,
                error,
            });
        }
    }

    let status = match (responses.is_empty(), errors.is_empty()) {
        (_, true) => StatusCode::OK,
        (false, _) => StatusCode::PARTIAL_CONTENT,
        (true, _) => StatusCode::INTERNAL_SERVER_ERROR,
    };

    Ok((
        status,
        Json(BatchIssueStatusResponse {
            results: responses,
            errors,
        }),
    ))
}

pub(crate) fn determine_blocking_qc_status(
    blocking_status: &mut BlockingQCStatus,
    blocking_numbers: &[u64],
    responses: &std::collections::HashMap<u64, IssueStatusResponse>,
    errors: &std::collections::HashMap<u64, String>,
) {
    blocking_status.total = blocking_numbers.len() as u32;
    for number in blocking_numbers {
        if let Some(response) = responses.get(number) {
            match response.qc_status.status {
                QCStatusEnum::Approved | QCStatusEnum::ChangesAfterApproval => {
                    blocking_status.approved_count += 1;
                    blocking_status.approved.push(BlockingQCItem {
                        issue_number: *number,
                        file_name: response.issue.title.clone(),
                    });
                }
                _ => {
                    blocking_status.not_approved.push(BlockingQCItemWithStatus {
                        issue_number: *number,
                        file_name: response.issue.title.clone(),
                        status: response.qc_status.status_detail.clone(),
                    });
                }
            }
        } else if let Some(error) = errors.get(number) {
            blocking_status.errors.push(BlockingQCError {
                issue_number: *number,
                error: error.to_string(),
            });
        } else {
            blocking_status.errors.push(BlockingQCError {
                issue_number: *number,
                error: "Failed to determine status".to_string(),
            });
        }
    }

    blocking_status.summary = if blocking_status.approved_count == blocking_status.total {
        "All blocking QCs approved".to_string()
    } else {
        format!(
            "{}/{} blocking QCs are approved",
            blocking_status.approved_count, blocking_status.total
        )
    };
}

/// GET /api/issues/{number}
pub async fn get_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<Issue>, ApiError> {
    let issue = state.git_info().get_issue(number).await.map(Issue::from)?;

    Ok(Json(issue))
}

/// GET /api/issues/{number}/blocked
pub async fn get_blocked_issues<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
) -> Result<Json<Vec<BlockedIssueStatus>>, ApiError> {
    // APIError / NoApi mean the endpoint doesn't exist on this GitHub instance → 501 so
    // the client can fall back to the simple unapprove UI.  Other errors (e.g. client
    // creation failure) stay as 502 since they indicate a real infrastructure problem.
    let blocking_issues = match state.git_info().get_blocked_issues(number).await {
        Ok(issues) => issues,
        Err(GitHubApiError::NoApi) => {
            return Err(ApiError::NotImplemented(
                "Blocked issues API is not available on this GitHub instance".to_string(),
            ));
        }
        Err(e) => return Err(ApiError::from(e)),
    };

    let mut blocked_statuses = Vec::new();
    let created_threads = CreatedThreads::create_threads(&blocking_issues, &state).await;
    if !created_threads.thread_errors.is_empty() {
        return Err(ApiError::Internal(format!(
            "Failed to determine status:\n  -{}",
            format_error_list(&created_threads.thread_errors)
        )));
    }

    // Merge cached statuses with newly fetched ones
    blocked_statuses.extend(created_threads.responses.into_values().map(|response| {
        BlockedIssueStatus {
            issue: response.issue,
            qc_status: response.qc_status,
        }
    }));

    Ok(Json(blocked_statuses))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RenameIssueRequest {
    pub new_path: String,
}

/// Comment body posted to the issue timeline when a rename is confirmed.
struct RenameComment {
    issue: OctocrabIssue,
    old_path: String,
    new_path: String,
    commit: String,
}

impl CommentBody for RenameComment {
    fn generate_body(&self, _git_info: &(impl GitHelpers + GitFileOps)) -> String {
        format!(
            "# QC File Rename\n`{}` \u{2192} `{}` (commit: {})",
            self.old_path, self.new_path, self.commit
        )
    }

    fn issue(&self) -> &OctocrabIssue {
        &self.issue
    }

    fn title(&self) -> &str {
        "QC File Rename"
    }
}

/// POST /api/issues/{number}/rename
///
/// Confirm a detected file rename: updates the issue title to `new_path`,
/// appends a `## File History` entry to the issue body recording the rename,
/// and posts a timeline comment so the rename is visible in the issue thread.
pub async fn rename_issue<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<RenameIssueRequest>,
) -> Result<StatusCode, ApiError> {
    if request.new_path.trim().is_empty() {
        return Err(ApiError::BadRequest("new_path must not be empty".to_string()));
    }

    // Fetch the current issue to get its title (old path) and body.
    let raw_issue = state
        .git_info()
        .get_issue(number)
        .await
        .map_err(ApiError::from)?;

    let old_path = raw_issue.title.clone();
    let current_body = raw_issue.body.as_deref().unwrap_or("").to_string();

    // Get the HEAD commit hash to record in history.
    let repo_path = state.git_info().path().to_path_buf();
    let commit_hash = tokio::task::spawn_blocking(move || {
        head_commit_hash(&repo_path).unwrap_or_else(|| "unknown".to_string())
    })
    .await
    .unwrap_or_else(|_| "unknown".to_string());

    let new_path = request.new_path;

    // Build the updated file history.
    let mut events = parse_file_history(&current_body);
    events.push(FileRenameEvent {
        old_path: old_path.clone(),
        new_path: new_path.clone(),
        commit: commit_hash.clone(),
    });
    let history_section = file_history_section(&events);

    // Splice history section into the body: insert before the checklist (first `# ` heading)
    // or append if no such heading exists.
    let new_body = splice_file_history(&current_body, &history_section);

    state
        .git_info()
        .update_issue(number, Some(new_path.clone()), Some(new_body))
        .await
        .map_err(ApiError::from)?;

    log::info!("Renamed issue #{number}: {:?} → {:?} (commit {})", old_path, new_path, commit_hash);

    // Post a timeline comment so the rename is visible in the issue thread.
    // A failure here is non-fatal: the title and body are already updated.
    let rename_comment = RenameComment {
        issue: raw_issue,
        old_path,
        new_path,
        commit: commit_hash,
    };
    if let Err(e) = state.git_info().post_comment(&rename_comment).await {
        log::warn!("Failed to post rename comment to issue #{number}: {e}");
    }

    Ok(StatusCode::NO_CONTENT)
}

