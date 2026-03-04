//! API request types.

use std::path::PathBuf;

use serde::Deserialize;

use crate::{
    Checklist, QCEntry, QCRelationship, RelevantFile, RelevantFileClass, RelevantFileEntry,
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
    pub previous_qc: Vec<RelevantIssue>,
    #[serde(default)]
    pub gating_qc: Vec<RelevantIssue>,
    #[serde(default)]
    pub relevant_qc: Vec<RelevantIssue>,
    #[serde(default)]
    pub relevant_files: Vec<RelevantFileInput>,
}

impl From<CreateIssueRequest> for QCEntry {
    fn from(request: CreateIssueRequest) -> Self {
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

        Self {
            title: PathBuf::from(&request.file),
            checklist: Checklist {
                name: request.checklist_name,
                content: request.checklist_content,
            },
            assignees: request.assignees,
            relevant_files,
        }
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
