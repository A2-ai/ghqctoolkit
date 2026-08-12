//! Writing a new QC round, and the pure text helpers that seed its checklist.
//!
//! [`crate::round`] *reads* a `# QC New Round` comment; this module *writes* one.
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
    CHECKLIST_NAME_KEY, ChecklistSource, NEW_ROUND_MARKER, NOTE_KEY, PREVIOUS_APPROVED_COMMIT_KEY,
    ROUND_COMMIT_KEY, ROUND_KEY, metadata_value,
};

/// Heading of the checklist section of a `# QC New Round` comment.
///
/// Two hashes is load-bearing: [`crate::round`]'s metadata section ends at the
/// next line starting with `## `, so a single-`#` heading would leave checklist
/// lines inside the metadata slice.
pub const CHECKLIST_HEADING: &str = "## Checklist";

/// Heading of the denormalised round cache block in an issue body.
pub const ROUND_MARKER_HEADING: &str = "## QC Round";
/// Round-marker key holding the current round number.
const CURRENT_ROUND_KEY: &str = "current round: ";
/// Round-marker key holding the URL of the round's `# QC New Round` comment.
const ROUND_COMMENT_KEY: &str = "round comment: ";
/// Preamble explaining that the checklist below the marker is the *initial* one.
const STRANDED_CHECKLIST_NOTE: &str = "> Checklist below is from Initial QC. The current round's checklist is in the round comment above.";

/// A `# QC New Round` comment: it opens a new round and carries that round's
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
    fn title(&self) -> &str {
        "QC New Round"
    }

    /// Marker, `## Metadata`, then `## Checklist` — nothing else.
    ///
    /// SHAs are written in full: the fold accepts abbreviations, but a full SHA
    /// can never become ambiguous.
    fn generate_body(&self, _git_info: &(impl GitHelpers + GitFileOps)) -> String {
        let mut metadata = vec![
            "## Metadata".to_string(),
            format!("{ROUND_KEY}{}", self.round),
            format!("{ROUND_COMMIT_KEY}{}", self.round_commit),
            format!(
                "{PREVIOUS_APPROVED_COMMIT_KEY}{}",
                self.previous_approved_commit
            ),
        ];
        if let Some(name) = &self.checklist_name {
            // One metadata line, like the note: the fold reads the remainder of the
            // line as the value.
            let name = flatten_to_one_line(name);
            if !name.is_empty() {
                metadata.push(format!("{CHECKLIST_NAME_KEY}{name}"));
            }
        }
        if let Some(note) = &self.note {
            // The fold reads a note as the remainder of a single metadata line,
            // so a multi-line note is flattened rather than silently truncated.
            let note = flatten_to_one_line(note);
            if !note.is_empty() {
                metadata.push(format!("{NOTE_KEY}{note}"));
            }
        }

        let mut body = vec![NEW_ROUND_MARKER.to_string(), metadata.join("\n* ")];

        let checklist = self.checklist_content.trim();
        if !checklist.is_empty() {
            body.push(format!("{CHECKLIST_HEADING}\n{checklist}"));
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

/// The body of `section_heading`'s section: everything after a line equal to it
/// (once trimmed) up to the next line starting with `## `, or the end of `body`.
///
/// `None` when the heading is absent or its section holds nothing but whitespace.
/// `###`-and-deeper subheadings — which is how nested checklists render — do not
/// terminate a section.
fn section_body(body: &str, section_heading: &str) -> Option<String> {
    let mut collected: Option<String> = None;
    for line in body.lines() {
        match &mut collected {
            None => {
                if line.trim() == section_heading {
                    collected = Some(String::new());
                }
            }
            Some(buffer) => {
                if line.starts_with("## ") {
                    break;
                }
                buffer.push_str(line);
                buffer.push('\n');
            }
        }
    }
    let collected = collected?;
    let trimmed = collected.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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

/// Extract the `## Checklist` section of a `# QC New Round` comment.
pub fn checklist_from_round_comment(body: &str) -> Option<String> {
    section_body(body, CHECKLIST_HEADING)
}

/// The checklist template name a `# QC New Round` comment recorded, if any.
///
/// Read with [`crate::round`]'s own metadata parser, so writer and reader cannot
/// drift apart.
pub fn checklist_name_from_round_comment(body: &str) -> Option<String> {
    metadata_value(body, CHECKLIST_NAME_KEY).map(|name| name.to_string())
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
/// the comment's own `# QC New Round` heading. The *first* such heading's text is
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
    fn body_has_no_mention_no_diff_and_a_level_two_checklist_heading() {
        let round = new_round(Some("note"));
        let body = round.generate_body(&MockGitHelpers);

        assert!(
            !body.contains('@'),
            "body must not @-mention anyone: {body}"
        );
        assert!(!body.contains("## File Difference"));
        assert!(!body.contains("commit comparison"));
        assert!(body.contains("\n## Checklist\n"));
        assert!(!body.contains("\n# Checklist"));
        // Full SHAs, never abbreviated.
        assert!(body.contains(C));
        assert!(body.contains(B));
        assert_eq!(round.title(), "QC New Round");
        assert_eq!(round.issue().number, round.issue.number);
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
        assert!(body.contains("* checklist: Stats Review Checklist\n"));
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

    #[test]
    fn multi_line_checklist_name_is_flattened_onto_one_metadata_line() {
        let mut round = new_round(None);
        round.checklist_name = Some("first line\nsecond line".to_string());
        let body = round.generate_body(&MockGitHelpers);
        assert!(body.contains("* checklist: first line second line\n"));
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

    #[test]
    fn checklist_from_round_comment_stops_at_the_next_level_two_heading() {
        let body = "# QC New Round\n\n## Metadata\n* round: 2\n\n## Checklist\n- [ ] one\n### Sub\n- [x] two\n\n## Something Else\n- [ ] not mine\n";
        assert_eq!(
            checklist_from_round_comment(body).as_deref(),
            Some("- [ ] one\n### Sub\n- [x] two")
        );
    }

    #[test]
    fn checklist_from_round_comment_is_none_when_absent_or_empty() {
        assert_eq!(
            checklist_from_round_comment("# QC New Round\n\n## Metadata\n* round: 2\n"),
            None
        );
        assert_eq!(
            checklist_from_round_comment("# QC New Round\n\n## Checklist\n\n## Metadata\n* x: 1\n"),
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
    fn seed_checklist_prefers_the_prior_round_comment_and_resets_boxes() {
        let prior = "# QC New Round\n\n## Metadata\n* round: 2\n\n## Checklist\n- [x] one\n  - [X] nested\n";
        let issue = "## Metadata\n* a: 1\n\n# Checklist\n- [x] from the issue body\n";
        assert_eq!(
            seed_checklist(Some(prior), Some(issue)),
            Some(SeededChecklist {
                content: "- [ ] one\n  - [ ] nested".to_string(),
                // The prior comment recorded no `checklist:` metadata.
                name: None,
            })
        );

        // ...and when it does record one, that name is carried over.
        let named = format!("{prior}\n");
        let named = named.replace("* round: 2", "* round: 2\n* checklist: Stats Review");
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
            seed_checklist(
                Some("# QC New Round\n\n## Metadata\n* round: 2\n"),
                Some(issue)
            )
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
