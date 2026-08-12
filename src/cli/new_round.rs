//! CLI: start a new QC round on an issue whose previous round is approved, and
//! repair one whose follow-up steps did not all land.
//!
//! A thin front end over [`crate::start_round`]. The interesting CLI-only
//! decisions are: where the checklist comes from (seeded from the previous round,
//! or a named template from the configuration), and what a *partial* success
//! means for the exit code — see [`report_result`].
//!
//! [`repair_open_round`] is the remedy that partial success points at: it fronts
//! [`crate::repair_round`], which completes only the steps the now-open round is
//! actually missing. Re-running `new-round` is never the fix — it would extend the
//! open round rather than open another one.

use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use inquire::Text;
use octocrab::models::{Milestone, issues::Issue};
use std::path::{Path, PathBuf};

use super::config_init::edit_markdown_checklist;
use super::interactive::{prompt_existing_milestone, prompt_issue};
use crate::{
    Configuration, DiskCache, GitHubReader, GitInfo, IssueThread, NotificationMode,
    RepairRoundRequest, RepairRoundResult, StartRoundRequest, StartRoundResult, get_issue_comments,
    prior_round_comment_body, repair_round, reset_checklist, seed_checklist, start_round,
};

/// CLI spelling of [`NotificationMode`].
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum NotificationArg {
    /// Post a QC Notification with the inline file diff.
    #[default]
    Full,
    /// Post a QC Notification with metadata only, no inline diff.
    MetadataOnly,
    /// Post no notification comment at all.
    None,
}

impl From<NotificationArg> for NotificationMode {
    fn from(arg: NotificationArg) -> Self {
        match arg {
            NotificationArg::Full => NotificationMode::Full,
            NotificationArg::MetadataOnly => NotificationMode::MetadataOnly,
            NotificationArg::None => NotificationMode::None,
        }
    }
}

/// Everything the `issue new-round` subcommand accepts.
#[derive(Debug, Clone)]
pub struct NewRoundArgs {
    pub milestone: Option<String>,
    pub file: Option<PathBuf>,
    /// Use this configuration checklist instead of the previous round's.
    pub checklist_name: Option<String>,
    pub note: Option<String>,
    pub notification: NotificationMode,
    /// Open the checklist in `$EDITOR` before posting. Implied interactively.
    pub edit: bool,
}

/// Start a new round, printing the outcome.
///
/// Errors — and so exits non-zero — when nothing was written, and also when the
/// round was recorded but a follow-up step failed (see [`report_result`]).
pub async fn new_round(
    args: NewRoundArgs,
    configuration: &Configuration,
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    let (issue, interactive) = match (&args.milestone, &args.file) {
        (Some(milestone), Some(file)) => (
            find_issue_any_state(milestone, file, milestones, git_info).await?,
            false,
        ),
        (None, None) => (prompt_round_issue(milestones, git_info).await?, true),
        _ => bail!(
            "Must provide both --milestone and --file arguments or neither to enter interactive mode"
        ),
    };

    let comments = get_issue_comments(&issue, cache, git_info).await?;
    let thread = IssueThread::from_issue_comments(&issue, &comments, git_info, cache)?;

    let (content, name) = resolve_checklist(
        args.checklist_name.as_deref(),
        configuration,
        &thread,
        &comments,
        issue.body.as_deref(),
    )?;

    // Interactively the editor *is* the review step for the seeded checklist; with
    // explicit arguments it is opt-in so the command stays scriptable.
    let content = if args.edit || interactive {
        edit_markdown_checklist(name.as_deref().unwrap_or("New round checklist"), &content)?
            .unwrap_or(content)
    } else {
        content
    };

    let note = match (args.note, interactive) {
        (Some(note), _) => Some(note),
        (None, true) => prompt_note()?,
        (None, false) => None,
    };

    let request = StartRoundRequest {
        issue,
        checklist_content: content,
        checklist_name: name,
        note,
        notification: args.notification,
    };

    let result = start_round(&request, &thread, git_info).await?;
    report_result(&result)
}

/// Print `result`, then fail if any step wants a repair.
///
/// The action deliberately succeeds with per-step failures, so a plain `Ok(())`
/// here would show a user a silent success after, say, the body marker failed.
/// The successful steps are printed either way — the error only sets the exit
/// code and names the remedy.
pub fn report_result(result: &StartRoundResult) -> Result<()> {
    print!("{result}");

    if result.needs_repair() {
        bail!(
            "Round {} was recorded, but one or more follow-up steps failed (see above). \
             Each failed step is idempotent and can be retried on its own with \
             `ghqc issue repair-round` — do not re-run this command, which would extend the \
             round rather than open another one.",
            result.round
        );
    }
    Ok(())
}

/// Everything the `issue repair-round` subcommand accepts.
#[derive(Debug, Clone)]
pub struct RepairRoundArgs {
    pub milestone: Option<String>,
    pub file: Option<PathBuf>,
    /// Post a `# QC Notification` if the open round has none. Defaults to *not*
    /// notifying: a round opened with `--notification none` is in exactly the state
    /// its author chose, so a repair never pings a reviewer on its own.
    pub notification: NotificationMode,
}

/// Complete the follow-up steps of an issue's open round, printing the outcome.
///
/// The counterpart to [`new_round`]: once a round is open the new-round action
/// cannot be re-run (a second `# QC New Round` comment would extend the round), so
/// this is how a partially-applied round start is finished.
pub async fn repair_open_round(
    args: RepairRoundArgs,
    milestones: &[Milestone],
    cache: Option<&DiskCache>,
    git_info: &GitInfo,
) -> Result<()> {
    let issue = match (&args.milestone, &args.file) {
        (Some(milestone), Some(file)) => {
            find_issue_any_state(milestone, file, milestones, git_info).await?
        }
        (None, None) => prompt_round_issue(milestones, git_info).await?,
        _ => bail!(
            "Must provide both --milestone and --file arguments or neither to enter interactive mode"
        ),
    };

    let comments = get_issue_comments(&issue, cache, git_info).await?;
    let thread = IssueThread::from_issue_comments(&issue, &comments, git_info, cache)?;

    let request = RepairRoundRequest {
        issue,
        notification: args.notification,
    };

    let result = repair_round(&request, &thread, git_info).await?;
    report_repair(&result)
}

/// Print `result`, then fail if a step it attempted still failed.
///
/// A repair that found nothing to do is a success: the point of the command is to
/// leave the round complete, and it already was.
pub fn report_repair(result: &RepairRoundResult) -> Result<()> {
    print!("{result}");

    if result.needs_repair() {
        bail!(
            "{} still has follow-up steps that failed (see above). Every step is idempotent, \
             so this command can be run again once the cause is fixed.",
            result.round_name
        );
    }
    Ok(())
}

/// Resolve the checklist for the new round: a named configuration template if one
/// was asked for, otherwise the previous round's checklist with boxes reset.
fn resolve_checklist(
    checklist_name: Option<&str>,
    configuration: &Configuration,
    thread: &IssueThread,
    comments: &[crate::GitComment],
    issue_body: Option<&str>,
) -> Result<(String, Option<String>)> {
    if let Some(name) = checklist_name {
        let checklist = configuration.checklists.get(name).ok_or_else(|| {
            let mut available: Vec<&str> = configuration
                .checklists
                .keys()
                .map(String::as_str)
                .collect();
            available.sort();
            anyhow!(
                "No checklist named '{name}' in the configuration. Available: {}",
                if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(", ")
                }
            )
        })?;
        return Ok((
            reset_checklist(&checklist.content),
            Some(checklist.name.clone()),
        ));
    }

    let seeded = seed_checklist(prior_round_comment_body(thread, comments), issue_body)
        .ok_or_else(|| {
            anyhow!(
                "Could not seed a checklist: neither the previous round's comment nor the issue \
                 body carries one. Pass --checklist-name to start from a configuration checklist."
            )
        })?;
    Ok((seeded.content, seeded.name))
}

/// Find an issue by file name in a milestone, regardless of its state.
///
/// Unlike [`crate::cli::find_issue`], closed issues are eligible: a new round is
/// normally started on an issue that was approved and therefore closed.
async fn find_issue_any_state(
    milestone_name: &str,
    file: &Path,
    milestones: &[Milestone],
    git_info: &impl GitHubReader,
) -> Result<Issue> {
    let milestone = milestones
        .iter()
        .find(|m| m.title == milestone_name)
        .ok_or_else(|| anyhow!("Milestone '{milestone_name}' not found"))?;

    let issues = git_info.get_issues(Some(milestone.number as u64)).await?;
    let file_str = file.to_string_lossy();

    // Same title matching as `find_issue`, so both commands accept the same paths.
    issues
        .into_iter()
        .find(|issue| issue.title.contains(file_str.as_ref()))
        .ok_or_else(|| {
            anyhow!("No issue found for file '{file_str}' in milestone '{milestone_name}'")
        })
}

/// Milestone → issue prompts. Every issue in the milestone is offered: whether a
/// new round is legal depends on the *derived* round state, which the action
/// itself checks (and rejects with a precise message).
async fn prompt_round_issue(milestones: &[Milestone], git_info: &GitInfo) -> Result<Issue> {
    println!("🔄 Welcome to GHQC New Round Mode!");

    let milestone = prompt_existing_milestone(milestones)?;
    let issues = git_info.get_issues(Some(milestone.number as u64)).await?;
    if issues.is_empty() {
        bail!("No issues found in milestone '{}'", milestone.title);
    }

    prompt_issue(&issues)
}

/// Optional free-text note, repeated in the round comment and the notification.
fn prompt_note() -> Result<Option<String>> {
    let note = Text::new("📝 Reason for the new round (optional):")
        .prompt()
        .map_err(|e| anyhow!("Input cancelled: {e}"))?;
    let note = note.trim();
    Ok((!note.is_empty()).then(|| note.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::Checklist;
    use crate::{ImpactedIssues, RepairRoundResult, StepOutcome};
    use gix::ObjectId;
    use std::str::FromStr;

    fn result(body_marker: StepOutcome) -> StartRoundResult {
        StartRoundResult {
            round: 2,
            round_comment_url: "https://github.com/o/r/issues/1#issuecomment-1".to_string(),
            anchor: ObjectId::from_str("aaaaaaa000000000000000000000000000000001").unwrap(),
            reopened: StepOutcome::Done,
            body_marker,
            notification: StepOutcome::Skipped,
            impacted_issues: ImpactedIssues::None,
        }
    }

    #[test]
    fn a_fully_successful_round_exits_zero() {
        assert!(report_result(&result(StepOutcome::Done)).is_ok());
    }

    /// A partial failure must not read as success: `main` turns this `Err` into a
    /// non-zero exit code, after the per-step report has already been printed.
    #[test]
    fn a_partial_failure_exits_non_zero() {
        let error = report_result(&result(StepOutcome::Failed("boom".to_string())))
            .expect_err("a failed step must set a non-zero exit code");
        let message = error.to_string();
        assert!(message.contains("Round 2 was recorded"));
        assert!(message.contains("retried on its own"));
    }

    fn repair_result(reopened: StepOutcome) -> RepairRoundResult {
        RepairRoundResult {
            round: 2,
            round_name: "Round 2".to_string(),
            round_comment_url: Some("https://github.com/o/r/issues/1#issuecomment-1".to_string()),
            plan: crate::RepairPlan {
                reopen: true,
                ..Default::default()
            },
            reopened,
            body_marker: StepOutcome::Skipped,
            notification: StepOutcome::Skipped,
        }
    }

    #[test]
    fn a_completed_repair_exits_zero() {
        assert!(report_repair(&repair_result(StepOutcome::Done)).is_ok());
    }

    /// A repair that found nothing to do has left the round complete, which is the
    /// point of the command: success, not a failure.
    #[test]
    fn a_repair_with_nothing_to_do_exits_zero() {
        assert!(report_repair(&repair_result(StepOutcome::Skipped)).is_ok());
    }

    #[test]
    fn a_repair_whose_step_failed_exits_non_zero() {
        let error = report_repair(&repair_result(StepOutcome::Failed("boom".to_string())))
            .expect_err("a failed step must set a non-zero exit code");
        let message = error.to_string();
        assert!(message.contains("Round 2 still has follow-up steps that failed"));
        assert!(message.contains("run again"));
    }

    #[test]
    fn a_named_configuration_checklist_wins_and_is_reset() {
        let mut configuration = Configuration::default();
        configuration.checklists.insert(
            "Code Review".to_string(),
            Checklist::new(
                "Code Review".to_string(),
                None,
                "- [x] Reviewed\n- [ ] Tested".to_string(),
            ),
        );
        let thread = thread();

        let (content, name) = resolve_checklist(
            Some("Code Review"),
            &configuration,
            &thread,
            &[],
            Some("# Ignored\n- [x] From the body"),
        )
        .expect("the named checklist exists");

        assert_eq!(content, "- [ ] Reviewed\n- [ ] Tested");
        assert_eq!(name.as_deref(), Some("Code Review"));
    }

    #[test]
    fn an_unknown_checklist_name_lists_the_available_ones() {
        let mut configuration = Configuration::default();
        configuration.checklists.insert(
            "Code Review".to_string(),
            Checklist::new(
                "Code Review".to_string(),
                None,
                "- [ ] Reviewed".to_string(),
            ),
        );

        let error = resolve_checklist(Some("Nope"), &configuration, &thread(), &[], None)
            .expect_err("an unknown name must fail loudly");
        assert!(error.to_string().contains("Available: Code Review"));
    }

    #[test]
    fn without_a_name_the_issue_body_seeds_the_checklist() {
        let (content, name) = resolve_checklist(
            None,
            &Configuration::default(),
            &thread(),
            &[],
            Some("## Metadata\nx\n\n# Code Review\n- [x] Reviewed"),
        )
        .expect("the body carries a checklist");

        assert_eq!(content, "- [ ] Reviewed");
        assert_eq!(name.as_deref(), Some("Code Review"));
    }

    #[test]
    fn without_any_checklist_at_all_the_command_says_so() {
        let error = resolve_checklist(None, &Configuration::default(), &thread(), &[], None)
            .expect_err("nothing to seed from");
        assert!(error.to_string().contains("--checklist-name"));
    }

    /// A thread whose only round is Initial QC, so seeding falls back to the body.
    fn thread() -> IssueThread {
        use crate::round::{ChecklistSource, Round, RoundOpen, RoundState};
        use std::collections::HashSet;

        let commit = ObjectId::from_str("aaaaaaa000000000000000000000000000000001").unwrap();
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            branch: "main".to_string(),
            open: false,
            commits: vec![crate::IssueCommit {
                hash: commit,
                message: "initial".to_string(),
                statuses: HashSet::new(),
                file_changed: true,
            }],
            milestone: "m1".to_string(),
            blocking_qcs: Vec::new(),
            rounds: vec![Round {
                index: 1,
                opened_at: commit,
                previous_approval: None,
                opened: RoundOpen::IssueCreated,
                checklist: ChecklistSource::IssueBody,
                checklist_name: None,
                state: RoundState::Open,
                events: Vec::new(),
                retractions: Vec::new(),
                extensions: Vec::new(),
            }],
            round_anomalies: Vec::new(),
        }
    }
}
