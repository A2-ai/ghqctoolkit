//! `ghqc issue round create` (C1) — the CLI half of §8.
//!
//! `create` is the **only** `round` subcommand (C2): `round list` is covered by
//! `ghqc issue status` (C3), and `round edit` / `round checklist` are rejected
//! outright because they would mean editing a posted comment.
//!
//! There is no `--commit` and no `--branch`: both come from the current checkout,
//! read-only, exactly like `issue create` (D23). There is no commit picker anywhere
//! in the round flow.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use gix::ObjectId;
use inquire::{Confirm, Editor, Text, validator::Validation};
use octocrab::models::Milestone;

use crate::cli::config_init::editor_command;
use crate::cli::interactive::{prompt_existing_milestone, prompt_issue, prompt_note};
use crate::issue::{qc_rounds_section, splice_qc_rounds};
use crate::qc_status::analyze_checklist_in_text;
use crate::{
    Configuration, DiskCache, GitHubReader, GitHubWriter, GitInfo, GitRepository, IssueThread,
    QCComment, QCRound, QCStatus, Round, configuration::Checklist,
};

/// A prepared `# QC Round N` comment plus the optional `# QC Notification` that
/// follows it (D5). The two comments are always posted as a pair by [`Self::post`],
/// mirroring the A5 route so the CLI and the API cannot drift apart.
pub struct QCRoundCreate {
    pub round: QCRound,
    /// D5: post the separate, ordinary `# QC Notification` comment after the round
    /// comment.
    pub notify: bool,
    pub note: Option<String>,
    pub include_diff: bool,
    /// D5: the prior round's approval commit — `previous commit` on the notification.
    previous_commit: Option<ObjectId>,
}

impl QCRoundCreate {
    #[allow(clippy::too_many_arguments)]
    pub async fn from_args(
        milestone_name: String,
        file: PathBuf,
        checklist_name: Option<String>,
        base_round: Option<u32>,
        note: Option<String>,
        notify: bool,
        no_diff: bool,
        milestones: &[Milestone],
        configuration: &Configuration,
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
    ) -> Result<Self> {
        // An approved QC is a *closed* issue, so the round lookup cannot use
        // `find_issue`'s open-only filter.
        let issue =
            crate::cli::context::find_issue_any_state(&milestone_name, &file, milestones, git_info)
                .await?;

        let thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        ensure_round_startable(&thread, issue.number)?;

        let base = resolve_base_round(&thread, base_round)?;
        let checklist = match checklist_name {
            Some(name) => configuration
                .checklists
                .get(&name)
                .cloned()
                .ok_or_else(|| anyhow!("No checklist named '{name}' in the configuration"))?,
            // `--base-round` seeds the new round from that round's checklist with
            // every `- [x]` reset (U2). Round 1's checklist is the default base.
            None => seed_checklist(base),
        };
        validate_checklist(&checklist)?;

        // D23: branch and start commit both come from the checkout, read-only.
        let (branch, start_commit) = checkout_position(git_info)?;
        let previous_commit = thread.latest_round().approved_commit().copied();
        let notify = resolve_notify(notify, &start_commit, previous_commit.as_ref());

        Ok(Self {
            round: QCRound::new(
                file,
                issue,
                thread.next_round_index(),
                branch,
                start_commit,
                checklist,
            ),
            notify,
            note,
            include_diff: !no_diff,
            previous_commit,
        })
    }

    /// The prompts mirror the `NewRoundModal` tabs (C1/U2): Round (read-only branch
    /// and start commit, notify + note, diff), then Checklist (base round, editable
    /// name and content).
    pub async fn from_interactive(
        milestones: &[Milestone],
        cache: Option<&DiskCache>,
        git_info: &GitInfo,
    ) -> Result<Self> {
        println!("🔄 Welcome to GHQC New Round Mode!");

        let milestone = prompt_existing_milestone(milestones)?;
        let issues = git_info.get_issues(Some(milestone.number as u64)).await?;

        // D46: the picker lists issues in **any** state. D12's gate is
        // `is_approved()`, a function of `RoundState` — not the raw GitHub
        // `IssueState`. The two normally coincide because approval closes the issue,
        // but an approved issue reopened on GitHub without a `# QC Un-Approval` stays
        // round-startable, and a `Closed` filter made it vanish from the picker.
        // `ensure_round_startable` rejects a bad selection below with its own
        // message, which is exactly how the `--milestone`/`--file` flag path behaves.
        if issues.is_empty() {
            bail!(
                "No issues found in milestone '{}' to start a new round for",
                milestone.title
            );
        }

        let issue = prompt_issue(&issues)?;
        let file = PathBuf::from(&issue.title);

        let thread = IssueThread::from_issue(&issue, cache, git_info).await?;
        ensure_round_startable(&thread, issue.number)?;

        // ── Round tab: branch and start commit are read-only from the checkout ──
        let (branch, start_commit) = checkout_position(git_info)?;
        let round_index = thread.next_round_index();
        println!();
        println!("📋 Round {round_index} starts from your current checkout (D23):");
        println!("   🌿 Branch: {branch}");
        println!("   📝 Start commit: {start_commit}");

        // ── Checklist tab ──────────────────────────────────────────────────────
        let base = prompt_base_round(&thread)?;
        let seed = seed_checklist(base);

        let name = Text::new("📋 Checklist name:")
            .with_initial_value(&seed.name)
            .with_validator(|input: &str| {
                if input.trim().is_empty() {
                    Ok(Validation::Invalid("Checklist name cannot be empty".into()))
                } else {
                    Ok(Validation::Valid)
                }
            })
            .prompt()
            .map_err(|e| anyhow!("Input cancelled: {e}"))?;

        // D37: the content excludes its `# ` heading — `Checklist`'s `Display`
        // emits the heading, so the editor must never show it.
        let content = Editor::new("Edit the round's checklist:")
            .with_predefined_text(&seed.content)
            .with_file_extension(".md")
            .with_editor_command(&editor_command())
            .prompt()
            .map_err(|e| anyhow!("Editor cancelled: {e}"))?;

        let checklist = Checklist {
            name: name.trim().to_string(),
            content: if content.ends_with('\n') {
                content
            } else {
                format!("{content}\n")
            },
        };
        validate_checklist(&checklist)?;

        // ── Notification (D5) ──────────────────────────────────────────────────
        let previous_commit = thread.latest_round().approved_commit().copied();
        let nothing_to_diff = previous_commit == Some(start_commit);
        let (notify, note, include_diff) = if nothing_to_diff {
            // U3: the empty-drift case — the checkout is the prior approval, so a
            // notification would diff a commit against itself.
            println!(
                "ℹ️  Notification skipped: the checkout is the previous approval commit, so there is nothing to diff"
            );
            (false, None, false)
        } else {
            let notify = Confirm::new("Notify the difference in a separate comment?")
                .with_default(true)
                .prompt()
                .map_err(|e| anyhow!("Selection cancelled: {e}"))?;
            if notify {
                let note = prompt_note()?;
                let include_diff = Confirm::new("Include the diff in the notification?")
                    .with_default(true)
                    .prompt()
                    .map_err(|e| anyhow!("Selection cancelled: {e}"))?;
                (true, note, include_diff)
            } else {
                (false, None, false)
            }
        };

        println!();
        println!("✨ Creating round {round_index} with:");
        println!("   🎯 Milestone: {}", milestone.title);
        println!("   🎫 Issue: #{} - {}", issue.number, issue.title);
        println!("   📁 File: {}", file.display());
        println!("   📋 Checklist: {}", checklist.name);
        println!("   💬 Notify: {}", if notify { "yes" } else { "no" });
        println!();

        Ok(Self {
            round: QCRound::new(file, issue, round_index, branch, start_commit, checklist),
            notify,
            note,
            include_diff,
            previous_commit,
        })
    }

    /// A5/D51's order, so the CLI writes exactly what the API writes: the
    /// `## QC Rounds` marker, then the round comment, then the re-open (D6), then the
    /// notification (D5).
    pub async fn post(&self, cache: Option<&DiskCache>, git_info: &GitInfo) -> Result<String> {
        let number = self.round.issue.number;

        // D51: the marker goes in first and its failure aborts the round — a cheap path
        // may skip the comment fetch when the marker is absent, and a multi-round issue
        // that looks single-round hands out a stale `Issue.branch`. Over-fetching (the
        // marker written but the comment post failing) is the safe direction.
        let new_body = splice_qc_rounds(
            self.round.issue.body.as_deref().unwrap_or_default(),
            &qc_rounds_section(self.round.round),
        );
        git_info
            .update_issue(number, None, Some(new_body))
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to write the `## QC Rounds` marker to issue #{number}, so round {} was not created: {e}",
                    self.round.round
                )
            })?;

        let comment_url = git_info.post_comment(&self.round).await?;

        // D6/D45: starting a round re-opens the issue. Non-fatal — the round comment is
        // already posted and is what the fold reads — but never silent: a failed re-open
        // leaves S3 reporting `ApprovalRequired` for a round just created.
        let reopened = match git_info.open_issue(number).await {
            Ok(()) => true,
            Err(e) => {
                log::warn!(
                    "Failed to re-open issue #{number} for round {}: {e}",
                    self.round.round
                );
                false
            }
        };

        let notification_url = if self.notify {
            let notification = QCComment {
                file: self.round.file.clone(),
                issue: self.round.issue.clone(),
                current_commit: self.round.start_commit,
                previous_commit: self.previous_commit,
                note: self.note.clone(),
                no_diff: !self.include_diff,
            };
            match git_info.post_comment(&notification).await {
                Ok(url) => Some(url),
                Err(e) => {
                    log::warn!(
                        "Failed to post round {} notification on #{number}: {e}",
                        self.round.round
                    );
                    None
                }
            }
        } else {
            None
        };

        // The round comment changes what the fold reads, so the cached comment list
        // is stale the moment it is posted.
        if let Some(cache) = cache {
            let cache_key = format!("issue_{number}");
            if let Err(e) = cache.invalidate(&["issues", "comments"], &cache_key) {
                log::warn!("Failed to invalidate comment cache for issue #{number}: {e}");
            }
        }

        let mut message = vec![
            format!("✅ QC Round {} created!", self.round.round),
            comment_url,
        ];
        if let Some(url) = notification_url {
            message.push("💬 Notification comment created!".to_string());
            message.push(url);
        } else if self.notify {
            // D45: "requested but failed" is not the same as "not requested", and the
            // user is the one who can post it manually.
            message.push(
                "⚠️  The notification comment could not be posted; the round itself was created"
                    .to_string(),
            );
        }
        if !reopened {
            // D45: visible, not logged — a closed issue with an unapproved latest round
            // reads as `ApprovalRequired` (S3).
            message.push(format!(
                "⚠️  Issue #{number} could not be re-opened; it will report \"approval required\" until it is"
            ));
        }

        Ok(message.join("\n"))
    }
}

/// D12: a new round may be started **only** when `QCStatus::is_approved()` —
/// `Approved` or `ChangesAfterApproval`. Starting from plain `Approved` is legal and
/// produces the D8 overlap case `start_{n+1} == approval_n`. Same gate as A5's.
pub(crate) fn ensure_round_startable(thread: &IssueThread, number: u64) -> Result<()> {
    // D55: refuse rather than guess — the error names the branch to fetch.
    let status = QCStatus::determine_status(thread)?;
    if !status.is_approved() {
        bail!("Issue #{number} is {status}; a new round may only be started from an approved QC");
    }
    Ok(())
}

/// D40: A5 validates the checklist server-side because I9 is no longer a fold-time
/// assertion. The CLI is the other sanctioned path, so it applies the same check.
pub(crate) fn validate_checklist(checklist: &Checklist) -> Result<()> {
    if analyze_checklist_in_text(&checklist.content).total == 0 {
        bail!("A new round requires a checklist with at least one item");
    }
    Ok(())
}

/// D23: read-only from the checkout, exactly like `issue create`.
fn checkout_position(git_info: &GitInfo) -> Result<(String, ObjectId)> {
    let branch = git_info.branch()?;
    let commit = git_info.commit()?;
    let start_commit = ObjectId::from_str(&commit)
        .map_err(|e| anyhow!("Could not read the checkout's commit '{commit}': {e}"))?;
    Ok((branch, start_commit))
}

/// U3: with nothing between the prior approval and the checkout there is no diff to
/// notify, so the request is downgraded rather than posting a self-comparison.
fn resolve_notify(notify: bool, start: &ObjectId, previous: Option<&ObjectId>) -> bool {
    if notify && previous == Some(start) {
        log::warn!(
            "Not notifying: the checkout is the previous approval commit, so there is nothing to diff"
        );
        return false;
    }
    notify
}

fn resolve_base_round(thread: &IssueThread, base_round: Option<u32>) -> Result<&Round> {
    match base_round {
        Some(index) => thread.round(index).ok_or_else(|| {
            anyhow!(
                "--base-round {index} does not exist; this QC has {} round(s)",
                thread.rounds.len()
            )
        }),
        None => Ok(thread.latest_round()),
    }
}

/// U2: the base-round select is shown only when there is more than one prior round.
fn prompt_base_round(thread: &IssueThread) -> Result<&Round> {
    if thread.rounds.len() < 2 {
        return Ok(thread.latest_round());
    }

    let options: Vec<String> = thread.rounds.iter().map(describe_base_round).collect();
    let selection = inquire::Select::new("📋 Seed the checklist from which round?", options)
        .with_starting_cursor(thread.rounds.len() - 1)
        .prompt()
        .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

    thread
        .rounds
        .iter()
        .find(|round| describe_base_round(round) == selection)
        .ok_or_else(|| anyhow!("Selected round not found"))
}

fn describe_base_round(round: &Round) -> String {
    let summary = round.checklist.summary();
    let name = if round.checklist.name.is_empty() {
        "(no checklist)"
    } else {
        &round.checklist.name
    };
    format!("Round {} — {name} {summary}", round.index)
}

/// Seed a new round's checklist from `base`, with every `- [x]` reset to `- [ ]`
/// (U2).
///
/// D37: `RoundChecklist.content` **excludes** its `# ` heading and
/// `configuration::Checklist`'s `Display` emits one, so the heading is carried in
/// `name` and never re-added to the content — re-adding it emits the H1 twice and
/// makes F4's "second H1" rule latch onto the wrong heading in the next round.
pub(crate) fn seed_checklist(base: &Round) -> Checklist {
    Checklist {
        name: base.checklist.name.clone(),
        content: reset_checkboxes(&base.checklist.content),
    }
}

fn reset_checkboxes(content: &str) -> String {
    content.replace("- [x]", "- [ ]").replace("- [X]", "- [ ]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::{Approval, Gap, IssueCommit, RoundChecklist, RoundPlacement, RoundState};
    use std::collections::HashSet;

    fn commit(hex: &str) -> ObjectId {
        ObjectId::from_hex(hex.as_bytes()).unwrap()
    }

    fn issue_commit(hex: &str) -> IssueCommit {
        IssueCommit {
            hash: commit(hex),
            message: "a commit".to_string(),
            statuses: HashSet::new(),
            file_changed: true,
        }
    }

    fn round(index: u32, checklist: RoundChecklist, state: RoundState) -> Round {
        let start = format!("{index}{}", "0".repeat(39));
        Round {
            index,
            branch: format!("round-{index}"),
            branch_inherited: false,
            placement: RoundPlacement::Placed,
            start_commit: commit(&start),
            preceding_gap: Gap::default(),
            checklist,
            commits: vec![issue_commit(&start)],
            state,
        }
    }

    fn thread(rounds: Vec<Round>, drift: Gap) -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/main.rs"),
            milestone: "v1".to_string(),
            open: false,
            blocking_qcs: Vec::new(),
            rounds,
            drift,
        }
    }

    fn checklist(name: &str, content: &str) -> RoundChecklist {
        RoundChecklist {
            name: name.to_string(),
            content: content.to_string(),
        }
    }

    /// `--base-round` seeds the new checklist from that round's checklist with all
    /// `- [x]` reset, and per D37 the heading is **not** in the content — so the
    /// rendered comment carries exactly one `# Name` line.
    #[test]
    fn test_base_round_seeds_an_unchecked_checklist_without_duplicating_the_heading() {
        let base = round(
            1,
            checklist(
                "First Pass",
                "\n- [x] ran the model\n- [X] eyeballed the output\n- [ ] not done\n",
            ),
            RoundState::Approved(Approval {
                commit: commit(&format!("1{}", "0".repeat(39))),
                comment_id: None,
            }),
        );

        let seeded = seed_checklist(&base);

        assert_eq!(seeded.name, "First Pass");
        assert!(!seeded.content.contains("- [x]"));
        assert!(!seeded.content.contains("- [X]"));
        assert_eq!(seeded.content.matches("- [ ]").count(), 3);

        // D37: the heading lives in `name`, never in `content`.
        assert!(!seeded.content.contains("# First Pass"));

        // `Checklist`'s `Display` is what `QCRound` renders, and it emits the
        // heading itself — exactly once.
        let rendered = seeded.to_string();
        assert_eq!(rendered.matches("# First Pass").count(), 1);
        assert!(rendered.starts_with("# First Pass\n"));
    }

    #[test]
    fn test_base_round_selection_addresses_the_requested_round() {
        let thread = thread(
            vec![
                round(
                    1,
                    checklist("First Pass", "- [x] one\n"),
                    RoundState::Unapproved,
                ),
                round(
                    2,
                    checklist("Second Pass", "- [ ] two\n"),
                    RoundState::Unapproved,
                ),
            ],
            Gap::default(),
        );

        assert_eq!(resolve_base_round(&thread, Some(1)).unwrap().index, 1);
        // Default is the latest round.
        assert_eq!(resolve_base_round(&thread, None).unwrap().index, 2);

        let error = resolve_base_round(&thread, Some(3))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--base-round 3"), "unexpected: {error}");
    }

    /// D12: the CLI applies the same gate as A5 — `is_approved()` only.
    #[test]
    fn test_d12_gate_rejects_a_non_approved_issue() {
        let open = thread(
            vec![round(
                1,
                checklist("First Pass", "- [ ] one\n"),
                RoundState::Unapproved,
            )],
            Gap::default(),
        );

        let error = ensure_round_startable(&open, 7).unwrap_err().to_string();
        assert!(error.contains("#7"), "unexpected: {error}");
        assert!(
            error.contains("only be started from an approved QC"),
            "unexpected: {error}"
        );
    }

    /// D46: the picker no longer filters on the raw GitHub `IssueState`, because D12's
    /// gate is `is_approved()` — a function of `RoundState`. An approved QC that someone
    /// reopened on GitHub *without* a `# QC Un-Approval` is still round-startable, and a
    /// `Closed` filter made it vanish from the picker entirely. The gate is what decides,
    /// after selection, exactly as on the `--milestone`/`--file` flag path.
    #[test]
    fn test_d46_gate_accepts_an_approved_qc_whose_issue_is_open_on_github() {
        let mut reopened_on_github = thread(
            vec![round(
                1,
                checklist("First Pass", "- [ ] one\n"),
                RoundState::Approved(Approval {
                    commit: commit(&format!("1{}", "0".repeat(39))),
                    comment_id: None,
                }),
            )],
            Gap::default(),
        );
        reopened_on_github.open = true;

        assert!(
            ensure_round_startable(&reopened_on_github, 7).is_ok(),
            "the GitHub issue state is not the gate (D46)"
        );
    }

    #[test]
    fn test_d12_gate_accepts_approved_and_changes_after_approval() {
        let approved = thread(
            vec![round(
                1,
                checklist("First Pass", "- [ ] one\n"),
                RoundState::Approved(Approval {
                    commit: commit(&format!("1{}", "0".repeat(39))),
                    comment_id: None,
                }),
            )],
            Gap::default(),
        );
        assert!(ensure_round_startable(&approved, 7).is_ok());

        // ChangesAfterApproval is `is_approved()` too (D12).
        let mut changed = approved.clone();
        changed.drift = Gap {
            commits: vec![issue_commit(&format!("a{}", "0".repeat(39)))],
            divergent: false,
        };
        assert!(ensure_round_startable(&changed, 7).is_ok());
    }

    /// D40: the CLI validates the checklist because the fold no longer asserts I9.
    #[test]
    fn test_empty_checklist_is_refused() {
        let empty = Checklist {
            name: "Second Pass".to_string(),
            content: "no items here\n".to_string(),
        };
        let error = validate_checklist(&empty).unwrap_err().to_string();
        assert!(error.contains("at least one item"), "unexpected: {error}");

        assert!(
            validate_checklist(&Checklist {
                name: "Second Pass".to_string(),
                content: "- [ ] one item\n".to_string(),
            })
            .is_ok()
        );
    }

    /// U3: the empty-drift case has nothing to diff, so the notification is dropped
    /// rather than posted as a self-comparison.
    #[test]
    fn test_notify_is_dropped_when_the_checkout_is_the_previous_approval() {
        let approval = commit(&format!("1{}", "0".repeat(39)));
        let newer = commit(&format!("a{}", "0".repeat(39)));

        assert!(!resolve_notify(true, &approval, Some(&approval)));
        assert!(resolve_notify(true, &newer, Some(&approval)));
        assert!(!resolve_notify(false, &newer, Some(&approval)));
    }
}
