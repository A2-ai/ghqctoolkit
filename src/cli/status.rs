use std::path::PathBuf;

use anyhow::{Result, bail};
use gix::ObjectId;
use octocrab::models::Milestone;

use crate::cli::interactive::{prompt_existing_milestone, prompt_issue};
use crate::cli::rename::alert_renames;
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
            &qc_status,
            &git_status.dirty,
            &file_commits,
            &blocking_qc_status,
        )
    );

    Ok(())
}

/// D55: `qc_status` is a `Result` because `determine_status` refuses to answer when the
/// latest round has no resolvable commit. The detail block still renders — including the
/// round table, which names the round and the branch to fetch — with the QC status line
/// saying it is undetermined and why. Never blank, never a substituted verdict.
pub fn single_issue_status(
    issue_thread: &IssueThread,
    git_status: &GitState,
    qc_status: &Result<QCStatus, crate::IssueError>,
    dirty_files: &[PathBuf],
    file_commits: &[&ObjectId],
    blocking_qc_status: &BlockingQCStatus,
) -> String {
    let mut res = vec![
        round_table(issue_thread),
        format!("- File:        {}", issue_thread.file.display()),
        format!("- Branch:      {}", issue_thread.branch()),
    ];
    res.push(format!(
        "- Issue State: {}",
        if issue_thread.open { "open" } else { "closed" }
    ));

    let qc_str = match qc_status {
        Ok(QCStatus::Approved) => format!("Approved"),
        Ok(QCStatus::ChangesAfterApproval(_)) => {
            format!("Approved. File has changed since approval")
        }
        Ok(QCStatus::AwaitingReview) => format!("Awaiting review. Latest commit notified"),
        Ok(QCStatus::ChangeRequested) => format!("Changes requested. Latest commit reviewed"),
        Ok(QCStatus::InProgress) => format!("Awaiting approval"),
        Ok(QCStatus::ApprovalRequired) => format!("Issue closed without approval"),
        Ok(QCStatus::ChangesToComment(commit)) => format!(
            "File change in '{}' not commented",
            commit.to_string()[..7].to_string()
        ),
        // D55: the round is named above in the table; here we say why there is no
        // verdict rather than inventing one.
        Err(e) => format!(
            "Undetermined for round {} — {e}",
            issue_thread.latest_round().index
        ),
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
    let (checklist_sum, checklist_sections) = latest_round_checklist(issue_thread);
    let indiv_checklist = checklist_sections
        .iter()
        .map(|(name, sum)| format!("{name}: {sum}"))
        .collect::<Vec<_>>();

    res.push(format!("- QC Status:   {qc_str}"));
    res.push(format!("- Git Status:  {git_str}"));
    res.push(format!(
        "- Checklist Summary: {checklist_sum}\n  - {}",
        indiv_checklist.join("\n  - ")
    ));
    res.push(format!("- {}", blocking_qc_status));

    res.join("\n")
}

/// R8: the checklist figures `ghqc issue status` prints come from the **latest round
/// only**. Reading `issue.body` would report round 1 (D3/§0.4): a completed `2/2` from
/// round 1 printed directly above a freshly opened round 2 reads to an auditor as a
/// finished checklist. D24 deleted the API's top-level `checklist_summary` for exactly
/// this bug class, and `round_table` already sources from `round.checklist.summary()`.
///
/// Returns the round's summary (M5) plus its per-section breakdown, so the detail block
/// keeps today's layout (C3).
fn latest_round_checklist(
    issue_thread: &IssueThread,
) -> (ChecklistSummary, Vec<(String, ChecklistSummary)>) {
    let checklist = &issue_thread.latest_round().checklist;
    // `content` excludes its `# ` heading (D37), so put the heading back before
    // splitting into sections — `analyze_issue_checklists` starts at the first H1.
    let sections = analyze_issue_checklists(Some(&format!(
        "# {}\n{}",
        checklist.name, checklist.content
    )));

    (checklist.summary(), sections)
}

/// C3: the per-round table `ghqc issue status` prints before the latest round's
/// detail.
///
/// One row per round: index, branch, start commit, approval commit, checklist n/m,
/// the preceding gap's commit count and its divergent flag. Round 1 has no
/// predecessor (I13), so its gap columns read `-`.
///
/// D53: an unplaceable round is listed like any other — it keeps its declared index —
/// with the branch to fetch spelled out below the table. D56: a round that inherited its
/// branch says so in the `Branch` column, because branch scopes both its own and its
/// gap's walk (D7/D9).
pub(crate) fn round_table(issue_thread: &IssueThread) -> String {
    struct Row {
        round: String,
        branch: String,
        start: String,
        approval: String,
        checklist: String,
        gap: String,
        divergent: String,
    }

    let rows: Vec<Row> = issue_thread
        .rounds
        .iter()
        .map(|round| {
            let summary = round.checklist.summary();
            // The preceding gap belongs to the round it precedes (D18); round 1 has
            // no predecessor to gap from or diverge from (I13).
            let (gap, divergent) = if round.index == 1 {
                ("-".to_string(), "-".to_string())
            } else {
                (
                    round.preceding_gap.commits.len().to_string(),
                    if round.preceding_gap.divergent {
                        "yes".to_string()
                    } else {
                        "no".to_string()
                    },
                )
            };

            Row {
                round: round.index.to_string(),
                branch: if round.branch_inherited {
                    // D56: kept, but never silent.
                    format!("{} (inherited)", round.branch)
                } else {
                    round.branch.clone()
                },
                start: short_commit(&round.start_commit),
                approval: round
                    .approved_commit()
                    .map(short_commit)
                    .unwrap_or_else(|| "-".to_string()),
                checklist: format!("{}/{}", summary.completed, summary.total),
                gap,
                divergent,
            }
        })
        .collect();

    let width = |header: &str, values: &dyn Fn(&Row) -> usize| {
        rows.iter().map(values).max().unwrap_or(0).max(header.len())
    };
    let round_w = width("Round", &|r: &Row| r.round.len());
    let branch_w = width("Branch", &|r: &Row| r.branch.len());
    let start_w = width("Start", &|r: &Row| r.start.len());
    let approval_w = width("Approval", &|r: &Row| r.approval.len());
    let checklist_w = width("Checklist", &|r: &Row| r.checklist.len());
    let gap_w = width("Gap", &|r: &Row| r.gap.len());
    let divergent_w = width("Divergent", &|r: &Row| r.divergent.len());

    let mut lines = vec![
        format!(
            "{:<round_w$} | {:<branch_w$} | {:<start_w$} | {:<approval_w$} | {:<checklist_w$} | {:<gap_w$} | {:<divergent_w$}",
            "Round", "Branch", "Start", "Approval", "Checklist", "Gap", "Divergent",
        ),
        format!(
            "{:-<round_w$}-+-{:-<branch_w$}-+-{:-<start_w$}-+-{:-<approval_w$}-+-{:-<checklist_w$}-+-{:-<gap_w$}-+-{:-<divergent_w$}",
            "", "", "", "", "", "", "",
        ),
    ];
    for row in &rows {
        lines.push(format!(
            "{:<round_w$} | {:<branch_w$} | {:<start_w$} | {:<approval_w$} | {:<checklist_w$} | {:<gap_w$} | {:<divergent_w$}",
            row.round,
            row.branch,
            row.start,
            row.approval,
            row.checklist,
            row.gap,
            row.divergent,
        ));
    }

    // D53/D55: an unplaceable round is rendered with its index and an explicit
    // fetch-this-branch state — never blank, never a substituted hash.
    for round in &issue_thread.rounds {
        if let Some(branch) = round.unplaceable_branch() {
            lines.push(format!(
                "⚠️  Round {}: start commit {} could not be placed — fetch '{}' to resolve it",
                round.index,
                short_commit(&round.start_commit),
                branch
            ));
        }
    }
    // D22: surfaced in the CLI as "no cohesive history". A divergent gap is an
    // allowable-but-undesired state, and D21/D39 forbid reading it as malformed.
    for round in &issue_thread.rounds {
        if round.preceding_gap.divergent {
            lines.push(format!(
                "⚠️  Round {}: no cohesive history — its start commit is not a descendant of the previous approval",
                round.index
            ));
        }
    }
    // D31/U6: a divergent drift means the approval was rewritten off its branch, so
    // the reported ChangesAfterApproval hash is not to be read as meaningful.
    if issue_thread.drift.divergent {
        lines.push(
            "⚠️  Approval commit not in branch history — the commits since approval cannot be bounded"
                .to_string(),
        );
    }

    lines.join("\n")
}

fn short_commit(commit: &ObjectId) -> String {
    commit.to_string()[..7].to_string()
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

                // Determine QC status. D55: when the latest round has no resolvable
                // commit the row says so, naming the branch to fetch — never a status
                // derived from a substituted commit.
                let qc_status = match QCStatus::determine_status(&issue_thread) {
                    Ok(status) => status.to_string(),
                    Err(e) => e.to_string(),
                };
                // R8: the milestone table reports the latest round, never the issue
                // body (which is round 1 — D3/§0.4).
                let checklist_summary = issue_thread.latest_round().checklist.summary();

                let mut git_status_str = git_status.format_for_file(&file_commits);
                if dirty_files.contains(&issue_thread.file) {
                    git_status_str.push_str(" (file has uncommitted local changes)");
                }

                let row = MilestoneStatusRow {
                    file: issue_thread.file.display().to_string(),
                    milestone: milestone.title.clone(),
                    branch: issue_thread.branch().to_string(),
                    issue_state: if issue_thread.open {
                        "open".to_string()
                    } else {
                        "closed".to_string()
                    },
                    qc_status,
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
    use crate::issue::{
        Approval, Gap, IssueCommit, Round, RoundChecklist, RoundPlacement, RoundState,
    };
    use std::collections::HashSet;

    fn commit(seed: char) -> ObjectId {
        ObjectId::from_hex(seed.to_string().repeat(40).as_bytes()).unwrap()
    }

    fn issue_commit(seed: char) -> IssueCommit {
        IssueCommit {
            hash: commit(seed),
            message: "a commit".to_string(),
            statuses: HashSet::from([crate::CommitStatus::Initial]),
            file_changed: true,
        }
    }

    fn round(
        index: u32,
        branch: &str,
        start: char,
        checklist: &str,
        state: RoundState,
        preceding_gap: Gap,
    ) -> Round {
        Round {
            index,
            branch: branch.to_string(),
            branch_inherited: false,
            placement: RoundPlacement::Placed,
            start_commit: commit(start),
            preceding_gap,
            checklist: RoundChecklist {
                name: format!("Pass {index}"),
                content: checklist.to_string(),
            },
            commits: vec![issue_commit(start)],
            state,
        }
    }

    /// C3: a per-round table — round, branch, start, approval, checklist n/m,
    /// preceding-gap commit count, divergent flag — for every round, not just the
    /// latest.
    #[test]
    fn test_round_table_renders_every_round_including_the_divergent_flag() {
        let thread = IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1".to_string(),
            open: true,
            blocking_qcs: Vec::new(),
            rounds: vec![
                round(
                    1,
                    "main",
                    'a',
                    "- [x] one\n- [x] two\n",
                    RoundState::Approved(Approval {
                        commit: commit('a'),
                        comment_id: None,
                    }),
                    Gap::default(),
                ),
                round(
                    2,
                    "feat/two",
                    'b',
                    "- [x] one\n- [ ] two\n",
                    // An unapproved non-last round is Superseded (D21/D27) — which
                    // D39.4 forbids reading as malformed, so the table just shows no
                    // approval commit.
                    RoundState::Unapproved,
                    Gap {
                        commits: vec![issue_commit('c'), issue_commit('d')],
                        // D22: the start commit is not descended from the previous
                        // approval.
                        divergent: true,
                    },
                ),
                round(
                    3,
                    "feat/three",
                    'e',
                    "- [ ] one\n- [ ] two\n- [ ] three\n",
                    RoundState::Unapproved,
                    Gap::default(),
                ),
            ],
            drift: Gap::default(),
        };

        let table = round_table(&thread);
        let lines: Vec<&str> = table.lines().collect();

        assert!(lines[0].starts_with("Round | Branch"));
        assert!(lines[0].contains("Start"));
        assert!(lines[0].contains("Approval"));
        assert!(lines[0].contains("Checklist"));
        assert!(lines[0].contains("Gap"));
        assert!(lines[0].contains("Divergent"));

        // Round 1: approved, and I13 means it has no gap columns to fill.
        assert_eq!(
            lines[2],
            "1     | main       | aaaaaaa | aaaaaaa  | 2/2       | -   | -        "
        );
        // Round 2: no approval, a two-commit divergent preceding gap.
        assert_eq!(
            lines[3],
            "2     | feat/two   | bbbbbbb | -        | 1/2       | 2   | yes      "
        );
        // Round 3: the latest round, an empty non-divergent gap (the D8 overlap
        // case is still a real, meaningful gap — W6).
        assert_eq!(
            lines[4],
            "3     | feat/three | eeeeeee | -        | 0/3       | 0   | no       "
        );

        // D22: surfaced in the CLI as "no cohesive history".
        assert!(table.contains("⚠️  Round 2: no cohesive history"));
        assert!(!table.contains("Round 3: no cohesive history"));
        assert!(!table.contains("Approval commit not in branch history"));
    }

    /// D53/D55: an unplaceable round is rendered with its **declared** index and an
    /// explicit fetch-this-branch state — never blank and never a substituted hash.
    /// D56: a round that inherited its branch says so in the `Branch` column.
    #[test]
    fn test_round_table_names_the_branch_to_fetch_and_the_inherited_branch() {
        let mut unplaceable = round(
            3,
            "feat/three",
            'e',
            "- [ ] one\n",
            RoundState::Unapproved,
            Gap::default(),
        );
        unplaceable.commits.clear();
        unplaceable.placement = RoundPlacement::Unplaceable {
            branch: "feat/three".to_string(),
        };
        let mut inherited = round(
            2,
            "main",
            'b',
            "- [x] one\n",
            RoundState::Unapproved,
            Gap::default(),
        );
        inherited.branch_inherited = true;

        let thread = IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1".to_string(),
            open: true,
            blocking_qcs: Vec::new(),
            rounds: vec![
                round(
                    1,
                    "main",
                    'a',
                    "- [x] one\n",
                    RoundState::Approved(Approval {
                        commit: commit('a'),
                        comment_id: None,
                    }),
                    Gap::default(),
                ),
                inherited,
                unplaceable,
            ],
            drift: Gap::default(),
        };

        let table = round_table(&thread);

        // The round is listed, with its declared index and its declared start commit.
        assert!(table.contains("3     | feat/three"), "got:\n{table}");
        assert!(table.contains(
            "⚠️  Round 3: start commit eeeeeee could not be placed — fetch 'feat/three' to \
             resolve it"
        ));
        // D56: the inheritance is visible where the round is viewed.
        assert!(table.contains("main (inherited)"), "got:\n{table}");
        assert!(
            !table.contains("1     | main (inherited)"),
            "round 1 declares its branch in the body"
        );
    }

    /// D55: `ghqc issue status` still renders. An unresolvable latest round produces an
    /// explicit undetermined line naming the round and the branch to fetch — the detail
    /// block is never blank, and no substituted verdict appears.
    #[test]
    fn test_single_issue_status_renders_an_undetermined_status() {
        let mut unplaceable = round(
            2,
            "feat/two",
            'b',
            "- [ ] one\n",
            RoundState::Unapproved,
            Gap::default(),
        );
        unplaceable.commits.clear();
        unplaceable.placement = RoundPlacement::Unplaceable {
            branch: "feat/two".to_string(),
        };
        let thread = IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1".to_string(),
            open: true,
            blocking_qcs: Vec::new(),
            rounds: vec![
                round(
                    1,
                    "main",
                    'a',
                    "- [x] one\n",
                    RoundState::Approved(Approval {
                        commit: commit('a'),
                        comment_id: None,
                    }),
                    Gap::default(),
                ),
                unplaceable,
            ],
            drift: Gap::default(),
        };

        let out = single_issue_status(
            &thread,
            &GitState::Clean,
            &Err(crate::IssueError::LocalBranchNotFound(
                "feat/two".to_string(),
            )),
            &[],
            &[],
            &BlockingQCStatus::default(),
        );

        assert!(
            out.contains("- QC Status:   Undetermined for round 2 — Branch 'feat/two'"),
            "got:\n{out}"
        );
        // The round table is still there, naming what to fetch.
        assert!(
            out.contains("fetch 'feat/two' to resolve it"),
            "got:\n{out}"
        );
    }

    /// D31/U6: a divergent drift is a different fact with a different message — the
    /// approval was rewritten off its branch, so the reported hash is not meaningful.
    #[test]
    fn test_round_table_flags_a_divergent_drift() {
        let thread = IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1".to_string(),
            open: false,
            blocking_qcs: Vec::new(),
            rounds: vec![round(
                1,
                "main",
                'a',
                "- [x] one\n",
                RoundState::Approved(Approval {
                    commit: commit('a'),
                    comment_id: None,
                }),
                Gap::default(),
            )],
            drift: Gap {
                commits: vec![issue_commit('b')],
                divergent: true,
            },
        };

        let table = round_table(&thread);
        assert!(table.contains("⚠️  Approval commit not in branch history"));
        assert!(!table.contains("no cohesive history"));
    }

    /// R8: the printed checklist summary must come from the LATEST round. Before this
    /// fix both this block and the milestone table read `analyze_issue_checklists(
    /// issue.body)` — the body is round 1 (D3/§0.4) — so a finished round-1 checklist
    /// was printed directly above "Awaiting review" for a freshly opened round 2.
    #[test]
    fn test_single_issue_status_checklist_summary_reflects_the_latest_round() {
        let thread = IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1".to_string(),
            open: true,
            blocking_qcs: Vec::new(),
            rounds: vec![
                round(
                    1,
                    "main",
                    'a',
                    "- [x] one\n- [x] two\n",
                    RoundState::Approved(Approval {
                        commit: commit('a'),
                        comment_id: None,
                    }),
                    Gap::default(),
                ),
                round(
                    2,
                    "feat/two",
                    'b',
                    "- [ ] one\n- [ ] two\n- [ ] three\n",
                    RoundState::Unapproved,
                    Gap::default(),
                ),
            ],
            drift: Gap::default(),
        };

        let out = single_issue_status(
            &thread,
            &GitState::Clean,
            &Ok(QCStatus::AwaitingReview),
            &[],
            &[],
            &BlockingQCStatus::default(),
        );

        // The latest round's figures, in today's layout (C3).
        assert!(
            out.contains("- Checklist Summary: 0/3 (0.0%)\n  - Pass 2: 0/3 (0.0%)"),
            "expected the latest round's checklist summary, got:\n{out}"
        );
        // Round 1's completed checklist belongs to the round table (C3) and nowhere
        // else: it must not leak into the detail block's figures.
        let summary_block: String = out
            .lines()
            .skip_while(|line| !line.starts_with("- Checklist Summary:"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !summary_block.contains("2/2"),
            "round 1's checklist leaked into the summary:\n{summary_block}"
        );
    }
}
