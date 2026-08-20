use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt, fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use futures::future;
use gix::ObjectId;
use inquire::{
    Autocomplete, Confirm, CustomUserError, MultiSelect, Select, Text, list_option::ListOption,
    validator::Validation,
};
use octocrab::models::Milestone;

use super::section_header;
use crate::round::{Placement, UnplaceableReason};
use crate::{
    ArchiveError, ArchiveQC, ArchiveTarget, DiskCache, GitCommitOps, GitHubReader, GitRepository,
    IssueThread, Round, archive::ArchiveFile, get_issue_comments, git::GitCommit, selected_round,
};

/// An issue thread paired with the issue number it was folded from.
///
/// The number is not part of [`IssueThread`] — the model has no use for it — but the
/// archive addresses rounds by issue number (`--round <issue#>=<n>`), so this path
/// carries it alongside.
#[derive(Debug, Clone)]
pub struct MilestoneIssueThread {
    pub number: u64,
    pub thread: IssueThread,
}

/// One file's archive selection: its thread, and the round the archive addresses.
///
/// The open/approved line is not a filter here — it is this per-file selection, and the
/// bytes follow from the round.
#[derive(Debug, Clone)]
pub struct ArchiveSelection {
    pub number: u64,
    pub thread: IssueThread,
    pub target: ArchiveTarget,
}

/// A selection the archive cannot serve: the round it addresses owns no locatable
/// commits, so there is no commit the archive could honestly point at.
#[derive(Debug, Clone)]
pub struct UnplaceableSelection {
    pub number: u64,
    pub file: PathBuf,
    /// The round that could not be placed, already named for a reader.
    pub round: String,
    pub reason: UnplaceableReason,
}

/// Abbreviated commit, as everywhere else the CLI prints one.
fn short(commit: &ObjectId) -> String {
    commit.to_string()[..7].to_string()
}

/// Whether any round on this thread has ever closed.
///
/// The whole of what `--include-unapproved` governs: it admits threads *no round of
/// which has ever closed*, and nothing else. It deliberately says nothing about an
/// approved-then-reopened file — that file has an approval standing in an earlier round,
/// and reaching it is a round selection rather than a filter.
pub fn has_closed_round(thread: &IssueThread) -> bool {
    thread
        .rounds()
        .any(|round| round.closing_commit().is_some())
}

/// Pair each thread with the round the archive addresses, defaulting to the latest.
///
/// A `--round` naming an issue outside the selected milestones is reported rather than
/// ignored: ignoring it would archive the latest round of every file while the user
/// believed one of them had been retargeted, and nothing in the output would say so.
pub fn resolve_selections(
    threads: Vec<MilestoneIssueThread>,
    targets: &HashMap<u64, ArchiveTarget>,
) -> Result<Vec<ArchiveSelection>> {
    let mut unknown: Vec<u64> = targets
        .keys()
        .copied()
        .filter(|number| !threads.iter().any(|thread| thread.number == *number))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        bail!(
            "--round names issue(s) that are not in the selected milestones: {}",
            unknown
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(threads
        .into_iter()
        .map(|thread| ArchiveSelection {
            // Absent from the map ⇒ the latest round: the archive shows current reality
            // first, and going back a round is the explicit act.
            target: targets
                .get(&thread.number)
                .copied()
                .unwrap_or(ArchiveTarget::Latest),
            number: thread.number,
            thread: thread.thread,
        })
        .collect())
}

/// Refuse a `--round` that no milestone selection can reach.
///
/// With no milestone selected there is no thread to address, so every arm that takes that
/// path either prompts for the round per file or archives no milestone file at all — and
/// in both cases the map is never consulted. Accepting the flag and honouring it nowhere
/// is the same silent retarget that reporting an unknown issue number exists to prevent:
/// the user believes one file was retargeted and nothing in the output says otherwise.
pub fn reject_unreachable_round_targets(targets: &HashMap<u64, ArchiveTarget>) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    bail!(
        "--round selects a round for an issue in a chosen milestone, and no milestone is selected: \
         name the milestones, or a milestone selection flag, alongside it. Interactive mode asks \
         for the round per file instead"
    )
}

/// Split selections into those the archive can serve and those it cannot.
///
/// The predicate is **not** implemented here: [`crate::selected_round`] owns it, and both
/// surfaces call it. The two had already drifted to opposite answers about which segment
/// to gate on within one wave — one surface archiving a file the other refused, from the
/// same repository — so the rule has one home and this function only sorts and renders.
/// Which round it gates on, and why an unplaceable trailing gap does not block, is
/// documented there.
///
/// A selection naming no round at all is deliberately left in the servable half: its
/// remedy is the out-of-range report from construction, which names the round count,
/// not an unplaceable callout naming a reason that does not apply to it.
pub fn partition_placeable(
    selections: Vec<ArchiveSelection>,
) -> (Vec<ArchiveSelection>, Vec<UnplaceableSelection>) {
    let mut placeable = Vec::new();
    let mut blocked = Vec::new();

    for selection in selections {
        match selected_round(&selection.thread, selection.target) {
            Ok(round) if !round.is_archivable() => {
                let reason = round
                    .unplaceable_reason()
                    .expect("a round that is not archivable is unplaceable");
                blocked.push(UnplaceableSelection {
                    number: selection.number,
                    file: selection.thread.file.clone(),
                    round: round.name,
                    reason,
                })
            }
            _ => placeable.push(selection),
        }
    }

    (placeable, blocked)
}

/// The blocking callout for files whose selected round could not be placed.
///
/// Naming them is the whole point: it replaces a note that said only how many files were
/// left out. Acknowledging it proceeds **without** these files — it never includes them.
/// An unplaceable round owns no commits, so there is no commit an archive could record
/// for one, and a plausible-looking substitute is exactly what an audit artifact must
/// not contain. A user who knows the commit they want adds the file directly with
/// `--additional-file <path>:<commit>`, which is what that mode is for.
pub fn unplaceable_callout(blocked: &[UnplaceableSelection]) -> String {
    let mut lines = vec![format!(
        "🚫 {} file(s) cannot be archived — the round selected for each owns no locatable commits:",
        blocked.len()
    )];
    for file in blocked {
        lines.push(format!(
            "   - {} (#{}, {}): {}",
            file.file.display(),
            file.number,
            file.round,
            file.reason.describe()
        ));
    }
    lines.push(
        "   Proceeding leaves these files out of the archive entirely. To archive one anyway, \
         add it at a commit you name with `--additional-file <path>:<commit>`."
            .to_string(),
    );
    lines.join("\n")
}

/// Build one [`ArchiveFile`] per selection, at the round it addresses.
pub fn build_archive_files(
    selections: &[ArchiveSelection],
    flatten: bool,
) -> Result<Vec<ArchiveFile>> {
    selections
        .iter()
        .map(|selection| {
            ArchiveFile::from_issue_thread(&selection.thread, flatten, selection.target).map_err(
                |err| match err {
                    // `ArchiveError` carries the path only, and the CLI addresses rounds
                    // by issue number, so the message has to name what the user typed.
                    ArchiveError::RoundSelection { round, rounds, .. } => anyhow::anyhow!(
                        "--round {issue}={round}: issue #{issue} ({file}) has {rounds} round(s), \
                         so the rounds that can be archived are 1..={rounds}",
                        issue = selection.number,
                        file = selection.thread.file.display(),
                    ),
                    // The library refuses an unplaceable selected round too — a gate a
                    // caller may decline to consult is advisory. Reaching this means the
                    // pre-flight partition was skipped, so it is reported through the
                    // same callout rather than as a bare error carrying no issue number.
                    ArchiveError::UnplaceableRound {
                        file,
                        round,
                        reason,
                    } => anyhow::anyhow!(
                        "{}",
                        unplaceable_callout(&[UnplaceableSelection {
                            number: selection.number,
                            file,
                            // The thread is in hand here, so the round names itself; the
                            // index form is the same authority for the case where the
                            // error's index names no round on the thread.
                            round: selection
                                .thread
                                .rounds()
                                .find(|candidate| candidate.index == round)
                                .map(Round::name)
                                .unwrap_or_else(|| Round::name_of(round)),
                            reason,
                        }])
                    ),
                    other => other.into(),
                },
            )
        })
        .collect()
}

/// Which bucket a file falls into at a given archive target.
///
/// **The one categorization behind both surfaces**: the archive's own pre-write report and
/// `ghqc milestone status`'s pre-archive summary. A readiness check that sorted files
/// differently from the archive it precedes would be worse than no check at all — the user
/// runs it precisely to decide whether the archive is the one they want — so neither
/// surface classifies for itself; both call this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveCategory {
    /// The selected round's approval, and provably still the newest QC state.
    ApprovedCurrent,
    /// The selected round's approval, but something newer may exist.
    ApprovedSuperseded,
    /// The archived bytes were never approved. At the default target this is where an
    /// approved-then-reopened file lands, because unapproved bytes are what *would be
    /// archived*; the earlier round's approval is reached by selecting that round, not by
    /// counting it here.
    Unapproved { round: u32 },
    /// The selected round owns no locatable commits, so the file cannot be archived.
    NotPlaceable(UnplaceableReason),
}

impl ArchiveCategory {
    /// Categorize from provenance the archive has already derived.
    ///
    /// `approval == None` implies `superseded`, so the unapproved arm ignores the flag
    /// rather than asserting on it.
    pub fn of_qc(qc: &ArchiveQC) -> Self {
        match (&qc.round.approval, qc.round.superseded) {
            (Some(_), false) => Self::ApprovedCurrent,
            (Some(_), true) => Self::ApprovedSuperseded,
            (None, _) => Self::Unapproved {
                round: qc.round.round,
            },
        }
    }
}

/// Categorize `thread` at `target` without writing anything.
///
/// The same gate and the same derivation the archive itself runs — [`selected_round`] then
/// [`ArchiveFile::from_issue_thread`] — so a summary cannot report one thing and the
/// archive produce another. `Err` only when the target names no round on the thread, which
/// is the error the archive reports for the same input.
pub fn categorize(
    thread: &IssueThread,
    target: ArchiveTarget,
) -> Result<ArchiveCategory, ArchiveError> {
    let selection = selected_round(thread, target)?;
    if let Some(reason) = selection.unplaceable_reason() {
        return Ok(ArchiveCategory::NotPlaceable(reason));
    }

    // `flatten` is not consulted by the derivation — it only names the path inside the
    // tarball — so the categorization is the same whichever way the archive is laid out.
    let file = ArchiveFile::from_issue_thread(thread, false, target)?;
    let qc = file
        .qc
        .as_ref()
        .expect("from_issue_thread always records provenance for a milestone thread");
    Ok(ArchiveCategory::of_qc(qc))
}

/// The counts behind the one-line archive summary both surfaces print.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveSummary {
    pub files: usize,
    pub approved_current: usize,
    pub approved_superseded: usize,
    pub unapproved: usize,
    /// The rounds the unapproved files are open at. Rendered only when they agree:
    /// "(round 3 open)" is a fact about one round and says nothing true about a mixed set.
    pub unapproved_rounds: BTreeSet<u32>,
    pub not_placeable: usize,
    /// Files added by path and commit. They carry no QC claim at all, so they are counted
    /// apart from every QC bucket rather than folded into one.
    pub added: usize,
}

impl ArchiveSummary {
    /// Count one categorized file.
    pub fn push(&mut self, category: ArchiveCategory) {
        self.files += 1;
        match category {
            ArchiveCategory::ApprovedCurrent => self.approved_current += 1,
            ArchiveCategory::ApprovedSuperseded => self.approved_superseded += 1,
            ArchiveCategory::Unapproved { round } => {
                self.unapproved += 1;
                self.unapproved_rounds.insert(round);
            }
            ArchiveCategory::NotPlaceable(_) => self.not_placeable += 1,
        }
    }

    /// Summarize the archive that is about to be written.
    ///
    /// Reads the provenance already derived for each file rather than re-deriving it, and
    /// through the same [`ArchiveCategory::of_qc`] the check above reaches.
    pub fn of_files(files: &[ArchiveFile]) -> Self {
        let mut summary = Self::default();
        for file in files {
            match &file.qc {
                Some(qc) => summary.push(ArchiveCategory::of_qc(qc)),
                None => {
                    summary.files += 1;
                    summary.added += 1;
                }
            }
        }
        summary
    }
}

impl fmt::Display for ArchiveSummary {
    /// `12 files · 9 approved & current · 2 approved but superseded · 1 unapproved (round 3 open)`
    ///
    /// Empty buckets are omitted: a line that reads `12 files · 12 approved & current` is
    /// the whole story, and printing four zeroes beside it would bury it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = vec![format!(
            "{} {}",
            self.files,
            if self.files == 1 { "file" } else { "files" }
        )];
        if self.approved_current > 0 {
            parts.push(format!("{} approved & current", self.approved_current));
        }
        if self.approved_superseded > 0 {
            parts.push(format!(
                "{} approved but superseded",
                self.approved_superseded
            ));
        }
        if self.unapproved > 0 {
            let detail = match self.unapproved_rounds.iter().copied().collect::<Vec<_>>()[..] {
                [1] => " (Initial QC open)".to_string(),
                [round] => format!(" (round {round} open)"),
                _ => String::new(),
            };
            parts.push(format!("{} unapproved{detail}", self.unapproved));
        }
        if self.not_placeable > 0 {
            parts.push(format!("{} not placeable", self.not_placeable));
        }
        if self.added > 0 {
            parts.push(format!("{} added", self.added));
        }
        write!(f, "{}", parts.join(" · "))
    }
}

/// One file's provenance line: the round the selection addressed, whether those bytes
/// were approved, the commit, and whether they were still the newest QC state.
///
/// The two round frames are kept apart on purpose. Nothing here claims the *selected*
/// round was approved — only that the commit taken is some round's approval — because a
/// round's anchor may be the previous round's closing commit, and collapsing the two
/// into one "approved round" is how approved content came to be labelled unapproved in
/// the first place.
fn provenance_line(qc: &ArchiveQC, commit: &ObjectId) -> String {
    let bytes = match &qc.round.approval {
        Some(approval) if approval.round == qc.round.round => format!(
            "approved by @{} on {} · {}",
            approval.by,
            approval.at.format("%Y-%m-%d"),
            short(commit)
        ),
        Some(approval) => format!(
            "bytes are {}'s approval by @{} on {} · {}",
            Round::name_of(approval.round),
            approval.by,
            approval.at.format("%Y-%m-%d"),
            short(commit)
        ),
        None => format!("unapproved · {}", short(commit)),
    };
    // The marker comes from the shared categorization rather than from a second reading
    // of the provenance: whatever `ghqc milestone status` counts as not-current is exactly
    // what this labels as not-current.
    let superseded = if ArchiveCategory::of_qc(qc) == ArchiveCategory::ApprovedCurrent {
        ""
    } else {
        " · ⚠️ not the newest QC state"
    };
    format!("{} · {bytes}{superseded}", Round::name_of(qc.round.round))
}

/// Print, per file, the provenance of the bytes about to be archived.
///
/// The default target is the *latest round*, not the newest approval, so an
/// approved-then-reopened file archives **unapproved** bytes unless the user retargets
/// it. That is intentional and nothing gates it: there is a reason a round is open, and
/// the archive shows current reality first. It is, however, never silent — every file
/// says which round it came from, whether those bytes were approved, at which commit,
/// and whether they were the newest QC state when the archive was cut.
pub fn report_archive_provenance(files: &[ArchiveFile]) {
    if files.is_empty() {
        return;
    }

    println!("\n{}", section_header("Archive contents"));
    for file in files {
        println!("  {}", file.repository_file.display());
        match &file.qc {
            Some(qc) => println!("    {}", provenance_line(qc, &file.commit)),
            // A file in no milestone carries no QC claim at all: calling it unapproved
            // would itself be a claim about a file nobody put under QC.
            None => println!("    added file · {}", short(&file.commit)),
        }
    }

    // The same sentence `ghqc milestone status` prints, from the same counts, so a user
    // can hold the pre-archive check and the archive side by side.
    let summary = ArchiveSummary::of_files(files);
    println!("\n  {summary}");

    let unapproved = summary.unapproved;
    if unapproved > 0 {
        println!(
            "\n⚠️  {unapproved} file(s) are archived at unapproved bytes: the round selected for \
             them is open. This is the default — the latest round, not the newest approval. \
             Target an earlier round to archive the approval that stands there."
        );
    }
}

pub async fn prompt_archive(
    milestones: &[Milestone],
    current_dir: &PathBuf,
    git_info: &(impl GitHubReader + GitCommitOps + GitRepository),
    cache: Option<&DiskCache>,
) -> Result<(Vec<ArchiveFile>, PathBuf)> {
    println!("📦 Welcome to GHQC Milestone Archive Mode!");

    let milestone_selection_method = Select::new(
        "📦 How would you like to select milestones for the archive?",
        vec![
            "📋 Select All Milestones",
            "🎯 Choose Specific Milestones",
            "🚫 Select No Milestones",
        ],
    )
    .prompt()
    .map_err(|e| anyhow::anyhow!("Selection cancelled: {e}"))?;

    let milestones = match milestone_selection_method {
        "📋 Select All Milestones" => {
            let mut filtered_milestones = prompt_open_milestones()?.filter_milestones(milestones);
            filtered_milestones.sort_by(|a, b| b.number.cmp(&a.number));
            if filtered_milestones.is_empty() {
                bail!(
                    "No milestones available with the selected filter. Try including open milestones or check if you have any milestones in your repository."
                );
            }
            filtered_milestones
        }
        "🎯 Choose Specific Milestones" => {
            let mut filtered_milestones = prompt_open_milestones()?.filter_milestones(milestones);
            filtered_milestones.sort_by(|a, b| b.number.cmp(&a.number));

            if filtered_milestones.is_empty() {
                bail!(
                    "No milestones available with the selected filter. Try including open milestones or check if you have any milestones in your repository."
                );
            }

            let milestone_options = filtered_milestones
                .into_iter()
                .map(|m| m.title.to_string())
                .collect::<Vec<_>>();

            let selected_strings =
                MultiSelect::new("📦 Select milestones for the archive:", milestone_options)
                    .with_validator(|selection: &[ListOption<&String>]| {
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

            milestones
                .iter()
                .filter(|m| selected_strings.contains(&m.title))
                .collect()
        }
        "🚫 Select No Milestones" => Vec::new(),
        _ => unreachable!("Milestone Selection Methods can only be 1 of 3 values"),
    };

    let (selections, select_additional_files) = if milestone_selection_method
        != "🚫 Select No Milestones"
    {
        // Narrow, and named for what it actually filters: whether any round on the
        // thread has ever closed. It does not decide anything about a file that was
        // approved and is now under review again — that file's approval is still on the
        // record, and reaching it is the round selection below.
        let skip_never_approved = Confirm::new("✅ Skip files that have never been approved?")
            .with_default(true)
            .with_help_message(
                "Y = skip files no round has ever closed on, n = include them. A file approved \
                 in an earlier round is kept either way; which round it is archived at is asked \
                 separately",
            )
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        let mut issue_threads = get_milestone_issue_threads(&milestones, git_info, cache).await?;

        if skip_never_approved {
            issue_threads.retain(|i| has_closed_round(&i.thread));
        };

        // A render-time comparison against the checkout, not a derivation: nothing about
        // any file's status or archived bytes depends on it. It exists for the
        // additional-file picker below, whose commits come from the checked-out history,
        // so it compares against each issue's *active* segment branch — the branch each
        // issue is being QC'd on now, which is the only "the issue's branch" there is.
        if !issue_threads.iter().any(|i| {
            git_info
                .branch()
                .map(|b| b == i.thread.active_branch())
                .unwrap_or(true)
        }) {
            println!(
                "⚠️ No issue in the selected milestones is being QC'd on the checked-out branch. Commits offered for additional files come from this branch, so they may share no history with the archived files"
            );
        }

        let selections = resolve_selections(issue_threads, &HashMap::new())?;

        // I4: an unplaceable thread never reaches archive construction. The override is
        // "acknowledge and proceed without these files", never "include them".
        let (selections, blocked) = partition_placeable(selections);
        if !blocked.is_empty() {
            println!("\n{}", unplaceable_callout(&blocked));
            let proceed = Confirm::new("🚫 Proceed without those file(s)?")
                .with_default(false)
                .with_help_message(
                    "N = stop here, y = archive the remaining files and leave those out",
                )
                .prompt()
                .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;
            if !proceed {
                bail!(
                    "Archive cancelled: {} file(s) could not be placed",
                    blocked.len()
                );
            }
        }

        let selections = prompt_round_targets(selections)?;

        let select_additional_files = Confirm::new("📄 Select additional files?")
            .with_default(false)
            .with_help_message("N = milestone files only, y = select additional files and commits")
            .prompt()
            .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        (selections, select_additional_files)
    } else {
        (Vec::new(), true)
    };

    let additional_files = if select_additional_files {
        let commits = git_info.commits(&None, None)?;
        let milestone_selected_files = selections
            .iter()
            .map(|s| s.thread.file.as_path())
            .collect::<Vec<_>>();
        prompt_archive_files(current_dir, &milestone_selected_files, &commits, git_info)?
    } else {
        Vec::new()
    };

    let flatten = Confirm::new("📁 Flatten archive directory structure?")
        .with_default(false)
        .with_help_message(
            "N = retain repository structure, y = strip folder structure from selected files",
        )
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    let mut archive_files = build_archive_files(&selections, flatten)?;
    for (file, commit) in additional_files {
        archive_files.push(ArchiveFile::from_file(file, commit, flatten));
    }

    // Generate default archive name based on milestones
    let default_archive_name = generate_archive_name(&milestones, git_info);
    let default_archive_path = PathBuf::from("archive").join(&default_archive_name);

    // Prompt user for archive path with default
    let archive_path_input = Text::new("📁 Enter archive path:")
        .with_default(&default_archive_path.to_string_lossy())
        .with_help_message("Press Enter to use the default path shown above")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

    let final_archive_path = PathBuf::from(archive_path_input.trim());

    Ok((archive_files, final_archive_path))
}

/// One round, described well enough to choose it: which round, whether it is open or
/// closed, the commit the archive would take, and who approved it and when.
///
/// This renders a round the user is *considering*, so it reads `Placement` for display
/// only — it is not a second copy of the archivability gate, which lives in
/// [`crate::selected_round`] and is asked once, about the round actually selected. A round
/// shown as not placeable here is labelled, not filtered: the choice is still the user's,
/// and choosing it is refused at the gate with the same wording.
fn describe_round(round: &Round) -> String {
    if let Placement::Unplaceable(reason) = &round.placement {
        return format!("{} · not placeable — {}", round.name(), reason.describe());
    }

    match &round.state {
        crate::RoundState::Closed { commit, by, at, .. } => format!(
            "{} · closed · approved by @{by} on {} · {}",
            round.name(),
            at.format("%Y-%m-%d"),
            short(commit)
        ),
        // An open round archives its newest *actioned* commit — its anchor, a
        // notification or a review — so that is the commit named here, not the newest
        // commit on the branch.
        crate::RoundState::Open => format!(
            "{} · open · latest actioned commit {}",
            round.name(),
            round
                .latest_actioned_commit()
                .map(|commit| short(&commit.hash))
                .unwrap_or_else(|| "unknown".to_string())
        ),
    }
}

/// Offer a per-file round selection, behind a single confirm.
///
/// Only files with more than one round are asked about: for everything else there is
/// nothing to choose, and the common case stays one keystroke. Declining leaves every
/// file at the default — its latest round.
fn prompt_round_targets(mut selections: Vec<ArchiveSelection>) -> Result<Vec<ArchiveSelection>> {
    let multi_round: Vec<usize> = selections
        .iter()
        .enumerate()
        .filter(|(_, selection)| selection.thread.rounds().count() > 1)
        .map(|(position, _)| position)
        .collect();

    if multi_round.is_empty() {
        return Ok(selections);
    }

    let customize = Confirm::new("🎯 Choose which round each file is archived at?")
        .with_default(false)
        .with_help_message(&format!(
            "N = the latest round of every file, y = choose a round for the {} file(s) \
             that have more than one",
            multi_round.len()
        ))
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

    if !customize {
        return Ok(selections);
    }

    for position in multi_round {
        // Collected before prompting so the thread is not borrowed while the selection
        // it belongs to is retargeted.
        let rounds: Vec<(u32, String)> = selections[position]
            .thread
            .rounds()
            .map(|round| (round.index, describe_round(round)))
            .collect();
        let file = selections[position].thread.file.clone();

        let chosen = Select::new(
            &format!("🎯 Round to archive {} at:", file.display()),
            rounds.iter().map(|(_, label)| label.clone()).collect(),
        )
        // The latest round is the default everywhere else, so the cursor starts there.
        .with_starting_cursor(rounds.len() - 1)
        .raw_prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        selections[position].target = ArchiveTarget::Round(rounds[chosen.index].0);
    }

    Ok(selections)
}

/// Every milestone file to archive, for the non-interactive path.
///
/// One implementation for all three milestone-selection flags: they differ only in which
/// milestones they resolve, and three copies of the filter is how the four disagreeing
/// definitions of "approved" got in.
#[allow(clippy::too_many_arguments)]
pub async fn milestone_archive_files(
    selected_milestones: &[&Milestone],
    round_targets: &HashMap<u64, ArchiveTarget>,
    include_unapproved: bool,
    skip_unplaceable: bool,
    flatten: bool,
    git_info: &(impl GitHubReader + GitCommitOps + GitRepository),
    cache: Option<&DiskCache>,
) -> Result<Vec<ArchiveFile>> {
    let mut threads = get_milestone_issue_threads(selected_milestones, git_info, cache).await?;

    if !include_unapproved {
        // Warned rather than silently honoured: a `--round` on a file the filter drops
        // reads as a request for that file, and the round would simply never be used.
        for dropped in threads
            .iter()
            .filter(|thread| !has_closed_round(&thread.thread))
            .filter(|thread| round_targets.contains_key(&thread.number))
        {
            println!(
                "⚠️ --round names issue #{} ({}), which no round has ever closed on: it is left out \
                 without --include-unapproved",
                dropped.number,
                dropped.thread.file.display()
            );
        }
        threads.retain(|thread| has_closed_round(&thread.thread));
    }

    let selections = resolve_selections(threads, round_targets)?;

    // I4: an unplaceable thread never reaches archive construction. Naming the files is
    // the block; `--skip-unplaceable` is the acknowledgement that proceeds without them,
    // and it never includes them.
    let (selections, blocked) = partition_placeable(selections);
    if !blocked.is_empty() {
        println!("{}", unplaceable_callout(&blocked));
        if !skip_unplaceable {
            bail!(
                "{} file(s) could not be placed (listed above). Re-run with --skip-unplaceable to \
                 archive the rest without them.",
                blocked.len()
            );
        }
    }

    build_archive_files(&selections, flatten)
}

fn prompt_open_milestones() -> Result<MilestoneSelectionFilter> {
    let include_open_milestones = Confirm::new("📦 Include open milestones?")
        .with_default(false)
        .with_help_message("N = include only closed milestones, y = include all milestones")
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {e}"))?;

    if include_open_milestones {
        Ok(MilestoneSelectionFilter::All)
    } else {
        Ok(MilestoneSelectionFilter::ClosedOnly)
    }
}

pub enum MilestoneSelectionFilter {
    OpenOnly,
    ClosedOnly,
    All,
}

impl MilestoneSelectionFilter {
    pub fn filter_milestones<'a>(&self, milestones: &'a [Milestone]) -> Vec<&'a Milestone> {
        match self {
            Self::OpenOnly => milestones
                .iter()
                .filter(|m| m.state.as_deref() == Some("open"))
                .collect(),
            Self::ClosedOnly => milestones
                .iter()
                .filter(|m| m.state.as_deref() == Some("closed"))
                .collect(),
            Self::All => milestones.iter().collect(),
        }
    }
}

/// Fold every issue in the selected milestones into a thread, keeping its issue number.
///
/// The number is kept because the archive addresses rounds by it: `--round <issue#>=<n>`
/// has to be matched against the issues actually selected in order to report one that
/// names none of them.
pub async fn get_milestone_issue_threads(
    milestones: &[&Milestone],
    git_info: &(impl GitHubReader + GitCommitOps + GitRepository),
    cache: Option<&DiskCache>,
) -> Result<Vec<MilestoneIssueThread>> {
    let futures = milestones
        .iter()
        .map(|&m| async move {
            let issues = git_info.get_issues(Some(m.number as u64)).await?;
            Ok::<_, anyhow::Error>(issues)
        })
        .collect::<Vec<_>>();
    let milestone_results = future::try_join_all(futures).await?;

    let mut seen_files: HashMap<String, Vec<String>> = HashMap::new();
    for issue in milestone_results.iter().flatten() {
        let entry = seen_files.entry(issue.title.to_string()).or_default();
        if let Some(milestone) = &issue.milestone {
            entry.push(milestone.title.to_string());
        }
    }
    let has_conflict = seen_files
        .iter()
        .filter(|(_, milestones)| milestones.len() > 1)
        .collect::<HashMap<_, _>>();

    if !has_conflict.is_empty() {
        bail!(
            "Files are listed multiple times in selected milestones:\n\t- {}",
            has_conflict
                .iter()
                .map(|(file, milestones)| format!("{file}: {}", milestones.join(", ")))
                .collect::<Vec<_>>()
                .join("\n\t- ")
        )
    }

    // Fetch all comments in parallel first
    let comment_futures = milestone_results
        .iter()
        .flatten()
        .map(|issue| async move { (issue, get_issue_comments(issue, cache, git_info).await) })
        .collect::<Vec<_>>();
    let comment_results = future::join_all(comment_futures).await;

    // Build IssueThreads
    let mut issue_thread_results = Vec::new();
    for (issue, comments_result) in comment_results {
        let comments = comments_result?;
        let issue_thread = IssueThread::from_issue_comments(issue, &comments, git_info, cache)?;
        issue_thread_results.push(MilestoneIssueThread {
            number: issue.number,
            thread: issue_thread,
        });
    }

    Ok(issue_thread_results)
}

/// Interactive file selection for archive with conflict detection and commit selection
fn prompt_archive_files(
    current_dir: &PathBuf,
    selected_files: &[&Path],
    commits: &[GitCommit],
    git_info: &impl GitCommitOps,
) -> Result<Vec<(PathBuf, ObjectId)>> {
    #[derive(Clone)]
    struct ArchiveFileCompleter {
        current_dir: PathBuf,
        excluded_files: HashSet<String>,
    }

    impl Autocomplete for ArchiveFileCompleter {
        fn get_suggestions(
            &mut self,
            input: &str,
        ) -> std::result::Result<Vec<String>, CustomUserError> {
            let input = input.trim();
            let mut suggestions = Vec::new();

            let (base_path, search_term) = if input.contains('/') {
                let mut parts = input.rsplitn(2, '/');
                let filename = parts.next().unwrap_or("");
                let dir_path = parts.next().unwrap_or("");
                (self.current_dir.join(dir_path), filename)
            } else {
                (self.current_dir.clone(), input)
            };

            if let Ok(entries) = fs::read_dir(&base_path) {
                let mut files = Vec::new();
                let mut dirs = Vec::new();

                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        // Skip hidden files/directories
                        if name.starts_with('.') {
                            continue;
                        }

                        if name.to_lowercase().starts_with(&search_term.to_lowercase()) {
                            let relative_path = if input.contains('/') {
                                let dir_part = input.rsplitn(2, '/').nth(1).unwrap_or("");
                                format!("{}/{}", dir_part, name)
                            } else {
                                name.clone()
                            };

                            if entry.path().is_file() {
                                // Check if this file is excluded
                                if self.excluded_files.contains(&relative_path) {
                                    // Mark as unavailable with styling
                                    files
                                        .push(format!("🚫 {} (already in archive)", relative_path));
                                } else {
                                    files.push(relative_path);
                                }
                            } else if entry.path().is_dir() {
                                // Add trailing slash to indicate directory
                                dirs.push(format!("{}/", relative_path));
                            }
                        }
                    }
                }

                // Sort directories and files separately, then combine
                dirs.sort();
                files.sort();
                suggestions.extend(dirs);
                suggestions.extend(files);
            }

            Ok(suggestions)
        }

        fn get_completion(
            &mut self,
            _input: &str,
            highlighted_suggestion: Option<String>,
        ) -> std::result::Result<inquire::autocompletion::Replacement, CustomUserError> {
            Ok(match highlighted_suggestion {
                Some(suggestion) => {
                    // If the suggestion is marked as unavailable, don't allow completion
                    if suggestion.starts_with("🚫 ") {
                        inquire::autocompletion::Replacement::None
                    } else {
                        inquire::autocompletion::Replacement::Some(suggestion)
                    }
                }
                None => inquire::autocompletion::Replacement::None,
            })
        }
    }

    // Build set of excluded files from existing issue threads
    let mut excluded_files: HashSet<String> = selected_files
        .iter()
        .map(|file| file.to_string_lossy().to_string())
        .collect();

    let mut selected_files: Vec<(PathBuf, ObjectId)> = Vec::new();

    loop {
        let prompt_text = if selected_files.is_empty() {
            "📁 Enter file path for archive (Tab for autocomplete, Enter for none):".to_string()
        } else {
            format!(
                "📁 Enter another file for archive (current: {}, Tab for autocomplete, Enter to finish):",
                selected_files
                    .iter()
                    .map(|(path, _)| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let file_completer = ArchiveFileCompleter {
            current_dir: current_dir.clone(),
            excluded_files: excluded_files.clone(),
        };

        let validator_dir = current_dir.clone();
        let validator_excluded = excluded_files.clone();
        let input = Text::new(&prompt_text)
            .with_autocomplete(file_completer)
            .with_validator(move |input: &str| {
                let trimmed = input.trim();
                // Handle case where user somehow enters the grayed-out format
                if trimmed.starts_with("🚫 ") {
                    return Ok(Validation::Invalid(
                        "This file is already in the archive. Please select a different file."
                            .into(),
                    ));
                }
                if trimmed.is_empty() {
                    Ok(Validation::Valid) // Empty is valid - means finish
                } else if trimmed.ends_with('/') {
                    Ok(Validation::Invalid(
                        "Cannot select a directory. Please select a file.".into(),
                    ))
                } else {
                    let path = validator_dir.join(trimmed);
                    if path.exists() && path.is_dir() {
                        Ok(Validation::Invalid(
                            "Path must be a file, not a directory".into(),
                        ))
                    } else if validator_excluded.contains(&trimmed.to_string()) {
                        Ok(Validation::Invalid(
                            "This file is already in the archive. Please select a different file."
                                .into(),
                        ))
                    } else {
                        Ok(Validation::Valid)
                    }
                }
            })
            .prompt()
            .map_err(|e| anyhow::anyhow!("Input cancelled: {}", e))?;

        let trimmed_input = input.trim();
        if trimmed_input.is_empty() {
            break; // User pressed Enter without input, finish
        }

        let file_path = PathBuf::from(trimmed_input);

        // Filter commits that actually change this file
        let touching = git_info
            .file_touching_commits(None, &file_path)
            .map_err(|e| anyhow::anyhow!("Failed to find commits for file: {}", e))?;
        let file_changing_commits: Vec<_> = commits
            .iter()
            .filter(|commit| touching.contains(&commit.commit.to_string()))
            .collect();

        if file_changing_commits.is_empty() {
            println!(
                "⚠️ No commits found that change file: {}",
                file_path.display()
            );
            continue;
        }

        // Present commit options
        let commit_options: Vec<String> = file_changing_commits
            .iter()
            .map(|commit| {
                let short_hash = commit.commit.to_string()[..8].to_string();
                let short_message = if commit.message.is_empty() {
                    "No message".to_string()
                } else {
                    // Take first line and truncate if too long
                    let first_line = commit.message.lines().next().unwrap_or("");
                    if first_line.len() > 50 {
                        format!("{}...", &first_line[..47])
                    } else {
                        first_line.to_string()
                    }
                };
                format!("{} - {}", short_hash, short_message)
            })
            .collect();

        let commit_selection = Select::new(
            &format!("📝 Select commit for file {}:", file_path.display()),
            commit_options,
        )
        .prompt()
        .map_err(|e| anyhow::anyhow!("Selection cancelled: {}", e))?;

        // Extract the commit hash from the selection
        let selected_hash_str = commit_selection.split(" - ").next().unwrap_or("");
        let selected_commit = file_changing_commits
            .iter()
            .find(|commit| commit.commit.to_string().starts_with(selected_hash_str))
            .ok_or_else(|| anyhow::anyhow!("Selected commit not found"))?;

        // Add to selected files and excluded set
        selected_files.push((file_path.clone(), selected_commit.commit));
        excluded_files.insert(trimmed_input.to_string());
    }

    Ok(selected_files)
}

/// Generate archive name based on milestones and repository name
pub fn generate_archive_name(milestones: &[&Milestone], git_info: &impl GitRepository) -> String {
    // Get repository name from git_info
    let repo_name = git_info.repo();

    let archive_name = if milestones.is_empty() {
        // No milestones: archive/<repo name>.tar.gz
        format!("{}.tar.gz", repo_name)
    } else {
        // With milestones: archive/<repo name>-<milestone1-milestone2>.tar.gz
        let milestone_names: Vec<String> = milestones
            .iter()
            .map(|m| {
                // Sanitize milestone names for filename usage
                m.title
                    .chars()
                    .map(|c| match c {
                        // Replace problematic characters with dashes
                        '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '-',
                        c => c,
                    })
                    .collect::<String>()
                    // Remove consecutive dashes
                    .split('-')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("-")
            })
            .collect();

        format!("{}-{}.tar.gz", repo_name, milestone_names.join("-"))
    };

    archive_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::IssueCommit;
    use crate::round::{ChecklistSource, Gap, GapContinuity, RoundEvent, RoundOpen, RoundState};
    use crate::{Approval, RoundProvenance, round::Segment};
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

    fn at() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_str("2026-07-04T15:12:09Z").unwrap()
    }

    fn round(index: u32, state: RoundState, placement: Placement) -> Segment {
        Segment::Round(Round {
            index,
            opened_at: oid(A),
            branch: "main".to_string(),
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
            at: at(),
            comment_index: 0,
            comment_id: None,
            comment_url: None,
        }
    }

    fn gap(commits: Vec<IssueCommit>, placement: Placement) -> Segment {
        Segment::Gap(Gap {
            branch: "main".to_string(),
            commits,
            continuity: GapContinuity::Linear,
            placement,
        })
    }

    fn thread(segments: Vec<Segment>) -> IssueThread {
        IssueThread {
            file: PathBuf::from("scripts/analysis.R"),
            open: true,
            milestone: "Milestone 3".to_string(),
            blocking_qcs: Vec::new(),
            segments,
            anomalies: Vec::new(),
        }
    }

    /// Accumulate a summary at the default target, the way `ghqc milestone status` does:
    /// `categorize()` per thread, then `push()`. Deliberately not a helper on
    /// `ArchiveSummary` — an exported convenience with no production caller reads as API,
    /// and the point of the test is that the *production* accumulation is what is asserted.
    fn summarize<'a>(threads: impl IntoIterator<Item = &'a IssueThread>) -> ArchiveSummary {
        let mut summary = ArchiveSummary::default();
        for thread in threads {
            summary.push(categorize(thread, ArchiveTarget::Latest).unwrap());
        }
        summary
    }

    fn selection(number: u64, thread: IssueThread, target: ArchiveTarget) -> ArchiveSelection {
        ArchiveSelection {
            number,
            thread,
            target,
        }
    }

    /// S4: the filter admits threads *no round of which has ever closed*, and nothing
    /// else. The approved-then-reopened file is the case the filter must not govern —
    /// it has an approval standing in Initial QC, so it is kept even though its latest
    /// round is open, and which round it is archived at is a selection.
    #[test]
    fn the_never_approved_filter_is_about_closed_rounds_only() {
        let never_approved = thread(vec![round(1, RoundState::Open, Placement::Placed)]);
        assert!(!has_closed_round(&never_approved));

        let approved = thread(vec![
            round(1, closed(), Placement::Placed),
            gap(Vec::new(), Placement::Placed),
        ]);
        assert!(has_closed_round(&approved));

        let approved_then_reopened = thread(vec![
            round(1, closed(), Placement::Placed),
            gap(vec![commit(C)], Placement::Placed),
            round(2, RoundState::Open, Placement::Placed),
        ]);
        assert!(has_closed_round(&approved_then_reopened));
    }

    /// C2: an issue named by `--round` gets that round; every other issue gets the
    /// latest round, which is the default the archive shows first.
    #[test]
    fn unlisted_issues_target_the_latest_round() {
        let threads = vec![
            MilestoneIssueThread {
                number: 42,
                thread: thread(vec![round(1, closed(), Placement::Placed)]),
            },
            MilestoneIssueThread {
                number: 43,
                thread: thread(vec![round(1, closed(), Placement::Placed)]),
            },
        ];
        let targets = HashMap::from([(43, ArchiveTarget::Round(1))]);

        let selections = resolve_selections(threads, &targets).unwrap();

        assert_eq!(selections[0].number, 42);
        assert_eq!(selections[0].target, ArchiveTarget::Latest);
        assert_eq!(selections[1].number, 43);
        assert_eq!(selections[1].target, ArchiveTarget::Round(1));
    }

    /// C2: a `--round` naming an issue outside the selected milestones is a user error.
    /// Ignoring it would archive the latest round of everything while the user believed
    /// one file had been retargeted.
    #[test]
    fn a_round_for_an_unselected_issue_is_reported() {
        let threads = vec![MilestoneIssueThread {
            number: 42,
            thread: thread(vec![round(1, closed(), Placement::Placed)]),
        }];
        let targets = HashMap::from([(99, ArchiveTarget::Round(1))]);

        let err = resolve_selections(threads, &targets)
            .unwrap_err()
            .to_string();

        assert!(err.contains("#99"), "error should name the issue: {err}");
    }

    /// C2: an out-of-range round is reported by construction, and the CLI message names
    /// the issue the user typed — `ArchiveError` carries only the path.
    #[test]
    fn an_out_of_range_round_names_the_issue_and_the_round_count() {
        let selections = vec![selection(
            42,
            thread(vec![round(1, closed(), Placement::Placed)]),
            ArchiveTarget::Round(5),
        )];

        let err = build_archive_files(&selections, false)
            .unwrap_err()
            .to_string();

        assert!(err.contains("#42"), "should name the issue: {err}");
        assert!(err.contains("--round 42=5"), "should quote the flag: {err}");
        assert!(err.contains("1 round(s)"), "should name the count: {err}");
        assert!(err.contains("1..=1"), "should name the valid range: {err}");
    }

    /// I4/§11.1: the round a selection addresses is where the bytes come from, so an
    /// unplaceable *round* is blocked while an unplaceable trailing gap is not — the
    /// approval's closing commit is still perfectly archivable.
    #[test]
    fn only_an_unplaceable_selected_round_blocks() {
        let unplaceable_round = selection(
            42,
            thread(vec![round(
                1,
                RoundState::Open,
                Placement::Unplaceable(UnplaceableReason::BranchUnavailable),
            )]),
            ArchiveTarget::Latest,
        );
        let unplaceable_trailing_gap = selection(
            43,
            thread(vec![
                round(1, closed(), Placement::Placed),
                gap(
                    Vec::new(),
                    Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable),
                ),
            ]),
            ArchiveTarget::Latest,
        );

        let (placeable, blocked) =
            partition_placeable(vec![unplaceable_round, unplaceable_trailing_gap]);

        assert_eq!(placeable.len(), 1);
        assert_eq!(placeable[0].number, 43);
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].number, 42);
        assert_eq!(blocked[0].round, "Initial QC");
        assert_eq!(blocked[0].reason, UnplaceableReason::BranchUnavailable);

        // The callout names every blocked file with its reason, rather than counting them.
        let callout = unplaceable_callout(&blocked);
        assert!(callout.contains("scripts/analysis.R"), "{callout}");
        assert!(callout.contains("#42"), "{callout}");
        assert!(
            callout.contains(UnplaceableReason::BranchUnavailable.describe()),
            "{callout}"
        );
    }

    /// §20.5: an explicit retarget to an older, good round survives a **later** round the
    /// client never asked about being unplaceable.
    ///
    /// This is the case that decided the gate. Round 1 is closed and fully placed; round 2
    /// is open and could not be located. Targeting round 1 is well-formed and resolves to a
    /// known commit, so it is archived — and archived at that approval, flagged
    /// `superseded` because currency past the selection cannot be established. An
    /// active-segment gate rejected it on account of round 2, defeating the retarget for
    /// exactly the situation retargeting exists to serve: getting away from a broken round
    /// to an older good approval.
    #[test]
    fn an_older_round_is_archivable_past_an_unplaceable_later_round() {
        let thread = thread(vec![
            round(1, closed(), Placement::Placed),
            gap(
                Vec::new(),
                Placement::Unplaceable(UnplaceableReason::NeighbourUnplaceable),
            ),
            round(
                2,
                RoundState::Open,
                Placement::Unplaceable(UnplaceableReason::AnchorUnreachable),
            ),
        ]);

        // The default target is refused, and says which round and why.
        let (placeable, blocked) =
            partition_placeable(vec![selection(42, thread.clone(), ArchiveTarget::Latest)]);
        assert!(placeable.is_empty());
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].round, "Round 2");
        assert_eq!(blocked[0].reason, UnplaceableReason::AnchorUnreachable);

        // The explicit retarget to round 1 is not.
        let (placeable, blocked) =
            partition_placeable(vec![selection(42, thread.clone(), ArchiveTarget::Round(1))]);
        assert!(
            blocked.is_empty(),
            "round 2 is not the selected round: {blocked:?}"
        );
        assert_eq!(placeable.len(), 1);

        // And it archives Initial QC's approval, labelled as not provably current.
        let files = build_archive_files(&placeable, false).unwrap();
        let qc = files[0].qc.as_ref().expect("a milestone file");
        assert_eq!(files[0].commit, oid(B));
        assert_eq!(qc.round.round, 1);
        assert_eq!(
            qc.round.approval.as_ref().map(|approval| approval.round),
            Some(1)
        );
        assert_eq!(
            ArchiveCategory::of_qc(qc),
            ArchiveCategory::ApprovedSuperseded
        );
    }

    /// A selection naming no round is left to construction, which reports the round
    /// count — not to the unplaceable callout, whose reasons do not apply to it.
    #[test]
    fn a_selection_naming_no_round_is_not_called_unplaceable() {
        let (placeable, blocked) = partition_placeable(vec![selection(
            42,
            thread(vec![round(1, closed(), Placement::Placed)]),
            ArchiveTarget::Round(9),
        )]);

        assert_eq!(placeable.len(), 1);
        assert!(blocked.is_empty());
    }

    /// C2: a `--round` on an invocation that selects no milestone can have no effect at
    /// all, so it is reported. Both no-milestone arms — interactive, and
    /// additional-files-only — go through this, because neither ever consults the map.
    #[test]
    fn a_round_with_no_milestone_selected_is_reported() {
        assert!(reject_unreachable_round_targets(&HashMap::new()).is_ok());

        let err = reject_unreachable_round_targets(&HashMap::from([(42, ArchiveTarget::Round(1))]))
            .unwrap_err()
            .to_string();

        assert!(err.contains("--round"), "should name the flag: {err}");
        assert!(
            err.contains("no milestone is selected"),
            "should say why it cannot be honoured: {err}"
        );
    }

    /// S2/U1: every file says which round it came from, whether those bytes were
    /// approved, at which commit, and whether they were still the newest QC state.
    #[test]
    fn the_provenance_line_carries_both_facts() {
        let approved = ArchiveQC {
            milestone: "Milestone 3".to_string(),
            round: RoundProvenance {
                round: 2,
                approval: Some(Approval {
                    round: 2,
                    commit: oid(B),
                    by: "wes".to_string(),
                    at: at(),
                }),
                superseded: false,
            },
        };
        let line = provenance_line(&approved, &oid(B));
        assert_eq!(
            line,
            format!("Round 2 · approved by @wes on 2026-07-04 · {}", &B[..7])
        );
        assert!(!line.contains("newest QC state"));

        let unapproved = ArchiveQC {
            milestone: "Milestone 3".to_string(),
            round: RoundProvenance {
                round: 3,
                approval: None,
                superseded: true,
            },
        };
        assert_eq!(
            provenance_line(&unapproved, &oid(C)),
            format!(
                "Round 3 · unapproved · {} · ⚠️ not the newest QC state",
                &C[..7]
            )
        );

        // Round 1 is `Initial QC`, never `Round 1` — a spec-pinned string, asserted on the
        // rendered line because that is the claim a user reads. It is the one thing the
        // deleted projection-equality test was protecting, kept at the user-visible layer
        // now that the naming rule has a single home on `Round`.
        let initial = ArchiveQC {
            milestone: "Milestone 3".to_string(),
            round: RoundProvenance {
                round: 1,
                approval: Some(Approval {
                    round: 1,
                    commit: oid(A),
                    by: "alice".to_string(),
                    at: at(),
                }),
                superseded: false,
            },
        };
        assert_eq!(
            provenance_line(&initial, &oid(A)),
            format!(
                "Initial QC · approved by @alice on 2026-07-04 · {}",
                &A[..7]
            )
        );
    }

    /// I2: "you were on round 2; the commit you took is Initial QC's approval." The two
    /// frames stay apart — nothing claims the selected round was approved.
    #[test]
    fn the_provenance_line_keeps_the_two_round_frames_apart() {
        let qc = ArchiveQC {
            milestone: "Milestone 3".to_string(),
            round: RoundProvenance {
                round: 2,
                approval: Some(Approval {
                    round: 1,
                    commit: oid(A),
                    by: "wes".to_string(),
                    at: at(),
                }),
                superseded: true,
            },
        };

        let line = provenance_line(&qc, &oid(A));

        assert!(line.starts_with("Round 2 · "), "{line}");
        assert!(line.contains("bytes are Initial QC's approval"), "{line}");
        assert!(!line.contains("Round 2 · approved"), "{line}");
    }

    /// C4/U5: the buckets a mixed milestone falls into, at the default target.
    #[test]
    fn the_summary_counts_every_bucket() {
        let approved_and_current = thread(vec![
            round(1, closed(), Placement::Placed),
            gap(Vec::new(), Placement::Placed),
        ]);
        let approved_but_superseded = thread(vec![
            round(1, closed(), Placement::Placed),
            gap(vec![commit(C)], Placement::Placed),
        ]);
        let never_approved = thread(vec![round(1, RoundState::Open, Placement::Placed)]);
        let not_placeable = thread(vec![round(
            1,
            RoundState::Open,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable),
        )]);

        let summary = summarize([
            &approved_and_current,
            &approved_but_superseded,
            &never_approved,
            &not_placeable,
        ]);

        assert_eq!(summary.files, 4);
        assert_eq!(summary.approved_current, 1);
        assert_eq!(summary.approved_superseded, 1);
        assert_eq!(summary.unapproved, 1);
        assert_eq!(summary.not_placeable, 1);
        assert_eq!(
            summary.to_string(),
            "4 files · 1 approved & current · 1 approved but superseded · \
             1 unapproved (Initial QC open) · 1 not placeable"
        );
    }

    /// S2/D9: an approved-then-reopened file counts as **unapproved**, because unapproved
    /// bytes are what the default target would archive. Counting it as approved on the
    /// strength of Initial QC's approval would make the readiness check disagree with the
    /// archive it precedes — about exactly the case rounds exist for.
    #[test]
    fn a_reopened_file_counts_as_unapproved() {
        let reopened = thread(vec![
            round(1, closed(), Placement::Placed),
            gap(vec![commit(C)], Placement::Placed),
            round(2, RoundState::Open, Placement::Placed),
        ]);

        assert_eq!(
            categorize(&reopened, ArchiveTarget::Latest).unwrap(),
            ArchiveCategory::Unapproved { round: 2 }
        );

        // And retargeting to the round that holds the approval moves it, which is the
        // whole point of the selection being a selection rather than a filter.
        assert_eq!(
            categorize(&reopened, ArchiveTarget::Round(1)).unwrap(),
            ArchiveCategory::ApprovedSuperseded
        );

        let summary = summarize([&reopened]);
        assert_eq!(summary.unapproved, 1);
        assert_eq!(summary.approved_current, 0);
        assert_eq!(summary.approved_superseded, 0);
        assert_eq!(summary.to_string(), "1 file · 1 unapproved (round 2 open)");
    }

    /// C4's actual requirement: the readiness check and the archive categorize the same
    /// threads identically. Asserted per thread against the provenance the archive writes,
    /// and again on the whole summary, so the two cannot drift apart.
    #[test]
    fn the_check_and_the_archive_agree() {
        let threads = vec![
            thread(vec![
                round(1, closed(), Placement::Placed),
                gap(Vec::new(), Placement::Placed),
            ]),
            thread(vec![
                round(1, closed(), Placement::Placed),
                gap(vec![commit(C)], Placement::Placed),
            ]),
            thread(vec![
                round(1, closed(), Placement::Placed),
                gap(vec![commit(C)], Placement::Placed),
                round(2, RoundState::Open, Placement::Placed),
            ]),
            thread(vec![round(1, RoundState::Open, Placement::Placed)]),
        ];

        let selections: Vec<ArchiveSelection> = threads
            .iter()
            .enumerate()
            .map(|(position, thread)| {
                selection(position as u64, thread.clone(), ArchiveTarget::Latest)
            })
            .collect();
        let files = build_archive_files(&selections, false).unwrap();

        // Per file: what the check said, and what the archive actually recorded.
        for (thread, file) in threads.iter().zip(&files) {
            let checked = categorize(thread, ArchiveTarget::Latest).unwrap();
            let archived = ArchiveCategory::of_qc(file.qc.as_ref().expect("a milestone file"));
            assert_eq!(checked, archived, "{}", thread.file.display());
        }

        // And in aggregate, which is the line the two commands print.
        let checked = summarize(&threads);
        assert_eq!(checked, ArchiveSummary::of_files(&files));
        assert_eq!(
            checked.to_string(),
            "4 files · 1 approved & current · 1 approved but superseded · 2 unapproved"
        );
    }

    /// A file added by path and commit carries no QC claim, so it is counted apart from
    /// every QC bucket instead of being folded into one of them.
    #[test]
    fn an_added_file_is_counted_apart_from_the_qc_buckets() {
        let files = vec![ArchiveFile::from_file("scripts/helpers.R", oid(C), false)];

        let summary = ArchiveSummary::of_files(&files);

        assert_eq!(summary.files, 1);
        assert_eq!(summary.added, 1);
        assert_eq!(summary.approved_current, 0);
        assert_eq!(summary.unapproved, 0);
        assert_eq!(summary.to_string(), "1 file · 1 added");
    }

    /// "(round 3 open)" is a fact about one round: with unapproved files open at different
    /// rounds the detail is dropped rather than made up.
    #[test]
    fn a_mixed_set_of_open_rounds_prints_no_round_detail() {
        let mut summary = ArchiveSummary::default();
        summary.push(ArchiveCategory::Unapproved { round: 2 });
        assert_eq!(summary.to_string(), "1 file · 1 unapproved (round 2 open)");

        summary.push(ArchiveCategory::Unapproved { round: 3 });
        assert_eq!(summary.to_string(), "2 files · 2 unapproved");
    }

    /// C3: a round option says enough to choose between rounds — which round, open or
    /// closed, the commit that would be archived, and who approved it and when.
    #[test]
    fn a_round_option_describes_the_commit_it_would_archive() {
        let closed_round = round(2, closed(), Placement::Placed);
        assert_eq!(
            describe_round(closed_round.as_round().unwrap()),
            format!(
                "Round 2 · closed · approved by @reviewer on 2026-07-04 · {}",
                &B[..7]
            )
        );

        // An open round archives its newest *actioned* commit, so that is the commit
        // offered: here a notification on the older commit, not the newer drift.
        let mut open_round = round(3, RoundState::Open, Placement::Placed);
        if let Segment::Round(open) = &mut open_round {
            open.events.push(RoundEvent::Notification {
                commit: oid(A),
                by: "wes".to_string(),
                at: at(),
                comment_index: 1,
                comment_id: None,
                comment_url: None,
            });
        }
        assert_eq!(
            describe_round(open_round.as_round().unwrap()),
            format!("Round 3 · open · latest actioned commit {}", &A[..7])
        );

        let unplaceable = round(
            4,
            RoundState::Open,
            Placement::Unplaceable(UnplaceableReason::BranchUnavailable),
        );
        assert_eq!(
            describe_round(unplaceable.as_round().unwrap()),
            format!(
                "Round 4 · not placeable — {}",
                UnplaceableReason::BranchUnavailable.describe()
            )
        );
    }
}
