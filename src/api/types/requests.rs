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
#[derive(Debug, Deserialize)]
pub struct ArchiveFileRequest {
    pub repository_file: PathBuf,
    pub commit: String,
    pub milestone: Option<String>,
    pub approved: Option<bool>,
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
