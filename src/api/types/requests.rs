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
    Exists(u64),
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
            RelevantIssueClass::Exists(issue_number) => {
                RelevantFileEntry::ExistingIssue(RelevantFile {
                    file_name: self.file_name.clone(),
                    class: match relation {
                        QCRelationship::GatingQC => RelevantFileClass::GatingQC {
                            issue_number: *issue_number,
                            issue_id: None,
                            description: self.description.clone(),
                        },
                        QCRelationship::PreviousQC => RelevantFileClass::PreviousQC {
                            issue_number: *issue_number,
                            issue_id: None,
                            description: self.description.clone(),
                        },
                        QCRelationship::RelevantQC => RelevantFileClass::RelevantQC {
                            issue_number: *issue_number,
                            description: self.description.clone(),
                        },
                    },
                })
            }
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

impl Into<QCEntry> for CreateIssueRequest {
    fn into(self) -> QCEntry {
        let relevant_files = [
            self.previous_qc
                .into_iter()
                .map(|r| r.to_entry(QCRelationship::PreviousQC))
                .collect::<Vec<_>>(),
            self.gating_qc
                .into_iter()
                .map(|r| r.to_entry(QCRelationship::GatingQC))
                .collect::<Vec<_>>(),
            self.relevant_qc
                .into_iter()
                .map(|r| r.to_entry(QCRelationship::RelevantQC))
                .collect::<Vec<_>>(),
            self.relevant_files
                .into_iter()
                .map(|r| RelevantFileEntry::File {
                    file_path: PathBuf::from(&r.file_path),
                    justification: r.justification,
                })
                .collect::<Vec<_>>(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        QCEntry {
            title: PathBuf::from(&self.file),
            checklist: Checklist {
                name: self.checklist_name,
                note: None,
                content: self.checklist_content,
            },
            assignees: self.assignees,
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
