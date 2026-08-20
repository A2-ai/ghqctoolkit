//! Archive generation endpoint.

use axum::{
    Json,
    extract::{FromRequest, Request, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use gix::ObjectId;
use std::path::{Component, PathBuf};

use crate::{
    GitProvider, IssueThread,
    api::{
        error::ApiError,
        state::AppState,
        types::{ArchiveFileRequest, ArchiveGenerateRequest, ArchiveGenerateResponse},
    },
    archive::{ArchiveError, ArchiveFile, ArchiveMetadata, ArchiveTarget, archive, selected_round},
    get_issue_comments,
    git::GitHubApiError,
    utils::StdEnvProvider,
};

/// `Json`, with the body-shape rejection wearing this API's error envelope.
///
/// The stock extractor answers a malformed `mode` or an unknown field with **plain text**,
/// so this one route would report some client errors in a shape the UI parses
/// (`{"error": …}`) and others in one it cannot — two accounts of one kind of fact, at the
/// transport layer. Every rejection now carries the envelope.
///
/// The status and the message are axum's own: the status because the contract pins a
/// body-shape rejection at 422 (and a syntax error at 400, a missing content type at 415),
/// the message because serde names the offending field — `files[0].mode: unknown variant
/// \`bogus\`` — which is the whole value of the body to a client fixing its request.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = JsonEnvelope;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| ApiJson(value))
            .map_err(JsonEnvelope::from)
    }
}

/// A `JsonRejection` rendered in the `{"error": …}` envelope.
pub struct JsonEnvelope {
    status: StatusCode,
    message: String,
}

impl From<JsonRejection> for JsonEnvelope {
    fn from(rejection: JsonRejection) -> Self {
        Self {
            status: rejection.status(),
            message: rejection.body_text(),
        }
    }
}

impl IntoResponse for JsonEnvelope {
    fn into_response(self) -> Response {
        // Built here rather than through `ApiError`, which has no 422 and whose envelope
        // struct is private: the shape is what must match, and it is one key.
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

/// POST /api/archive/generate
///
/// Accepts an `ArchiveGenerateRequest` JSON body, builds an archive at
/// `output_path` containing each file at the specified commit, and returns
/// the resolved absolute path of the written archive.
pub async fn generate_archive<G: GitProvider + 'static>(
    State(state): State<AppState<G>>,
    ApiJson(request): ApiJson<ArchiveGenerateRequest>,
) -> Result<Json<ArchiveGenerateResponse>, ApiError> {
    if request.output_path.is_empty() {
        return Err(ApiError::BadRequest("output_path is required".to_string()));
    }

    let raw = PathBuf::from(&request.output_path);

    // Reject user-supplied paths that explicitly traverse upward
    for comp in raw.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(ApiError::BadRequest(
                "output_path must not contain '..' components".to_string(),
            ));
        }
    }

    // Canonicalize the repo root first so a relative -d ../project doesn't inject '..'
    let repo_root_for_join = state
        .git_info()
        .path()
        .canonicalize()
        .map_err(|e| ApiError::Internal(format!("Failed to canonicalize repo root: {e}")))?;
    let output_path = if raw.is_absolute() {
        raw
    } else {
        repo_root_for_join.join(&raw)
    };

    let repo_root = repo_root_for_join;

    // Create parent dir if needed, then canonicalize to validate containment
    let parent = output_path.parent().unwrap_or(&output_path);
    std::fs::create_dir_all(parent).map_err(|e| {
        ApiError::BadRequest(format!(
            "Failed to create output directory {}: {e}",
            parent.display()
        ))
    })?;
    let canonical_parent = parent.canonicalize().map_err(|e| {
        ApiError::Internal(format!(
            "Failed to canonicalize output directory {}: {e}",
            parent.display()
        ))
    })?;
    if !canonical_parent.starts_with(&repo_root) {
        return Err(ApiError::BadRequest(
            "output_path must resolve within the repository".to_string(),
        ));
    }
    let output_path = canonical_parent.join(output_path.file_name().unwrap_or_default());

    let flatten = request.flatten;
    let archive_files = build_archive_files(&state, request.files, flatten).await?;

    let env = StdEnvProvider;
    let metadata = ArchiveMetadata::new(archive_files, &env)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let git_info = state.git_info().clone();
    let output_path_clone = output_path.clone();
    tokio::task::spawn_blocking(move || archive(metadata, &git_info, &output_path_clone))
        .await
        .map_err(|e| ApiError::Internal(format!("Archive task panicked: {e}")))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ArchiveGenerateResponse {
        output_path: output_path.to_string_lossy().into_owned(),
    }))
}

/// Fetch an issue and fold its comment thread into an [`IssueThread`], as
/// `routes/rounds.rs` does.
///
/// Mode 1 names an issue and nothing else, so this endpoint now performs a read it never
/// used to: it resolves issue → thread instead of trusting a client-sent path, commit and
/// approval bit. That is where the 404 and the 502 on this endpoint come from.
async fn issue_thread<G: GitProvider + 'static>(
    state: &AppState<G>,
    number: u64,
) -> Result<IssueThread, ApiError> {
    let issue = state
        .git_info()
        .get_issue(number)
        .await
        .map_err(|error| issue_fetch_error(number, error))?;
    let comments = get_issue_comments(&issue, state.disk_cache(), state.git_info()).await?;
    Ok(IssueThread::from_issue_comments(
        &issue,
        &comments,
        state.git_info(),
        state.disk_cache(),
    )?)
}

/// A number naming no issue is the client's mistake, so it is a 404; every other GitHub
/// failure stays the 502 that a failed upstream read is everywhere else.
fn issue_fetch_error(number: u64, error: GitHubApiError) -> ApiError {
    let status = match &error {
        GitHubApiError::APIError(octocrab::Error::GitHub { source, .. }) => {
            Some(source.status_code)
        }
        _ => None,
    };
    classify_issue_fetch(number, status, || error.to_string())
}

/// Split from the extraction above only so it can be tested: `octocrab::GitHubError` is
/// `non_exhaustive`, so no test can build one, and the status is the whole of what the
/// decision turns on.
fn classify_issue_fetch(
    number: u64,
    status: Option<http::StatusCode>,
    detail: impl FnOnce() -> String,
) -> ApiError {
    if status == Some(http::StatusCode::NOT_FOUND) {
        ApiError::NotFound(format!("Issue #{number} not found"))
    } else {
        ApiError::GitHubApi(detail())
    }
}

/// The library error carries the file path, which a mode-1 request never sent — so the
/// message is rebuilt around the one handle the client gave us.
fn selection_error(number: u64, error: ArchiveError) -> ApiError {
    match error {
        ArchiveError::RoundSelection { round, rounds, .. } => ApiError::BadRequest(format!(
            "Round {round} does not exist for issue #{number}: it has {rounds} round(s)"
        )),
        // The library's own refusal for an unplaceable selected round. Unreachable on
        // this route, which gates first so it can name the round the way a client reads
        // it — mapped anyway, and to the same 400, because a gate a caller may decline to
        // consult is advisory, and this is the door rather than a second opinion.
        ArchiveError::UnplaceableRound { round, reason, .. } => ApiError::BadRequest(format!(
            "Issue #{number}: Round {round} cannot be archived: {}. Add the file with an \
             explicit commit instead.",
            reason.describe()
        )),
        // The round resolved to no commit at all — "nothing could be placed", which is a
        // different fact from an unplaceable selection. Also the last line of defence.
        ArchiveError::CommitDetermination(_) => ApiError::BadRequest(format!(
            "No commit could be determined for issue #{number}: its QC round owns no commits"
        )),
        other => ApiError::Internal(other.to_string()),
    }
}

async fn build_archive_files<G: GitProvider + 'static>(
    state: &AppState<G>,
    files: Vec<ArchiveFileRequest>,
    flatten: bool,
) -> Result<Vec<ArchiveFile>, ApiError> {
    // Every mode-1 thread in flight at once, as the status endpoint fetches its issues:
    // a 60-file milestone archive was 60 serial round trips.
    //
    // Results are collected in **request order** and consumed in request order below, so
    // the failure reported is the first entry the client listed rather than whichever
    // fetch lost the race. A parallel fetch must not make the error a function of
    // scheduling.
    let fetches = files
        .iter()
        .filter_map(|file_req| match file_req {
            ArchiveFileRequest::Issue { issue_number, .. } => Some(*issue_number),
            ArchiveFileRequest::File { .. } => None,
        })
        .map(|number| issue_thread(state, number));
    let mut threads = futures::future::join_all(fetches).await.into_iter();

    let mut archive_files = Vec::with_capacity(files.len());

    for file_req in files {
        let archive_file = match file_req {
            ArchiveFileRequest::Issue {
                issue_number,
                round,
            } => {
                // In request order: `fetches` skipped exactly the mode-2 entries this
                // arm does not consume.
                let thread = threads
                    .next()
                    .expect("one fetch was queued for every mode-1 entry")?;
                // The one derivation, shared with the CLI: the handler chooses a target
                // and nothing else. The path, the commit, the approval and `superseded`
                // all follow from the thread, so the two producers of the metadata file
                // cannot disagree about them again.
                let target = match round {
                    None => ArchiveTarget::Latest,
                    Some(round) => ArchiveTarget::Round(round),
                };
                // The gate is the **selected** round, and the predicate is the library's,
                // never a copy: this endpoint used to refuse on the *active* segment,
                // which rejected a well-formed retarget to an older, placed, closed round
                // on account of a later round the client never asked about — defeating
                // the retarget for exactly the case it exists to serve. The two surfaces
                // had also drifted to opposite answers about which segment to gate on, so
                // the shared call is what makes that unrepresentable rather than fixed.
                let selection = selected_round(&thread, target)
                    .map_err(|error| selection_error(issue_number, error))?;
                if !selection.is_archivable() {
                    return Err(ApiError::BadRequest(format!(
                        "Issue #{issue_number}: {} cannot be archived: {}. Add the file \
                         with an explicit commit instead.",
                        selection.name,
                        selection
                            .refusal()
                            .expect("a round that is not archivable has a reason")
                    )));
                }
                ArchiveFile::from_issue_thread(&thread, flatten, target)
                    .map_err(|error| selection_error(issue_number, error))?
            }
            ArchiveFileRequest::File {
                repository_file,
                commit,
            } => {
                let commit = ObjectId::from_hex(commit.as_bytes()).map_err(|e| {
                    ApiError::BadRequest(format!("Invalid commit hash '{}': {}", commit, e))
                })?;

                let archive_file_path = if flatten {
                    repository_file
                        .file_name()
                        .map(PathBuf::from)
                        .ok_or_else(|| {
                            ApiError::BadRequest(format!(
                                "File has no name: {}",
                                repository_file.display()
                            ))
                        })?
                } else {
                    // Strip all root/prefix components (e.g. leading / or ..)
                    let stripped: PathBuf = repository_file
                        .components()
                        .filter(|c| matches!(c, Component::Normal(_)))
                        .collect();
                    if stripped.as_os_str().is_empty() {
                        return Err(ApiError::BadRequest(format!(
                            "File path resolves to empty after normalization: {}",
                            repository_file.display()
                        )));
                    }
                    stripped
                };

                ArchiveFile {
                    repository_file,
                    archive_file: archive_file_path,
                    commit,
                    // A manually added file carries no QC claim at all — not an
                    // `approved: false` one.
                    qc: None,
                }
            }
        };

        archive_files.push(archive_file);
    }

    Ok(archive_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Configuration;
    use crate::GitComment;
    use crate::api::tests::helpers::MockGitInfo;
    use crate::test_utils::create_test_issue;
    use crate::utils::MockEnvProvider;

    /// Initial QC's anchor, and the older of the two walked commits.
    const A: &str = "aaaaaaa000000000000000000000000000000001";
    /// The branch tip, and Initial QC's approval in the single-round fixture.
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    /// A commit the walk never returns: an anchor nobody can reach.
    const C: &str = "ccccccc000000000000000000000000000000003";

    fn issue_body() -> String {
        format!(
            "Quality check issue for src/test.rs\n\n## Metadata\ninitial qc commit: {A}\ngit branch: main\nauthor: The Octocat <octocat@example.com>\n\n# Code Review Checklist\n- [x] Reviewed the logic\n"
        )
    }

    /// Round 2's body marker, so the round-2 fixture is not reported as needing repair.
    fn round_two_body() -> String {
        format!(
            "Quality check issue for src/test.rs\n\n## Metadata\ninitial qc commit: {A}\ngit branch: main\nauthor: The Octocat <octocat@example.com>\n\n## QC Round\n* current round: 2\n* round comment: https://github.com/o/r/issues/1#issuecomment-456\n\n# Code Review Checklist\n- [ ] Reviewed the logic\n"
        )
    }

    fn approval(commit: &str) -> GitComment {
        GitComment {
            body: format!("# QC Approval\n\n## Metadata\napproved qc commit: {commit}\n"),
            author_login: "reviewer".to_string(),
            created_at: chrono::Utc::now(),
            id: Some(7),
            html_url: Some("https://github.com/o/r/issues/1#issuecomment-7".to_string()),
            html: None,
        }
    }

    fn new_round() -> GitComment {
        new_round_at(B)
    }

    /// Round 2's comment, anchored wherever the fixture needs it.
    fn new_round_at(anchor: &str) -> GitComment {
        GitComment {
            body: format!(
                "# QC Round\n\n## Metadata\n* round: 2\n* initial qc round commit: {anchor}\n* previous approved commit: {A}\n* git branch: main\n\n# Code Review Checklist\n- [ ] Reviewed the logic\n"
            ),
            author_login: "author".to_string(),
            created_at: chrono::Utc::now(),
            id: Some(456),
            html_url: Some("https://github.com/o/r/issues/1#issuecomment-456".to_string()),
            html: None,
        }
    }

    /// The walk every branch reports, newest first. Every walked commit is treated as
    /// touching the file, so a non-empty gap always means drift.
    fn walk() -> Vec<crate::GitCommit> {
        [B, A]
            .iter()
            .map(|hash| crate::GitCommit {
                commit: gix::ObjectId::from_hex(hash.as_bytes()).expect("a 40 hex digit sha"),
                message: format!("commit {hash}"),
            })
            .collect()
    }

    fn state(mock: MockGitInfo) -> AppState<MockGitInfo> {
        AppState::new(mock, Configuration::default(), None, None)
    }

    /// One round, closed at the branch tip: approved, current, nothing trailing it.
    fn approved_and_current() -> AppState<MockGitInfo> {
        let issue = create_test_issue("o", "r", 1, "src/test.rs", &issue_body(), Some(1), "closed");
        state(
            MockGitInfo::builder()
                .with_issue(1, issue)
                .with_comments(1, vec![approval(B)])
                .with_commits(walk())
                .with_branch_tip(Some(B.to_string()))
                .build(),
        )
    }

    /// Round 1 closed at `A`; round 2 opened at `B` and is still open, with nothing
    /// notified in it.
    fn reopened() -> AppState<MockGitInfo> {
        let issue = create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            &round_two_body(),
            Some(1),
            "closed",
        );
        state(
            MockGitInfo::builder()
                .with_issue(1, issue)
                .with_comments(1, vec![approval(A), new_round()])
                .with_commits(walk())
                .with_branch_tip(Some(B.to_string()))
                .build(),
        )
    }

    fn issue_entry(round: Option<u32>) -> ArchiveFileRequest {
        ArchiveFileRequest::Issue {
            issue_number: 1,
            round,
        }
    }

    /// What the archive's metadata file carries for one mode-1 file, as JSON.
    ///
    /// Asserted on `files[0]` rather than on the whole object so the surrounding
    /// metadata fields stay free to change without pinning them here.
    fn file_metadata(files: Vec<ArchiveFile>) -> serde_json::Value {
        let mut env = MockEnvProvider::new();
        env.expect_var()
            .with(mockall::predicate::eq("USER"))
            .returning(|_| Ok("wes".to_string()));
        let metadata =
            ArchiveMetadata::new(files, &env).expect("one file cannot collide with itself");
        let json =
            serde_json::to_value(&metadata).expect("the metadata file is what gets serialized");
        json["files"][0].clone()
    }

    /// The whole point of the migration: the handler picks a target and the backend
    /// derives everything else, so the metadata carries two independent facts — the
    /// provenance of the bytes and whether they were the newest QC state — and no
    /// `approved` bool.
    #[tokio::test]
    async fn a_mode_one_file_records_the_provenance_the_backend_derived() {
        let files = build_archive_files(&approved_and_current(), vec![issue_entry(None)], false)
            .await
            .expect("an approved, placed thread archives");

        let file = file_metadata(files);
        assert_eq!(file["repository_file"], "src/test.rs");
        assert_eq!(file["archive_file"], "src/test.rs");
        // Derived from the round, never client-supplied.
        assert_eq!(file["commit"], B);
        // Read off the thread, not off the request.
        assert_eq!(file["milestone"], "v1.0");
        assert_eq!(file["round"]["round"], 1);
        assert_eq!(file["round"]["approval"]["round"], 1);
        assert_eq!(file["round"]["approval"]["commit"], B);
        assert_eq!(file["round"]["approval"]["by"], "reviewer");
        assert!(
            file["round"]["approval"]["at"].is_string(),
            "the approval carries when it landed: {file}"
        );
        // The latest round's approval with no file changes since, as of archive time.
        assert_eq!(file["round"]["superseded"], false);
        assert!(
            file.as_object()
                .expect("a file is a map")
                .get("approved")
                .is_none(),
            "the approved bool is gone from the metadata: {file}"
        );
    }

    /// The default target is the latest round, not the newest approval: a reopened file
    /// archives its open round's latest actioned commit, unapproved, and says so.
    #[tokio::test]
    async fn the_default_target_is_the_latest_round_even_when_it_is_open() {
        let files = build_archive_files(&reopened(), vec![issue_entry(None)], false)
            .await
            .expect("an open round 2 archives its latest actioned commit");

        let file = file_metadata(files);
        assert_eq!(file["round"]["round"], 2);
        assert_eq!(file["commit"], B, "round 2's anchor is what was actioned");
        // Present and null: "never approved" is the load-bearing fact here, and an
        // absent key would read as "this writer did not know".
        assert!(
            file["round"]
                .as_object()
                .expect("provenance is a map")
                .contains_key("approval"),
            "the approval key is always emitted: {file}"
        );
        assert_eq!(file["round"]["approval"], serde_json::Value::Null);
        // No approval implies superseded — the latest round being open is what makes it
        // so, and it is why an unapproved entry is never unflagged.
        assert_eq!(file["round"]["superseded"], true);
    }

    /// Retargeting to an older round is how the standing approval is archived, and the
    /// entry is flagged as no longer the newest QC state without asserting anything
    /// false about the round that is open.
    #[tokio::test]
    async fn an_older_round_can_be_addressed_and_is_flagged_superseded() {
        let files = build_archive_files(&reopened(), vec![issue_entry(Some(1))], false)
            .await
            .expect("round 1 is addressable");

        let file = file_metadata(files);
        assert_eq!(file["round"]["round"], 1);
        assert_eq!(file["commit"], A);
        assert_eq!(file["round"]["approval"]["round"], 1);
        assert_eq!(file["round"]["approval"]["commit"], A);
        assert_eq!(file["round"]["superseded"], true);
    }

    /// A round outside `1..=n` names no round. The library error carries only the path,
    /// which a mode-1 request never sent, so the message is rebuilt around the issue
    /// number the client did send.
    #[tokio::test]
    async fn a_round_beyond_the_threads_count_is_a_bad_request_naming_the_issue() {
        let error = build_archive_files(&approved_and_current(), vec![issue_entry(Some(2))], false)
            .await
            .expect_err("a one-round thread has no round 2");

        let ApiError::BadRequest(message) = error else {
            panic!("a selection outside the range is a 400, not {error:?}");
        };
        assert!(
            message.contains("#1") && message.contains("Round 2"),
            "the message must name the issue and the round: {message}"
        );
        assert!(
            !message.contains("src/test.rs"),
            "mode 1 sent no path, so the message must not key on one: {message}"
        );
    }

    /// Rounds are 1-based, so `0` is as much a non-existent round as `n + 1` is.
    #[tokio::test]
    async fn round_zero_is_a_bad_request_naming_the_issue() {
        let error = build_archive_files(&approved_and_current(), vec![issue_entry(Some(0))], false)
            .await
            .expect_err("there is no round 0");

        let ApiError::BadRequest(message) = error else {
            panic!("a selection outside the range is a 400, not {error:?}");
        };
        assert!(
            message.contains("#1") && message.contains("Round 0"),
            "the message must name the issue and the round: {message}"
        );
    }

    /// An unplaceable **selected** round owns no commits, so there is nothing an archive
    /// could honestly point at: generation is refused with the shared wording, not
    /// silently short by one file.
    #[tokio::test]
    async fn an_unplaceable_selected_round_is_refused_with_its_reason() {
        // An anchor the branch does not contain — a force-push or a gc — so the round
        // owns no commits at all.
        let body = issue_body().replace(A, C);
        let issue = create_test_issue("o", "r", 1, "src/test.rs", &body, Some(1), "open");
        let unreachable = state(
            MockGitInfo::builder()
                .with_issue(1, issue)
                .with_comments(1, Vec::new())
                .with_commits(walk())
                .with_branch_tip(Some(B.to_string()))
                .build(),
        );

        let error = build_archive_files(&unreachable, vec![issue_entry(None)], false)
            .await
            .expect_err("an unplaceable thread never reaches archive construction");

        let ApiError::BadRequest(message) = error else {
            panic!("an unplaceable thread is a 400, not {error:?}");
        };
        assert!(
            message.contains("#1")
                && message.contains("Initial QC")
                && message.contains("commits are not on that branch"),
            "the message must name the issue, the round and the shared reason: {message}"
        );
    }

    /// Round 1 closed and fully placed; round 2 open and unplaceable, its anchor off
    /// every walked branch.
    fn later_round_unplaceable() -> AppState<MockGitInfo> {
        let issue = create_test_issue(
            "o",
            "r",
            1,
            "src/test.rs",
            &round_two_body().replace(&format!("initial qc round commit: {B}"), ""),
            Some(1),
            "closed",
        );
        state(
            MockGitInfo::builder()
                .with_issue(1, issue)
                // Round 2 anchors on a commit the walk never returns.
                .with_comments(1, vec![approval(A), new_round_at(C)])
                .with_commits(walk())
                .with_branch_tip(Some(B.to_string()))
                .build(),
        )
    }

    /// The case the gate used to get wrong: an explicit retarget to an older, placed,
    /// closed round must succeed even though a *later* round cannot be placed. Refusing it
    /// rejected a well-formed request on account of a round the client never asked about,
    /// defeating retargeting for exactly the case retargeting exists to serve — moving away
    /// from a broken round to an older good approval.
    #[tokio::test]
    async fn an_older_round_is_archivable_when_a_later_round_is_unplaceable() {
        // The precondition: round 2 is the active segment and it is unplaceable, so the
        // old active-segment gate would have refused this request.
        let thread = issue_thread(&later_round_unplaceable(), 1)
            .await
            .expect("the thread folds");
        assert!(
            !thread.active_segment().is_placed(),
            "the fixture must have an unplaceable active segment"
        );

        let files = build_archive_files(
            &later_round_unplaceable(),
            vec![issue_entry(Some(1))],
            false,
        )
        .await
        .expect("round 1 is placed, closed, and exactly what the client asked for");

        let file = file_metadata(files);
        assert_eq!(file["round"]["round"], 1);
        assert_eq!(file["commit"], A);
        assert_eq!(file["round"]["approval"]["commit"], A);
        // Currency cannot be established behind an unplaceable later round, and the
        // metadata never claims it.
        assert_eq!(file["round"]["superseded"], true);

        // The same thread still refuses the *default* target, which does name round 2.
        let error = build_archive_files(&later_round_unplaceable(), vec![issue_entry(None)], false)
            .await
            .expect_err("round 2 owns no commits");
        assert!(
            matches!(error, ApiError::BadRequest(ref message) if message.contains("Round 2")),
            "unexpected error: {error:?}"
        );
    }

    /// A manually added file carries no QC claim at all — not `milestone`, not `round`,
    /// and certainly not `approved: false`, which is what the UI used to send here.
    #[tokio::test]
    async fn a_mode_two_file_carries_no_qc_claim() {
        let files = build_archive_files(
            &approved_and_current(),
            vec![ArchiveFileRequest::File {
                repository_file: PathBuf::from("scripts/helpers.R"),
                commit: B.to_string(),
            }],
            false,
        )
        .await
        .expect("a path and a commit are all mode 2 needs");

        let file = file_metadata(files);
        let keys = file.as_object().expect("a file is a map");
        assert_eq!(
            keys.len(),
            3,
            "a flattened `None` contributes zero keys: {file}"
        );
        for absent in ["milestone", "round", "approved"] {
            assert!(
                !keys.contains_key(absent),
                "{absent} must not appear on a manually added file: {file}"
            );
        }
        assert_eq!(file["commit"], B);
    }

    /// Mode 2's commit is still the client's, and still parsed as hex.
    #[tokio::test]
    async fn a_mode_two_file_with_a_bad_commit_is_a_bad_request() {
        let error = build_archive_files(
            &approved_and_current(),
            vec![ArchiveFileRequest::File {
                repository_file: PathBuf::from("scripts/helpers.R"),
                commit: "not-a-sha".to_string(),
            }],
            false,
        )
        .await
        .expect_err("mode 2 parses the commit it was given");

        assert!(
            matches!(error, ApiError::BadRequest(ref message) if message.contains("not-a-sha")),
            "unexpected error: {error:?}"
        );
    }

    /// The endpoint newly reads an issue, so an upstream read failure is reported as one
    /// — a 502, never a 404 for an issue that may well exist.
    #[tokio::test]
    async fn an_upstream_read_failure_is_a_bad_gateway() {
        let empty = state(MockGitInfo::builder().build());

        let error = build_archive_files(&empty, vec![issue_entry(None)], false)
            .await
            .expect_err("the mock has no API");

        assert!(
            matches!(error, ApiError::GitHubApi(_)),
            "an upstream failure is a 502, not {error:?}"
        );
    }

    // ── Parallel fetches, deterministic errors ───────────────────────────────

    /// Every mode-1 thread is fetched concurrently, so the reported failure must be a
    /// function of the **request**, not of which fetch lost the race. The two entries fail
    /// in different ways — one thread cannot fold (no milestone), one issue does not exist
    /// — and swapping their order swaps the reported error, which is the whole claim.
    #[tokio::test]
    async fn the_first_failing_entry_in_request_order_is_the_one_reported() {
        // Issue 2 exists but carries no milestone, so its thread cannot fold; issue 9 is
        // not in the repository at all.
        let unfoldable =
            create_test_issue("o", "r", 2, "src/test.rs", &issue_body(), None, "closed");
        let mock = MockGitInfo::builder()
            .with_issue(2, unfoldable)
            .with_comments(2, vec![approval(B)])
            .with_commits(walk())
            .with_branch_tip(Some(B.to_string()))
            .build();
        let entry = |number: u64| ArchiveFileRequest::Issue {
            issue_number: number,
            round: None,
        };

        let error = build_archive_files(&state(mock.clone()), vec![entry(2), entry(9)], false)
            .await
            .expect_err("both entries fail");
        assert!(
            matches!(error, ApiError::Internal(_)),
            "the first entry's failure is the one reported, not {error:?}"
        );

        let error = build_archive_files(&state(mock), vec![entry(9), entry(2)], false)
            .await
            .expect_err("both entries fail");
        assert!(
            matches!(error, ApiError::GitHubApi(_)),
            "reversing the request reverses the answer, not {error:?}"
        );
    }

    /// Mode-2 entries queue no fetch, so the results must still line up with the mode-1
    /// entries when the two kinds are interleaved.
    #[tokio::test]
    async fn interleaved_modes_keep_their_threads_in_request_order() {
        let files = build_archive_files(
            &approved_and_current(),
            vec![
                ArchiveFileRequest::File {
                    repository_file: PathBuf::from("scripts/helpers.R"),
                    commit: B.to_string(),
                },
                issue_entry(None),
            ],
            false,
        )
        .await
        .expect("a mode-2 entry before a mode-1 entry archives both");

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].repository_file, PathBuf::from("scripts/helpers.R"));
        assert!(files[0].qc.is_none());
        assert_eq!(files[1].repository_file, PathBuf::from("src/test.rs"));
        assert!(
            files[1].qc.is_some(),
            "the thread landed on the right entry"
        );
    }

    // ── The error envelope, through the real router ──────────────────────────

    /// Axum's own `Json` rejection is plain text, so a malformed `mode` used to answer in a
    /// shape the UI cannot parse while every other client error on this route answered in
    /// `{"error": …}`. Asserted through the router the server builds, since the extractor
    /// is where the rejection is produced.
    #[tokio::test]
    async fn a_malformed_mode_is_rejected_in_the_error_envelope() {
        use axum::body::Body;
        use axum::extract::Request as HttpRequest;
        use tower::ServiceExt;

        let app = crate::api::server::create_router::<_, crate::GitCommand>(approved_and_current());
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/archive/generate")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"output_path": "out.tar.gz", "flatten": false,
                            "files": [{"mode": "bogus", "issue_number": 1}]}"#,
                    ))
                    .expect("a well-formed HTTP request"),
            )
            .await
            .expect("the router answers");

        // The status the contract pins for a body-shape rejection, unchanged.
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a bounded body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("the body is this API's error envelope");
        let message = json["error"]
            .as_str()
            .expect("the envelope carries one `error` key");
        assert!(
            message.contains("mode") && message.contains("bogus"),
            "serde's message names the offending field and value: {message}"
        );
    }

    /// The same envelope for an unknown field, which is how a client still sending the
    /// deleted `approved` bit finds out.
    #[tokio::test]
    async fn an_unknown_field_is_rejected_in_the_error_envelope() {
        use axum::body::Body;
        use axum::extract::Request as HttpRequest;
        use tower::ServiceExt;

        let app = crate::api::server::create_router::<_, crate::GitCommand>(approved_and_current());
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/archive/generate")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"output_path": "out.tar.gz", "flatten": false,
                            "files": [{"mode": "issue", "issue_number": 1, "round": null,
                                       "approved": false}]}"#,
                    ))
                    .expect("a well-formed HTTP request"),
            )
            .await
            .expect("the router answers");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a bounded body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("the body is this API's error envelope");
        assert!(
            json["error"]
                .as_str()
                .expect("the envelope carries one `error` key")
                .contains("approved"),
            "the rejection must name the deleted field: {json}"
        );
    }

    /// A syntax error keeps axum's 400 and gains the envelope too: one shape for every
    /// client error on this route.
    #[tokio::test]
    async fn malformed_json_keeps_its_status_and_gains_the_envelope() {
        use axum::body::Body;
        use axum::extract::Request as HttpRequest;
        use tower::ServiceExt;

        let app = crate::api::server::create_router::<_, crate::GitCommand>(approved_and_current());
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/archive/generate")
                    .header("content-type", "application/json")
                    .body(Body::from("{not json"))
                    .expect("a well-formed HTTP request"),
            )
            .await
            .expect("the router answers");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a bounded body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("the body is this API's error envelope");
        assert!(json["error"].is_string(), "one `error` key: {json}");
    }

    /// The fourth and last rejection class: a body that never reached serde at all. It is
    /// the only status this route passes straight through from axum that no other test
    /// pins, so a refactor hardcoding one status inside the wrapper would break 415 while
    /// the 400 and 422 tests above stayed green.
    #[tokio::test]
    async fn a_wrong_content_type_is_rejected_in_the_error_envelope() {
        use axum::body::Body;
        use axum::extract::Request as HttpRequest;
        use tower::ServiceExt;

        let app = crate::api::server::create_router::<_, crate::GitCommand>(approved_and_current());
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/archive/generate")
                    .header("content-type", "text/plain")
                    // Well-formed JSON: the body is never the reason for this rejection.
                    .body(Body::from(
                        r#"{"output_path": "out.tar.gz", "flatten": false, "files": []}"#,
                    ))
                    .expect("a well-formed HTTP request"),
            )
            .await
            .expect("the router answers");

        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a bounded body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("the body is this API's error envelope");
        let message = json["error"]
            .as_str()
            .expect("the envelope carries one `error` key");
        assert!(
            message.contains("Content-Type") && message.contains("application/json"),
            "axum's own message says what the client should have sent: {message}"
        );
    }

    /// The fifth rejection class, found while auditing the endpoint's documented statuses
    /// against the ones it can actually produce: a body over axum's default 2 MB limit is
    /// a `BytesRejection`, which is a **413**, not one of the three statuses the other
    /// envelope tests cover. It reaches the client through the same wrapper, so it wears
    /// the same envelope — pinned here because the schema now lists it.
    #[tokio::test]
    async fn an_oversized_body_is_rejected_in_the_error_envelope() {
        use axum::body::Body;
        use axum::extract::Request as HttpRequest;
        use tower::ServiceExt;

        // Valid JSON, and irrelevant: the body never reaches serde.
        let padding = "x".repeat(2 * 1024 * 1024 + 1);
        let body = format!(
            r#"{{"output_path": "out.tar.gz", "flatten": false, "files": [],
                 "padding": "{padding}"}}"#
        );
        let app = crate::api::server::create_router::<_, crate::GitCommand>(approved_and_current());
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/archive/generate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("a well-formed HTTP request"),
            )
            .await
            .expect("the router answers");

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("a bounded body");
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the body is this API's error envelope");
        assert!(
            json["error"]
                .as_str()
                .expect("the envelope carries one `error` key")
                .contains("length limit"),
            "axum's own message says why the body was refused: {json}"
        );
    }

    /// A missing issue is the client's mistake, so it is a 404 with the number it named;
    /// every other upstream status stays a 502.
    #[test]
    fn an_issue_that_does_not_exist_is_a_not_found() {
        let error = classify_issue_fetch(9, Some(http::StatusCode::NOT_FOUND), || {
            "Not Found".to_string()
        });
        assert!(
            matches!(error, ApiError::NotFound(ref message) if message.contains("#9")),
            "unexpected error: {error:?}"
        );

        let error = classify_issue_fetch(9, Some(http::StatusCode::FORBIDDEN), || {
            "rate limited".to_string()
        });
        assert!(
            matches!(error, ApiError::GitHubApi(ref message) if message.contains("rate limited")),
            "unexpected error: {error:?}"
        );
    }
}
