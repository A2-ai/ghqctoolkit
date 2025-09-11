use std::path::PathBuf;

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::git::GitHelpers;

pub struct QCApprove {
    pub(crate) file: PathBuf,
    pub(crate) commit: ObjectId,
    pub(crate) issue: Issue,
    pub(crate) note: Option<String>,
}

impl QCApprove {
    pub fn body(&self, git_info: &impl GitHelpers) -> String {
        let short_sha = &self.commit.to_string()[..7];
        let metadata = vec![
            "## Metadata".to_string(),
            format!("approved qc commit: {}", self.commit),
            format!(
                "[file contents at approved qc commit]({})",
                git_info.file_content_url(short_sha, &self.file)
            ),
        ];

        let mut body = vec!["# QC Approved".to_string()];

        if let Some(note) = &self.note {
            body.push(note.clone());
        }

        body.push(metadata.join("\n* "));
        body.join("\n\n")
    }
}

pub struct QCUnapprove {
    pub(crate) issue: Issue,
    pub(crate) reason: String,
}

impl QCUnapprove {
    pub fn body(&self) -> String {
        vec!["# QC Un-Approval", &self.reason].join("\n")
    }
}
