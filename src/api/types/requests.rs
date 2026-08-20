//! API request types.

use std::path::PathBuf;

use serde::Deserialize;

use crate::{
    Checklist, QCEntry, QCRelationship, RelevantFile, RelevantFileClass, RelevantFileEntry,
    create::normalize_collaborator_entries,
};

/// Request to create a new milestone.
#[derive(Debug, Deserialize)]
pub struct CreateMilestoneRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Whether the referenced issue is created or to be created
#[derive(Debug, Deserialize)]
pub enum RelevantIssueClass {
    Exists {
        issue_number: u64,
        issue_id: Option<u64>,
    },
    New(PathBuf),
}

/// Reference to a related QC issue.
#[derive(Debug, Deserialize)]
pub struct RelevantIssue {
    pub file_name: PathBuf,
    pub issue_class: RelevantIssueClass,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub include_diff: bool,
}

impl RelevantIssue {
    fn to_entry(&self, relation: QCRelationship) -> RelevantFileEntry {
        match &self.issue_class {
            RelevantIssueClass::Exists {
                issue_number,
                issue_id,
            } => RelevantFileEntry::ExistingIssue(RelevantFile {
                file_name: self.file_name.clone(),
                class: match relation {
                    QCRelationship::GatingQC => RelevantFileClass::GatingQC {
                        issue_number: *issue_number,
                        issue_id: *issue_id,
                        description: self.description.clone(),
                    },
                    QCRelationship::PreviousQC => RelevantFileClass::PreviousQC {
                        issue_number: *issue_number,
                        issue_id: *issue_id,
                        description: self.description.clone(),
                        include_diff: self.include_diff,
                    },
                    QCRelationship::RelevantQC => RelevantFileClass::RelevantQC {
                        issue_number: *issue_number,
                        description: self.description.clone(),
                    },
                },
            }),
            RelevantIssueClass::New(path) => RelevantFileEntry::NewIssue {
                file_path: path.clone(),
                relationship: relation,
                description: self.description.clone(),
                include_diff: self.include_diff,
            },
        }
    }
}

/// Reference to a relevant file.
#[derive(Debug, Deserialize)]
pub struct RelevantFileInput {
    pub file_path: String,
    pub justification: String,
}

/// Request to create a new QC issue.
#[derive(Debug, Deserialize)]
pub struct CreateIssueRequest {
    pub file: String,
    pub checklist_name: String,
    pub checklist_content: String,
    #[serde(default)]
    pub assignees: Vec<String>,
    #[serde(default)]
    pub collaborators: Option<Vec<String>>,
    #[serde(default)]
    pub previous_qc: Vec<RelevantIssue>,
    #[serde(default)]
    pub gating_qc: Vec<RelevantIssue>,
    #[serde(default)]
    pub relevant_qc: Vec<RelevantIssue>,
    #[serde(default)]
    pub relevant_files: Vec<RelevantFileInput>,
}

impl TryFrom<CreateIssueRequest> for QCEntry {
    type Error = String;

    fn try_from(request: CreateIssueRequest) -> Result<Self, Self::Error> {
        let relevant_files = [
            request
                .relevant_files
                .into_iter()
                .map(|f| RelevantFileEntry::File {
                    file_path: PathBuf::from(&f.file_path),
                    justification: f.justification,
                })
                .collect::<Vec<_>>(),
            request
                .previous_qc
                .into_iter()
                .map(|q| q.to_entry(QCRelationship::PreviousQC))
                .collect::<Vec<_>>(),
            request
                .gating_qc
                .into_iter()
                .map(|q| q.to_entry(QCRelationship::GatingQC))
                .collect::<Vec<_>>(),
            request
                .relevant_qc
                .into_iter()
                .map(|q| q.to_entry(QCRelationship::RelevantQC))
                .collect::<Vec<_>>(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        let collaborators = request
            .collaborators
            .map(|entries| normalize_collaborator_entries(&entries))
            .transpose()?;

        Ok(Self {
            title: PathBuf::from(&request.file),
            checklist: Checklist {
                name: request.checklist_name,
                content: request.checklist_content,
            },
            assignees: request.assignees,
            collaborators,
            relevant_files,
        })
    }
}

/// Request to create a commit-to-commit comment.
#[derive(Debug, Deserialize)]
pub struct CreateCommentRequest {
    pub current_commit: String,
    #[serde(default)]
    pub previous_commit: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default = "default_true")]
    pub include_diff: bool,
}

/// Request to approve and close an issue.
#[derive(Debug, Deserialize)]
pub struct ApproveRequest {
    pub commit: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// Query parameters for approve endpoint.
#[derive(Debug, Deserialize)]
pub struct ApproveQuery {
    #[serde(default)]
    pub force: bool,
}

/// Request to unapprove and reopen an issue.
#[derive(Debug, Deserialize)]
pub struct UnapproveRequest {
    pub reason: String,
}

/// Whether — and how loudly — reviewers are notified about a new round.
/// Mirrors [`crate::NotificationMode`].
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationModeRequest {
    /// Post a `# QC Notification` with the inline file diff.
    #[default]
    Full,
    /// Post a `# QC Notification` with metadata only, no inline diff.
    MetadataOnly,
    /// Post no notification comment at all.
    None,
}

impl From<NotificationModeRequest> for crate::NotificationMode {
    fn from(mode: NotificationModeRequest) -> Self {
        match mode {
            NotificationModeRequest::Full => crate::NotificationMode::Full,
            NotificationModeRequest::MetadataOnly => crate::NotificationMode::MetadataOnly,
            NotificationModeRequest::None => crate::NotificationMode::None,
        }
    }
}

/// Request to start a new QC round on an issue.
///
/// The anchor is deliberately absent: it is always HEAD of the issue's branch at
/// open time, read from the repository rather than accepted from the caller.
#[derive(Debug, Deserialize)]
pub struct StartRoundApiRequest {
    /// Author-edited checklist markdown for the new round.
    pub checklist_content: String,
    /// Name of the checklist template the content came from, for the audit record.
    #[serde(default)]
    pub checklist_name: Option<String>,
    /// Why the round is being opened, recorded on the `# QC Round` comment.
    #[serde(default)]
    pub note: Option<String>,
    /// Context for the reviewer, carried by the `# QC Notification` comment only.
    #[serde(default)]
    pub notification_note: Option<String>,
    #[serde(default)]
    pub notification: NotificationModeRequest,
}

/// Request to repair the follow-up steps of an issue's currently open round.
///
/// Nothing else is accepted: which steps are incomplete is derived from the issue
/// and its derived round, never taken from the caller. The notification is the one
/// exception, and it defaults to *not* notifying — a round opened with
/// `notification: none` is in exactly the state its author chose, so a reviewer is
/// only pinged when a caller explicitly asks.
// Deliberately no `Default`: `NotificationModeRequest`'s own default is `Full`, so
// a derived `Default` here would silently mean "notify" — the opposite of the rule
// above. An absent field goes through `no_notification` instead.
#[derive(Debug, Deserialize)]
pub struct RepairRoundApiRequest {
    #[serde(default = "no_notification")]
    pub notification: NotificationModeRequest,
    /// Context for the reviewer. Absent falls back to the round's own note — see
    /// [`crate::RepairRoundRequest::notification_note`].
    #[serde(default)]
    pub notification_note: Option<String>,
}

fn no_notification() -> NotificationModeRequest {
    NotificationModeRequest::None
}

/// Request to post a working directory review.
#[derive(Debug, Deserialize)]
pub struct ReviewRequest {
    pub commit: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default = "default_true")]
    pub include_diff: bool,
    #[serde(default = "default_true")]
    pub auto_stash: bool,
}

/// Request to preview a Previous QC diff comment during issue creation.
#[derive(Debug, Deserialize)]
pub struct PreviousQCDiffPreviewRequest {
    pub current_file: String,
    pub previous_file: String,
    pub previous_issue_number: u64,
    pub current_commit: String,
}

fn default_true() -> bool {
    true
}

/// A single file entry for archive generation.
///
/// The mode is **declared**, never inferred from which optional fields happen to be
/// present. The flat struct this replaces carried `repository_file`, `commit`,
/// `milestone` and `approved` as four options with sixteen combinations, of which two
/// were legal — so a request carrying both a round selection and a hand-picked commit
/// was representable, and the handler had to reject the illegal pairs by hand. An
/// `untagged` enum would be worse still: it deserializes such a mixed request
/// successfully and silently discards the commit, which is exactly the "two sources of
/// truth for one fact, neither checked against the other" defect the round rework
/// exists to remove. Internally tagged plus `deny_unknown_fields` makes the invalid
/// shape unconstructible and names the offending field on rejection.
///
/// `approved` is gone and does not come back: the bit was unfalsifiable after the first
/// approval and the two producers set it from different predicates. The only approval
/// statement on this boundary is the `RoundProvenance.approval` the **server** writes
/// into the archive metadata.
#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveFileRequest {
    /// A file under QC in a milestone. The issue number is the only handle: the backend
    /// reads the thread and derives the path, the milestone, the commit, the approval
    /// and whether the bytes were superseded from it.
    Issue {
        issue_number: u64,
        /// The round this selection addresses, 1-based; `1` is Initial QC. `None`
        /// targets the latest round — which for a reopened file is unapproved content,
        /// intentionally and ungated.
        ///
        /// Callers should always send the key, as `null`, so there is one encoding of
        /// "latest" on the wire; `default` is here for `curl` and script callers, the
        /// server being the more permissive of the two.
        #[serde(default)]
        round: Option<u32>,
    },
    /// A file in no milestone. Unchanged: the user picks the commit directly, and the
    /// archive makes no QC claim about it.
    File {
        repository_file: PathBuf,
        commit: String,
    },
}

/// Request to generate an archive.
#[derive(Debug, Deserialize)]
pub struct ArchiveGenerateRequest {
    pub output_path: String,
    pub flatten: bool,
    pub files: Vec<ArchiveFileRequest>,
}

#[derive(serde::Deserialize)]
pub struct SetupConfigurationRequest {
    pub url: String,
}

/// Position of a context PDF relative to the QC Record.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordContextPosition {
    Prepend,
    Append,
}

/// A single context PDF file for record generation.
#[derive(Debug, serde::Deserialize)]
pub struct RecordContextFileRequest {
    /// Absolute path on the server (or uploaded temp path).
    pub server_path: String,
    pub position: RecordContextPosition,
}

/// Request body for record preview and generation.
#[derive(Debug, serde::Deserialize)]
pub struct RecordRequest {
    pub milestone_numbers: Vec<u64>,
    #[serde(default)]
    pub tables_only: bool,
    /// Output path — used only for generate, ignored for preview.
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub context_files: Vec<RecordContextFileRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parsed from the bytes a client sends, not from a `serde_json::Value`: key order
    /// is part of what `deny_unknown_fields` reports, and a `Value` re-sorts it.
    fn parse(json: &str) -> Result<ArchiveFileRequest, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The default target is sent as an explicit `null`, so the UI has exactly one
    /// encoding of "the latest round".
    #[test]
    fn a_mode_one_entry_with_a_null_round_targets_the_latest_round() {
        let request = parse(r#"{"mode": "issue", "issue_number": 42, "round": null}"#)
            .expect("the pinned mode-1 shape deserializes");

        assert!(matches!(
            request,
            ArchiveFileRequest::Issue {
                issue_number: 42,
                round: None
            }
        ));
    }

    /// An absent key is tolerated for `curl` and script callers — the server is the more
    /// permissive of the two, which is safe in that direction only.
    #[test]
    fn an_absent_round_key_is_also_the_latest_round() {
        let request =
            parse(r#"{"mode": "issue", "issue_number": 42}"#).expect("`round` carries a default");

        assert!(matches!(
            request,
            ArchiveFileRequest::Issue { round: None, .. }
        ));
    }

    /// A retargeted selection: the round the selection addresses, which is not a claim
    /// that the round was approved.
    #[test]
    fn a_mode_one_entry_can_address_an_older_round() {
        let request = parse(r#"{"mode": "issue", "issue_number": 43, "round": 1}"#)
            .expect("an explicit round deserializes");

        assert!(matches!(
            request,
            ArchiveFileRequest::Issue {
                issue_number: 43,
                round: Some(1)
            }
        ));
    }

    #[test]
    fn a_mode_two_entry_carries_a_path_and_a_commit() {
        let request = parse(
            r#"{"mode": "file",
                "repository_file": "scripts/helpers.R",
                "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}"#,
        )
        .expect("the pinned mode-2 shape deserializes");

        let ArchiveFileRequest::File {
            repository_file,
            commit,
        } = request
        else {
            panic!("`mode: file` is mode 2");
        };
        assert_eq!(repository_file, PathBuf::from("scripts/helpers.R"));
        assert_eq!(commit, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    }

    /// The whole reason for the tagging choice. An `untagged` enum deserializes this
    /// mixed request as a mode-1 entry and **silently discards the commit** — two claims
    /// in, one acted on, nothing reported. Internally tagged plus `deny_unknown_fields`
    /// rejects it and names the field that does not belong to the declared mode.
    #[test]
    fn a_mixed_request_is_rejected_and_names_the_offending_field() {
        let error = parse(
            r#"{"mode": "issue", "issue_number": 7, "round": null,
                "repository_file": "a.R", "commit": "abc"}"#,
        )
        .expect_err("a request carrying both a round and a commit is not honoured");

        let message = error.to_string();
        assert!(
            message.contains("unknown field `repository_file`"),
            "the rejection must name the field: {message}"
        );
    }

    /// The mode is declared, never inferred: a body with no `mode` is not guessed at.
    #[test]
    fn an_entry_with_no_mode_is_rejected() {
        let error = parse(
            r#"{"repository_file": "a.R",
                "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}"#,
        )
        .expect_err("the tag is required");

        assert!(
            error.to_string().contains("mode"),
            "the rejection must name the tag: {error}"
        );
    }

    /// `approved` is gone from the wire, and a client still sending it is told so rather
    /// than having it quietly ignored. This is also the pre-existing live 400: every
    /// manually added file used to be sent as `{repository_file, commit, approved:
    /// false}`, which the old handler rejected because `milestone` was absent.
    #[test]
    fn the_old_shape_carrying_approved_no_longer_deserializes() {
        let error = parse(
            r#"{"mode": "file",
                "repository_file": "scripts/helpers.R",
                "commit": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "approved": false}"#,
        )
        .expect_err("no request field asserts approval any more");

        assert!(
            error.to_string().contains("unknown field `approved`"),
            "the rejection must name the deleted field: {error}"
        );
    }

    /// A mode-1 entry may not smuggle a milestone, path or commit in beside its round:
    /// each is derivable from the thread, and a client-sent copy is a second source of
    /// truth for a fact the server already holds.
    #[test]
    fn a_mode_one_entry_may_not_carry_a_client_supplied_milestone() {
        let error = parse(
            r#"{"mode": "issue", "issue_number": 42, "round": null,
                "milestone": "Milestone 3"}"#,
        )
        .expect_err("the milestone comes from the thread");

        assert!(
            error.to_string().contains("unknown field `milestone`"),
            "the rejection must name the deleted field: {error}"
        );
    }
}
