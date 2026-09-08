//! Issue endpoints.

use crate::api::error::ApiError;
use crate::api::fetch_helpers::{CreatedThreads, FetchedIssues, format_error_list};
use crate::api::state::AppState;
use crate::api::types::{
    BatchIssueStatusResponse, BlockedIssueStatus, BlockingQCError, BlockingQCItem,
    BlockingQCItemWithStatus, BlockingQCStatus, CreateIssueRequest, CreateIssueResponse,
    CreateRoundRequest, CreateRoundResponse, Issue, IssueStatusError, IssueStatusErrorKind,
    IssueStatusResponse, NotificationOutcome, QCStatusEnum,
};
use crate::comment_system::CommentBody;
use crate::configuration::Checklist;
use crate::create::QCIssueError;
use crate::git::{GitFileOps, GitHelpers, GitHubApiError};
use crate::issue::{qc_rounds_section, splice_qc_rounds};
use crate::qc_status::analyze_checklist_in_text;
use crate::round::QCRound;
use crate::{
    FileRenameEvent, GitProvider, IssueThread, QCComment, QCEntry, batch_post_qc_entries,
    create_labels_if_needed, file_history_section, get_issue_comments, get_repo_users,
    head_commit_hash, parse_file_history, splice_file_history,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use octocrab::models::issues::Issue as OctocrabIssue;
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
    fetched_issues.errors.extend(created_threads.thread_errors);

    let mut errors: Vec<IssueStatusError> = Vec::new();
    let mut responses: Vec<IssueStatusResponse> = Vec::new();

    // Lookup of issue number → file title for fetched-but-failed issues, so
    // BlockingQCError entries can name the file even when the thread couldn't
    // be built.
    let titles: std::collections::HashMap<u64, String> = fetched_issues
        .issues
        .iter()
        .map(|i| (i.number, i.title.clone()))
        .collect();

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
                &titles,
            );
            responses.push(response);
        } else {
            let was_fetched = fetched_issues
                .issues
                .iter()
                .any(|i| i.number == *issue_number);
            let default_kind = if was_fetched {
                IssueStatusErrorKind::ProcessingFailed
            } else {
                IssueStatusErrorKind::FetchFailed
            };
            let default_msg = if was_fetched {
                "Failed to determine issue status"
            } else {
                "Failed to fetch issue"
            };
            let (kind, error, branch) = match fetched_issues.errors.get(issue_number) {
                Some(e) => classify_issue_error(e, default_kind),
                None => (default_kind, default_msg.to_string(), None),
            };
            errors.push(IssueStatusError {
                issue_number: *issue_number,
                kind,
                error,
                branch,
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

/// Classify an `IssueError` into the API-facing `(kind, message, branch)` tuple.
/// `default_kind` is used when the error doesn't match a more specific case.
pub(crate) fn classify_issue_error(
    e: &crate::IssueError,
    default_kind: IssueStatusErrorKind,
) -> (IssueStatusErrorKind, String, Option<String>) {
    // D60: both D54 cases classify as `branch_not_local` and carry the branch —
    // `IssueError::branch_not_local` is the one place that pairing is decided, so the
    // wire contract does not depend on this match listing every variant.
    match e.branch_not_local() {
        Some(branch) => (
            IssueStatusErrorKind::BranchNotLocal,
            e.to_string(),
            Some(branch.to_string()),
        ),
        None => (default_kind, e.to_string(), None),
    }
}

pub(crate) fn determine_blocking_qc_status(
    blocking_status: &mut BlockingQCStatus,
    blocking_numbers: &[u64],
    responses: &std::collections::HashMap<u64, IssueStatusResponse>,
    errors: &std::collections::HashMap<u64, crate::IssueError>,
    titles: &std::collections::HashMap<u64, String>,
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
            let (kind, msg, branch) =
                classify_issue_error(error, IssueStatusErrorKind::FetchFailed);
            blocking_status.errors.push(BlockingQCError {
                issue_number: *number,
                error: msg,
                kind,
                file_name: titles.get(number).cloned(),
                branch,
            });
        } else {
            blocking_status.errors.push(BlockingQCError {
                issue_number: *number,
                error: "Failed to determine status".to_string(),
                kind: IssueStatusErrorKind::FetchFailed,
                file_name: titles.get(number).cloned(),
                branch: None,
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

/// POST /api/issues/{number}/rounds
///
/// Start a new QC round (A5): validate the checklist, write the `## QC Rounds` marker
/// into the body (D49/D51), post the `# QC Round N` comment (D3), re-open the issue
/// (D6), then post the optional `# QC Notification` (D5).
///
/// The round comment is still the only *declaration* — never a label, and the body
/// marker declares nothing (D3/D51): it is prose plus a presence bit. But it is written
/// **first and fatally** (D51): a cheap path may skip the comment fetch when the marker
/// is absent, so a missing marker on a multi-round issue makes a consumer trust a stale
/// `Issue.branch`. Marker-written-but-comment-failed only over-fetches, which is the
/// safe direction (D39.3).
pub async fn create_round<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    Path(number): Path<u64>,
    Json(request): Json<CreateRoundRequest>,
) -> Result<(StatusCode, Json<CreateRoundResponse>), ApiError> {
    // D40: I9 was demoted to a logged fold-time expectation precisely so the check
    // could live here, where it can actually be enforced. First, so an empty
    // checklist is reported as such rather than as whatever the next gate says.
    if analyze_checklist_in_text(&request.checklist.content).total == 0 {
        return Err(ApiError::BadRequest(
            "A new round requires a checklist with at least one item".to_string(),
        ));
    }

    let start_commit: gix::ObjectId = request
        .start_commit
        .parse()
        .map_err(|e: gix::hash::decode::Error| ApiError::BadRequest(e.to_string()))?;

    let issue = state.git_info().get_issue(number).await?;
    let comments = get_issue_comments(&issue, state.disk_cache(), state.git_info()).await?;
    let thread =
        IssueThread::from_issue_comments(&issue, &comments, state.git_info(), state.disk_cache())?;

    // D12: gate on `is_approved()` only — `Approved` or `ChangesAfterApproval`.
    // Starting from plain `Approved` is legal and produces the D8 overlap case
    // `start_{n+1} == approval_n`.
    // D55: an unresolvable latest round is reported, not guessed at — the gate cannot
    // run without a status, and the branch to fetch is the actionable half.
    let status = crate::QCStatus::determine_status(&thread)
        .map_err(|e| ApiError::Conflict(e.to_string()))?;
    if !status.is_approved() {
        return Err(ApiError::Conflict(format!(
            "Issue #{number} is {status}; a new round may only be started from an approved QC"
        )));
    }

    // D5: `previous commit` is the prior round's approval, `current commit` the new
    // round's start.
    let previous_commit = thread.latest_round().approved_commit().copied();
    let round_index = thread.next_round_index();
    let file = PathBuf::from(&issue.title);

    let round = QCRound::new(
        file.clone(),
        issue.clone(),
        round_index,
        request.branch,
        start_commit,
        Checklist {
            name: request.checklist.name,
            content: request.checklist.content,
        },
    );

    // D51: the marker goes in first, and its failure aborts the round. Nothing is
    // inconsistent when it fails — no round was created.
    let new_body = splice_qc_rounds(
        issue.body.as_deref().unwrap_or_default(),
        &qc_rounds_section(round_index),
    );
    state
        .git_info()
        .update_issue(number, None, Some(new_body))
        .await
        .map_err(|e| {
            ApiError::Internal(format!(
                "Failed to write the `## QC Rounds` marker to issue #{number}, \
                 so round {round_index} was not created: {e}"
            ))
        })?;

    let comment_url = state.git_info().post_comment(&round).await?;

    // D6/D45: starting a round re-opens the issue. Non-fatal — the round comment is
    // already posted and is what the fold reads — but reported, because a failed
    // re-open leaves S3 reporting `ApprovalRequired` for a round just created.
    let reopened = match state.git_info().open_issue(number).await {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Failed to re-open issue #{number} for round {round_index}: {e}");
            false
        }
    };

    let notification = if request.notify {
        let notification = QCComment {
            file,
            issue,
            current_commit: start_commit,
            previous_commit,
            note: request.note,
            no_diff: !request.include_diff,
        };
        match state.git_info().post_comment(&notification).await {
            Ok(url) => NotificationOutcome::Posted { url },
            Err(e) => {
                log::warn!("Failed to post round {round_index} notification on #{number}: {e}");
                NotificationOutcome::Failed {
                    error: e.to_string(),
                }
            }
        }
    } else {
        NotificationOutcome::NotRequested
    };

    // The round comment changes what the fold reads, so the cached comment list is
    // stale the moment it is posted.
    if let Some(cache) = state.disk_cache() {
        let cache_key = format!("issue_{number}");
        if let Err(e) = cache.invalidate(&["issues", "comments"], &cache_key) {
            log::warn!("Failed to invalidate comment cache for issue #{number}: {e}");
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(CreateRoundResponse {
            round_index,
            comment_url,
            reopened,
            notification,
        }),
    ))
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
        return Err(ApiError::BadRequest(
            "new_path must not be empty".to_string(),
        ));
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

    log::info!(
        "Renamed issue #{number}: {:?} → {:?} (commit {})",
        old_path,
        new_path,
        commit_hash
    );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::api::tests::helpers::{MockGitInfo, WriteCall};
    use crate::api::types::RoundChecklistRequest;

    /// The commit `MockGitInfo`'s walk returns — the only one a mocked thread can
    /// anchor on.
    const MOCK_COMMIT: &str = "456def789abc012345678901234567890123cdef";

    fn qc_issue() -> OctocrabIssue {
        crate::test_utils::create_test_issue(
            "test-owner",
            "test-repo",
            1,
            "src/test.rs",
            &format!("## Metadata\n* initial qc commit: {MOCK_COMMIT}\n* git branch: main\n"),
            Some(1),
            "open",
        )
    }

    fn round_request(checklist_content: &str) -> CreateRoundRequest {
        CreateRoundRequest {
            start_commit: MOCK_COMMIT.to_string(),
            branch: "main".to_string(),
            checklist: RoundChecklistRequest {
                name: "Second Pass".to_string(),
                content: checklist_content.to_string(),
            },
            notify: true,
            note: Some("please re-check".to_string()),
            include_diff: true,
        }
    }

    fn state_with(comments: Vec<&str>) -> AppState<MockGitInfo> {
        let issue = qc_issue();
        let mock = MockGitInfo::builder()
            .with_issue(issue.number, issue)
            .with_comments(1, comments)
            .with_commit(MOCK_COMMIT)
            .with_branch("main")
            .build();
        AppState::new(mock, Configuration::default(), None, None)
    }

    /// D57: notification defaults **ON**, matching the CLI's `!no_notify`. A plain
    /// `#[serde(default)]` yielded `false`, so a client that omitted the field silently
    /// got the opposite of what the other sanctioned path does.
    #[test]
    fn test_create_round_request_defaults_notify_to_true() {
        let request: CreateRoundRequest = serde_json::from_str(
            r#"{"start_commit":"abc1234","branch":"main",
                 "checklist":{"name":"Second Pass","content":"- [ ] one\n"}}"#,
        )
        .expect("the minimal body must deserialize");

        assert!(request.notify, "D57: notify defaults ON");
        // An explicit `false` is still honoured — the default is not a floor.
        let opted_out: CreateRoundRequest = serde_json::from_str(
            r#"{"start_commit":"abc1234","branch":"main","notify":false,
                 "checklist":{"name":"Second Pass","content":"- [ ] one\n"}}"#,
        )
        .unwrap();
        assert!(!opted_out.notify);
    }

    /// D55: a thread whose latest round has no resolvable commit reaches the client as
    /// the **existing** `branch_not_local` error, carrying the branch to fetch. No new
    /// `QCStatus` variant is involved (S5).
    #[test]
    fn test_unresolvable_latest_round_classifies_as_branch_not_local() {
        let (kind, message, branch) = classify_issue_error(
            &crate::IssueError::LocalBranchNotFound("feature".to_string()),
            IssueStatusErrorKind::ProcessingFailed,
        );

        assert!(matches!(kind, IssueStatusErrorKind::BranchNotLocal));
        assert_eq!(branch, Some("feature".to_string()));
        assert!(message.contains("feature"), "got {message}");
    }

    /// D60: the off-branch-approval error is a **distinct variant with its own message**
    /// but the **same wire classification** — `branch_not_local` plus the branch — so the
    /// existing contract and the UI affordance built on it need no change.
    #[test]
    fn test_off_branch_approval_also_classifies_as_branch_not_local() {
        let commit = gix::ObjectId::from_hex(b"1234567890123456789012345678901234567890").unwrap();
        let (kind, message, branch) = classify_issue_error(
            &crate::IssueError::ApprovalNotOnBranch {
                commit,
                branch: "feature".to_string(),
            },
            IssueStatusErrorKind::ProcessingFailed,
        );

        assert!(matches!(kind, IssueStatusErrorKind::BranchNotLocal));
        assert_eq!(branch, Some("feature".to_string()));
        assert!(message.contains("feature"), "got {message}");
        // The message is the only difference from `LocalBranchNotFound`.
        assert!(
            !message.contains("not checked out locally"),
            "got {message}"
        );
    }

    /// D40: I9 is no longer a fold-time assertion, so an empty checklist has to be
    /// refused here — and before the D12 gate, so the caller is told what is actually
    /// wrong. The issue in this test is also unapproved, which is why the assertion
    /// on the message matters.
    #[tokio::test]
    async fn test_create_round_rejects_an_empty_checklist() {
        let state = state_with(Vec::new());

        let error = create_round(
            State(state.clone()),
            Path(1),
            Json(round_request("no items\n")),
        )
        .await
        .expect_err("an empty checklist must be refused");

        match error {
            ApiError::BadRequest(message) => assert!(
                message.contains("checklist"),
                "unexpected message: {message}"
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }

        // Nothing was written: the refusal precedes every GitHub call.
        assert!(state.git_info().write_calls().is_empty());
    }

    /// D12: a new round may be started only when `QCStatus::is_approved()`.
    #[tokio::test]
    async fn test_create_round_requires_an_approved_qc() {
        let state = state_with(Vec::new());

        let error = create_round(
            State(state.clone()),
            Path(1),
            Json(round_request("- [ ] item\n")),
        )
        .await
        .expect_err("an unapproved QC must not start a round");

        match error {
            ApiError::Conflict(message) => assert!(
                message.contains("approved"),
                "unexpected message: {message}"
            ),
            other => panic!("expected Conflict, got {other:?}"),
        }

        assert!(state.git_info().write_calls().is_empty());
    }

    /// The A5/D51 happy path: marker write, round comment, re-open (D6), then the
    /// notification (D5).
    #[tokio::test]
    async fn test_create_round_posts_the_comment_reopens_and_notifies() {
        let state = state_with(vec![&format!("approved qc commit: {MOCK_COMMIT}")]);

        let (status, response) = create_round(
            State(state.clone()),
            Path(1),
            Json(round_request("- [ ] item\n")),
        )
        .await
        .expect("an approved QC starts a round");

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(response.round_index, 2);
        assert!(response.reopened, "the D6 re-open succeeded");
        assert!(matches!(
            response.notification,
            NotificationOutcome::Posted { .. }
        ));

        let calls = state.git_info().write_calls();

        // D51: the marker is written before anything is posted, so a crash mid-flight
        // can only ever over-fetch.
        let marker_position = calls
            .iter()
            .position(|call| matches!(call, WriteCall::UpdateIssue { .. }))
            .expect("the `## QC Rounds` marker is written");
        let first_post = calls
            .iter()
            .position(|call| matches!(call, WriteCall::PostComment { .. }))
            .expect("the round comment is posted");
        assert!(
            marker_position < first_post,
            "the marker write precedes the round comment (D51): {calls:?}"
        );

        let posted = calls
            .iter()
            .filter_map(|call| match call {
                WriteCall::PostComment { comment_type } => Some(comment_type.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            posted[0].ends_with("QCRound"),
            "the round comment is posted first: {posted:?}"
        );
        assert!(
            posted[1].ends_with("QCComment"),
            "the notification follows it: {posted:?}"
        );
        assert!(
            calls.contains(&WriteCall::OpenIssue { issue_number: 1 }),
            "starting a round re-opens the issue (D6): {calls:?}"
        );

        // The marker landed in the body as an H2 of prose, and carries no metadata key
        // anything could parse (D49.3).
        let body = crate::GitHubReader::get_issue(state.git_info(), 1)
            .await
            .unwrap()
            .body
            .unwrap_or_default();
        assert!(body.contains("## QC Rounds"), "unexpected body: {body}");
        assert!(!body.contains("# QC Rounds\n#"), "must stay an H2: {body}");
    }

    /// The notification is optional; nothing else about the round changes (D5/D45).
    #[tokio::test]
    async fn test_create_round_without_notify_posts_only_the_round_comment() {
        let state = state_with(vec![&format!("approved qc commit: {MOCK_COMMIT}")]);
        let mut request = round_request("- [ ] item\n");
        request.notify = false;

        let (_, response) = create_round(State(state.clone()), Path(1), Json(request))
            .await
            .expect("an approved QC starts a round");

        // D45: "not requested" is spelled differently from "requested but failed".
        assert_eq!(response.notification, NotificationOutcome::NotRequested);
        let posted = state
            .git_info()
            .write_calls()
            .into_iter()
            .filter(|call| matches!(call, WriteCall::PostComment { .. }))
            .count();
        assert_eq!(posted, 1);
    }

    /// D45: a failed re-open is reported, not logged. Left silent, S3 reads the closed
    /// issue with an unapproved latest round as `ApprovalRequired` — a round that was
    /// just created reporting "approval required".
    #[tokio::test]
    async fn test_create_round_reports_a_failed_reopen() {
        let issue = qc_issue();
        let mock = MockGitInfo::builder()
            .with_issue(issue.number, issue)
            .with_comments(1, vec![&format!("approved qc commit: {MOCK_COMMIT}")])
            .with_commit(MOCK_COMMIT)
            .with_branch("main")
            .with_failing_open_issue()
            .build();
        let state = AppState::new(mock, Configuration::default(), None, None);

        let (status, response) = create_round(
            State(state.clone()),
            Path(1),
            Json(round_request("- [ ] item\n")),
        )
        .await
        .expect("a failed re-open is non-fatal: the round comment is already posted");

        assert_eq!(status, StatusCode::CREATED);
        assert!(
            !response.reopened,
            "a failed re-open must be visible on the response (D45)"
        );
        // Still non-fatal: the round exists and the notification still went out.
        assert!(matches!(
            response.notification,
            NotificationOutcome::Posted { .. }
        ));
    }

    /// D51: the marker write is a **precondition**, not best-effort. If it fails the
    /// round is aborted before the comment is posted — a cheap path may skip the
    /// comment fetch when the marker is absent, so a multi-round issue that looks
    /// single-round would hand out a stale `Issue.branch`. Under-fetching is the
    /// dangerous direction.
    #[tokio::test]
    async fn test_create_round_aborts_when_the_marker_write_fails() {
        let issue = qc_issue();
        let mock = MockGitInfo::builder()
            .with_issue(issue.number, issue)
            .with_comments(1, vec![&format!("approved qc commit: {MOCK_COMMIT}")])
            .with_commit(MOCK_COMMIT)
            .with_branch("main")
            .with_failing_update_issue()
            .build();
        let state = AppState::new(mock, Configuration::default(), None, None);

        let error = create_round(
            State(state.clone()),
            Path(1),
            Json(round_request("- [ ] item\n")),
        )
        .await
        .expect_err("a failed marker write aborts the round (D51)");

        match error {
            ApiError::Internal(message) => assert!(
                message.contains("QC Rounds"),
                "unexpected message: {message}"
            ),
            other => panic!("expected Internal, got {other:?}"),
        }

        let calls = state.git_info().write_calls();
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, WriteCall::PostComment { .. })),
            "no round comment may be posted after a failed marker write: {calls:?}"
        );
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, WriteCall::OpenIssue { .. })),
            "and the issue is not re-opened either: {calls:?}"
        );
    }
}
