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

/// A round's checklist (A5). Nested, matching `configuration::Checklist`: `content`
/// excludes the `# {name}` heading line, which `Display` emits (D37).
#[derive(Debug, Deserialize)]
pub struct RoundChecklistRequest {
    pub name: String,
    pub content: String,
}

/// Request to start a new QC round (A5).
///
/// `start_commit` and `branch` both come from the caller's checkout, read-only —
/// there is no commit picker anywhere in the round flow (D23).
#[derive(Debug, Deserialize)]
pub struct CreateRoundRequest {
    pub start_commit: String,
    pub branch: String,
    pub checklist: RoundChecklistRequest,
    /// D57: notification defaults **ON**, matching the CLI's `!no_notify`. A plain
    /// `#[serde(default)]` here yielded `false`, which made the two sanctioned paths
    /// disagree about what "start a round" does.
    #[serde(default = "default_true")]
    pub notify: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default = "default_true")]
    pub include_diff: bool,
}

/// Request to preview a `# QC Round N` comment (D47).
///
/// Mirrors `CreateRoundRequest`'s round-comment half — nothing about the optional
/// notification, which has its own preview endpoint. The round index is **not** a field:
/// the server derives it exactly as `POST /rounds` does, so the preview cannot claim a
/// round number the creation would not use. Same reason the whole endpoint exists: the
/// `[file contents at initial qc commit](url)` line comes from `GitHelpers`, which the UI
/// cannot compute without guessing the host.
#[derive(Debug, Deserialize)]
pub struct PreviewRoundRequest {
    pub issue_number: u64,
    pub start_commit: String,
    pub branch: String,
    pub checklist: RoundChecklistRequest,
}

/// Request to preview the file diff a new round would be started over.
///
/// Carries only the *new* end of the comparison. The old end — the prior round's
/// approval — is derived server-side exactly as `create_round` derives the
/// notification's `previous commit` (D5): a client that could choose both ends could
/// show a diff for a transition that is not the one about to happen.
#[derive(Debug, Deserialize)]
pub struct PreviewRoundDiffRequest {
    pub issue_number: u64,
    pub start_commit: String,
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
    /// A6: the round the selected commit belongs to.
    #[serde(default)]
    pub round: Option<u32>,
    /// A6: whether file-changing commits exist after the selected commit (R11/R12).
    #[serde(default)]
    pub subsequent_file_changes: Option<bool>,
}

/// One file the client declares it is **omitting** from the archive, and why (D62).
///
/// Mirrors [`crate::archive::SkippedFile`]. `ArchiveFileRequest` carries an explicit
/// `commit` and no issue number, so the server cannot resolve a round for this path — the
/// skip decision is necessarily the client's and the server's job is to record it verbatim.
/// Same footing as `round`/`approved` (D28.3): only the client holds the round it selected.
#[derive(Debug, Deserialize)]
pub struct SkippedFileRequest {
    pub repository_file: PathBuf,
    /// The 1-based index of the round that was selected but could not be resolved.
    pub round: u32,
    pub branch: String,
    pub reason: String,
}

/// Request to generate an archive.
#[derive(Debug, Deserialize)]
pub struct ArchiveGenerateRequest {
    pub output_path: String,
    pub flatten: bool,
    pub files: Vec<ArchiveFileRequest>,
    /// D62: files the client is omitting, recorded verbatim in the archive manifest so a
    /// partial archive declares itself partial whichever interface produced it. `default`
    /// keeps existing clients working unchanged.
    #[serde(default)]
    pub skipped: Vec<SkippedFileRequest>,
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
