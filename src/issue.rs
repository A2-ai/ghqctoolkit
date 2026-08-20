use std::{fmt, path::PathBuf, str::FromStr, sync::LazyLock};

use gix::ObjectId;
use octocrab::models::{IssueState, issues::Issue};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
    cache::{DiskCache, get_issue_comments},
    git::{
        GitComment, GitCommitOps, GitFileOpsError, GitHubApiError, GitHubReader,
        find_or_cache_file_changes, get_commits_robust,
    },
    round::{
        BranchWalk, BranchWalks, Round, RoundAnomaly, Segment, UnplaceableReason,
        fold_rounds_from_comments, resolve_segments, round_branches,
    },
};

static MARKDOWN_LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());

/// Regex to extract file name and issue number from markdown links to issues
/// Pattern: [file_name](url/issues/123) - captures link text and issue number
/// Works with any host (github.com, GHE, etc.)
static BLOCKING_QC_LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\/issues\/(\d+)[^)]*\)").unwrap());

pub(crate) static HTML_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a\s+[^>]*href\s*=\s*["']([^"']+)["'][^>]*>([^<]*)</a>"#).unwrap()
});

/// Relationship type for blocking QC issues
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum BlockingRelationship {
    /// A QC that was done previously on this file or a closely related one
    PreviousQC,
    /// A QC which the issue of interest is developed based on
    GatingQC,
    /// Relationship could not be determined (issue not found in child's body)
    Unknown,
}

impl fmt::Display for BlockingRelationship {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let self_str = match self {
            Self::PreviousQC => "previous QC",
            Self::GatingQC => "gating QC",
            Self::Unknown => "unknown relationship",
        };
        write!(f, "{self_str}")
    }
}

/// A blocking QC issue parsed from the issue body
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockingQC {
    /// The issue number of the blocking QC
    pub issue_number: u64,
    /// The file name associated with the blocking QC (link text)
    pub file_name: PathBuf,
    /// The relationship type (GatingQC or PreviousQC)
    pub relationship: BlockingRelationship,
}

/// A commit owned by one [`Segment`].
///
/// Deliberately carries no statuses: what the comments said about a commit is the
/// owning segment's events and state, parsed once, and a status set derived from two
/// independent parses of the same thread is exactly how those two views came to
/// disagree.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueCommit {
    pub hash: ObjectId,
    pub message: String,
    pub file_changed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IssueThread {
    pub file: PathBuf,
    /// GitHub's issue state.
    pub open: bool,
    pub milestone: String,
    /// Blocking QC issues parsed from issue body
    /// Includes both Gating QC and Previous QC sections
    pub blocking_qcs: Vec<BlockingQC>,
    /// The issue's history as a strict alternation of rounds and the drift between
    /// them, oldest first. `segments[0]` is always Initial QC, and the last segment is
    /// always an open round or a gap.
    ///
    /// There is deliberately no `branch` and no thread-wide `commits`: every round
    /// carries the branch it was reviewed on, so "the issue's branch" is not a concept,
    /// and every commit is owned by the segment that covers it.
    pub segments: Vec<Segment>,
    /// Problems found while folding segments. Folding never fails.
    pub anomalies: Vec<RoundAnomaly>,
}

impl IssueThread {
    /// Create IssueThread from issue and pre-fetched comments
    pub fn from_issue_comments(
        issue: &Issue,
        comments: &[GitComment],
        git_info: &impl GitCommitOps,
        disk_cache: Option<&DiskCache>,
    ) -> Result<Self, IssueError> {
        let file = PathBuf::from(&issue.title);
        let issue_is_open = matches!(issue.state, IssueState::Open);
        let milestone = if let Some(m) = &issue.milestone {
            m.title.to_string()
        } else {
            return Err(IssueError::MilestoneNotFound);
        };

        // 1. The issue body's branch is Initial QC's branch — nothing more. Later
        //    rounds each declare their own.
        let body_branch = issue
            .body
            .as_ref()
            .and_then(|body| parse_branch_from_body(body))
            .ok_or(IssueError::BranchNotFound)?;

        // 2. Initial QC's anchor.
        let initial_commit_str = issue
            .body
            .as_ref()
            .and_then(|body| parse_commit_from_pattern(body, "initial qc commit: "))
            .ok_or(IssueError::InitialCommitNotFound)?;

        // 3. Fold stage 1 — pure over comment text — *before* any walk. The walk needs
        //    each round's branch, and stage 2 needs the walk to resolve that round's
        //    anchor, so this is the only order that works.
        let initial_checklist_name = issue
            .body
            .as_deref()
            .and_then(crate::new_round::checklist_from_issue_body)
            .and_then(|seeded| seeded.name);
        let (raw_rounds, raw_anomalies) = fold_rounds_from_comments(
            initial_commit_str,
            initial_checklist_name.as_deref(),
            comments,
        );
        let branches = round_branches(&raw_rounds, &body_branch);

        // 4. Any previously-known file names from `## File History`: commits made
        //    against the old name still count as changing this issue's file.
        let old_paths: Vec<PathBuf> = issue
            .body
            .as_deref()
            .map(|body| {
                parse_file_history(body)
                    .into_iter()
                    .map(|e| PathBuf::from(e.old_path))
                    .collect()
            })
            .unwrap_or_default();

        // 5. One walk per *distinct* branch, so a single-branch issue does exactly one
        //    walk — which is why existing issues cannot regress. Each walk is fed that
        //    segment's own anchor as its reference commit, a better input for
        //    `get_commits_robust` than one arbitrary thread-wide commit.
        let stop_at = ObjectId::from_str(initial_commit_str).ok();
        let mut walks: BranchWalks = std::collections::HashMap::new();
        for (position, branch) in branches.iter().enumerate() {
            let Some(branch) = branch else {
                continue;
            };
            if walks.contains_key(branch) {
                continue;
            }
            let anchor = ObjectId::from_str(raw_rounds[position].opened_at).ok();
            let walk = walk_branch(
                git_info,
                branch,
                anchor.as_ref(),
                stop_at,
                &file,
                &old_paths,
                disk_cache,
            );
            if let Err(reason) = &walk {
                log::debug!("branch '{branch}' could not be walked: {reason:?}");
            }
            walks.insert(branch.clone(), walk);
        }

        // 6. Fold stage 2: place every commit in exactly one segment.
        let merge_base = |a: &ObjectId, b: &ObjectId| git_info.merge_base(a, b).ok().flatten();
        let (segments, anomalies) =
            resolve_segments(raw_rounds, &branches, &walks, &merge_base, raw_anomalies);
        log::debug!(
            "derived {} segment(s) with {} anomal(ies) for {}",
            segments.len(),
            anomalies.len(),
            file.display()
        );

        let blocking_qcs = issue
            .body
            .as_ref()
            .map(|body| parse_blocking_qcs(body))
            .unwrap_or_default();

        Ok(IssueThread {
            file,
            open: issue_is_open,
            milestone,
            blocking_qcs,
            segments,
            anomalies,
        })
    }

    // TODO: order the notification commits based on commit timeline
    pub async fn from_issue(
        issue: &Issue,
        disk_cache: Option<&DiskCache>,
        git_info: &(impl GitHubReader + GitCommitOps),
    ) -> Result<Self, IssueError> {
        let comments = get_issue_comments(issue, disk_cache, git_info).await?;
        Self::from_issue_comments(issue, &comments, git_info, disk_cache)
    }

    // ── Segment accessors ───────────────────────────────────────────────────
    //
    // Everything a consumer needs about round scope is answered here, positionally.
    // Nothing re-derives which commits a round covers.

    /// The segment the issue is currently in: always the last one, which by
    /// construction is an open round or a gap.
    pub fn active_segment(&self) -> &Segment {
        self.segments
            .last()
            .expect("an IssueThread always has at least Initial QC")
    }

    /// The branch the issue is currently being QC'd on — the active segment's.
    ///
    /// This is the only "the issue's branch" there is. It is deliberately not compared
    /// against the viewer's checkout here: status must not depend on who is looking.
    pub fn active_branch(&self) -> &str {
        self.active_segment().branch()
    }

    /// Rounds only, oldest first.
    pub fn rounds(&self) -> impl DoubleEndedIterator<Item = &Round> {
        self.segments.iter().filter_map(Segment::as_round)
    }

    /// The approval the **round** at `position` builds on: the closing commit of the
    /// round two positions back, since exactly one gap always sits between two rounds.
    ///
    /// Positions alternate `0 = Round, 1 = Gap, 2 = Round, …`, so this offset is only
    /// meaningful for an even `position`. A gap's own bounding rounds are its immediate
    /// neighbours at `position - 1` and `position + 1`.
    pub fn previous_approval_of(&self, position: usize) -> Option<&ObjectId> {
        self.segments
            .get(position.checked_sub(2)?)?
            .as_round()?
            .closing_commit()
    }

    /// The approval that currently stands, if any.
    ///
    /// `None` while a round is open: a round under review supersedes the approval it
    /// was opened on top of, so there is nothing standing to report. With a gap active
    /// it is the closing commit of the round bounding that gap's older end — its
    /// immediate neighbour, one position back.
    pub fn standing_approval(&self) -> Option<&ObjectId> {
        match self.active_segment() {
            Segment::Gap(_) => self
                .segments
                .get(self.segments.len().checked_sub(2)?)?
                .as_round()?
                .closing_commit(),
            Segment::Round(_) => None,
        }
    }

    /// The newest closing commit across all rounds, ungated by whether a round is
    /// open. Distinct from [`Self::standing_approval`] on purpose: this one answers
    /// "what was last approved, ever".
    pub fn last_approved_commit(&self) -> Option<&ObjectId> {
        self.rounds().rev().find_map(Round::closing_commit)
    }

    /// The newest commit of the active segment. `None` when it owns none — an empty
    /// gap, or a segment that could not be placed.
    pub fn latest_commit(&self) -> Option<&IssueCommit> {
        self.active_segment().commits().first()
    }

    /// Initial QC's anchor: the `initial qc commit` from the issue body.
    ///
    /// `None` when Initial QC is unplaceable — an anchor we could not resolve carries a
    /// null-OID placeholder, which is not a commit anyone may address.
    pub fn initial_commit(&self) -> Option<&ObjectId> {
        self.segments
            .first()
            .and_then(Segment::as_round)
            .filter(|round| round.is_placed())
            .map(|round| &round.opened_at)
    }

    /// Every commit that touched this issue's file, across all segments, newest-first.
    pub fn file_commits(&self) -> Vec<&ObjectId> {
        self.segments
            .iter()
            .flat_map(|segment| segment.commits())
            .filter(|commit| commit.file_changed)
            .map(|commit| &commit.hash)
            .collect()
    }

    /// The commit a new notification should diff against.
    ///
    /// An open round diffs against what it has already notified, or — before it has
    /// notified anything — against its own anchor, which is the commit it opened at. A
    /// trailing gap has no round to notify, so the standing approval is the only
    /// sensible base.
    ///
    /// `None` when the active segment could not be placed: its anchor and closing commit
    /// may be null-OID placeholders, and a diff against those is not a diff.
    ///
    /// The round branch is the same computation as
    /// [`Round::latest_actioned_commit`] — the round's newest actioned commit — and
    /// calls it rather than repeating it, so the notification base and the archive's
    /// "latest update commit" (archive-rounds **M4**) cannot drift apart.
    pub fn next_notification_from(&self) -> Option<ObjectId> {
        if !self.active_segment().is_placed() {
            return None;
        }
        match self.active_segment() {
            Segment::Round(round) => Some(
                round
                    .latest_actioned_commit()
                    .map(|commit| commit.hash)
                    // A placed round always owns its anchor, so this only fires if the
                    // fold placed a round that owns nothing; the anchor stays the answer.
                    .unwrap_or(round.opened_at),
            ),
            Segment::Gap(_) => self.standing_approval().copied(),
        }
    }
}

/// Walk `branch` and flag which of its commits touch `file` (or any name it used to
/// have).
///
/// Any git failure degrades the whole branch to [`UnplaceableReason`] rather than
/// failing the issue: a round on a branch that was never fetched is grayed, not fatal.
fn walk_branch(
    git_info: &impl GitCommitOps,
    branch: &str,
    anchor: Option<&ObjectId>,
    stop_at: Option<ObjectId>,
    file: &std::path::Path,
    old_paths: &[PathBuf],
    disk_cache: Option<&DiskCache>,
) -> BranchWalk {
    let branch_opt = Some(branch.to_string());
    let commits = match get_commits_robust(git_info, &branch_opt, anchor, stop_at, disk_cache) {
        Ok(commits) => commits,
        Err(GitFileOpsError::LocalBranchNotFound(_)) => {
            return Err(UnplaceableReason::BranchUnavailable);
        }
        Err(error) => {
            log::warn!("failed to walk branch '{branch}': {error}");
            return Err(UnplaceableReason::BranchUnavailable);
        }
    };

    // One subprocess call per (branch, file) pair, cached.
    let hashes: Vec<String> = commits.iter().map(|c| c.commit.to_string()).collect();
    let mut touching =
        find_or_cache_file_changes(&hashes, git_info, branch_opt.clone(), file, disk_cache)
            .map_err(|error| {
                log::warn!("failed to find file changes on '{branch}': {error}");
                UnplaceableReason::BranchUnavailable
            })?;
    for old_path in old_paths {
        if let Ok(old_touching) =
            find_or_cache_file_changes(&hashes, git_info, branch_opt.clone(), old_path, disk_cache)
        {
            touching.extend(old_touching);
        }
    }

    Ok(commits
        .into_iter()
        .map(|commit| IssueCommit {
            file_changed: touching.contains(&commit.commit.to_string()),
            hash: commit.commit,
            message: commit.message,
        })
        .collect())
}

/// Parse a commit from a body using the given pattern
/// Supports both full and short SHAs with minimum 7 character length
pub(crate) fn parse_commit_from_pattern<'a>(body: &'a str, pattern: &str) -> Option<&'a str> {
    let start = body.find(pattern)?;
    let commit_start = start + pattern.len();

    let remaining = &body[commit_start..];
    remaining.lines().next()?.split_whitespace().next()
}

/// Parse branch name from issue body
/// Only looks for the "git branch: <branch-name>" pattern
/// Branch name can be plain text, markdown link text, or HTML link text
pub fn parse_branch_from_body(body: &str) -> Option<String> {
    let pattern = "git branch: ";
    let start = body.find(pattern)?;
    let branch_start = start + pattern.len();
    let remaining = &body[branch_start..];
    let line = remaining.lines().next()?;

    // Check if the branch name is a markdown link [name](url)
    if let Some(md_captures) = MARKDOWN_LINK_REGEX.captures(line) {
        if let Some(link_text) = md_captures.get(1) {
            let branch_name = link_text.as_str().trim();
            if !branch_name.is_empty() {
                return Some(branch_name.to_string());
            }
        }
    }

    // Check if the branch name is an HTML link <a href="url">text</a>
    if let Some(html_captures) = HTML_LINK_REGEX.captures(line) {
        if let Some(link_text) = html_captures.get(2) {
            let branch_name = link_text.as_str().trim();
            if !branch_name.is_empty() {
                return Some(branch_name.to_string());
            }
        }
    }

    // Fall back to plain text branch name
    let branch_name = line.trim();
    if !branch_name.is_empty() {
        Some(branch_name.to_string())
    } else {
        None
    }
}

/// Parse blocking QC issues from issue body
///
/// Looks for the "## Relevant Files" section and extracts:
/// - `### Gating QC` subsection → `BlockingRelationship::GatingQC`
/// - `### Previous QC` subsection → `BlockingRelationship::PreviousQC`
///
/// Extracts file name (link text) and issue number from markdown links.
pub fn parse_blocking_qcs(body: &str) -> Vec<BlockingQC> {
    let mut blocking_qcs = Vec::new();

    // Find the start of "## Relevant Files" section
    let relevant_files_start = match body.find("## Relevant Files") {
        Some(pos) => pos,
        None => return blocking_qcs,
    };

    let relevant_section = &body[relevant_files_start..];

    // Find the end of the relevant files section (next level 2 header or end of body)
    let section_end = relevant_section[17..] // Skip "## Relevant Files"
        .find("\n## ")
        .map(|pos| pos + 17)
        .unwrap_or(relevant_section.len());

    let relevant_section = &relevant_section[..section_end];

    // Parse Gating QC section
    if let Some(gating_start) = relevant_section.find("### Gating QC") {
        let gating_section = &relevant_section[gating_start..];
        let gating_end = gating_section[13..] // Skip "### Gating QC"
            .find("\n### ")
            .map(|pos| pos + 13)
            .unwrap_or(gating_section.len());
        let gating_section = &gating_section[..gating_end];

        for capture in BLOCKING_QC_LINK_REGEX.captures_iter(gating_section) {
            if let (Some(file_name), Some(issue_number)) = (capture.get(1), capture.get(2)) {
                if let Ok(issue_num) = issue_number.as_str().parse::<u64>() {
                    blocking_qcs.push(BlockingQC {
                        issue_number: issue_num,
                        file_name: PathBuf::from(file_name.as_str()),
                        relationship: BlockingRelationship::GatingQC,
                    });
                }
            }
        }
    }

    // Parse Previous QC section
    if let Some(previous_start) = relevant_section.find("### Previous QC") {
        let previous_section = &relevant_section[previous_start..];
        let previous_end = previous_section[15..] // Skip "### Previous QC"
            .find("\n### ")
            .map(|pos| pos + 15)
            .unwrap_or(previous_section.len());
        let previous_section = &previous_section[..previous_end];

        for capture in BLOCKING_QC_LINK_REGEX.captures_iter(previous_section) {
            if let (Some(file_name), Some(issue_number)) = (capture.get(1), capture.get(2)) {
                if let Ok(issue_num) = issue_number.as_str().parse::<u64>() {
                    blocking_qcs.push(BlockingQC {
                        issue_number: issue_num,
                        file_name: PathBuf::from(file_name.as_str()),
                        relationship: BlockingRelationship::PreviousQC,
                    });
                }
            }
        }
    }

    blocking_qcs
}

/// Determine the relationship type from a child's body by finding where the parent issue appears
///
/// The relationship type is stored in the child's body - the child lists its blockers
/// under "### Gating QC" or "### Previous QC" sections.
pub fn determine_relationship_from_body(
    body: &str,
    parent_issue_number: u64,
) -> BlockingRelationship {
    let blocking_qcs = parse_blocking_qcs(body);

    for qc in blocking_qcs {
        if qc.issue_number == parent_issue_number {
            return qc.relationship;
        }
    }

    // Parent not found in child's body - indicates data inconsistency
    BlockingRelationship::Unknown
}

/// A single file rename event stored in the "## File History" section of an issue body.
///
/// Format: `* \`old_path\` → \`new_path\` (commit: abc1234)`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileRenameEvent {
    pub old_path: String,
    pub new_path: String,
    pub commit: String,
}

/// Parse file rename events from the "## File History" section of an issue body.
///
/// Each line has the form: `* \`old_path\` → \`new_path\` (commit: abc1234)`
pub fn parse_file_history(body: &str) -> Vec<FileRenameEvent> {
    let section_start = match body.find("## File History") {
        Some(pos) => pos,
        None => return vec![],
    };

    let section = &body[section_start..];
    let section_end = section["## File History".len()..]
        .find("\n## ")
        .map(|pos| pos + "## File History".len())
        .unwrap_or(section.len());
    let section = &section[..section_end];

    let mut events = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with("* `") {
            continue;
        }
        // Line: * `old_path` → `new_path` (commit: abc1234)
        let rest = &line[3..]; // skip "* `"
        let old_end = match rest.find('`') {
            Some(i) => i,
            None => continue,
        };
        let old_path = rest[..old_end].to_string();

        let after_old = &rest[old_end + 1..]; // after closing `
        // find " → `"
        let arrow = " \u{2192} `";
        let new_start = match after_old.find(arrow) {
            Some(i) => i + arrow.len(),
            None => continue,
        };
        let after_arrow = &after_old[new_start..];
        let new_end = match after_arrow.find('`') {
            Some(i) => i,
            None => continue,
        };
        let new_path = after_arrow[..new_end].to_string();

        // find "(commit: ...)"
        let commit_prefix = "(commit: ";
        let commit = match after_arrow[new_end + 1..].find(commit_prefix) {
            Some(i) => {
                let commit_start = i + commit_prefix.len();
                let rest = &after_arrow[new_end + 1 + commit_start..];
                match rest.find(')') {
                    Some(end) => rest[..end].to_string(),
                    None => continue,
                }
            }
            None => continue,
        };

        events.push(FileRenameEvent {
            old_path,
            new_path,
            commit,
        });
    }

    events
}

/// Insert (or replace) the `## File History` section in the issue body.
///
/// If the section already exists it is replaced in-place.
/// Otherwise it is inserted immediately before the first `# ` checklist heading,
/// or appended at the end if no such heading exists.
pub fn splice_file_history(body: &str, history_section: &str) -> String {
    let history_trimmed = history_section.trim_end();

    if let Some(start) = body.find("## File History") {
        let before = body[..start].trim_end();
        let after_header = &body[start + "## File History".len()..];
        let after = match after_header.find("\n## ") {
            Some(p) => &after_header[p + 1..],
            None => "",
        };
        return if after.is_empty() {
            format!("{}\n{}", before, history_trimmed)
        } else {
            format!("{}\n{}\n\n{}", before, history_trimmed, after)
        };
    }

    if let Some(checklist_pos) = find_checklist_start(body) {
        let before = body[..checklist_pos].trim_end();
        let rest = &body[checklist_pos..];
        format!("{}\n\n{}\n\n{}", before, history_trimmed, rest)
    } else {
        format!("{}\n\n{}", body.trim_end(), history_trimmed)
    }
}

/// Find the byte offset of the first `# ` heading that is NOT `## `.
pub fn find_checklist_start(body: &str) -> Option<usize> {
    let mut pos = 0usize;
    for line in body.lines() {
        if line.starts_with("# ") && !line.starts_with("## ") {
            return Some(pos);
        }
        pos += line.len() + 1;
    }
    None
}

/// Generate the markdown for a "## File History" section.
pub fn file_history_section(events: &[FileRenameEvent]) -> String {
    let mut section = String::from("## File History\n");
    for event in events {
        section.push_str(&format!(
            "* `{}` \u{2192} `{}` (commit: {})\n",
            event.old_path, event.new_path, event.commit
        ));
    }
    section
}

#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    #[error(transparent)]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Branch '{0}' is not checked out locally")]
    LocalBranchNotFound(String),
    #[error(transparent)]
    GitFileOpsError(GitFileOpsError),
    #[error("Initial commit not found in issue body")]
    InitialCommitNotFound,
    #[error("Branch not found in issue body")]
    BranchNotFound,
    #[error("Milestone not found for issue")]
    MilestoneNotFound,
    #[error("Commit string '{0}' could not be parsed to a valid ObjectId")]
    CommitNotParseable(String),
    #[error("No commits found for file: {0}")]
    CommitNotFound(PathBuf),
}

impl From<GitFileOpsError> for IssueError {
    fn from(e: GitFileOpsError) -> Self {
        match e {
            GitFileOpsError::LocalBranchNotFound(name) => IssueError::LocalBranchNotFound(name),
            other => IssueError::GitFileOpsError(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{
        GitComment, GitCommit, GitCommitOps, GitFileOps, GitFileOpsError, GitHubReader,
    };
    use octocrab::models::issues::Issue;
    use std::path::PathBuf;
    use std::str::FromStr;

    /// A `GitComment` as loaded from the cache: no comment id or URL.
    fn test_comment(body: String, author_login: String) -> GitComment {
        GitComment {
            body,
            author_login,
            created_at: chrono::Utc::now(),
            id: None,
            html_url: None,
            html: None,
        }
    }

    fn load_issue(file_name: &str) -> Issue {
        let path = format!("src/tests/issue_threads/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    fn load_comments(file_name: &str) -> Vec<serde_json::Value> {
        let path = format!("src/tests/issue_threads/comments/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read comments file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse comments file {}: {}", path, e))
    }

    fn create_test_commits() -> Vec<(ObjectId, String)> {
        vec![
            (
                ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                "Initial commit".to_string(),
            ),
            (
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                "Second commit".to_string(),
            ),
            (
                ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                "Third commit".to_string(),
            ),
            (
                ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap(),
                "Fourth commit".to_string(),
            ),
            (
                ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap(),
                "Fifth commit".to_string(),
            ),
            (
                ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap(),
                "Sixth commit".to_string(),
            ),
            (
                ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap(),
                "Seventh commit".to_string(),
            ),
            // Additional commits for test_from_issue_open_with_approval_and_notification
            (
                ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap(),
                "Initial test commit".to_string(),
            ),
            (
                ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap(),
                "Second test commit".to_string(),
            ),
            (
                ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap(),
                "Third test commit".to_string(),
            ),
        ]
    }

    // Simple mock for IssueThread tests
    struct SimpleMockGitInfo {
        commits: Vec<(ObjectId, String)>,
        comments: Vec<GitComment>,
    }

    impl SimpleMockGitInfo {
        fn new() -> Self {
            Self {
                commits: Vec::new(),
                comments: Vec::new(),
            }
        }

        fn with_commits(mut self, commits: Vec<(ObjectId, String)>) -> Self {
            self.commits = commits;
            self
        }

        fn with_comments(mut self, comments: Vec<GitComment>) -> Self {
            self.comments = comments;
            self
        }
    }

    impl GitCommitOps for SimpleMockGitInfo {
        fn merge_base(
            &self,
            _a: &ObjectId,
            _b: &ObjectId,
        ) -> Result<Option<ObjectId>, GitFileOpsError> {
            // Single-branch fixtures: every gap's bounds sit on the one walk, so no
            // gap ever needs a merge-base.
            Ok(None)
        }
        fn commits(
            &self,
            _branch: &Option<String>,
            _stop_at: Option<ObjectId>,
        ) -> Result<Vec<GitCommit>, GitFileOpsError> {
            Ok(self
                .commits
                .iter()
                .map(|(commit, message)| GitCommit {
                    commit: *commit,
                    message: message.clone(),
                })
                .collect())
        }

        fn branch_tip(&self, _branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
            Err(GitFileOpsError::LocalBranchNotFound("mock".to_string()))
        }

        fn file_touching_commits(
            &self,
            _branch: Option<String>,
            _file: &std::path::Path,
        ) -> Result<std::collections::HashSet<String>, GitFileOpsError> {
            // Return all commit hashes as "touching" since tests use a single file
            Ok(self.commits.iter().map(|(id, _)| id.to_string()).collect())
        }

        fn get_branches_containing_commit(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<String>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn find_merged_into_branch(
            &self,
            _target_commit: &ObjectId,
        ) -> Result<Option<String>, GitFileOpsError> {
            Ok(None)
        }
    }

    impl GitFileOps for SimpleMockGitInfo {
        fn authors(
            &self,
            _file: &std::path::Path,
        ) -> Result<Vec<crate::git::GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &std::path::Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    impl GitHubReader for SimpleMockGitInfo {
        async fn get_milestones(
            &self,
        ) -> Result<Vec<octocrab::models::Milestone>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issues(
            &self,
            _milestone: Option<u64>,
        ) -> Result<Vec<Issue>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issue(&self, _issue_number: u64) -> Result<Issue, crate::git::GitHubApiError> {
            Err(crate::git::GitHubApiError::NoApi)
        }

        async fn get_assignees(&self) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_user_details(
            &self,
            _username: &str,
        ) -> Result<crate::RepoUser, crate::git::GitHubApiError> {
            Ok(crate::RepoUser {
                login: _username.to_string(),
                name: None,
            })
        }

        async fn get_labels(&self) -> Result<Vec<String>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_issue_comments(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<GitComment>, crate::git::GitHubApiError> {
            Ok(self.comments.clone())
        }

        async fn get_issue_events(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<serde_json::Value>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_blocked_issues(
            &self,
            _issue_number: u64,
        ) -> Result<Vec<Issue>, crate::git::GitHubApiError> {
            Ok(Vec::new())
        }

        async fn get_current_user(&self) -> Result<Option<String>, crate::git::GitHubApiError> {
            Ok(None)
        }
    }

    /// Load a fixture thread's issue and comments.
    fn fixture(issue_file: &str, comments_file: &str) -> (Issue, Vec<GitComment>) {
        let issue = load_issue(issue_file);
        let comments = load_comments(comments_file)
            .into_iter()
            .map(|comment| {
                test_comment(
                    comment["body"].as_str().unwrap().to_string(),
                    comment["user"]["login"]
                        .as_str()
                        .unwrap_or("test-user")
                        .to_string(),
                )
            })
            .collect();
        (issue, comments)
    }

    /// The commits every event of the round at `position` names, in comment order.
    fn event_commits(thread: &IssueThread, position: usize) -> Vec<ObjectId> {
        thread.segments[position]
            .as_round()
            .expect("expected a Round")
            .events
            .iter()
            .map(|event| *event.commit())
            .collect()
    }

    #[tokio::test]
    async fn test_from_issue_open_with_notifications() {
        // Comment sequence:
        // 1. Initial commit: abc123def456789012345678901234567890abcd (from issue body)
        // 2. Notification: current commit: def456789abc012345678901234567890123abcd
        // 3. Notification: current commit: 123abcd (short SHA)
        // No approval commits in this test
        let (issue, comments) = fixture(
            "open_issue_with_notifications.json",
            "open_issue_notifications.json",
        );

        let git_info = SimpleMockGitInfo::new()
            .with_commits(create_test_commits())
            .with_comments(comments);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        assert_eq!(
            result.initial_commit(),
            Some(&ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap())
        );

        // What the comments said is the round's events — the only record of it. Both
        // full and short SHAs resolve.
        assert_eq!(
            event_commits(&result, 0),
            vec![
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                // 123abcd matches this commit
                ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap(),
            ]
        );

        // An open round is the active segment, so nothing stands approved.
        assert_eq!(result.standing_approval(), None);
        assert_eq!(result.last_approved_commit(), None);
        assert_eq!(result.file, PathBuf::from("src/main.rs"));
        assert_eq!(result.active_branch(), "feature/new-feature");
        assert_eq!(
            crate::round::segment_invariants(&result.segments, None),
            Ok(()),
            "{:?}",
            result.segments
        );
    }

    #[tokio::test]
    async fn test_from_issue_closed_with_approval() {
        // Comment sequence:
        // 1. Initial commit: def456abc789012345678901234567890123abcd (from issue body)
        // 2. Notification: current commit: 456def789abc012345678901234567890123cdef
        // 3. Approval: approved qc commit: 456def789abc012345678901234567890123cdef
        // No unapproval - approval remains valid
        let (issue, comments) = fixture(
            "closed_approved_issue.json",
            "closed_approved_comments.json",
        );

        let git_info = SimpleMockGitInfo::new()
            .with_commits(create_test_commits())
            .with_comments(comments);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        let approved = ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap();
        assert_eq!(
            result.initial_commit(),
            Some(&ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap())
        );

        // One commit both notified and approved: the notification is the round's event,
        // the approval is the state that closed it.
        assert_eq!(event_commits(&result, 0), vec![approved]);
        assert_eq!(
            result.segments[0].as_round().unwrap().closing_commit(),
            Some(&approved)
        );

        // A closed round is never last: the drift since its approval is its own segment.
        assert!(result.segments.last().unwrap().as_gap().is_some());
        assert_eq!(result.standing_approval(), Some(&approved));
        assert_eq!(result.last_approved_commit(), Some(&approved));

        assert_eq!(result.file, PathBuf::from("src/lib.rs"));
        assert_eq!(result.active_branch(), "bugfix/memory-leak");
        assert_eq!(
            crate::round::segment_invariants(&result.segments, None),
            Ok(())
        );
    }

    #[tokio::test]
    async fn test_from_issue_with_unapproval() {
        // Comment sequence:
        // 1. Initial commit: 789abc12def345678901234567890123456789ef (from issue body)
        // 2. Notification: current commit: 890cdef123abc456789012345678901234567890
        // 3. Approval: approved qc commit: 890cdef123abc456789012345678901234567890
        // 4. Notification: current commit: abc1234 (short SHA)
        // 5. Unapproval: # QC Un-Approval (invalidates the approval from step 3)
        let (issue, comments) = fixture("unapproved_issue.json", "unapproved_comments.json");

        let git_info = SimpleMockGitInfo::new()
            .with_commits(create_test_commits())
            .with_comments(comments);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        let retracted = ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap();
        assert_eq!(
            result.initial_commit(),
            Some(&ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap())
        );
        assert_eq!(
            event_commits(&result, 0),
            vec![
                retracted,
                ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap(),
            ]
        );

        // The un-approval reopened the round, so it is the only segment and nothing
        // stands approved — not even as a past approval, since it was taken back.
        assert_eq!(result.segments.len(), 1);
        assert_eq!(
            result.segments[0].as_round().unwrap().retractions[0].retracted_commit,
            retracted
        );
        assert_eq!(result.standing_approval(), None);
        assert_eq!(result.last_approved_commit(), None);
        assert_eq!(result.file, PathBuf::from("src/utils.rs"));
        assert_eq!(result.active_branch(), "feature/utils-refactor");
    }

    #[tokio::test]
    async fn test_from_issue_open_with_approval_and_notification() {
        // Comment sequence:
        // 1. Initial commit: 111def456789012345678901234567890123abcd (from issue body)
        // 2. Notification: current commit: 222abc123456789012345678901234567890def
        // 3. Approval: approved qc commit: 222abc123456789012345678901234567890def
        // 4. Notification: current commit: 333cdef78 (short SHA)
        // Issue is open but approval remains valid (no unapproval)
        let (issue, comments) = fixture(
            "open_issue_with_approval_and_notification.json",
            "open_issue_approval_and_notification.json",
        );

        let test_commits = vec![
            (
                ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap(),
                "Initial".to_string(),
            ),
            (
                ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap(),
                "Second".to_string(),
            ),
            (
                ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap(),
                "Third".to_string(),
            ),
        ];

        let git_info = SimpleMockGitInfo::new()
            .with_commits(test_commits)
            .with_comments(comments);

        let result = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        let approved = ObjectId::from_str("222abc123456789012345678901234567890def0").unwrap();
        assert_eq!(
            result.initial_commit(),
            Some(&ObjectId::from_str("111def456789012345678901234567890123abcd").unwrap())
        );

        // Both notifications are events of round 1; the second arrived after the
        // approval, which is recorded rather than dropped.
        assert_eq!(
            event_commits(&result, 0),
            vec![
                approved,
                ObjectId::from_str("333cdef789012345678901234567890123456789").unwrap(),
            ]
        );
        assert!(
            result
                .anomalies
                .contains(&crate::round::RoundAnomaly::EventAfterClose { comment_index: 2 }),
            "anomalies: {:?}",
            result.anomalies
        );

        // The approval still stands despite the issue being open.
        assert_eq!(result.last_approved_commit(), Some(&approved));
        assert_eq!(result.standing_approval(), Some(&approved));

        assert_eq!(result.file, PathBuf::from("src/test.rs"));
        assert_eq!(result.active_branch(), "feature/test-branch");
        assert!(result.open);
        assert_eq!(
            crate::round::segment_invariants(&result.segments, None),
            Ok(())
        );
    }

    /// Initial QC anchored at a commit no walk knows: the round is unplaceable and its
    /// `opened_at` is the null-OID placeholder, so every accessor below must refuse it.
    async fn thread_with_unplaceable_initial_qc() -> IssueThread {
        let (issue, comments) = fixture(
            "open_issue_with_notifications.json",
            "open_issue_notifications.json",
        );

        // A branch we can walk, but on which nothing the comments name exists.
        let git_info = SimpleMockGitInfo::new()
            .with_commits(vec![(
                ObjectId::from_str("9999999999999999999999999999999999999999").unwrap(),
                "unrelated".to_string(),
            )])
            .with_comments(comments);

        let thread = IssueThread::from_issue(&issue, None, &git_info)
            .await
            .unwrap();

        let initial = thread.segments[0].as_round().expect("expected a Round");
        assert!(!initial.is_placed(), "placement: {:?}", initial.placement);
        assert_eq!(initial.opened_at, ObjectId::null(gix::hash::Kind::Sha1));
        thread
    }

    /// A grayed round has no commit to offer: the placeholder is not an address.
    #[tokio::test]
    async fn an_unplaceable_initial_qc_has_no_initial_commit() {
        let thread = thread_with_unplaceable_initial_qc().await;
        assert_eq!(thread.initial_commit(), None);
    }

    /// The notification diff base is a write-path input, so a null OID here would post a
    /// comment diffing against nothing at all.
    #[tokio::test]
    async fn an_unplaceable_active_round_has_no_notification_base() {
        let thread = thread_with_unplaceable_initial_qc().await;
        assert_eq!(thread.next_notification_from(), None);
    }

    /// Every existing fixture thread predates the `# QC Round` marker, so each one must
    /// fold to exactly one round: "Initial QC", anchored at the issue body's initial
    /// commit and walked on the issue body's branch.
    #[tokio::test]
    async fn test_existing_fixtures_fold_to_initial_qc_only() {
        let fixtures = [
            (
                "open_issue_with_notifications.json",
                "open_issue_notifications.json",
                "feature/new-feature",
            ),
            (
                "closed_approved_issue.json",
                "closed_approved_comments.json",
                "bugfix/memory-leak",
            ),
            (
                "unapproved_issue.json",
                "unapproved_comments.json",
                "feature/utils-refactor",
            ),
            (
                "open_issue_with_approval_and_notification.json",
                "open_issue_approval_and_notification.json",
                "feature/test-branch",
            ),
        ];

        for (issue_file, comments_file, branch) in fixtures {
            let (issue, comments) = fixture(issue_file, comments_file);
            let git_info = SimpleMockGitInfo::new()
                .with_commits(create_test_commits())
                .with_comments(comments);

            let thread = IssueThread::from_issue(&issue, None, &git_info)
                .await
                .unwrap();

            assert_eq!(
                thread.rounds().count(),
                1,
                "{issue_file} should have one round"
            );
            let round = thread.segments[0].as_round().expect("{issue_file}");
            assert_eq!(round.index, 1, "{issue_file}");
            assert_eq!(round.name(), "Initial QC", "{issue_file}");
            assert_eq!(
                Some(&round.opened_at),
                thread.initial_commit(),
                "{issue_file}"
            );
            // A single-branch thread is walked exactly as it always was.
            assert_eq!(round.branch, branch, "{issue_file}");
            assert!(round.is_placed(), "{issue_file}: {:?}", round.placement);
            assert_eq!(thread.previous_approval_of(0), None, "{issue_file}");
            // The only anomaly these fixtures legitimately produce is a notification
            // posted after an approval and before the matching un-approval.
            assert!(
                thread.anomalies.iter().all(|anomaly| matches!(
                    anomaly,
                    crate::round::RoundAnomaly::EventAfterClose { .. }
                )),
                "{issue_file} anomalies: {:?}",
                thread.anomalies
            );
            // A closed round is followed by a gap, and that gap's older bound is the
            // approval that stands.
            assert_eq!(
                thread.standing_approval(),
                round.closing_commit(),
                "{issue_file}"
            );
            assert_eq!(
                crate::round::segment_invariants(&thread.segments, None),
                Ok(()),
                "{issue_file}"
            );
        }
    }

    #[test]
    fn test_parse_commit_from_pattern_full_sha() {
        let body = "approved qc commit: abc123def456789012345678901234567890abcd";

        let result = parse_commit_from_pattern(body, "approved qc commit: ");
        assert_eq!(result, Some("abc123def456789012345678901234567890abcd"));
    }

    #[test]
    fn test_parse_commit_from_pattern_short_sha() {
        let body = "current commit: abc123d";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("abc123d"));
    }

    #[test]
    fn test_parse_commit_from_pattern_minimum_length() {
        let body = "current commit: abc123";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("abc123"));
    }

    #[test]
    fn test_parse_commit_from_pattern_no_match() {
        let body = "current commit: nonexistent123";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, Some("nonexistent123"));
    }

    #[test]
    fn test_parse_commit_from_pattern_not_found() {
        let body = "some other content";

        let result = parse_commit_from_pattern(body, "current commit: ");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_basic() {
        let body = "## Metadata\ninitial qc commit: abc123\ngit branch: feature/new-feature\nauthor: John Doe";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_with_extra_whitespace() {
        let body = "git branch:   main  \nother content";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_complex_branch_name() {
        let body = "git branch: feature/JIRA-123_fix-memory-leak\n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/JIRA-123_fix-memory-leak".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_not_found() {
        let body = "## Metadata\ninitial qc commit: abc123\nauthor: John Doe";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_empty_branch() {
        let body = "git branch: \n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_only_spaces() {
        let body = "git branch:    \n";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link() {
        let body = "git branch: [feature/new-feature](https://github.com/owner/repo/tree/feature/new-feature)";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link_main() {
        let body = "git branch: [main](https://github.com/owner/repo) branch.";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link_complex_name() {
        let body = "git branch: [bugfix/JIRA-123_memory-leak](https://github.com/owner/repo/tree/bugfix/JIRA-123_memory-leak)";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("bugfix/JIRA-123_memory-leak".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_markdown_link_http_ignored() {
        let body = "Check [https://example.com](https://example.com) for details.";
        let result = parse_branch_from_body(body);
        assert_eq!(result, None); // Should ignore HTTP URLs
    }

    #[test]
    fn test_parse_branch_from_body_prefers_git_branch_pattern() {
        let body =
            "git branch: main\n\nSee also [develop](https://github.com/owner/repo/tree/develop)";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string())); // Should prefer git branch pattern
    }

    #[test]
    fn test_parse_branch_from_body_git_branch_markdown_link() {
        let body = "git branch: [main](https://github.com/A2-ai/ghqc_status_project2/tree/main)\nauthor: test";
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_git_branch_html_link() {
        let body = r#"git branch: <a href="https://github.com/A2-ai/ghqc_status_project2/tree/main" target="_blank">main</a>
author: test"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_html_link_in_content() {
        let body = r#"git branch: <a href="https://github.com/owner/repo/tree/feature/new-feature">feature/new-feature</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("feature/new-feature".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_html_link_extract_from_url() {
        let body = r#"git branch: <a href="https://github.com/A2-ai/repo/tree/bugfix/memory-leak" target="_blank">file contents</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("file contents".to_string())); // Should extract link text, not URL
    }

    #[test]
    fn test_parse_branch_from_body_complex_example() {
        let body = r#"## Metadata

* initial qc commit: a7075606219a40c7536af8cd1b5f0b761965826c
* git branch: [main](https://github.com/A2-ai/ghqc_status_project2/tree/a7075606219a40c7536af8cd1b5f0b761965826c)
* author: jenna-a2ai <jenna@a2-ai.com>
* <a href="https://github.com/A2-ai/ghqc_status_project2/blob/a70756/dvs.yaml" target="_blank">file contents at initial qc commit</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, Some("main".to_string()));
    }

    #[test]
    fn test_parse_branch_from_body_html_link_with_spaces_ignored() {
        let body = r#"<a href="https://docs.com">Code Review Process</a>"#;
        let result = parse_branch_from_body(body);
        assert_eq!(result, None); // Should ignore links with spaces in text
    }

    // Tests for parse_blocking_qcs

    #[test]
    fn test_parse_blocking_qcs_from_body() {
        let body = r#"## Metadata

* initial qc commit: abc123
* git branch: main
* author: test

## Relevant Files

### Previous QC
- [previous.R](https://github.com/owner/repo/issues/123) - Previous version of this file
- [old_analysis.R](https://github.com/owner/repo/issues/124)

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/200) - Upstream dependency

### Relevant QC
- [related.R](https://github.com/owner/repo/issues/300)

# Code Review Checklist
- [ ] Check 1
- [ ] Check 2"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 3);

        // Check Previous QC entries
        let previous_qcs: Vec<_> = result
            .iter()
            .filter(|qc| qc.relationship == BlockingRelationship::PreviousQC)
            .collect();
        assert_eq!(previous_qcs.len(), 2);
        assert!(previous_qcs.iter().any(|qc| qc.issue_number == 123));
        assert!(previous_qcs.iter().any(|qc| qc.issue_number == 124));

        // Check Gating QC entry
        let gating_qcs: Vec<_> = result
            .iter()
            .filter(|qc| qc.relationship == BlockingRelationship::GatingQC)
            .collect();
        assert_eq!(gating_qcs.len(), 1);
        assert_eq!(gating_qcs[0].issue_number, 200);
        assert_eq!(gating_qcs[0].file_name, PathBuf::from("upstream.R"));
    }

    #[test]
    fn test_parse_blocking_qcs_empty() {
        let body = "## Metadata\n\n* commit: abc123\n\n# Checklist\n- [ ] Item";
        let result = parse_blocking_qcs(body);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_blocking_qcs_no_relevant_files_section() {
        let body = "## Metadata\n\nSome content without relevant files section";
        let result = parse_blocking_qcs(body);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_blocking_qcs_partial() {
        // Only Gating QC section present, no Previous QC
        let body = r#"## Relevant Files

### Gating QC
- [gating.R](https://github.com/owner/repo/issues/50)

### Relevant QC
- [other.R](https://github.com/owner/repo/issues/60)"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].issue_number, 50);
        assert_eq!(result[0].relationship, BlockingRelationship::GatingQC);
    }

    #[test]
    fn test_parse_blocking_qcs_ghe_url() {
        // Test with GitHub Enterprise URLs
        let body = r#"## Relevant Files

### Gating QC
- [script.R](https://ghe.company.com/org/repo/issues/42) - Enterprise issue"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].issue_number, 42);
        assert_eq!(result[0].file_name, PathBuf::from("script.R"));
    }

    #[test]
    fn test_parse_blocking_qcs_multiple_urls() {
        let body = r#"## Relevant Files

### Gating QC
- [file1.R](https://github.com/owner/repo/issues/1)
- [file2.R](https://github.com/owner/repo/issues/2)
- [file3.R](https://github.com/owner/repo/issues/3)"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 3);
        let issue_numbers: Vec<u64> = result.iter().map(|qc| qc.issue_number).collect();
        assert!(issue_numbers.contains(&1));
        assert!(issue_numbers.contains(&2));
        assert!(issue_numbers.contains(&3));
    }

    #[test]
    fn test_parse_blocking_qcs_extracts_file_name() {
        let body = r#"## Relevant Files

### Previous QC
- [path/to/complex-file_name.R](https://github.com/owner/repo/issues/99)"#;

        let result = parse_blocking_qcs(body);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].file_name,
            PathBuf::from("path/to/complex-file_name.R")
        );
    }

    // Tests for determine_relationship_from_body

    #[test]
    fn test_determine_relationship_from_child_body_gating() {
        let body = r#"## Relevant Files

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/50)

### Previous QC
- [old.R](https://github.com/owner/repo/issues/60)"#;

        let result = determine_relationship_from_body(body, 50);
        assert_eq!(result, BlockingRelationship::GatingQC);
    }

    #[test]
    fn test_determine_relationship_from_child_body_previous() {
        let body = r#"## Relevant Files

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/50)

### Previous QC
- [old.R](https://github.com/owner/repo/issues/60)"#;

        let result = determine_relationship_from_body(body, 60);
        assert_eq!(result, BlockingRelationship::PreviousQC);
    }

    #[test]
    fn test_determine_relationship_from_child_body_not_found() {
        let body = r#"## Relevant Files

### Gating QC
- [upstream.R](https://github.com/owner/repo/issues/50)"#;

        let result = determine_relationship_from_body(body, 999);
        assert_eq!(result, BlockingRelationship::Unknown);
    }

    #[test]
    fn test_determine_relationship_no_relevant_files() {
        let body = "## Metadata\n\nNo relevant files section";
        let result = determine_relationship_from_body(body, 123);
        assert_eq!(result, BlockingRelationship::Unknown);
    }

    // ── parse_file_history ────────────────────────────────────────────────────

    #[test]
    fn test_parse_file_history_no_section() {
        let body = "## Metadata\nsome content\n";
        let events = parse_file_history(body);
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_file_history_single_event() {
        let body = "## File History\n* `old/path.R` → `new/path.R` (commit: abc1234)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].old_path, "old/path.R");
        assert_eq!(events[0].new_path, "new/path.R");
        assert_eq!(events[0].commit, "abc1234");
    }

    #[test]
    fn test_parse_file_history_multiple_events() {
        let body = "## File History\n\
            * `a.R` → `b.R` (commit: 111aaaa)\n\
            * `b.R` → `c.R` (commit: 222bbbb)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].old_path, "a.R");
        assert_eq!(events[0].new_path, "b.R");
        assert_eq!(events[0].commit, "111aaaa");
        assert_eq!(events[1].old_path, "b.R");
        assert_eq!(events[1].new_path, "c.R");
        assert_eq!(events[1].commit, "222bbbb");
    }

    #[test]
    fn test_parse_file_history_malformed_lines_skipped() {
        let body = "## File History\n\
            * `good.R` → `better.R` (commit: abc0001)\n\
            * malformed line without backticks\n\
            * `missing_arrow.R` something wrong\n\
            * `also_good.R` → `also_better.R` (commit: abc0002)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].old_path, "good.R");
        assert_eq!(events[1].old_path, "also_good.R");
    }

    #[test]
    fn test_parse_file_history_terminates_at_next_section() {
        let body = "## File History\n\
            * `old.R` → `new.R` (commit: abc1234)\n\
            ## Other Section\n\
            * `should_not.R` → `be_parsed.R` (commit: 000000)\n";
        let events = parse_file_history(body);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].old_path, "old.R");
    }

    // ── file_history_section ──────────────────────────────────────────────────

    #[test]
    fn test_file_history_section_empty_slice() {
        let section = file_history_section(&[]);
        assert_eq!(section, "## File History\n");
    }

    #[test]
    fn test_file_history_section_single_event() {
        let events = vec![FileRenameEvent {
            old_path: "src/old.R".to_string(),
            new_path: "src/new.R".to_string(),
            commit: "deadbeef".to_string(),
        }];
        let section = file_history_section(&events);
        assert_eq!(
            section,
            "## File History\n* `src/old.R` → `src/new.R` (commit: deadbeef)\n"
        );
    }

    #[test]
    fn test_file_history_section_multiple_events() {
        let events = vec![
            FileRenameEvent {
                old_path: "a.R".to_string(),
                new_path: "b.R".to_string(),
                commit: "aaa1111".to_string(),
            },
            FileRenameEvent {
                old_path: "b.R".to_string(),
                new_path: "c.R".to_string(),
                commit: "bbb2222".to_string(),
            },
        ];
        let section = file_history_section(&events);
        assert!(section.starts_with("## File History\n"));
        assert!(section.contains("* `a.R` → `b.R` (commit: aaa1111)\n"));
        assert!(section.contains("* `b.R` → `c.R` (commit: bbb2222)\n"));
    }

    // ── find_checklist_start ──────────────────────────────────────────────────

    #[test]
    fn test_find_checklist_start_finds_h1() {
        let body = "## Metadata\nsome text\n# Checklist\n- item\n";
        let pos = find_checklist_start(body);
        assert!(pos.is_some());
        let offset = pos.unwrap();
        assert!(body[offset..].starts_with("# Checklist"));
    }

    #[test]
    fn test_find_checklist_start_only_h2_returns_none() {
        let body = "## Metadata\n## File History\n## Another\n";
        let pos = find_checklist_start(body);
        assert!(pos.is_none());
    }

    #[test]
    fn test_find_checklist_start_empty_body() {
        let pos = find_checklist_start("");
        assert!(pos.is_none());
    }

    // ── splice_file_history ───────────────────────────────────────────────────

    #[test]
    fn test_splice_file_history_no_section_with_checklist() {
        let body = "## Metadata\nsome text\n\n# Checklist\n- [ ] item\n";
        let history = "## File History\n* `old.R` → `new.R` (commit: abc)\n";
        let result = splice_file_history(body, history);
        // History must appear before the checklist
        let history_pos = result.find("## File History").expect("history missing");
        let checklist_pos = result.find("# Checklist").expect("checklist missing");
        assert!(history_pos < checklist_pos);
    }

    #[test]
    fn test_splice_file_history_no_section_no_checklist() {
        let body = "## Metadata\nsome text\n";
        let history = "## File History\n* `old.R` → `new.R` (commit: abc)\n";
        let result = splice_file_history(body, history);
        // History appended at end
        assert!(result.contains("## File History"));
        assert!(result.ends_with("## File History\n* `old.R` → `new.R` (commit: abc)"));
    }

    #[test]
    fn test_splice_file_history_replaces_existing_section() {
        let body = "## Metadata\nsome text\n\n## File History\n* `old.R` → `mid.R` (commit: 111)\n\n# Checklist\n- [ ] item\n";
        let new_history = "## File History\n* `old.R` → `mid.R` (commit: 111)\n* `mid.R` → `new.R` (commit: 222)\n";
        let result = splice_file_history(body, new_history);
        // Only one File History section
        assert_eq!(result.matches("## File History").count(), 1);
        assert!(result.contains("commit: 222"));
    }

    #[test]
    fn test_splice_file_history_existing_section_at_end() {
        let body = "## Metadata\nsome text\n\n## File History\n* `old.R` → `new.R` (commit: abc)\n";
        let new_history = "## File History\n* `old.R` → `new.R` (commit: abc)\n* `new.R` → `newest.R` (commit: def)\n";
        let result = splice_file_history(body, new_history);
        assert_eq!(result.matches("## File History").count(), 1);
        assert!(result.contains("commit: def"));
    }
}
