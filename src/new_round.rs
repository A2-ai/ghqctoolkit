//! Writing a new QC round, and the pure text helpers that seed its checklist.
//!
//! [`crate::round`] *reads* a round comment; this module *writes* one.
//! The two must agree exactly, so every metadata key used here is imported from
//! [`crate::round`] rather than retyped, and a round-trip test in this module
//! generates a body and folds it back with
//! [`crate::round::fold_rounds_from_comments`].
//!
//! Three groups of things live here:
//!
//! 1. [`QCNewRound`] — the comment body itself: marker, metadata, checklist. It
//!    deliberately carries **no** reviewer @-mention, no diff and no comparison
//!    links; a separate QC Notification comment does the notifying.
//! 2. Checklist seeding ([`reset_checklist`], [`checklist_from_round_comment`],
//!    [`checklist_from_issue_body`], [`seed_checklist`]) — pure functions over
//!    markdown, so a new round starts from the previous round's checklist with
//!    every box unchecked.
//! 3. The issue-body round marker ([`RoundMarker`], [`upsert_round_marker`],
//!    [`parse_round_marker`]) — a denormalised cache block so a body-only fetch
//!    can answer "what round, and where is its checklist". It is *only* a cache;
//!    the comment thread stays the source of truth.

use gix::ObjectId;
use octocrab::models::issues::Issue;
use regex::Regex;
use std::sync::LazyLock;

use crate::comment_system::CommentBody;
use crate::git::{GitFileOps, GitHelpers};
use crate::round::{
    ChecklistSource, INITIAL_ROUND_COMMIT_KEY, NOTE_KEY, PREVIOUS_APPROVED_COMMIT_KEY,
    ROUND_HEADING, ROUND_KEY, UNNAMED_CHECKLIST_HEADING, checklist_name_from_body,
    find_checklist_heading,
};

/// Heading of the denormalised round cache block in an issue body.
pub const ROUND_MARKER_HEADING: &str = "## QC Round";
/// Round-marker key holding the current round number.
const CURRENT_ROUND_KEY: &str = "current round: ";
/// Round-marker key holding the URL of the round's round comment.
const ROUND_COMMENT_KEY: &str = "round comment: ";
/// Preamble explaining that the checklist below the marker is the *initial* one.
const STRANDED_CHECKLIST_NOTE: &str = "> Checklist below is from Initial QC. The current round's checklist is in the round comment above.";

/// A round comment: it opens a new round and carries that round's
/// checklist.
#[derive(Debug, Clone)]
pub struct QCNewRound {
    pub issue: Issue,
    /// The new round's index (1-based, so always >= 2 in practice).
    pub round: u32,
    /// Anchor commit: HEAD at the time the round was opened.
    pub round_commit: ObjectId,
    /// The approval the new round builds on (the prior round's closing commit).
    pub previous_approved_commit: ObjectId,
    /// Name of the checklist template this round is QC'd against. Recorded because
    /// the round comment is the audit record of what the round was checked against,
    /// and the name cannot be derived from the checklist content.
    pub checklist_name: Option<String>,
    pub note: Option<String>,
    /// Checklist markdown, already seeded and reset (see [`seed_checklist`]).
    pub checklist_content: String,
}

impl CommentBody for QCNewRound {
    /// Only ever a label — for logs and for the part headers of a split comment. The
    /// round number lives in the body's heading and in its `round:` metadata.
    fn title(&self) -> &str {
        "QC Round"
    }

    /// `# QC Round <n>`, `## Metadata`, then the named checklist — nothing else.
    ///
    /// The number in the heading is for readers; the fold identifies the round from
    /// the `round:` metadata line, so the two can never disagree about identity.
    ///
    /// SHAs are written in full: the fold accepts abbreviations, but a full SHA
    /// can never become ambiguous.
    fn generate_body(&self, _git_info: &(impl GitHelpers + GitFileOps)) -> String {
        let mut metadata = vec![
            "## Metadata".to_string(),
            format!("{ROUND_KEY}{}", self.round),
            format!("{INITIAL_ROUND_COMMIT_KEY}{}", self.round_commit),
            format!(
                "{PREVIOUS_APPROVED_COMMIT_KEY}{}",
                self.previous_approved_commit
            ),
        ];
        if let Some(note) = &self.note {
            // The fold reads a note as the remainder of a single metadata line,
            // so a multi-line note is flattened rather than silently truncated.
            let note = flatten_to_one_line(note);
            if !note.is_empty() {
                metadata.push(format!("{NOTE_KEY}{note}"));
            }
        }

        let mut body = vec![
            format!("{ROUND_HEADING} {}", self.round),
            metadata.join("\n* "),
        ];

        // The checklist is named by a level-1 heading, exactly as in an issue body,
        // so the two read the same way and there is no separate `checklist:`
        // metadata line to keep in step with it. An unnamed checklist falls back to
        // the `## Checklist` heading: the metadata section ends at the next heading,
        // so this section always needs one.
        let checklist = self.checklist_content.trim();
        let name = self
            .checklist_name
            .as_deref()
            .map(flatten_to_one_line)
            .filter(|name| !name.is_empty());
        match (name, checklist.is_empty()) {
            // Emitted even with empty content: the heading is the only record of the
            // name, so dropping the section would lose it.
            (Some(name), _) => body.push(format!("# {name}\n{checklist}").trim_end().to_string()),
            (None, false) => body.push(format!("{UNNAMED_CHECKLIST_HEADING}\n{checklist}")),
            (None, true) => {}
        }

        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }
}

/// Collapse all whitespace runs containing a newline into a single space.
fn flatten_to_one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Checklist seeding ───────────────────────────────────────────────────────

/// A checked checkbox at the start of a line, with its `- ` prefix captured.
///
/// Deliberately narrower than [`crate::qc_status`]'s checklist regex in one way
/// only: the whitespace classes are `[ \t]` rather than `\s`, so a match can
/// never span a line break and indentation is preserved byte-for-byte. Like that
/// regex it tolerates `-   [x]` spacing, and only matches at the start of a line,
/// so a `[x]` inside prose is left alone.
static CHECKED_BOX_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^([ \t]*-[ \t]*)\[[xX]\]").expect("Failed to compile checked box regex")
});

/// Reset every checkbox in a checklist to unchecked.
///
/// All other text, indentation and nesting is preserved exactly; already
/// unchecked boxes are untouched.
pub fn reset_checklist(content: &str) -> String {
    CHECKED_BOX_REGEX
        .replace_all(content, "${1}[ ]")
        .into_owned()
}

/// A checklist recovered from an issue body or a round comment, together with the
/// name of the template it came from.
///
/// The name is carried alongside the content — rather than left inside it as a
/// heading — because [`QCNewRound`] records it as metadata, where the fold can
/// read it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededChecklist {
    /// Checklist markdown, with no `# <name>` heading line.
    pub content: String,
    /// The template name, when the source recorded one.
    pub name: Option<String>,
}

/// Extract the checklist from a round comment.
///
/// The checklist begins at its heading — `# <name>`, or `## Checklist` when no
/// template name was recorded — and runs to the **end of the body**, deliberately
/// *not* to the next `## ` heading. Real checklists carry their own `## `-level
/// section titles — `## Technical Review`, `## Rendering Instructions and Other
/// Comments` — so stopping at the first one truncated the checklist to whatever few
/// lines preceded it, and every later round inherited the stump.
/// [`QCNewRound::generate_body`] puts the checklist last, so end-of-body is exactly
/// the section's extent, and this is the same rule [`checklist_from_issue_body`]
/// applies to issue bodies.
///
/// The trade-off is intentional. Prose hand-appended after the checklist would be
/// absorbed into it, which the author can see and delete in the seeded editor,
/// whereas a silently truncated checklist loses items with no visible cause.
///
/// The heading line itself is never part of the content; the name comes back from
/// [`checklist_name_from_round_comment`].
///
/// `None` when there is no checklist heading, or the section holds only whitespace.
pub fn checklist_from_round_comment(body: &str) -> Option<String> {
    let start = find_checklist_heading(body)?.start();
    let trimmed = body[start..].trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The checklist template name a round comment recorded, if any.
///
/// Delegates to [`crate::round`]'s own reader — the one the fold uses — so the
/// writer here and the reader there cannot drift apart, and both formats are
/// understood identically.
pub fn checklist_name_from_round_comment(body: &str) -> Option<String> {
    checklist_name_from_body(body).map(|name| name.to_string())
}

/// Extract the checklist from an issue body, for seeding round 2 from Initial QC.
///
/// [`crate::Checklist`]'s `Display` renders a checklist as `# <name>` followed by
/// its content, and `QCIssue::body` appends it last, after `## `-level metadata
/// and relevant-files sections. So — exactly as
/// [`crate::find_checklist_start`] already codifies — the checklist begins at the
/// first level-1 `# ` heading and runs to the end of the body.
///
/// A body may hold more than one such section (see
/// [`crate::analyze_issue_checklists`], which iterates sections); all of them are
/// returned, concatenated in document order. The `# <name>` heading lines
/// themselves are dropped from the content: the round comment supplies its own
/// `## Checklist` heading, and a level-1 heading nested under it would misnest
/// the comment's own `# QC Round` heading. The *first* such heading's text is
/// returned as [`SeededChecklist::name`] instead, so the template a round was
/// QC'd against survives into the round comment's metadata.
pub fn checklist_from_issue_body(body: &str) -> Option<SeededChecklist> {
    // (heading text, section content), in document order.
    let mut sections: Vec<(Option<String>, String)> = Vec::new();
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("# ") {
            let heading = heading.trim();
            sections.push((
                (!heading.is_empty()).then(|| heading.to_string()),
                String::new(),
            ));
        } else if let Some((_, buffer)) = sections.last_mut() {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }

    // Empty sections carry neither content nor a usable name.
    let sections: Vec<(Option<String>, &str)> = sections
        .iter()
        .map(|(heading, content)| (heading.clone(), content.trim()))
        .filter(|(_, content)| !content.is_empty())
        .collect();

    let name = sections.first().and_then(|(heading, _)| heading.clone());
    let content = sections
        .iter()
        .map(|(_, content)| *content)
        .collect::<Vec<_>>()
        .join("\n\n");

    if content.is_empty() {
        None
    } else {
        Some(SeededChecklist { content, name })
    }
}

/// Seed a new round's checklist, with every box reset to unchecked.
///
/// The most recent prior round comment's checklist wins; the issue body's
/// (Initial QC's) checklist is the fallback. `None` when neither carries one.
///
/// The name travels with the content it describes: a checklist seeded from a prior
/// round comment takes that comment's `checklist:` metadata, while one seeded from
/// the issue body takes its `# <name>` heading.
pub fn seed_checklist(
    prior_round_comment_body: Option<&str>,
    issue_body: Option<&str>,
) -> Option<SeededChecklist> {
    if let Some(prior) = prior_round_comment_body
        && let Some(content) = checklist_from_round_comment(prior)
    {
        return Some(SeededChecklist {
            content: reset_checklist(&content),
            name: checklist_name_from_round_comment(prior),
        });
    }
    let seeded = issue_body.and_then(checklist_from_issue_body)?;
    Some(SeededChecklist {
        content: reset_checklist(&seeded.content),
        name: seeded.name,
    })
}

/// A checklist a new round can be based on: one entry per existing round whose
/// checklist could be recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistOption {
    /// [`crate::Round::index`] of the round this checklist belongs to.
    pub round: u32,
    /// That round's display name, e.g. `"Initial QC"` or `"Round 2"`.
    pub round_name: String,
    /// Template name that round recorded, when it recorded one.
    pub checklist_name: Option<String>,
    /// Checklist markdown with every box reset, ready to seed an editor.
    pub content: String,
}

/// Every round's checklist, oldest round first.
///
/// A new round does not have to follow the one before it: rounds diverge, and the
/// author may want to go back to the original checklist rather than inherit a
/// trimmed one. So all of them are offered, and the caller picks.
///
/// Rounds whose checklist cannot be recovered — no `## Checklist` section, or an
/// opening comment no longer present in `comments` — are omitted rather than
/// returned empty, so every entry is a choice that will actually work. The result
/// is therefore not indexed by round: match on [`ChecklistOption::round`].
pub fn available_checklists(
    thread: &crate::IssueThread,
    comments: &[crate::GitComment],
    issue_body: Option<&str>,
) -> Vec<ChecklistOption> {
    thread
        .rounds
        .iter()
        .filter_map(|round| {
            let (content, checklist_name) = match &round.checklist {
                ChecklistSource::IssueBody => {
                    let seeded = issue_body.and_then(checklist_from_issue_body)?;
                    (seeded.content, seeded.name)
                }
                ChecklistSource::Comment { comment_index, .. } => {
                    let body = comments.get(*comment_index)?.body.as_str();
                    (
                        checklist_from_round_comment(body)?,
                        checklist_name_from_round_comment(body),
                    )
                }
            };
            Some(ChecklistOption {
                round: round.index,
                round_name: round.name(),
                checklist_name,
                content: reset_checklist(&content),
            })
        })
        .collect()
}

/// The body of the comment that opened `thread`'s most recent round, which is
/// what [`seed_checklist`] wants as its first argument.
///
/// `None` when that round is Initial QC (whose checklist lives in the issue body)
/// or when the comment is no longer present in `comments`.
pub fn prior_round_comment_body<'a>(
    thread: &crate::IssueThread,
    comments: &'a [crate::GitComment],
) -> Option<&'a str> {
    match &thread.rounds.last()?.checklist {
        ChecklistSource::IssueBody => None,
        ChecklistSource::Comment { comment_index, .. } => comments
            .get(*comment_index)
            .map(|comment| comment.body.as_str()),
    }
}

// ── Issue-body round marker ─────────────────────────────────────────────────

/// The denormalised `## QC Round` cache block of an issue body.
///
/// Never authoritative: the comment thread is the source of truth. It exists so
/// a body-only fetch can tell which round is current and where its checklist
/// lives without reading the whole thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundMarker {
    pub round: u32,
    pub comment_url: String,
}

impl RoundMarker {
    /// The block exactly as it is written into an issue body.
    fn render(&self) -> String {
        format!(
            "{ROUND_MARKER_HEADING}\n* {CURRENT_ROUND_KEY}{}\n* {ROUND_COMMENT_KEY}{}\n\n{STRANDED_CHECKLIST_NOTE}",
            self.round, self.comment_url
        )
    }
}

/// Byte range of an existing `## QC Round` block: from the heading line to the
/// next heading line of any level, or the end of the body.
fn round_marker_range(issue_body: &str) -> Option<(usize, usize)> {
    let mut start: Option<usize> = None;
    let mut offset = 0usize;
    for raw_line in issue_body.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\n', '\r']);
        match start {
            None => {
                if line.trim() == ROUND_MARKER_HEADING {
                    start = Some(offset);
                }
            }
            Some(begin) => {
                if line.starts_with('#') {
                    return Some((begin, offset));
                }
            }
        }
        offset += raw_line.len();
    }
    start.map(|begin| (begin, issue_body.len()))
}

/// Insert or replace the `## QC Round` block in an issue body.
///
/// An existing block is replaced where it stands, so the block never appears
/// twice and nothing else in the body moves. A first-time block is inserted
/// immediately before the checklist — the first level-1 `# ` heading, as
/// [`crate::find_checklist_start`] defines it — so its stranded-checklist note
/// reads as a preamble to that checklist; with no such heading it is appended.
///
/// Idempotent: applying it twice yields the same string.
pub fn upsert_round_marker(issue_body: &str, marker: &RoundMarker) -> String {
    let block = marker.render();

    let (before, rest) = match round_marker_range(issue_body) {
        Some((start, end)) => (&issue_body[..start], &issue_body[end..]),
        None => match crate::issue::find_checklist_start(issue_body) {
            Some(position) => (&issue_body[..position], &issue_body[position..]),
            None => (issue_body, ""),
        },
    };

    let before = before.trim_end();
    let rest = rest.trim_start_matches('\n');

    let mut out = String::new();
    if !before.is_empty() {
        out.push_str(before);
        out.push_str("\n\n");
    }
    out.push_str(&block);
    if !rest.is_empty() {
        out.push_str("\n\n");
        out.push_str(rest);
    }
    out
}

/// Read the `## QC Round` cache block of an issue body, if it has one.
pub fn parse_round_marker(issue_body: &str) -> Option<RoundMarker> {
    let (start, end) = round_marker_range(issue_body)?;
    let block = &issue_body[start..end];

    let value = |key: &str| -> Option<String> {
        block.lines().skip(1).find_map(|line| {
            let mut rest = line.trim();
            if let Some(stripped) = rest.strip_prefix("* ").or_else(|| rest.strip_prefix("- ")) {
                rest = stripped.trim_start();
            }
            let value = rest.strip_prefix(key)?.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
    };

    let round = value(CURRENT_ROUND_KEY)?.parse::<u32>().ok()?;
    let comment_url = value(ROUND_COMMENT_KEY)?;
    Some(RoundMarker { round, comment_url })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitAuthor, GitComment, GitFileOpsError};
    use crate::round::{ChecklistSource, RawRoundOpen, RawRoundState, fold_rounds_from_comments};
    use std::path::Path;
    use std::str::FromStr;

    const A: &str = "aaaaaaa000000000000000000000000000000001";
    const B: &str = "bbbbbbb000000000000000000000000000000002";
    const C: &str = "ccccccc000000000000000000000000000000003";

    struct MockGitHelpers;

    impl GitHelpers for MockGitHelpers {
        fn file_content_url(&self, commit_sha: &str, file: &Path) -> String {
            format!(
                "https://github.com/owner/repo/blob/{}/{}",
                commit_sha,
                file.display()
            )
        }

        fn commit_comparison_url(
            &self,
            _current_commit: &ObjectId,
            _previous_commit: &ObjectId,
        ) -> String {
            "https://github.com/owner/repo/compare/abc123..def456".to_string()
        }

        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/owner/repo/issues/{issue_number}")
        }
    }

    impl GitFileOps for MockGitHelpers {
        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    fn load_issue(name: &str) -> Issue {
        let json_str =
            std::fs::read_to_string(format!("src/tests/github_api/issues/{}.json", name)).unwrap();
        serde_json::from_str(&json_str).unwrap()
    }

    fn new_round(note: Option<&str>) -> QCNewRound {
        QCNewRound {
            issue: load_issue("main_file_issue"),
            round: 2,
            round_commit: ObjectId::from_str(C).unwrap(),
            previous_approved_commit: ObjectId::from_str(B).unwrap(),
            checklist_name: Some("Code Review Checklist".to_string()),
            note: note.map(String::from),
            checklist_content: "- [ ] item one\n- [ ] item two".to_string(),
        }
    }

    // ── Comment body ─────────────────────────────────────────────────────────

    #[test]
    fn test_qc_new_round_body_with_note() {
        let body =
            new_round(Some("Second pass after the refactor.")).generate_body(&MockGitHelpers);
        insta::assert_snapshot!(body);
    }

    #[test]
    fn test_qc_new_round_body_without_note() {
        let body = new_round(None).generate_body(&MockGitHelpers);
        insta::assert_snapshot!(body);
    }

    #[test]
    fn body_has_no_mention_no_diff_and_names_the_checklist_with_a_level_one_heading() {
        let round = new_round(Some("note"));
        let body = round.generate_body(&MockGitHelpers);

        assert!(
            !body.contains('@'),
            "body must not @-mention anyone: {body}"
        );
        assert!(!body.contains("## File Difference"));
        assert!(!body.contains("commit comparison"));
        // The heading carries the round number, and the checklist is named by a
        // level-1 heading exactly as in an issue body.
        assert!(body.starts_with("# QC Round 2\n"), "{body}");
        assert!(body.contains("\n# Code Review Checklist\n"), "{body}");
        assert!(!body.contains("## Checklist"), "{body}");
        // Identity stays in the metadata, not in the heading.
        assert!(body.contains("* round: 2\n"), "{body}");
        // Full SHAs, never abbreviated.
        assert!(body.contains(C));
        assert!(body.contains(B));
        assert_eq!(round.title(), "QC Round");
        assert_eq!(round.issue().number, round.issue.number);
    }

    /// The heading number is decoration; `round:` is the identity. A heading that
    /// disagrees with the metadata must not change which round the fold sees.
    #[test]
    fn the_round_number_comes_from_metadata_not_the_heading() {
        let comment = |body: &str| GitComment {
            body: body.to_string(),
            author_login: "tester".to_string(),
            created_at: chrono::Utc::now(),
            id: None,
            html_url: None,
            html: None,
        };
        let approval = format!("# QC Approval\n\n## Metadata\napproved qc commit: {B}\n");
        let body = format!(
            "# QC Round 97\n\n## Metadata\n* round: 2\n* initial qc round commit: {C}\n\
             * previous approved commit: {B}\n\n# Code Review Checklist\n- [ ] one\n"
        );

        let comments = vec![comment(&approval), comment(&body)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);

        assert_eq!(rounds.len(), 2);
        // Derived from the fold, not read from "97".
        assert_eq!(rounds[1].index, 2);
        assert_eq!(rounds[1].opened_at, C);
    }

    #[test]
    fn multi_line_note_is_flattened_onto_one_metadata_line() {
        let mut round = new_round(None);
        round.note = Some("first line\nsecond line".to_string());
        let body = round.generate_body(&MockGitHelpers);
        assert!(body.contains("* note: first line second line\n"));
    }

    /// The whole point of this module: what we write is what the fold reads.
    #[test]
    fn generated_body_folds_back_into_the_intended_round() {
        let round = new_round(Some("Second pass after the refactor."));
        let body = round.generate_body(&MockGitHelpers);

        let comment = |body: &str| GitComment {
            body: body.to_string(),
            author_login: "tester".to_string(),
            created_at: chrono::Utc::now(),
            id: None,
            html_url: None,
            html: None,
        };
        let comments = vec![
            comment(&format!(
                "# QC Approval\n\n## Metadata\napproved qc commit: {B}\n"
            )),
            comment(&body),
        ];

        let (rounds, anomalies) = fold_rounds_from_comments(A, None, &comments);
        assert!(anomalies.is_empty(), "unexpected anomalies: {anomalies:?}");
        assert_eq!(rounds.len(), 2);

        let second = &rounds[1];
        assert_eq!(second.index, 2);
        assert_eq!(second.opened_at, C);
        assert_eq!(second.previous_approval, Some(B));
        assert_eq!(second.state, RawRoundState::Open);
        assert_eq!(second.checklist_name, Some("Code Review Checklist"));
        assert!(
            matches!(
                second.checklist,
                ChecklistSource::Comment {
                    comment_index: 1,
                    ..
                }
            ),
            "unexpected checklist source: {:?}",
            second.checklist
        );
        assert!(matches!(
            &second.opened,
            RawRoundOpen::NewRound {
                comment_index: 1,
                note: Some("Second pass after the refactor."),
                ..
            }
        ));

        // ...and the checklist we wrote is recoverable from the same comment.
        assert_eq!(
            checklist_from_round_comment(&body).as_deref(),
            Some("- [ ] item one\n- [ ] item two")
        );
    }

    /// The checklist template name is written as metadata and read back by the fold.
    #[test]
    fn checklist_name_round_trips_writer_to_fold_and_is_none_when_absent() {
        let comment = |body: &str| GitComment {
            body: body.to_string(),
            author_login: "tester".to_string(),
            created_at: chrono::Utc::now(),
            id: None,
            html_url: None,
            html: None,
        };
        let approval = format!("# QC Approval\n\n## Metadata\napproved qc commit: {B}\n");

        let mut round = new_round(None);
        round.checklist_name = Some("Stats Review Checklist".to_string());
        let body = round.generate_body(&MockGitHelpers);
        // The name *is* the checklist heading — there is no separate metadata line
        // that could drift out of step with it.
        assert!(body.contains("\n# Stats Review Checklist\n"), "{body}");
        assert!(!body.contains("checklist: "), "{body}");
        // The written value is recoverable both by the fold and by the reader used
        // when seeding the next round.
        assert_eq!(
            checklist_name_from_round_comment(&body).as_deref(),
            Some("Stats Review Checklist")
        );
        let comments = vec![comment(&approval), comment(&body)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);
        assert_eq!(rounds[1].checklist_name, Some("Stats Review Checklist"));

        // No name written ⇒ no metadata line, and the round folds with `None`.
        round.checklist_name = None;
        let body = round.generate_body(&MockGitHelpers);
        assert!(!body.contains("checklist: "));
        assert_eq!(checklist_name_from_round_comment(&body), None);
        let comments = vec![comment(&approval), comment(&body)];
        let (rounds, _) = fold_rounds_from_comments(A, None, &comments);
        assert_eq!(rounds[1].checklist_name, None);

        // Initial QC's name comes from the issue body instead.
        let (rounds, _) = fold_rounds_from_comments(A, Some("Code Review Checklist"), &[]);
        assert_eq!(rounds[0].checklist_name, Some("Code Review Checklist"));
    }

    /// A heading is one line by definition, so a multi-line name is flattened rather
    /// than breaking the heading in two.
    #[test]
    fn multi_line_checklist_name_is_flattened_onto_one_heading_line() {
        let mut round = new_round(None);
        round.checklist_name = Some("first line\nsecond line".to_string());
        let body = round.generate_body(&MockGitHelpers);
        assert!(body.contains("\n# first line second line\n"), "{body}");
        assert_eq!(
            checklist_name_from_round_comment(&body).as_deref(),
            Some("first line second line")
        );
    }

    // ── reset_checklist ──────────────────────────────────────────────────────

    #[test]
    fn reset_checklist_unchecks_both_cases() {
        assert_eq!(
            reset_checklist("- [x] one\n- [X] two\n- [ ] three\n"),
            "- [ ] one\n- [ ] two\n- [ ] three\n"
        );
    }

    #[test]
    fn reset_checklist_preserves_indentation_nesting_and_spacing() {
        let input = "### Set-up\n\n- [x] top\n  - [X] nested\n\t- [x] tabbed\n-   [x] loose\n- [ ] already\n";
        let expected = "### Set-up\n\n- [ ] top\n  - [ ] nested\n\t- [ ] tabbed\n-   [ ] loose\n- [ ] already\n";
        assert_eq!(reset_checklist(input), expected);
    }

    #[test]
    fn reset_checklist_leaves_prose_checkboxes_alone() {
        let input = "- [ ] mark the box as [x] when done\nsee - [x] in the docs\n";
        assert_eq!(reset_checklist(input), input);
    }

    #[test]
    fn reset_checklist_is_idempotent_and_handles_empty_input() {
        assert_eq!(reset_checklist(""), "");
        let once = reset_checklist("- [x] a\n");
        assert_eq!(reset_checklist(&once), once);
    }

    // ── Checklist extraction ─────────────────────────────────────────────────

    #[test]
    fn checklist_from_round_comment_reads_the_section() {
        let body = new_round(None).generate_body(&MockGitHelpers);
        assert_eq!(
            checklist_from_round_comment(&body).as_deref(),
            Some("- [ ] item one\n- [ ] item two")
        );
    }

    /// The checklist runs to the end of the body, so a later `## ` section is part
    /// of it. That is the deliberate trade-off for not truncating checklists at
    /// their own section titles — see [`checklist_from_round_comment`].
    #[test]
    fn checklist_from_round_comment_runs_to_the_end_of_the_body() {
        let body = "# QC Round\n\n## Metadata\n* round: 2\n\n## Checklist\n- [ ] one\n### Sub\n- [x] two\n\n## Something Else\n- [ ] also mine\n";
        assert_eq!(
            checklist_from_round_comment(body).as_deref(),
            Some("- [ ] one\n### Sub\n- [x] two\n\n## Something Else\n- [ ] also mine")
        );
    }

    #[test]
    fn checklist_from_round_comment_is_none_when_absent_or_empty() {
        assert_eq!(
            checklist_from_round_comment("# QC Round\n\n## Metadata\n* round: 2\n"),
            None
        );
        // Heading present but nothing below it.
        assert_eq!(
            checklist_from_round_comment(
                "# QC Round\n\n## Metadata\n* x: 1\n\n## Checklist\n\n   \n"
            ),
            None
        );
    }

    #[test]
    fn checklist_from_issue_body_reads_from_the_first_level_one_heading() {
        let body = "## Metadata\n* initial qc commit: abc1234\n\n## Relevant Files\n\n### Previous QC\n- [file](url)\n\n# Code Review Checklist\n\nNOTE\n\n- [ ] compiles\n- [x] tests pass\n";
        assert_eq!(
            checklist_from_issue_body(body),
            Some(SeededChecklist {
                content: "NOTE\n\n- [ ] compiles\n- [x] tests pass".to_string(),
                name: Some("Code Review Checklist".to_string()),
            })
        );
    }

    #[test]
    fn checklist_from_issue_body_concatenates_multiple_sections_in_order() {
        let body = "## Metadata\n* a: 1\n\n# First\n- [ ] one\n\n# Second\n- [x] two\n";
        // The first section's heading names the seeded checklist.
        assert_eq!(
            checklist_from_issue_body(body),
            Some(SeededChecklist {
                content: "- [ ] one\n\n- [x] two".to_string(),
                name: Some("First".to_string()),
            })
        );
    }

    #[test]
    fn checklist_from_issue_body_is_none_without_a_level_one_section() {
        assert_eq!(
            checklist_from_issue_body("## Metadata\n* a: 1\n\n## Relevant Files\n- none\n"),
            None
        );
        // A heading with no content is not a checklist.
        assert_eq!(checklist_from_issue_body("# Checklist\n\n"), None);
    }

    // ── seed_checklist ───────────────────────────────────────────────────────

    #[test]
    /// A real checklist carries `## `-level section titles. Reading the round
    /// comment's `## Checklist` section only as far as the next `## ` truncated it
    /// to the couple of lines before the first title, and because every later round
    /// seeds from the previous one, the stump was inherited forever.
    #[test]
    fn round_comment_checklist_survives_its_own_level_two_headings() {
        let body = "\
# QC Round

## Metadata
* round: 2
* initial qc round commit: f564644
* previous approved commit: 13dc55a

# PK NONMEM Model Diagnostics

**Clockify code**: [INSERT]

**QC review due date**: [INSERT]

## Rendering Instructions and Other Comments

[INSERT]

## Technical Review

### General

- [x] Script renders free of error within the Rproject space
- [ ] Relative paths are used for all files

## Parameter Table

- [X] Condition number < 1,000
";

        let extracted = checklist_from_round_comment(body).expect("checklist section");

        // Everything after the heading, including the later `## ` sections.
        assert!(extracted.contains("**Clockify code**: [INSERT]"));
        assert!(extracted.contains("## Rendering Instructions and Other Comments"));
        assert!(extracted.contains("## Technical Review"));
        assert!(extracted.contains("### General"));
        assert!(extracted.contains("Script renders free of error"));
        assert!(extracted.contains("## Parameter Table"));
        assert!(extracted.contains("Condition number < 1,000"));
        // And nothing from above the heading leaks in.
        assert!(!extracted.contains("## Metadata"));
        assert!(!extracted.contains("round commit:"));
        assert!(!extracted.contains("# QC Round"));

        // Seeding round 3 from it keeps the whole thing and resets every box.
        let seeded = seed_checklist(Some(body), None).expect("seeded checklist");
        assert_eq!(seeded.name.as_deref(), Some("PK NONMEM Model Diagnostics"));
        assert!(seeded.content.contains("## Technical Review"));
        assert!(
            seeded
                .content
                .contains("- [ ] Script renders free of error")
        );
        assert!(seeded.content.contains("- [ ] Condition number < 1,000"));
        assert!(!seeded.content.contains("[x]"));
        assert!(!seeded.content.contains("[X]"));
    }

    #[test]
    fn seed_checklist_prefers_the_prior_round_comment_and_resets_boxes() {
        let prior =
            "# QC Round\n\n## Metadata\n* round: 2\n\n## Checklist\n- [x] one\n  - [X] nested\n";
        let issue = "## Metadata\n* a: 1\n\n# Checklist\n- [x] from the issue body\n";
        assert_eq!(
            seed_checklist(Some(prior), Some(issue)),
            Some(SeededChecklist {
                content: "- [ ] one\n  - [ ] nested".to_string(),
                // The prior comment recorded no `checklist:` metadata.
                name: None,
            })
        );

        // ...and when it names its checklist, that name is carried over.
        let named = prior.replace("## Checklist", "# Stats Review");
        assert_eq!(
            seed_checklist(Some(&named), Some(issue)),
            Some(SeededChecklist {
                content: "- [ ] one\n  - [ ] nested".to_string(),
                name: Some("Stats Review".to_string()),
            })
        );
    }

    #[test]
    fn seed_checklist_falls_back_to_the_issue_body() {
        let issue = "## Metadata\n* a: 1\n\n# Checklist\n- [x] from the issue body\n";
        assert_eq!(
            seed_checklist(None, Some(issue)),
            Some(SeededChecklist {
                content: "- [ ] from the issue body".to_string(),
                name: Some("Checklist".to_string()),
            })
        );
        // A prior comment without a checklist section falls back too.
        assert_eq!(
            seed_checklist(Some("# QC Round\n\n## Metadata\n* round: 2\n"), Some(issue))
                .map(|seeded| seeded.content),
            Some("- [ ] from the issue body".to_string())
        );
        assert_eq!(seed_checklist(None, None), None);
    }

    // ── Round marker ─────────────────────────────────────────────────────────

    fn marker() -> RoundMarker {
        RoundMarker {
            round: 2,
            comment_url: "https://github.com/org/repo/issues/41#issuecomment-123456".to_string(),
        }
    }

    const ISSUE_BODY: &str = "## Metadata\n* initial qc commit: abc1234\n* git branch: main\n\n## Relevant Files\n\n### Previous QC\n- [file](url)\n\n# Code Review Checklist\n\n- [ ] compiles\n";

    #[test]
    fn upsert_round_marker_inserts_before_the_checklist() {
        let out = upsert_round_marker(ISSUE_BODY, &marker());

        let marker_at = out.find(ROUND_MARKER_HEADING).unwrap();
        let checklist_at = out.find("# Code Review Checklist").unwrap();
        let relevant_at = out.find("## Relevant Files").unwrap();
        assert!(relevant_at < marker_at && marker_at < checklist_at);

        assert!(out.contains("* current round: 2\n"));
        assert!(out.contains(
            "* round comment: https://github.com/org/repo/issues/41#issuecomment-123456\n"
        ));
        assert!(out.contains(STRANDED_CHECKLIST_NOTE));
        // Nothing else in the body was disturbed.
        assert!(out.starts_with("## Metadata\n* initial qc commit: abc1234\n* git branch: main\n"));
        assert!(out.ends_with("# Code Review Checklist\n\n- [ ] compiles\n"));
        assert!(out.contains("### Previous QC\n- [file](url)"));
    }

    #[test]
    fn upsert_round_marker_replaces_in_place_and_is_idempotent() {
        let once = upsert_round_marker(ISSUE_BODY, &marker());
        let twice = upsert_round_marker(&once, &marker());
        assert_eq!(once, twice, "upsert must be idempotent");
        assert_eq!(twice.matches(ROUND_MARKER_HEADING).count(), 1);

        let updated = upsert_round_marker(
            &once,
            &RoundMarker {
                round: 3,
                comment_url: "https://example.com/c/9".to_string(),
            },
        );
        assert_eq!(updated.matches(ROUND_MARKER_HEADING).count(), 1);
        assert!(updated.contains("* current round: 3\n"));
        assert!(!updated.contains("issuecomment-123456"));
        // The block stayed put; everything around it is unchanged.
        assert_eq!(
            updated.find(ROUND_MARKER_HEADING),
            once.find(ROUND_MARKER_HEADING)
        );
        assert!(updated.ends_with("# Code Review Checklist\n\n- [ ] compiles\n"));
        assert!(updated.starts_with("## Metadata\n* initial qc commit: abc1234\n"));
    }

    #[test]
    fn upsert_round_marker_appends_when_there_is_no_checklist_heading() {
        let body = "## Metadata\n* initial qc commit: abc1234\n";
        let once = upsert_round_marker(body, &marker());
        assert!(once.starts_with(body.trim_end()));
        assert!(once.trim_end().ends_with(STRANDED_CHECKLIST_NOTE));
        assert_eq!(upsert_round_marker(&once, &marker()), once);
    }

    #[test]
    fn parse_round_marker_round_trips_and_is_none_without_a_block() {
        let out = upsert_round_marker(ISSUE_BODY, &marker());
        assert_eq!(parse_round_marker(&out), Some(marker()));
        assert_eq!(parse_round_marker(ISSUE_BODY), None);
        // Present but unparsable round number.
        assert_eq!(
            parse_round_marker("## QC Round\n* current round: two\n* round comment: url\n"),
            None
        );
        // Present heading, missing URL.
        assert_eq!(
            parse_round_marker("## QC Round\n* current round: 2\n"),
            None
        );
    }

    #[test]
    fn round_marker_does_not_capture_the_following_checklist() {
        let out = upsert_round_marker(ISSUE_BODY, &marker());
        let (start, end) = round_marker_range(&out).unwrap();
        assert!(!out[start..end].contains("- [ ] compiles"));
    }
}
