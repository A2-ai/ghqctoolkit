use std::path::{Path, PathBuf};

use crate::GitHelpers;

#[derive(Debug, Clone)]
pub enum RelevantFileClass {
    /// A QC that was done previously on this file or a closely related one (re-QC of an analysis)
    /// Must be approved before approving the current QC
    PreviousQC {
        issue_number: u64,
        /// GitHub internal issue ID (needed for creating blocking relationships)
        issue_id: Option<u64>,
        description: Option<String>,
    },
    /// A QC which the issue of interest is developed based on.
    /// Must be approved before approving the current QC
    GatingQC {
        issue_number: u64,
        /// GitHub internal issue ID (needed for creating blocking relationships)
        issue_id: Option<u64>,
        description: Option<String>,
    },
    /// A QC which provides previous context to the current QC but does not directly impact the analysis
    /// Approval status has no baring on the ability to approve the current QC
    RelevantQC {
        issue_number: u64,
        description: Option<String>,
    },
    /// A file which has no associated issue that is relevant to the current QC.
    /// A justification for the lack of QC is required
    File { justification: String },
}

#[derive(Debug, Clone)]
pub struct RelevantFile {
    pub(crate) file_name: PathBuf,
    pub(crate) class: RelevantFileClass,
}

pub(crate) fn relevant_files_section(
    relevant_files: &[RelevantFile],
    git_info: &impl GitHelpers,
) -> String {
    let mut previous = Vec::new();
    let mut gating_qc = Vec::new();
    let mut non_gating_qc = Vec::new();
    let mut rel_file = Vec::new();

    let make_issue_bullet = |issue_number: &u64, description: &Option<String>, file_name: &Path| {
        format!(
            "[{}]({}){}",
            file_name.display(),
            git_info.issue_url(*issue_number),
            description
                .as_ref()
                .map(|d| format!(" - {d}"))
                .unwrap_or_default()
        )
    };

    for file in relevant_files {
        match &file.class {
            RelevantFileClass::PreviousQC {
                issue_number,
                description,
                ..
            } => {
                previous.push(make_issue_bullet(
                    issue_number,
                    description,
                    &file.file_name,
                ));
            }
            RelevantFileClass::GatingQC {
                issue_number,
                description,
                ..
            } => {
                gating_qc.push(make_issue_bullet(
                    issue_number,
                    description,
                    &file.file_name,
                ));
            }
            RelevantFileClass::RelevantQC {
                issue_number,
                description,
                ..
            } => {
                non_gating_qc.push(make_issue_bullet(
                    issue_number,
                    description,
                    &file.file_name,
                ));
            }
            RelevantFileClass::File { justification } => {
                rel_file.push(format!(
                    "**{}** - {justification}",
                    file.file_name.display()
                ));
            }
        }
    }

    let mut res = vec!["## Relevant Files".to_string()];

    if !previous.is_empty() {
        res.push(format!("### Previous QC\n- {}", previous.join("\n- ")));
    }

    if !gating_qc.is_empty() {
        res.push(format!("### Gating QC\n- {}", gating_qc.join("\n- ")));
    }

    if !non_gating_qc.is_empty() {
        res.push(format!("### Relevant QC\n- {}", non_gating_qc.join("\n- ")));
    }

    if !rel_file.is_empty() {
        res.push(format!("### Relevant File\n- {}", rel_file.join("\n- ")));
    }

    if res.len() > 1 {
        res.join("\n\n")
    } else {
        String::new()
    }
}
