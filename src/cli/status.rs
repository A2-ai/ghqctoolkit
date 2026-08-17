use std::path::PathBuf;

use anyhow::{Result, bail};
use gix::ObjectId;
use octocrab::models::Milestone;

use crate::cli::interactive::{prompt_existing_milestone, prompt_issue};
use crate::cli::rename::alert_renames;
use crate::round::{GapContinuity, Placement, Segment};
use crate::{
    BlockingQCStatus, ChecklistSummary, DiskCache, GitHubReader, GitInfo, GitState, IssueThread,
    QCStatus, analyze_issue_checklists, get_blocking_qc_status, get_git_status,
};

pub async fn interactive_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    println!("📊 Welcome to GHQC Status Mode!");

    // Select milestone (existing only)
    let milestone = prompt_existing_milestone(milestones)?;

    // Get issues for this milestone
    let issues = git_info.get_issues(Some(milestone.number as u64)).await?;
    log::debug!(
        "Found {} total issues in milestone '{}'",
        issues.len(),
        milestone.title
    );

    if issues.is_empty() {
        bail!("No issues found in milestone '{}'", milestone.title);
    }

    // Alert about any pending file renames (run `ghqc issue rename` to confirm).
    alert_renames(git_info, &issues).await?;

    // Select issue by title
    let issue = prompt_issue(&issues)?;
    let checklist_summary = analyze_issue_checklists(issue.body.as_deref());

    // Create IssueThread from the selected issue
    let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;
    let file_commits = issue_thread.file_commits();

    // Get git status for the file
    let git_status = get_git_status(git_info)?;

    // Determine QC status
    let qc_status = QCStatus::determine_status(&issue_thread);
    let blocking_qc_status =
        get_blocking_qc_status(&issue_thread.blocking_qcs, git_info, cache).await;

    // Display the status
    println!(
        "\n{}",
        single_issue_status(
            &issue_thread,
            &git_status.state,
            qc_status.as_ref(),
            &git_status.dirty,
            &file_commits,
            &checklist_summary,
            &blocking_qc_status,
        )
    );

    Ok(())
}

/// Abbreviated commit, as everywhere else the CLI prints one.
fn short(commit: &ObjectId) -> String {
    commit.to_string()[..7].to_string()
}

fn plural_commits(count: usize) -> String {
    if count == 1 {
        "1 commit".to_string()
    } else {
        format!("{count} commits")
    }
}

/// A short suffix for the status line when the active segment cannot be taken at face
/// value — the CLI's equivalent of the card graying (**D15**/**U3 revised**).
///
/// The status itself stays a function of the record alone (**D8**); this only says the
/// record is not describing the history the user is looking at. Kept to a phrase per
/// **Q5** — the `Segments:` block below carries the detail.
fn trust_marker(issue_thread: &IssueThread) -> Option<String> {
    let active = issue_thread.active_segment();
    if let Placement::Unplaceable(reason) = active.placement() {
        return Some(format!(" (not placeable — {})", reason.describe()));
    }

    // Defensive only: a trailing gap walks the previous round's branch, and a placed
    // closed round has its closing commit on that walk, so the fold cannot produce an
    // unrelated trailing gap. Cheap to keep in case the model grows one.
    match active.as_gap().map(|gap| &gap.continuity) {
        Some(GapContinuity::Unrelated) => Some(" (history unrelated to the approval)".to_string()),
        _ => None,
    }
}

/// One line per segment: each round's index, branch, state and approval, and how much
/// drifted between two of them (**C1**).
///
/// High level on purpose (**Q5**): this says what the issue's history *is*, not what
/// every commit in it did.
fn segment_summary(issue_thread: &IssueThread) -> Vec<String> {
    issue_thread
        .segments
        .iter()
        .enumerate()
        .map(|(position, segment)| match segment {
            Segment::Round(round) => {
                // An unplaceable segment owns no commits, so it reports why rather than
                // a state derived from nothing.
                let state = match (&round.placement, round.closing_commit()) {
                    (Placement::Unplaceable(reason), _) => {
                        format!("unknown — {}", reason.describe())
                    }
                    (Placement::Placed, Some(commit)) => {
                        format!(
                            "approved at {} ({})",
                            short(commit),
                            plural_commits(round.commits.len())
                        )
                    }
                    (Placement::Placed, None) => {
                        format!("open ({})", plural_commits(round.commits.len()))
                    }
                };
                format!("{} on '{}': {state}", round.name(), round.branch)
            }
            Segment::Gap(gap) => {
                // A gap's bounding rounds are its immediate neighbours — one position
                // back and one forward, never two.
                let previous = position
                    .checked_sub(1)
                    .and_then(|older| issue_thread.segments.get(older))
                    .and_then(Segment::as_round);
                let next = issue_thread
                    .segments
                    .get(position + 1)
                    .and_then(Segment::as_round);
                let between = match (previous, next) {
                    (Some(previous), Some(next)) => {
                        format!("Between {} and {}", previous.name(), next.name())
                    }
                    (Some(previous), None) => format!("Since {}'s approval", previous.name()),
                    _ => "Outside any round".to_string(),
                };
                let continuity = match gap.continuity {
                    GapContinuity::Linear => String::new(),
                    GapContinuity::Diverged { merge_base } => {
                        format!(", diverged — histories meet at {}", short(&merge_base))
                    }
                    GapContinuity::Unrelated => {
                        ", unrelated histories — no comparison is meaningful".to_string()
                    }
                };
                match &gap.placement {
                    Placement::Unplaceable(reason) => {
                        format!("{between}: unknown — {}", reason.describe())
                    }
                    Placement::Placed => format!(
                        "{between}: {} on '{}'{continuity}",
                        plural_commits(gap.commits.len()),
                        gap.branch
                    ),
                }
            }
        })
        .collect()
}

pub fn single_issue_status(
    issue_thread: &IssueThread,
    git_status: &GitState,
    qc_status: Option<&QCStatus>,
    dirty_files: &[PathBuf],
    file_commits: &[&ObjectId],
    checklist_summaries: &[(String, ChecklistSummary)],
    blocking_qc_status: &BlockingQCStatus,
) -> String {
    let mut res = vec![
        format!("- File:        {}", issue_thread.file.display()),
        // The branch the issue is being QC'd on now, which is the active segment's —
        // there is no thread-wide branch to print (**C2**).
        format!("- Branch:      {}", issue_thread.active_branch()),
    ];
    res.push(format!(
        "- Issue State: {}",
        if issue_thread.open { "open" } else { "closed" }
    ));

    let qc_str = match qc_status {
        Some(QCStatus::Approved) => "Approved".to_string(),
        Some(QCStatus::ChangesAfterApproval(_)) => {
            "Approved. File has changed since approval".to_string()
        }
        Some(QCStatus::AwaitingReview) => "Awaiting review. Latest commit notified".to_string(),
        Some(QCStatus::ChangeRequested) => "Changes requested. Latest commit reviewed".to_string(),
        Some(QCStatus::InProgress) => "Awaiting approval".to_string(),
        Some(QCStatus::ApprovalRequired) => "Issue closed without approval".to_string(),
        Some(QCStatus::ChangesToComment(commit)) => {
            format!("File change in '{}' not commented", short(commit))
        }
        // An unplaceable active segment has no status to report, and saying so is
        // honest where a guess would not be.
        None => match issue_thread.active_segment().placement() {
            Placement::Unplaceable(reason) => {
                format!("Unknown — {}", reason.describe())
            }
            Placement::Placed => "Unknown".to_string(),
        },
    };
    // A status that exists can still be reporting on a segment nobody should trust: a
    // closed-without-approval issue is answered before the segment is even consulted.
    // The `None` arm above already names its reason, so it is not marked twice.
    let trust = match qc_status {
        Some(_) => trust_marker(issue_thread).unwrap_or_default(),
        None => String::new(),
    };
    let is_dirty = dirty_files.contains(&issue_thread.file);

    let git_str = match git_status {
        GitState::Clean => {
            log::debug!("Repository git status: clean");
            if is_dirty {
                "File has local, uncommitted changes"
            } else {
                "File is up to date!"
            }
        }
        GitState::Ahead(commits) => {
            log::debug!("Repository git status: ahead");
            match (file_commits.iter().any(|c| commits.contains(c)), is_dirty) {
                (true, true) => "File has local, committed changes and uncommitted changes",
                (true, false) => "File has local, committed changes",
                (false, true) => "File has uncommitted changes",
                (false, false) => "File is up to date!",
            }
        }
        GitState::Behind(commits) => {
            log::debug!("Repository git status: behind");
            match (file_commits.iter().any(|c| commits.contains(c)), is_dirty) {
                (true, true) => {
                    "File has remote changes that have not been pulled locally and uncommitted changes. Stash the changes and pull"
                }
                (true, false) => "File has remote changes that have not been pulled locally",
                (false, true) => "File has uncommitted changes",
                (false, false) => "File is up to date!",
            }
        }
        GitState::Diverged { ahead, behind } => {
            log::debug!("Repository git status: diverged");
            let is_ahead = file_commits.iter().any(|c| ahead.contains(c));
            let is_behind = file_commits.iter().any(|c| behind.contains(c));

            match (is_ahead, is_behind, is_dirty) {
                (true, true, true) => {
                    "File has diverged and has local, committed and uncommitted changes and remote, unpulled changes"
                }
                (true, true, false) => {
                    "File has diverged and has local, committed and remote, unpulled changes"
                }
                (true, false, true) => "File has local, committed changes and uncommitted changes",
                (true, false, false) => "File has local, committed changes",
                (false, true, true) => {
                    "File has remote changes that have not been pulled locally and uncommitted changes. Stash the changes and pull"
                }
                (false, true, false) => "File has remote changes that have not been pulled locally",
                (false, false, true) => "File has uncommitted changes",
                (false, false, false) => "File is up to date!",
            }
        }
    };
    let indiv_checklist = checklist_summaries
        .iter()
        .map(|(name, sum)| format!("{name}: {sum}"))
        .collect::<Vec<_>>();
    let checklist_sum = ChecklistSummary::sum(checklist_summaries.iter().map(|(_, c)| c));

    res.push(format!("- QC Status:   {qc_str}{trust}"));
    res.push(format!("- Git Status:  {git_str}"));
    res.push(format!(
        "- Segments:\n  - {}",
        segment_summary(issue_thread).join("\n  - ")
    ));
    res.push(format!(
        "- Checklist Summary: {checklist_sum}\n  - {}",
        indiv_checklist.join("\n  - ")
    ));
    res.push(format!("- {}", blocking_qc_status));

    res.join("\n")
}

#[derive(Debug, Clone)]
pub struct MilestoneStatusRow {
    pub file: String,
    pub milestone: String,
    pub branch: String,
    pub issue_state: String,
    pub qc_status: String,
    pub git_status: String,
    pub checklist_summary: ChecklistSummary,
    pub blocking_qc_status: BlockingQCStatus,
}

pub async fn interactive_milestone_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    println!("📊 Welcome to GHQC Milestone Status Mode!");

    if milestones.is_empty() {
        bail!("No milestones found in repository");
    }

    use inquire::{MultiSelect, Select};

    // First ask if they want to select all or choose specific ones
    let choice = Select::new(
        "📊 How would you like to select milestones?",
        vec!["📋 Select All Milestones", "🎯 Choose Specific Milestones"],
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let mut sorted_milestones: Vec<&Milestone> = milestones.iter().collect();
    sorted_milestones.sort_by(|a, b| b.number.cmp(&a.number));

    let selected_milestones: Vec<&Milestone> = if choice == "📋 Select All Milestones" {
        sorted_milestones
    } else {
        // Multi-select specific milestones
        let milestone_options: Vec<String> = sorted_milestones
            .iter()
            .map(|m| format!("{} ({})", m.title, m.number))
            .collect();

        let selected_strings =
            MultiSelect::new("📊 Select milestones to check:", milestone_options)
                .with_validator(|selection: &[inquire::list_option::ListOption<&String>]| {
                    if selection.is_empty() {
                        Ok(inquire::validator::Validation::Invalid(
                            "Please select at least one milestone".into(),
                        ))
                    } else {
                        Ok(inquire::validator::Validation::Valid)
                    }
                })
                .prompt()
                .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        // Filter milestones based on selected strings
        sorted_milestones
            .into_iter()
            .filter(|m| {
                let milestone_display = format!("{} ({})", m.title, m.number);
                selected_strings.contains(&milestone_display)
            })
            .collect()
    };

    if selected_milestones.is_empty() {
        bail!("No milestones selected");
    }

    // Get status for all selected milestones
    let status_rows = get_milestone_status_rows(&selected_milestones, cache, git_info).await?;

    // Display results
    display_milestone_status_table(&status_rows);

    Ok(())
}

pub async fn milestone_status(
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    if milestones.is_empty() {
        bail!("No milestones provided");
    }

    // Convert to &[&Milestone] for the function call
    let milestone_refs: Vec<&Milestone> = milestones.iter().collect();

    // Get status for all milestones
    let status_rows = get_milestone_status_rows(&milestone_refs, cache, git_info).await?;

    // Display results
    display_milestone_status_table(&status_rows);

    Ok(())
}

async fn get_milestone_status_rows(
    milestones: &[&Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<Vec<MilestoneStatusRow>> {
    let mut rows = Vec::new();

    // Fetch once before processing issues (same result for all issues)
    let (git_status, dirty_files) = match get_git_status(git_info) {
        Ok(status) => (status.state, status.dirty),
        Err(_) => (GitState::Clean, Vec::new()),
    };

    for milestone in milestones {
        // Get all issues for this milestone
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

        // Alert about any pending file renames (run `ghqc issue rename` to confirm).
        alert_renames(git_info, &issues).await?;

        for issue in issues {
            // Create IssueThread for each issue
            if let Ok(issue_thread) = IssueThread::from_issue(&issue, cache, git_info).await {
                let file_commits = issue_thread.file_commits();

                // Determine QC status
                let qc_status = QCStatus::determine_status(&issue_thread);
                let checklist_summaries = analyze_issue_checklists(issue.body.as_deref());
                let checklist_summary =
                    ChecklistSummary::sum(checklist_summaries.iter().map(|(_, c)| c));

                let mut git_status_str = git_status.format_for_file(&file_commits);
                if dirty_files.contains(&issue_thread.file) {
                    git_status_str.push_str(" (file has uncommitted local changes)");
                }

                let row = MilestoneStatusRow {
                    file: issue_thread.file.display().to_string(),
                    milestone: milestone.title.clone(),
                    branch: issue_thread.active_branch().to_string(),
                    issue_state: if issue_thread.open {
                        "open".to_string()
                    } else {
                        "closed".to_string()
                    },
                    qc_status: qc_status
                        .map(|status| status.to_string())
                        // One row per issue, so an unplaceable active segment says so
                        // in place of a status rather than dropping the issue.
                        .unwrap_or_else(|| "Unknown".to_string()),
                    git_status: git_status_str,
                    checklist_summary,
                    blocking_qc_status: get_blocking_qc_status(
                        &issue_thread.blocking_qcs,
                        git_info,
                        cache,
                    )
                    .await,
                };
                rows.push(row);
            }
        }
    }

    // Sort by milestone name, then by file name
    rows.sort_by(|a, b| {
        a.milestone
            .cmp(&b.milestone)
            .then_with(|| a.file.cmp(&b.file))
    });

    Ok(rows)
}

fn display_milestone_status_table(rows: &[MilestoneStatusRow]) {
    if rows.is_empty() {
        println!("No issues found in selected milestones.");
        return;
    }

    // Calculate column widths
    let file_width = rows.iter().map(|r| r.file.len()).max().unwrap_or(4).max(4);
    let milestone_width = rows
        .iter()
        .map(|r| r.milestone.len())
        .max()
        .unwrap_or(9)
        .max(9);
    let branch_width = rows
        .iter()
        .map(|r| r.branch.len())
        .max()
        .unwrap_or(6)
        .max(6);
    let issue_state_width = rows
        .iter()
        .map(|r| r.issue_state.len())
        .max()
        .unwrap_or(11)
        .max(11);
    let qc_status_width = rows
        .iter()
        .map(|r| r.qc_status.len())
        .max()
        .unwrap_or(9)
        .max(9);
    let git_status_width = rows
        .iter()
        .map(|r| r.git_status.len())
        .max()
        .unwrap_or(10)
        .max(10);
    let checklist_width = rows
        .iter()
        .map(|r| r.checklist_summary.to_string().len())
        .max()
        .unwrap_or(9)
        .max(9);
    let blocking_qc_width = rows
        .iter()
        .map(|r| r.blocking_qc_status.as_summary_string().len())
        .max()
        .unwrap_or(12)
        .max(12);

    // Print header
    println!();
    println!(
        "{:<file_width$} | {:<milestone_width$} | {:<branch_width$} | {:<issue_state_width$} | {:<qc_status_width$} | {:<git_status_width$} | {:<checklist_width$} | {:<blocking_qc_width$}",
        "File",
        "Milestone",
        "Branch",
        "Issue State",
        "QC Status",
        "Git Status",
        "Checklist",
        "Blocking QCs",
        file_width = file_width,
        milestone_width = milestone_width,
        branch_width = branch_width,
        issue_state_width = issue_state_width,
        qc_status_width = qc_status_width,
        git_status_width = git_status_width,
        checklist_width = checklist_width,
        blocking_qc_width = blocking_qc_width,
    );

    // Print separator
    println!(
        "{:-<file_width$}-+-{:-<milestone_width$}-+-{:-<branch_width$}-+-{:-<issue_state_width$}-+-{:-<qc_status_width$}-+-{:-<git_status_width$}-+-{:-<checklist_width$}-+-{:-<blocking_qc_width$}",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        file_width = file_width,
        milestone_width = milestone_width,
        branch_width = branch_width,
        issue_state_width = issue_state_width,
        qc_status_width = qc_status_width,
        git_status_width = git_status_width,
        checklist_width = checklist_width,
        blocking_qc_width = blocking_qc_width,
    );

    // Print rows
    for row in rows {
        let checklist_str = row.checklist_summary.to_string();
        let blocking_qc_str = row.blocking_qc_status.as_summary_string();
        println!(
            "{:<file_width$} | {:<milestone_width$} | {:<branch_width$} | {:<issue_state_width$} | {:<qc_status_width$} | {:<git_status_width$} | {:<checklist_width$} | {:<blocking_qc_width$}",
            row.file,
            row.milestone,
            row.branch,
            row.issue_state,
            row.qc_status,
            row.git_status,
            checklist_str,
            blocking_qc_str,
            file_width = file_width,
            milestone_width = milestone_width,
            branch_width = branch_width,
            issue_state_width = issue_state_width,
            qc_status_width = qc_status_width,
            git_status_width = git_status_width,
            checklist_width = checklist_width,
            blocking_qc_width = blocking_qc_width,
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::IssueCommit;
    use crate::round::{ChecklistSource, Gap, Round, RoundOpen, RoundState, UnplaceableReason};
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";

    fn oid(sha: &str) -> ObjectId {
        ObjectId::from_str(sha).unwrap()
    }

    fn commit(sha: &str) -> IssueCommit {
        IssueCommit {
            hash: oid(sha),
            message: format!("commit {sha}"),
            file_changed: true,
        }
    }

    fn round(index: u32, branch: &str, state: RoundState, placement: Placement) -> Segment {
        Segment::Round(Round {
            index,
            opened_at: oid(A),
            branch: branch.to_string(),
            opened: RoundOpen::IssueCreated,
            checklist: ChecklistSource::IssueBody,
            checklist_name: None,
            state,
            events: Vec::new(),
            retractions: Vec::new(),
            extensions: Vec::new(),
            commits: if placement.is_placed() {
                vec![commit(B), commit(A)]
            } else {
                Vec::new()
            },
            placement,
        })
    }

    fn closed() -> RoundState {
        RoundState::Closed {
            commit: oid(B),
            by: "reviewer".to_string(),
            at: chrono::Utc::now(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn thread(segments: Vec<Segment>) -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            open: true,
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies: Vec::new(),
        }
    }

    /// C1: every round's branch, state and approval, and every gap's size — including
    /// the empty ones, which are the *reason* an issue reads as approved.
    #[test]
    fn the_summary_names_every_round_and_gap() {
        let summary = segment_summary(&thread(vec![
            round(1, "main", closed(), Placement::Placed),
            Segment::Gap(Gap {
                branch: "feature/x".to_string(),
                commits: vec![commit(C)],
                continuity: GapContinuity::Diverged { merge_base: oid(A) },
                placement: Placement::Placed,
            }),
            round(2, "feature/x", RoundState::Open, Placement::Placed),
        ]));

        assert_eq!(
            summary,
            vec![
                format!("Initial QC on 'main': approved at {} (2 commits)", &B[..7]),
                format!(
                    "Between Initial QC and Round 2: 1 commit on 'feature/x', diverged — \
                     histories meet at {}",
                    &A[..7]
                ),
                "Round 2 on 'feature/x': open (2 commits)".to_string(),
            ]
        );
    }

    /// D4: an unplaceable segment is grayed with its reason, never guessed at — and the
    /// gap beside it says which fact is missing rather than borrowing the other round's
    /// branch.
    #[test]
    fn an_unplaceable_round_reports_why_instead_of_a_state() {
        let summary = segment_summary(&thread(vec![
            round(1, "main", closed(), Placement::Placed),
            Segment::Gap(Gap {
                branch: String::new(),
                commits: Vec::new(),
                continuity: GapContinuity::Linear,
                placement: Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable),
            }),
            round(
                2,
                "",
                RoundState::Open,
                Placement::Unplaceable(UnplaceableReason::BranchNotDeclared),
            ),
        ]));

        assert_eq!(
            summary[1],
            "Between Initial QC and Round 2: unknown — the round bounding it could not be placed"
        );
        assert_eq!(
            summary[2],
            "Round 2 on '': unknown — its round comment declared no branch"
        );
    }

    /// S4: no status at all is a state the CLI has to render, and "Unknown" with the
    /// reason is the honest rendering — not an error, and not a stale status.
    #[test]
    fn an_unplaceable_active_segment_renders_as_unknown() {
        let thread = thread(vec![round(
            1,
            "main",
            RoundState::Open,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable),
        )]);
        assert!(QCStatus::determine_status(&thread).is_none());

        let rendered = single_issue_status(
            &thread,
            &GitState::Clean,
            None,
            &[],
            &[],
            &[],
            &BlockingQCStatus::default(),
        );
        assert!(
            rendered.contains("QC Status:   Unknown — its branch is unavailable locally"),
            "unexpected: {rendered}"
        );
        // C2: the branch line is the active segment's, not a thread-wide field.
        assert!(
            rendered.contains("Branch:      main"),
            "unexpected: {rendered}"
        );
        assert!(rendered.contains("Segments:"), "unexpected: {rendered}");
    }

    /// D15: a status can exist *and* be about a segment nobody should trust — a closed
    /// issue reports `ApprovalRequired` before the active segment is consulted. The card
    /// grays here, so the status line has to say so too, or `ghqc issue status` and the
    /// UI disagree about the same issue (§0).
    #[test]
    fn a_status_over_an_unplaceable_segment_is_marked_on_the_status_line() {
        let mut thread = thread(vec![round(
            1,
            "main",
            RoundState::Open,
            Placement::Unplaceable(UnplaceableReason::AnchorUnreachable),
        )]);
        thread.open = false;

        let status = QCStatus::determine_status(&thread);
        assert!(matches!(status, Some(QCStatus::ApprovalRequired)));

        let rendered = single_issue_status(
            &thread,
            &GitState::Clean,
            status.as_ref(),
            &[],
            &[],
            &[],
            &BlockingQCStatus::default(),
        );
        assert!(
            rendered.contains(
                "QC Status:   Issue closed without approval (not placeable — its commits are \
                 not on that branch)"
            ),
            "unexpected: {rendered}"
        );
    }

    /// The other half of the marker, distinguished from *not placeable* because the
    /// remedies differ. Hand-built on purpose: the fold cannot produce an unrelated
    /// trailing gap (a trailing gap walks the previous round's branch, where a placed
    /// closed round's approval necessarily is), so this pins the guard, not a reachable
    /// state.
    #[test]
    fn an_unrelated_trailing_gap_reads_approved_and_says_the_history_is_unrelated() {
        let thread = thread(vec![
            round(1, "main", closed(), Placement::Placed),
            Segment::Gap(Gap {
                branch: "main".to_string(),
                commits: Vec::new(),
                continuity: GapContinuity::Unrelated,
                placement: Placement::Placed,
            }),
        ]);

        let status = QCStatus::determine_status(&thread);
        assert!(matches!(status, Some(QCStatus::Approved)));

        let rendered = single_issue_status(
            &thread,
            &GitState::Clean,
            status.as_ref(),
            &[],
            &[],
            &[],
            &BlockingQCStatus::default(),
        );
        assert!(
            rendered.contains("QC Status:   Approved (history unrelated to the approval)"),
            "unexpected: {rendered}"
        );
    }
}
