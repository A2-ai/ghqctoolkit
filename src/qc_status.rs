use gix::ObjectId;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::LazyLock;

use crate::GitCommitOps;
use crate::cache::DiskCache;
use crate::git::{GitHubApiError, GitHubReader};
use crate::issue::{BlockingQC, IssueError, IssueThread};

static CHECKLIST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*-\s*\[([xX\s])\]").expect("Failed to compile checklist regex")
});

#[derive(Debug, Clone)]
pub enum QCStatus {
    Approved,
    ChangesAfterApproval(ObjectId),
    // closed without approval
    ApprovalRequired,
    // latest comment = latest commit
    AwaitingReview,
    // latest comment = latest commit, but reviewed
    ChangeRequested,
    InProgress,
    ChangesToComment(ObjectId),
}

impl std::fmt::Display for QCStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status_text = match self {
            QCStatus::Approved => "Approved",
            QCStatus::ChangesAfterApproval(_) => "Approved; subsequent file changes",
            QCStatus::AwaitingReview => "Awaiting review",
            QCStatus::InProgress => "In progress",
            QCStatus::ApprovalRequired => "Approval required",
            QCStatus::ChangeRequested => "Changes requested",
            QCStatus::ChangesToComment(_) => "Changes to comment",
        };
        write!(f, "{}", status_text)
    }
}

impl QCStatus {
    /// Round-scoped status (§5). `R = latest_round()`.
    ///
    /// S0: dispatch on `R.state` **first** — `R.is_closed()` gates S1/S2,
    /// `!R.is_closed()` gates S3/S4. Drift emptiness is never used to infer which
    /// branch applies: an open round's drift is also empty (I14), so inferring from
    /// emptiness would report `Approved` for an unreviewed QC.
    ///
    /// S7: `Superseded` never reaches here — under D27 the last round is by
    /// definition never superseded.
    ///
    /// **D55: this refuses rather than guesses.** It requires a *placed* latest round
    /// whose representative commit resolves (`Round::latest_commit()`, D54); when it does
    /// not, the caller gets the error from `Round::unresolved_commit_error()` — D60's
    /// `LocalBranchNotFound` or `ApprovalNotOnBranch`, both of which the status endpoint
    /// renders as `IssueStatusError { kind: branch_not_local, branch }`.
    /// `QCStatus` gains **no** variant for it — S5 still holds. Per D10 only the *latest*
    /// round matters here, so an earlier unplaceable round does not block status.
    pub fn determine_status(issue_thread: &IssueThread) -> Result<Self, IssueError> {
        let round = issue_thread.latest_round();

        // Either the round could not be placed (D54.1) or its approval is no longer
        // among the commits it owns (D54.2). Both would otherwise be answered with a
        // status derived from a commit set the round does not really own.
        // D60: `Round::unresolved_commit_error` names the branch in both cases and tells
        // them apart in the message — "fetch this branch" is wrong for a rewritten
        // approval — while both still classify as `branch_not_local` for clients.
        if let Some(error) = round.unresolved_commit_error() {
            return Err(error);
        }

        Ok(if round.is_closed() {
            // S1/S2: read `thread.drift` only. D10 forbids reading an earlier round
            // or any `preceding_gap`.
            match issue_thread.drift.newest_file_change() {
                // S1
                Some(hash) => Self::ChangesAfterApproval(*hash),
                // S2 (including an empty drift)
                None => Self::Approved,
            }
        } else if !issue_thread.open {
            // S3: not approved and the issue is closed.
            Self::ApprovalRequired
        } else {
            // S4: the pre-rounds algorithm **verbatim**, now scoped to `R.commits`
            // (D8) rather than the whole thread — this is the §0.5 fix. Verbatim
            // includes the `unwrap_or(false)` "no status commits at all ⇒
            // ChangesToComment" edge and all three arms of the `None` case; D33/R26
            // keep `CommitStatus::Initial` in storage precisely so an `Initial`-only
            // commit still counts as status-bearing here.
            let commits = &round.commits;

            // Find the newest (lowest index) file-changing commit.
            // Commits are stored newest-first, so lower index = more recent.
            let latest_file_entry = commits.iter().enumerate().find(|(_, c)| c.file_changed);

            match latest_file_entry {
                Some((file_idx, latest_fc)) => {
                    // Find the newest commit that carries any status.
                    // A status commit "covers" the file change if it is at the same
                    // position or newer (index ≤ file_idx).
                    let latest_status_entry = commits
                        .iter()
                        .enumerate()
                        .find(|(_, c)| !c.statuses.is_empty());

                    let covered = latest_status_entry
                        .map(|(si, _)| si <= file_idx)
                        .unwrap_or(false);

                    if covered {
                        let status_commit = latest_status_entry.unwrap().1;
                        if status_commit
                            .statuses
                            .contains(&crate::issue::CommitStatus::Reviewed)
                        {
                            Self::ChangeRequested
                        } else {
                            Self::AwaitingReview
                        }
                    } else {
                        Self::ChangesToComment(latest_fc.hash)
                    }
                }
                None => {
                    // No file-changing commit found, but the issue has been posted
                    // (an Initial/Notification commit exists). Treat like a covered
                    // status commit: awaiting review unless already reviewed.
                    let latest_status_entry = commits.iter().find(|c| !c.statuses.is_empty());
                    match latest_status_entry {
                        Some(sc) if sc.statuses.contains(&crate::issue::CommitStatus::Reviewed) => {
                            Self::ChangeRequested
                        }
                        Some(_) => Self::AwaitingReview,
                        None => Self::InProgress,
                    }
                }
            }
        })
    }

    /// Returns true if this status represents an approved issue
    /// (either pure Approved or ChangesAfterApproval)
    pub fn is_approved(&self) -> bool {
        matches!(self, QCStatus::Approved | QCStatus::ChangesAfterApproval(_))
    }

    pub async fn from_blocking_qc(
        blocking_qc: &BlockingQC,
        cache: Option<&DiskCache>,
        git_info: &(impl GitHubReader + GitCommitOps),
    ) -> Result<Self, QCStatusError> {
        let issue = git_info.get_issue(blocking_qc.issue_number).await?;
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        let status = QCStatus::determine_status(&issue_thread)?;
        Ok(status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistSummary {
    pub completed: usize,
    pub total: usize,
}

impl ChecklistSummary {
    pub fn new(completed: usize, total: usize) -> Self {
        Self { completed, total }
    }

    pub fn completion_percentage(&self) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            (self.completed as f64 / self.total as f64) * 100.0
        }
    }

    pub fn is_complete(&self) -> bool {
        self.completed == self.total && self.total > 0
    }

    pub fn sum<'a, I>(summaries: I) -> Self
    where
        I: IntoIterator<Item = &'a Self>,
    {
        let mut total_completed = 0;
        let mut total_items = 0;

        for summary in summaries {
            total_completed += summary.completed;
            total_items += summary.total;
        }

        Self::new(total_completed, total_items)
    }
}

impl std::fmt::Display for ChecklistSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} ({:.1}%)",
            self.completed,
            self.total,
            self.completion_percentage()
        )
    }
}

/// Analyze checklists within an issue's body
/// Returns a vector of (checklist_name, summary) tuples
pub fn analyze_issue_checklists(issue_body: Option<&str>) -> Vec<(String, ChecklistSummary)> {
    let body = match issue_body {
        Some(body) => body,
        None => return vec![],
    };

    let mut checklists = Vec::new();

    // Split body into sections by headers (any level # to ######)
    let sections = split_body_into_sections(body);

    for (section_name, section_content) in sections {
        let summary = analyze_checklist_in_text(&section_content);

        // Only include sections that have checklist items
        if summary.total > 0 {
            checklists.push((section_name, summary));
        }
    }

    checklists
}

/// Split the issue body into sections based on markdown headers
/// Only processes content starting from the first level 1 header (ignoring Metadata section)
fn split_body_into_sections(body: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_section = String::new();
    let mut current_header: Option<String> = None;
    let mut found_first_level1_header = false;

    for line in body.lines() {
        if let Some(header_text) = extract_header_text(line) {
            let is_level1_header =
                line.trim_start().starts_with("# ") && !line.trim_start().starts_with("## ");

            // Only start processing after we find the first level 1 header
            if !found_first_level1_header && !is_level1_header {
                continue; // Skip non-level-1 headers before the first level 1 header
            }

            if !found_first_level1_header && is_level1_header {
                found_first_level1_header = true;
            }

            // Save the previous section if it has content and a header
            if found_first_level1_header {
                if let Some(ref header) = current_header {
                    if !current_section.trim().is_empty() {
                        sections.push((header.clone(), current_section.clone()));
                    }
                }
            }

            // Start new section
            current_header = Some(header_text);
            current_section.clear();
        } else if found_first_level1_header {
            // Only collect content after we've found the first level 1 header
            current_section.push_str(line);
            current_section.push('\n');
        }
        // Ignore everything before the first level 1 header (like Metadata section)
    }

    // Don't forget the last section
    if found_first_level1_header {
        if let Some(header) = current_header {
            if !current_section.trim().is_empty() {
                sections.push((header, current_section));
            }
        }
    }

    sections
}

/// Extract header text from a line if it's a markdown header (# to ######)
/// Returns None if the line is not a valid header
fn extract_header_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start();

    if !trimmed.starts_with('#') {
        return None;
    }

    // Count the number of # symbols at the start
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();

    // Must be 1-6 # symbols followed by a space
    if hash_count < 1 || hash_count > 6 || trimmed.chars().nth(hash_count) != Some(' ') {
        return None;
    }

    // Extract the text after the # symbols and space
    let header_text = trimmed
        .chars()
        .skip(hash_count + 1)
        .collect::<String>()
        .trim()
        .to_string();

    if header_text.is_empty() {
        None
    } else {
        Some(header_text)
    }
}

/// Analyze checklist items in a text block
/// Recognizes patterns like:
/// - [ ] Unchecked item
/// - [x] Checked item
/// - [X] Checked item
pub(crate) fn analyze_checklist_in_text(text: &str) -> ChecklistSummary {
    let mut total = 0;
    let mut completed = 0;

    for capture in CHECKLIST_REGEX.captures_iter(text) {
        total += 1;

        // Check if the item is marked as complete
        if let Some(checkbox) = capture.get(1) {
            let checkbox_content = checkbox.as_str().trim();
            if checkbox_content.eq_ignore_ascii_case("x") {
                completed += 1;
            }
        }
    }

    ChecklistSummary::new(completed, total)
}

/// Status of blocking QC issues for a given issue
///
/// Contains three HashMaps categorizing blocking QCs by their status:
/// - `approved`: Issue numbers and file names of approved blocking QCs
/// - `not_approved`: Issue numbers, file names, and status descriptions of unapproved blocking QCs
/// - `errors`: Issue numbers and errors encountered while fetching status
#[derive(Debug, Clone, Default)]
pub struct BlockingQCStatus {
    /// Blocking QC issues that are approved
    pub approved: HashMap<u64, PathBuf>,
    /// Blocking QC issues that are not approved (issue_number -> (file_name, status_description))
    pub not_approved: HashMap<u64, (PathBuf, QCStatus)>,
    /// Blocking QC issues where status could not be determined
    pub errors: HashMap<u64, String>,
}

impl BlockingQCStatus {
    /// Check if all blocking QCs are approved
    pub fn all_approved(&self) -> bool {
        self.not_approved.is_empty() && self.errors.is_empty()
    }

    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Total number of blocking QCs
    pub fn total(&self) -> usize {
        self.approved.len() + self.not_approved.len() + self.errors.len()
    }

    /// Number of approved blocking QCs
    pub fn approved_count(&self) -> usize {
        self.approved.len()
    }

    /// Number of errors
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Format as a summary string for milestone status table
    /// Returns "-" if no blocking QCs, otherwise "approved/total (percent%)" with optional error suffix
    pub fn as_summary_string(&self) -> String {
        let total = self.total();
        if total == 0 {
            return "-".to_string();
        }

        let approved = self.approved_count();
        let percent = (approved as f64 / total as f64) * 100.0;
        let error_suffix = if self.has_errors() {
            format!(" (+{} err)", self.error_count())
        } else {
            String::new()
        };

        format!("{}/{} ({:.1}%){}", approved, total, percent, error_suffix)
    }
}

impl fmt::Display for BlockingQCStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.total() == 0 {
            return write!(f, "No blocking QCs");
        }

        writeln!(f, "Blocking QCs:")?;

        for (issue_num, file_name) in &self.approved {
            writeln!(
                f,
                "  ✅ #{} - {} (Approved)",
                issue_num,
                file_name.display()
            )?;
        }

        for (issue_num, (file_name, status)) in &self.not_approved {
            writeln!(
                f,
                "  ❌ #{} - {} ({})",
                issue_num,
                file_name.display(),
                status
            )?;
        }

        for (issue_num, error) in &self.errors {
            writeln!(f, "  ⚠️ #{} - Error: {}", issue_num, error)?;
        }

        Ok(())
    }
}

/// Get the approval status for a list of blocking QCs.
///
/// This function works directly with a slice of `BlockingQC` without requiring an `IssueThread`,
/// which allows it to be used when IssueThread construction might fail (e.g., missing metadata).
pub async fn get_blocking_qc_status(
    blocking_qcs: &[BlockingQC],
    git_info: &(impl GitHubReader + GitCommitOps),
    cache: Option<&DiskCache>,
) -> BlockingQCStatus {
    let mut status = BlockingQCStatus::default();

    if blocking_qcs.is_empty() {
        return status;
    }

    for qc in blocking_qcs {
        log::debug!(
            "Getting status for #{} - {}",
            qc.issue_number,
            qc.file_name.display()
        );
        let result = QCStatus::from_blocking_qc(qc, cache, git_info).await;
        match result {
            Ok(s) => {
                if s.is_approved() {
                    status
                        .approved
                        .insert(qc.issue_number, qc.file_name.clone());
                } else {
                    status
                        .not_approved
                        .insert(qc.issue_number, (qc.file_name.clone(), s));
                }
            }
            Err(e) => {
                status.errors.insert(qc.issue_number, e.to_string());
            }
        }
    }

    status
}

#[derive(Debug, thiserror::Error)]
pub enum QCStatusError {
    #[error("Failed to determine commits for issue due to: {0}")]
    IssueError(#[from] IssueError),
    #[error(transparent)]
    ApiError(#[from] GitHubApiError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_analyze_complex_issue_checklist() {
        let issue_body = include_str!("tests/qc_status/complex_issue_checklist.md");
        let result = analyze_issue_checklists(Some(issue_body));
        insta::assert_debug_snapshot!(result);
    }

    #[test]
    fn test_change_requested_status_matrix() {
        use crate::issue::{
            Approval, CommitStatus, Gap, IssueCommit, IssueThread, Round, RoundChecklist,
            RoundPlacement, RoundState,
        };
        use gix::ObjectId;
        use std::path::PathBuf;
        use std::str::FromStr;

        fn make_statuses(statuses: &[CommitStatus]) -> std::collections::HashSet<CommitStatus> {
            statuses.iter().cloned().collect()
        }

        let test_cases = vec![
            // (scenario_name, commits: [(state, file_changed, reviewed)], issue_open, expected_status)
            (
                "I/R -> C: AwaitingReview",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (make_statuses(&[CommitStatus::Notification]), true, false),
                ],
                true,
                "AwaitingReview",
            ),
            (
                "I/R -> C/R: ChangeRequested",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (
                        make_statuses(&[CommitStatus::Notification, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                ],
                true,
                "ChangeRequested",
            ),
            (
                "I/R -> R: ChangeRequested",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (make_statuses(&[CommitStatus::Reviewed]), true, false),
                ],
                true,
                "ChangeRequested",
            ),
            (
                "I/R -> R -> C: AwaitingReview",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (make_statuses(&[CommitStatus::Reviewed]), false, false), // reviewed but no file change
                    (make_statuses(&[CommitStatus::Notification]), true, false), // new notification with file change
                ],
                true,
                "AwaitingReview",
            ),
            (
                "I/R -> A/R: Approved",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (make_statuses(&[CommitStatus::Reviewed]), true, true),
                ],
                true,
                "Approved",
            ),
            (
                "I/R -> A -> R: ChangesAfterApproval",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (make_statuses(&[]), true, true),
                    (make_statuses(&[CommitStatus::Initial]), true, false), // file change after approval (use Initial for uncommitted files)
                ],
                true,
                "ChangesAfterApproval",
            ),
            (
                "I/R -> C -> N: ChangesToComment",
                vec![
                    (
                        make_statuses(&[CommitStatus::Initial, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                    (make_statuses(&[CommitStatus::Notification]), true, false), // latest comment
                    (make_statuses(&[]), true, false), // new file changes, not commented or reviewed
                ],
                true,
                "ChangesToComment",
            ),
            (
                "I -> C_nofile: AwaitingReview (notification on non-file-changing commit after last file change)",
                vec![
                    (make_statuses(&[CommitStatus::Initial]), true, false), // oldest: file change with initial
                    (make_statuses(&[CommitStatus::Notification]), false, false), // newest: notification, no file change
                ],
                true,
                "AwaitingReview",
            ),
            (
                "I -> R_nofile: ChangeRequested (review on non-file-changing commit after last file change)",
                vec![
                    (make_statuses(&[CommitStatus::Initial]), true, false), // oldest: file change
                    (make_statuses(&[CommitStatus::Reviewed]), false, false), // newest: reviewed, no file change
                ],
                true,
                "ChangeRequested",
            ),
            (
                "I -> C_nofile -> nofile_nostatuses: ChangesToComment (uncommitted changes after notification-on-non-file-commit)",
                vec![
                    (make_statuses(&[CommitStatus::Initial]), true, false), // oldest: file change
                    (make_statuses(&[CommitStatus::Notification]), false, false), // middle: notification, no file change
                    (make_statuses(&[]), true, false), // newest: new file change, no status
                ],
                true,
                "ChangesToComment",
            ),
            (
                "Closed without approval: ApprovalRequired",
                vec![
                    (make_statuses(&[CommitStatus::Initial]), true, false),
                    (
                        make_statuses(&[CommitStatus::Notification, CommitStatus::Reviewed]),
                        true,
                        false,
                    ),
                ],
                false, // issue closed
                "ApprovalRequired",
            ),
            (
                "I/A: Approved",
                vec![(make_statuses(&[CommitStatus::Initial]), false, true)],
                false,
                "Approved",
            ),
        ];

        for (scenario, commit_data, issue_open, expected_status) in test_cases {
            // Oldest-first, as authored.
            let authored: Vec<(IssueCommit, bool)> = commit_data
                .into_iter()
                .enumerate()
                .map(|(i, (statuses, file_changed, approved))| {
                    (
                        IssueCommit {
                            hash: ObjectId::from_str(&format!("{:040x}", i + 1)).unwrap(),
                            message: format!("Commit {}", i + 1),
                            statuses,
                            file_changed,
                        },
                        approved,
                    )
                })
                .collect();

            // D8: the round owns `[start ..= approval]`; everything after the
            // approval is the thread's drift. With no approval the round owns
            // `[start .. tip]` and the drift is empty (I14).
            let approved_position = authored.iter().position(|(_, approved)| *approved);
            let (round_commits, drift_commits) = match approved_position {
                Some(position) => (&authored[..=position], &authored[position + 1..]),
                None => (&authored[..], &authored[authored.len()..]),
            };
            let newest_first = |slice: &[(IssueCommit, bool)]| -> Vec<IssueCommit> {
                slice
                    .iter()
                    .rev()
                    .map(|(commit, _)| commit.clone())
                    .collect()
            };
            let round_commits = newest_first(round_commits);
            let state = match approved_position {
                Some(_) => RoundState::Approved(Approval {
                    commit: round_commits[0].hash,
                    comment_id: None,
                }),
                None => RoundState::Unapproved,
            };

            let issue_thread = IssueThread {
                file: PathBuf::from("test.rs"),
                milestone: "milestone".to_string(),
                open: issue_open,
                blocking_qcs: vec![],
                rounds: vec![Round {
                    index: 1,
                    branch: "main".to_string(),
                    branch_inherited: false,
                    placement: RoundPlacement::Placed,
                    start_commit: round_commits.last().unwrap().hash,
                    preceding_gap: Gap::default(),
                    checklist: RoundChecklist::default(),
                    commits: round_commits,
                    state,
                }],
                drift: Gap {
                    commits: newest_first(drift_commits),
                    divergent: false,
                },
            };

            let status = QCStatus::determine_status(&issue_thread).expect("placed round");
            let actual_status = match status {
                QCStatus::Approved => "Approved",
                QCStatus::ChangesAfterApproval(_) => "ChangesAfterApproval",
                QCStatus::AwaitingReview => "AwaitingReview",
                QCStatus::ChangeRequested => "ChangeRequested",
                QCStatus::InProgress => "InProgress",
                QCStatus::ApprovalRequired => "ApprovalRequired",
                QCStatus::ChangesToComment(_) => "ChangesToComment",
            };

            assert_eq!(
                actual_status, expected_status,
                "Failed for scenario: {}. Expected {}, got {}",
                scenario, expected_status, actual_status
            );
        }
    }

    // §5 round-scoped status. `thread_with` builds the minimal legal shape: one
    // round plus the thread's drift, so each test names exactly the two inputs S0
    // dispatches on — `RoundState` and `drift` — and nothing else.
    mod round_scoped_status {
        use super::*;
        use crate::issue::{
            Approval, CommitStatus, Gap, IssueCommit, IssueThread, Round, RoundChecklist,
            RoundPlacement, RoundState,
        };

        fn oid(n: u8) -> ObjectId {
            ObjectId::from_str(&format!("{:040x}", n)).unwrap()
        }

        fn commit(n: u8, statuses: &[CommitStatus], file_changed: bool) -> IssueCommit {
            IssueCommit {
                hash: oid(n),
                message: format!("Commit {}", n),
                statuses: statuses.iter().cloned().collect(),
                file_changed,
            }
        }

        /// `round_commits` and `drift_commits` are newest-first, per D8/M4.
        fn thread_with(
            issue_open: bool,
            state: RoundState,
            round_commits: Vec<IssueCommit>,
            drift_commits: Vec<IssueCommit>,
        ) -> IssueThread {
            IssueThread {
                file: PathBuf::from("test.rs"),
                milestone: "milestone".to_string(),
                open: issue_open,
                blocking_qcs: vec![],
                rounds: vec![Round {
                    index: 1,
                    branch: "main".to_string(),
                    branch_inherited: false,
                    placement: RoundPlacement::Placed,
                    start_commit: round_commits.last().unwrap().hash,
                    preceding_gap: Gap::default(),
                    checklist: RoundChecklist::default(),
                    commits: round_commits,
                    state,
                }],
                drift: Gap {
                    commits: drift_commits,
                    divergent: false,
                },
            }
        }

        fn approved_at(n: u8) -> RoundState {
            RoundState::Approved(Approval {
                commit: oid(n),
                comment_id: Some(7),
            })
        }

        /// S1: closed round, drift carries a `file_changed` commit. The hash reported
        /// is the *newest* such commit in drift — drift is newest-first, and the
        /// newer commit here does not touch the file, so the scan must skip it.
        #[test]
        fn s1_changes_after_approval_from_nonempty_drift() {
            let thread = thread_with(
                true,
                approved_at(1),
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![commit(3, &[], false), commit(2, &[], true)],
            );

            match QCStatus::determine_status(&thread).expect("placed round") {
                QCStatus::ChangesAfterApproval(hash) => assert_eq!(hash, oid(2)),
                other => panic!("expected ChangesAfterApproval, got {:?}", other),
            }
        }

        /// S2: closed round, empty drift.
        #[test]
        fn s2_approved_from_empty_drift() {
            let thread = thread_with(
                true,
                approved_at(1),
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![],
            );

            assert!(matches!(
                QCStatus::determine_status(&thread),
                Ok(QCStatus::Approved)
            ));
        }

        /// S2 again: a non-empty drift with no `file_changed` commit is still
        /// `Approved`. Emptiness is not the test — `file_changed` is.
        #[test]
        fn s2_approved_from_drift_without_file_changes() {
            let thread = thread_with(
                true,
                approved_at(1),
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![commit(2, &[], false)],
            );

            assert!(matches!(
                QCStatus::determine_status(&thread),
                Ok(QCStatus::Approved)
            ));
        }

        /// **The S0 trap.** An open round's drift is empty too (I14), so a dispatch
        /// on drift emptiness instead of `RoundState` reports `Approved` for a QC
        /// nobody has reviewed. This unreviewed round must reach S4.
        #[test]
        fn s0_open_round_with_empty_drift_is_not_approved() {
            let thread = thread_with(
                true,
                RoundState::Unapproved,
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![],
            );

            let status = QCStatus::determine_status(&thread).expect("placed round");
            assert!(
                !status.is_approved(),
                "an unapproved round must never report approved; got {:?}",
                status
            );
            assert!(
                matches!(status, QCStatus::AwaitingReview),
                "got {:?}",
                status
            );
        }

        /// S3: unapproved round, issue closed.
        #[test]
        fn s3_approval_required_when_issue_closed_unapproved() {
            let thread = thread_with(
                false,
                RoundState::Unapproved,
                vec![
                    commit(2, &[CommitStatus::Notification], true),
                    commit(1, &[CommitStatus::Initial], true),
                ],
                vec![],
            );

            assert!(matches!(
                QCStatus::determine_status(&thread),
                Ok(QCStatus::ApprovalRequired)
            ));
        }

        /// D55: `determine_status` refuses rather than guessing when the latest round
        /// is unplaceable. The error carries the branch, which the status endpoint
        /// renders as `IssueStatusError { kind: branch_not_local, branch }` — and
        /// `QCStatus` gains no variant for it (S5).
        #[test]
        fn d55_status_refuses_an_unplaceable_latest_round() {
            let mut thread = thread_with(
                true,
                RoundState::Unapproved,
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![],
            );
            thread.rounds[0].placement = RoundPlacement::Unplaceable {
                branch: "feature".to_string(),
            };
            thread.rounds[0].commits.clear();

            match QCStatus::determine_status(&thread) {
                Err(IssueError::LocalBranchNotFound(branch)) => assert_eq!(branch, "feature"),
                other => panic!("expected LocalBranchNotFound, got {other:?}"),
            }
        }

        /// D55, the other `None` case (D54.2): the latest round is placed and approved,
        /// but its approval is no longer among the commits it owns. Status refuses; it
        /// must never answer from a substituted commit.
        ///
        /// D60: and it refuses with `ApprovalNotOnBranch`, **not**
        /// `LocalBranchNotFound` — the branch is local, so "fetch it" would be wrong.
        #[test]
        fn d60_status_refuses_an_off_branch_approval_with_a_distinct_error() {
            let thread = thread_with(
                true,
                RoundState::Approved(Approval {
                    commit: oid(9),
                    comment_id: None,
                }),
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![],
            );

            let error = QCStatus::determine_status(&thread).unwrap_err();
            let IssueError::ApprovalNotOnBranch { commit, branch } = &error else {
                panic!("expected ApprovalNotOnBranch, got {error:?}")
            };
            assert_eq!(*commit, oid(9));
            assert_eq!(branch, "main");
            // The message must not tell the user to fetch a branch they already have.
            let message = error.to_string();
            assert!(!message.contains("not checked out locally"), "{message}");
            assert!(message.contains("no longer reachable"), "{message}");
            assert!(message.contains("rewritten"), "{message}");
            // D60: still one case for clients — `branch_not_local` + the branch.
            assert_eq!(error.branch_not_local(), Some("main"));
        }

        /// D60: the *other* `None` case keeps `LocalBranchNotFound`'s message — the
        /// branch really is missing there — and the two classify identically on the wire.
        #[test]
        fn d60_the_two_unresolved_cases_differ_in_message_only() {
            let mut unplaceable = thread_with(
                true,
                RoundState::Unapproved,
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![],
            );
            unplaceable.rounds[0].placement = RoundPlacement::Unplaceable {
                branch: "feature".to_string(),
            };
            unplaceable.rounds[0].commits.clear();

            let off_branch = thread_with(
                true,
                RoundState::Approved(Approval {
                    commit: oid(9),
                    comment_id: None,
                }),
                vec![commit(1, &[CommitStatus::Initial], true)],
                vec![],
            );

            let a = QCStatus::determine_status(&unplaceable).unwrap_err();
            let b = QCStatus::determine_status(&off_branch).unwrap_err();

            assert_ne!(a.to_string(), b.to_string());
            assert!(a.to_string().contains("not checked out locally"), "{a}");
            // The distinction is the point: only the unplaceable round says "fetch it".
            assert!(!b.to_string().contains("not checked out locally"), "{b}");
            // Same wire classification for both (`branch_not_local` + a branch).
            assert_eq!(a.branch_not_local(), Some("feature"));
            assert_eq!(b.branch_not_local(), Some("main"));
        }

        /// D10: only the **latest** round feeds status, so an *earlier* unplaceable
        /// round must not block it.
        #[test]
        fn d55_an_earlier_unplaceable_round_does_not_block_status() {
            let mut thread = thread_with(
                true,
                RoundState::Unapproved,
                vec![
                    commit(3, &[CommitStatus::Notification], true),
                    commit(2, &[CommitStatus::Initial], true),
                ],
                vec![],
            );
            let mut stale = thread.rounds[0].clone();
            stale.index = 1;
            stale.commits = Vec::new();
            stale.placement = RoundPlacement::Unplaceable {
                branch: "old-feature".to_string(),
            };
            thread.rounds[0].index = 2;
            thread.rounds.insert(0, stale);

            assert!(matches!(
                QCStatus::determine_status(&thread),
                Ok(QCStatus::AwaitingReview)
            ));
        }

        /// S5/D10: the S4 scan is bounded by the round, and never reads drift. A
        /// drift populated behind an *open* round (which I14 forbids, but which a
        /// rewritten history could produce) must not change the verdict.
        #[test]
        fn s4_scan_ignores_drift() {
            let commits = vec![
                commit(2, &[CommitStatus::Reviewed], false),
                commit(1, &[CommitStatus::Initial], true),
            ];
            let clean = thread_with(true, RoundState::Unapproved, commits.clone(), vec![]);
            let polluted = thread_with(
                true,
                RoundState::Unapproved,
                commits,
                vec![commit(3, &[], true)],
            );

            assert!(matches!(
                QCStatus::determine_status(&clean),
                Ok(QCStatus::ChangeRequested)
            ));
            assert!(matches!(
                QCStatus::determine_status(&polluted),
                Ok(QCStatus::ChangeRequested)
            ));
        }
    }

    // Tests for BlockingQCStatus

    #[test]
    fn test_blocking_qc_status_all_approved() {
        let mut status = BlockingQCStatus::default();
        status.approved.insert(1, PathBuf::from("file1.R"));
        status.approved.insert(2, PathBuf::from("file2.R"));

        assert!(status.all_approved());
        assert!(!status.has_errors());
        assert_eq!(status.total(), 2);
        assert_eq!(status.approved_count(), 2);
        assert_eq!(status.as_summary_string(), "2/2 (100.0%)");
    }

    #[test]
    fn test_blocking_qc_status_mixed() {
        let mut status = BlockingQCStatus::default();
        status.approved.insert(1, PathBuf::from("approved.R"));
        status
            .not_approved
            .insert(2, (PathBuf::from("pending.R"), QCStatus::AwaitingReview));

        assert!(!status.all_approved());
        assert!(!status.has_errors());
        assert_eq!(status.total(), 2);
        assert_eq!(status.approved_count(), 1);
        assert_eq!(status.as_summary_string(), "1/2 (50.0%)");
    }

    #[test]
    fn test_blocking_qc_status_with_errors() {
        let mut status = BlockingQCStatus::default();
        status.approved.insert(1, PathBuf::from("file1.R"));
        status
            .not_approved
            .insert(2, (PathBuf::from("file2.R"), QCStatus::InProgress));
        status.errors.insert(3, "404 Not Found".to_string());

        assert!(!status.all_approved());
        assert!(status.has_errors());
        assert_eq!(status.total(), 3);
        assert_eq!(status.approved_count(), 1);
        assert_eq!(status.error_count(), 1);
        assert_eq!(status.as_summary_string(), "1/3 (33.3%) (+1 err)");
    }

    #[test]
    fn test_blocking_qc_status_empty() {
        let status = BlockingQCStatus::default();
        assert!(status.all_approved()); // No blocking QCs means all are approved
        assert!(!status.has_errors());
        assert_eq!(status.total(), 0);
        assert_eq!(status.as_summary_string(), "-");
    }

    #[test]
    fn test_blocking_qc_status_display() {
        let mut status = BlockingQCStatus::default();
        status.approved.insert(1, PathBuf::from("approved.R"));
        status
            .not_approved
            .insert(2, (PathBuf::from("pending.R"), QCStatus::AwaitingReview));
        status.errors.insert(3, "API error".to_string());

        let display = format!("{}", status);
        assert!(display.contains("Blocking QCs:"));
        assert!(display.contains("#1"));
        assert!(display.contains("approved.R"));
        assert!(display.contains("#2"));
        assert!(display.contains("pending.R"));
        assert!(display.contains("#3"));
        assert!(display.contains("API error"));
    }

    #[test]
    fn test_qc_status_is_approved() {
        assert!(QCStatus::Approved.is_approved());
        assert!(
            QCStatus::ChangesAfterApproval(
                ObjectId::from_str("0000000000000000000000000000000000000001").unwrap()
            )
            .is_approved()
        );
        assert!(!QCStatus::AwaitingReview.is_approved());
        assert!(!QCStatus::InProgress.is_approved());
        assert!(!QCStatus::ApprovalRequired.is_approved());
        assert!(!QCStatus::ChangeRequested.is_approved());
        assert!(
            !QCStatus::ChangesToComment(
                ObjectId::from_str("0000000000000000000000000000000000000001").unwrap()
            )
            .is_approved()
        );
    }
}
