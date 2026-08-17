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
use crate::round::{Round, RoundEvent, RoundOpen, Segment};

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

impl Round {
    /// This round's status, from its **own** commits and events only.
    ///
    /// `None` when the round could not be placed: there is nothing to report, and the
    /// UI grays it rather than showing a status derived from no commits.
    ///
    /// A commit is *covered* when the round announced it — an event naming a commit at
    /// the same or a newer position in this round's commits, or, for Initial QC only,
    /// its anchor: the issue body *is* the announcement of its `initial qc commit`.
    /// A round comment is not one, so round >= 2's anchor covers nothing.
    pub fn status(&self) -> Option<QCStatus> {
        if !self.is_placed() {
            return None;
        }
        // **Unreachable from the fold, kept as a total guard.** A `Placed` round always
        // owns at least one commit — its own `opened_at` (W2 walks a round inclusive of
        // its anchor), so a placed round with no commits at all is a shape only a
        // hand-built fixture can produce.
        //
        // Do **not** read that as "`InProgress` is dead" and delete the variant: the
        // `(None, None)` arm below is a *different* condition and is genuinely
        // reachable — a round that owns commits, none of which touched the QC'd file,
        // and that announced nothing (`NotificationMode::None`). See the comment there.
        if self.commits.is_empty() && self.events.is_empty() {
            return Some(QCStatus::InProgress);
        }

        // "Newest" is by file change: drift that never touched this file is not
        // something a reviewer is asked to comment on.
        let newest_file_change = self.commits.iter().position(|commit| commit.file_changed);

        // The newest thing this round announced, and whether it was a review.
        //
        // The anchor seeds this for Initial QC only: creating the issue announces its
        // initial commit, so a brand-new issue awaits review rather than asking its
        // author to comment the very commit the issue names. A *round* comment is not a
        // notification — a round may open with none at all
        // ([`crate::NotificationMode::None`]) — so round >= 2 reports its anchor as
        // still needing one.
        let mut covering: Option<(usize, bool)> = match self.opened {
            RoundOpen::IssueCreated => self
                .commit_position(&self.opened_at)
                .map(|position| (position, false)),
            RoundOpen::NewRound { .. } => None,
        };
        for event in &self.events {
            let Some(position) = self.commit_position(event.commit()) else {
                continue;
            };
            let is_review = matches!(event, RoundEvent::Review { .. });
            match covering {
                // A review and a notification on the same commit means changes were
                // requested against it.
                Some((newest, was_review)) if position == newest => {
                    covering = Some((newest, was_review || is_review));
                }
                Some((newest, _)) if position > newest => {}
                _ => covering = Some((position, is_review)),
            }
        }

        let status = match (newest_file_change, covering) {
            // Nothing announced yet, but the file has changed: those changes are what
            // the next notification is for.
            (Some(file_change), None) => QCStatus::ChangesToComment(self.commits[file_change].hash),
            (Some(file_change), Some((announced, _))) if announced > file_change => {
                QCStatus::ChangesToComment(self.commits[file_change].hash)
            }
            (_, Some((_, true))) => QCStatus::ChangeRequested,
            (_, Some((_, false))) => QCStatus::AwaitingReview,
            // No file change and nothing announced inside this round: an event may
            // still name a commit the round does not own, which is enough to say the
            // round has been posted.
            (None, None) => {
                if self.events.is_empty() {
                    // **The reachable `InProgress`** (S2, last row). The round owns
                    // commits — none of which touched the QC'd file — and announced
                    // nothing at all, which a round opened with
                    // `NotificationMode::None` legitimately does. `AwaitingReview`
                    // would claim something was announced and `ChangesToComment` would
                    // claim a file change to comment on; neither exists. The round is
                    // simply open and in progress.
                    QCStatus::InProgress
                } else if self
                    .events
                    .iter()
                    .next_back()
                    .is_some_and(|event| matches!(event, RoundEvent::Review { .. }))
                {
                    QCStatus::ChangeRequested
                } else {
                    QCStatus::AwaitingReview
                }
            }
        };
        Some(status)
    }
}

impl QCStatus {
    /// The issue's status: a function of the active segment alone.
    ///
    /// `None` when the active segment could not be placed — the one degradation path,
    /// which the UI renders grayed.
    pub fn determine_status(issue_thread: &IssueThread) -> Option<Self> {
        // A closed issue with no standing approval was closed without being approved,
        // whatever its segments say.
        if !issue_thread.open && issue_thread.standing_approval().is_none() {
            return Some(Self::ApprovalRequired);
        }

        match issue_thread.active_segment() {
            // "Approved; subsequent file changes" is not an eighth status concept: it
            // is simply a non-empty trailing gap, which is also why starting a new
            // round is its natural resolution.
            Segment::Gap(gap) if gap.is_placed() => {
                match gap.commits.iter().find(|commit| commit.file_changed) {
                    Some(changed) => Some(Self::ChangesAfterApproval(changed.hash)),
                    None => Some(Self::Approved),
                }
            }
            Segment::Gap(_) => None,
            Segment::Round(round) => round.status(),
        }
    }

    /// Returns true if this status represents an approved issue
    /// (either pure Approved or ChangesAfterApproval)
    pub fn is_approved(&self) -> bool {
        matches!(self, QCStatus::Approved | QCStatus::ChangesAfterApproval(_))
    }

    /// The status of a blocking QC. `None` when its active segment could not be
    /// placed, which is a fact about the repository rather than an error.
    pub async fn from_blocking_qc(
        blocking_qc: &BlockingQC,
        cache: Option<&DiskCache>,
        git_info: &(impl GitHubReader + GitCommitOps),
    ) -> Result<Option<Self>, QCStatusError> {
        let issue = git_info.get_issue(blocking_qc.issue_number).await?;
        let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        Ok(QCStatus::determine_status(&issue_thread))
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
fn analyze_checklist_in_text(text: &str) -> ChecklistSummary {
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
            // An unplaceable active segment has no status to compare, so it is reported
            // as a problem rather than silently counted as unapproved.
            Ok(None) => {
                status.errors.insert(
                    qc.issue_number,
                    "status unavailable: the active segment could not be placed".to_string(),
                );
            }
            Ok(Some(s)) => {
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
    use crate::issue::IssueCommit;
    use crate::round::{
        ChecklistSource, Gap, GapContinuity, Placement, RoundOpen, RoundState, UnplaceableReason,
    };
    use std::str::FromStr;

    #[test]
    fn test_analyze_complex_issue_checklist() {
        let issue_body = include_str!("tests/qc_status/complex_issue_checklist.md");
        let result = analyze_issue_checklists(Some(issue_body));
        insta::assert_debug_snapshot!(result);
    }

    // ── Status fixtures ──────────────────────────────────────────────────────

    fn oid(n: u8) -> ObjectId {
        ObjectId::from_str(&format!("{:040x}", n)).unwrap()
    }

    fn epoch() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(0, 0).unwrap()
    }

    /// A commit, named by the low byte of its hash.
    fn commit(n: u8, file_changed: bool) -> IssueCommit {
        IssueCommit {
            hash: oid(n),
            message: format!("commit {n}"),
            file_changed,
        }
    }

    /// The commits a segment owns, newest-first, from an oldest-first list of
    /// `(name, file_changed)` pairs — the order the scenarios read in.
    fn owned(commits: &[(u8, bool)]) -> Vec<IssueCommit> {
        commits
            .iter()
            .rev()
            .map(|(n, file_changed)| commit(*n, *file_changed))
            .collect()
    }

    fn notification(n: u8) -> RoundEvent {
        RoundEvent::Notification {
            commit: oid(n),
            by: "author".to_string(),
            at: epoch(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn review(n: u8) -> RoundEvent {
        RoundEvent::Review {
            commit: oid(n),
            by: "reviewer".to_string(),
            at: epoch(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn closed_at(n: u8) -> RoundState {
        RoundState::Closed {
            commit: oid(n),
            by: "reviewer".to_string(),
            at: epoch(),
            comment_index: 1,
            comment_id: None,
            comment_url: None,
        }
    }

    fn round(
        index: u32,
        anchor: u8,
        state: RoundState,
        commits: &[(u8, bool)],
        events: Vec<RoundEvent>,
    ) -> Round {
        Round {
            index,
            opened_at: oid(anchor),
            branch: "main".to_string(),
            opened: if index == 1 {
                RoundOpen::IssueCreated
            } else {
                RoundOpen::NewRound {
                    comment_index: 2,
                    comment_id: None,
                    comment_url: None,
                    author: "author".to_string(),
                    at: epoch(),
                    note: None,
                    branch: Some("main".to_string()),
                }
            },
            checklist: ChecklistSource::IssueBody,
            checklist_name: None,
            state,
            events,
            retractions: vec![],
            extensions: vec![],
            commits: owned(commits),
            placement: Placement::Placed,
        }
    }

    fn gap(commits: &[(u8, bool)]) -> Gap {
        Gap {
            branch: "main".to_string(),
            commits: owned(commits),
            continuity: GapContinuity::Linear,
            placement: Placement::Placed,
        }
    }

    fn thread(open: bool, segments: Vec<Segment>) -> IssueThread {
        assert_eq!(
            crate::round::segment_invariants(&segments, None),
            Ok(()),
            "a status fixture must be a legal segment list"
        );
        IssueThread {
            file: PathBuf::from("scripts/file_b.R"),
            open,
            milestone: "milestone".to_string(),
            blocking_qcs: vec![],
            segments,
            anomalies: vec![],
        }
    }

    fn name_of(status: Option<&QCStatus>) -> &'static str {
        match status {
            Some(QCStatus::Approved) => "Approved",
            Some(QCStatus::ChangesAfterApproval(_)) => "ChangesAfterApproval",
            Some(QCStatus::AwaitingReview) => "AwaitingReview",
            Some(QCStatus::ChangeRequested) => "ChangeRequested",
            Some(QCStatus::InProgress) => "InProgress",
            Some(QCStatus::ApprovalRequired) => "ApprovalRequired",
            Some(QCStatus::ChangesToComment(_)) => "ChangesToComment",
            None => "no status",
        }
    }

    /// Every status a single round can report, driven by its own commits and events.
    ///
    /// `I` is the round's anchor, `C` a notification, `R` a review and `A` the approval
    /// that closes it — the same scenarios the flat model was pinned to, expressed as
    /// segments.
    #[test]
    fn test_change_requested_status_matrix() {
        let cases: Vec<(&str, bool, Vec<Segment>, &str)> = vec![
            (
                "I/R -> C: AwaitingReview",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, true)],
                    vec![review(1), notification(2)],
                ))],
                "AwaitingReview",
            ),
            (
                "I/R -> C/R: ChangeRequested",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, true)],
                    vec![review(1), notification(2), review(2)],
                ))],
                "ChangeRequested",
            ),
            (
                "I/R -> R: ChangeRequested",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, true)],
                    vec![review(1), review(2)],
                ))],
                "ChangeRequested",
            ),
            (
                "I/R -> R -> C: AwaitingReview",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    // reviewed commit 2 touched nothing; commit 3 is a new notification
                    &[(1, true), (2, false), (3, true)],
                    vec![review(1), review(2), notification(3)],
                ))],
                "AwaitingReview",
            ),
            (
                "I/R -> A/R: Approved",
                true,
                vec![
                    Segment::Round(round(
                        1,
                        1,
                        closed_at(2),
                        &[(1, true), (2, true)],
                        vec![review(1), review(2)],
                    )),
                    Segment::Gap(gap(&[])),
                ],
                "Approved",
            ),
            (
                "I/R -> A -> file change: ChangesAfterApproval",
                true,
                vec![
                    Segment::Round(round(
                        1,
                        1,
                        closed_at(2),
                        &[(1, true), (2, true)],
                        vec![review(1)],
                    )),
                    Segment::Gap(gap(&[(3, true)])),
                ],
                "ChangesAfterApproval",
            ),
            (
                "I/R -> C -> uncommented change: ChangesToComment",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, true), (3, true)],
                    vec![review(1), notification(2)],
                ))],
                "ChangesToComment",
            ),
            (
                "I -> C_nofile: AwaitingReview (notification on a non-file-changing commit)",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, false)],
                    vec![notification(2)],
                ))],
                "AwaitingReview",
            ),
            (
                "I -> R_nofile: ChangeRequested (review on a non-file-changing commit)",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, false)],
                    vec![review(2)],
                ))],
                "ChangeRequested",
            ),
            (
                "I -> C_nofile -> file change: ChangesToComment",
                true,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, false), (3, true)],
                    vec![notification(2)],
                ))],
                "ChangesToComment",
            ),
            (
                "Closed without approval: ApprovalRequired",
                false,
                vec![Segment::Round(round(
                    1,
                    1,
                    RoundState::Open,
                    &[(1, true), (2, true)],
                    vec![notification(2), review(2)],
                ))],
                "ApprovalRequired",
            ),
            (
                "I/A: Approved",
                false,
                vec![
                    Segment::Round(round(1, 1, closed_at(1), &[(1, false)], vec![])),
                    Segment::Gap(gap(&[])),
                ],
                "Approved",
            ),
            (
                // The reachable `InProgress`: round 2 opened with no notification
                // (`NotificationMode::None`) on a commit that did not touch the QC'd
                // file. It owns a commit, so this is *not* the unreachable
                // "placed round with no commits" arm — that shape cannot come out of
                // the fold, because a placed round always owns its own anchor.
                "Round 2 owns only a non-file-changing commit and announced nothing: InProgress",
                true,
                vec![
                    Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                    Segment::Gap(gap(&[])),
                    Segment::Round(round(2, 3, RoundState::Open, &[(3, false)], vec![])),
                ],
                "InProgress",
            ),
        ];

        for (scenario, issue_open, segments, expected) in cases {
            let issue_thread = thread(issue_open, segments);
            let status = QCStatus::determine_status(&issue_thread);
            assert_eq!(
                name_of(status.as_ref()),
                expected,
                "Failed for scenario: {scenario}"
            );
        }
    }

    /// Before a new round exists, a file change after the approval is exactly
    /// `ChangesAfterApproval` — which is now simply "the trailing gap is not empty".
    #[test]
    fn changes_after_approval_is_a_non_empty_trailing_gap() {
        let thread = thread(
            true,
            vec![
                Segment::Round(round(
                    1,
                    1,
                    closed_at(2),
                    &[(1, true), (2, true)],
                    vec![notification(2)],
                )),
                Segment::Gap(gap(&[(3, true)])),
            ],
        );

        let status = QCStatus::determine_status(&thread);
        assert!(
            matches!(status, Some(QCStatus::ChangesAfterApproval(hash)) if hash == oid(3)),
            "expected ChangesAfterApproval, got {status:?}"
        );
        assert_eq!(thread.standing_approval(), Some(&oid(2)));
        assert_eq!(thread.latest_commit().map(|c| c.hash), Some(oid(3)));
    }

    /// Drift that never touched this file is not something a reviewer is asked to
    /// comment on, so the issue is still simply approved.
    #[test]
    fn a_trailing_gap_of_unrelated_commits_is_still_approved() {
        let thread = thread(
            true,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[(3, false), (4, false)])),
            ],
        );
        assert!(matches!(
            QCStatus::determine_status(&thread),
            Some(QCStatus::Approved)
        ));
    }

    /// Once round 2 is open the issue is under review again, so it must not be reported
    /// as approved-with-changes: approve, commit, start a new round, and the card used
    /// to still read "Approved".
    #[test]
    fn an_open_round_supersedes_the_previous_rounds_approval() {
        let thread = thread(
            true,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[])),
                Segment::Round(round(
                    2,
                    3,
                    RoundState::Open,
                    &[(3, true)],
                    vec![notification(3)],
                )),
            ],
        );

        let status = QCStatus::determine_status(&thread);
        assert!(
            !status.as_ref().is_some_and(QCStatus::is_approved),
            "an open round must not report as approved, got {status:?}"
        );
        assert!(
            matches!(status, Some(QCStatus::AwaitingReview)),
            "expected AwaitingReview for a notified open round, got {status:?}"
        );

        // Nothing stands approved while a round is open, but the archive still needs to
        // be able to ask what was last approved.
        assert_eq!(thread.standing_approval(), None);
        assert_eq!(thread.last_approved_commit(), Some(&oid(2)));
        // The card's "Latest" row reads this: the open round's own newest commit, never
        // the previous round's approval.
        assert_eq!(thread.latest_commit().map(|c| c.hash), Some(oid(3)));
    }

    /// A brand-new issue, before anybody has commented: opening it announced its initial
    /// commit, so it awaits review rather than asking the author to comment the very
    /// commit the issue names. This is the case the anchor-as-covering rule exists for.
    #[test]
    fn a_fresh_initial_qc_awaits_review_of_its_own_initial_commit() {
        let thread = thread(
            true,
            vec![Segment::Round(round(
                1,
                1,
                RoundState::Open,
                &[(1, true)],
                vec![],
            ))],
        );
        let status = QCStatus::determine_status(&thread);
        assert!(
            matches!(status, Some(QCStatus::AwaitingReview)),
            "expected AwaitingReview for a fresh Initial QC, got {status:?}"
        );
    }

    /// An open round two whose anchor is the newest commit has announced *nothing*: the
    /// round comment that opened it is not a notification, and a round can legitimately
    /// open with none at all, so the anchor is what the next notification is for.
    #[test]
    fn a_fresh_round_two_reports_changes_to_comment_because_a_round_comment_is_not_a_notification()
    {
        let thread = thread(
            true,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[])),
                Segment::Round(round(2, 3, RoundState::Open, &[(3, true)], vec![])),
            ],
        );
        let status = QCStatus::determine_status(&thread);
        assert!(
            matches!(status, Some(QCStatus::ChangesToComment(hash)) if hash == oid(3)),
            "expected ChangesToComment(3) for an unannounced round-two anchor, got {status:?}"
        );
    }

    /// The whole of the anchor-as-covering rule, as one pair: the same round shape, and
    /// only *how it was opened* differs. Creating the issue announces its initial commit;
    /// posting a round comment does not.
    #[test]
    fn only_an_issue_created_rounds_anchor_counts_as_an_announcement() {
        let announced = round(1, 1, RoundState::Open, &[(1, true)], vec![]);
        let mut unannounced = announced.clone();
        unannounced.opened = RoundOpen::NewRound {
            comment_index: 2,
            comment_id: None,
            comment_url: None,
            author: "author".to_string(),
            at: epoch(),
            note: None,
            branch: Some("main".to_string()),
        };

        let by_issue = QCStatus::determine_status(&thread(true, vec![Segment::Round(announced)]));
        assert!(
            matches!(by_issue, Some(QCStatus::AwaitingReview)),
            "an issue-created round's anchor is announced, got {by_issue:?}"
        );

        let by_comment =
            QCStatus::determine_status(&thread(true, vec![Segment::Round(unannounced)]));
        assert!(
            matches!(by_comment, Some(QCStatus::ChangesToComment(hash)) if hash == oid(1)),
            "a round-comment-opened round's anchor is not announced, got {by_comment:?}"
        );
    }

    /// Drift arriving after a round opened, before it has notified: those commits are
    /// what the next notification is for.
    #[test]
    fn drift_after_an_open_rounds_anchor_is_changes_to_comment() {
        let thread = thread(
            true,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[])),
                Segment::Round(round(
                    2,
                    3,
                    RoundState::Open,
                    &[(3, true), (4, true)],
                    vec![],
                )),
            ],
        );
        let status = QCStatus::determine_status(&thread);
        assert!(
            matches!(status, Some(QCStatus::ChangesToComment(hash)) if hash == oid(4)),
            "expected ChangesToComment(4), got {status:?}"
        );
        assert_eq!(thread.next_notification_from(), Some(oid(3)));
    }

    /// With every round closed, the approval is what the issue reports — the
    /// long-standing behaviour for approved issues.
    #[test]
    fn a_closed_round_reports_its_approval_as_what_stands() {
        let thread = thread(
            true,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[])),
            ],
        );
        assert_eq!(thread.standing_approval(), Some(&oid(2)));
        assert_eq!(thread.last_approved_commit(), Some(&oid(2)));
        assert_eq!(thread.next_notification_from(), Some(oid(2)));
        assert!(matches!(
            QCStatus::determine_status(&thread),
            Some(QCStatus::Approved)
        ));
    }

    /// A closed issue that *was* approved is approved, not "approval required".
    #[test]
    fn a_closed_issue_with_a_standing_approval_is_approved() {
        let thread = thread(
            false,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[])),
            ],
        );
        assert!(matches!(
            QCStatus::determine_status(&thread),
            Some(QCStatus::Approved)
        ));
    }

    /// S2's last row, and the whole reason `InProgress` survives: a round that owns
    /// commits — none of which touched the QC'd file — and announced nothing.
    ///
    /// Reachable exactly as written: initial QC approved, round 2 opened on a declared
    /// branch with `NotificationMode::None`, and its anchor changed some other file.
    /// `AwaitingReview` would claim an announcement that never happened and
    /// `ChangesToComment` a file change that does not exist, so `InProgress` is the only
    /// honest answer — and it is emphatically **not** the unplaceable case: this round
    /// is placed, and what it is missing is a *file change*, not its history.
    #[test]
    fn a_placed_round_with_no_file_change_and_no_events_is_in_progress_not_unknown() {
        let mut open_round = round(2, 3, RoundState::Open, &[(3, false)], vec![]);
        let segments = |round2: Round| {
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(gap(&[])),
                Segment::Round(round2),
            ]
        };

        // The premise, spelled out: placed, owns a commit, no events, nothing touched
        // the file. Remove any one of those and this is a different row of S2.
        assert!(open_round.is_placed());
        assert_eq!(open_round.commits.len(), 1);
        assert!(open_round.events.is_empty());
        assert!(!open_round.commits.iter().any(|c| c.file_changed));

        let placed = thread(true, segments(open_round.clone()));
        let status = QCStatus::determine_status(&placed);
        assert!(
            matches!(status, Some(QCStatus::InProgress)),
            "a placed round with a non-file-changing commit and no events is in progress, got {status:?}"
        );

        // The discrimination that matters: the *same* round, unplaceable, has no status
        // at all. These two must never collapse into one another — `Unknown` claims
        // nothing can be asserted, which is false of the round above.
        // I5: an unplaceable segment owns nothing, so the commits go with the placement.
        // Leaving them behind builds a segment list the fold cannot emit, and `thread`'s
        // invariant check rejects it.
        open_round.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        open_round.commits.clear();
        let unplaceable = thread(true, segments(open_round));
        assert!(
            QCStatus::determine_status(&unplaceable).is_none(),
            "an unplaceable round yields no status, never InProgress"
        );
    }

    // ── Degradation: an unplaceable active segment has no status ─────────────

    #[test]
    fn an_unplaceable_active_round_has_no_status() {
        let mut unplaceable = round(1, 1, RoundState::Open, &[], vec![]);
        unplaceable.placement = Placement::Unplaceable(UnplaceableReason::BranchUnavailable);
        let thread = thread(true, vec![Segment::Round(unplaceable)]);

        assert!(
            QCStatus::determine_status(&thread).is_none(),
            "a grayed segment must not be given a status"
        );
        assert_eq!(thread.latest_commit(), None);
    }

    #[test]
    fn an_unplaceable_active_gap_has_no_status() {
        let mut unplaceable = gap(&[]);
        unplaceable.placement = Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable);
        let thread = thread(
            true,
            vec![
                Segment::Round(round(1, 1, closed_at(2), &[(1, true), (2, true)], vec![])),
                Segment::Gap(unplaceable),
            ],
        );

        // An empty gap would read as `Approved`, which is precisely the wrong answer
        // when the reason it is empty is that nothing could be walked.
        assert!(QCStatus::determine_status(&thread).is_none());
    }

    /// A closed issue reports `ApprovalRequired` before the active segment is even
    /// consulted, so an unplaceable segment cannot hide that it was never approved.
    #[test]
    fn a_closed_issue_without_an_approval_is_approval_required_even_when_grayed() {
        let mut unplaceable = round(1, 1, RoundState::Open, &[], vec![]);
        unplaceable.placement = Placement::Unplaceable(UnplaceableReason::AnchorUnreachable);
        let thread = thread(false, vec![Segment::Round(unplaceable)]);

        assert!(matches!(
            QCStatus::determine_status(&thread),
            Some(QCStatus::ApprovalRequired)
        ));
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
