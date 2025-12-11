use std::path::{Path, PathBuf};

use crate::{
    configuration::Checklist,
    git::{GitAuthor, GitFileOps, GitFileOpsError, GitHelpers, GitRepository, GitRepositoryError},
    relevant_files::{RelevantFile, RelevantFileClass},
};

#[derive(Debug, thiserror::Error)]
pub enum QCIssueError {
    #[error(transparent)]
    GitRepositoryError(#[from] GitRepositoryError),
    #[error(transparent)]
    GitFileOpsError(#[from] GitFileOpsError),
}

#[derive(Debug, Clone)]
pub struct QCIssue {
    pub(crate) milestone_id: u64,
    pub title: PathBuf,
    commit: String,
    pub(crate) branch: String,
    authors: Vec<GitAuthor>,
    checklist: Checklist,
    pub(crate) assignees: Vec<String>,
    relevant_files: Vec<RelevantFile>,
}

impl QCIssue {
    pub(crate) fn body(&self, git_info: &impl GitHelpers) -> String {
        let author = self
            .authors
            .first()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        let mut metadata = vec![
            "## Metadata".to_string(),
            format!("initial qc commit: {}", self.commit),
            format!("git branch: {}", self.branch),
            format!("author: {author}"),
        ];

        if self.authors.len() > 1 {
            metadata.push(format!(
                "collaborators: {}",
                self.authors
                    .iter()
                    .skip(1)
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        metadata.push(format!(
            "[file contents at initial qc commit]({})",
            git_info.file_content_url(&self.commit[..7], &self.title)
        ));

        let mut body = vec![metadata.join("\n* ")];

        body.push(relevant_files_section(&self.relevant_files, git_info));

        body.push(self.checklist.to_string());

        body.join("\n\n")
    }

    pub(crate) fn title(&self) -> String {
        self.title.to_string_lossy().to_string()
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn new(
        file: impl AsRef<Path>,
        git_info: &(impl GitRepository + GitFileOps),
        milestone_id: u64,
        assignees: Vec<String>,
        checklist: Checklist,
        relevant_files: Vec<RelevantFile>,
    ) -> Result<Self, QCIssueError> {
        Ok(Self {
            title: file.as_ref().to_path_buf(),
            commit: git_info.commit()?,
            branch: git_info.branch()?,
            authors: git_info.authors(file.as_ref())?,
            checklist,
            assignees,
            milestone_id,
            relevant_files,
        })
    }
}

fn relevant_files_section(relevant_files: &[RelevantFile], git_info: &impl GitHelpers) -> String {
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
        res.push(format!("### Previous QC\n > Issues which are previous QCs of this file (or a similar file)\n- {}", previous.join("\n- ")));
    }

    if !gating_qc.is_empty() {
        res.push(format!("### Gating QC\n > Issues which must be approved before the approval of this issue \n- {}", gating_qc.join("\n- ")));
    }

    if !non_gating_qc.is_empty() {
        res.push(format!("### Relevant QC\n > Issues related to the file, but do not have a direct impact on results \n- {}", non_gating_qc.join("\n- ")));
    }

    if !rel_file.is_empty() {
        res.push(format!(
            "### Relevant File\n > Files relevant to the QC, but do not require QC \n- {}",
            rel_file.join("\n- ")
        ));
    }

    if res.len() > 1 {
        res.join("\n\n")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitAuthor, GitHelpers};
    use std::path::PathBuf;

    fn create_test_issue() -> QCIssue {
        use crate::configuration::Checklist;

        QCIssue {
            milestone_id: 1,
            title: PathBuf::from("src/example.rs"),
            commit: "abc123def456789".to_string(),
            branch: "feature/new-feature".to_string(),
            authors: vec![
                GitAuthor {
                    name: "John Doe".to_string(),
                    email: "john@example.com".to_string(),
                },
                GitAuthor {
                    name: "Jane Smith".to_string(),
                    email: "jane@example.com".to_string(),
                }
            ],
            checklist: Checklist::new(
                "Code Review Checklist".to_string(),
                Some("NOTE".to_string()),
                "- [ ] Code compiles without warnings\n- [ ] Tests pass\n- [ ] Documentation updated".to_string(),
            ),
            assignees: vec!["reviewer1".to_string(), "reviewer2".to_string()],
            relevant_files: vec![
                RelevantFile {
                    file_name: PathBuf::from("previous.R"),
                    class: RelevantFileClass::PreviousQC { issue_number: 1, description: Some("This file has been previously QCed".to_string()) },
                },
                RelevantFile {
                    file_name: PathBuf::from("gating.R"),
                    class: RelevantFileClass::GatingQC { issue_number: 2, description: Some("This file gates the approval of this QC".to_string()) }
                },
                RelevantFile {
                    file_name: PathBuf::from("related.R"),
                    class: RelevantFileClass::RelevantQC { issue_number: 3, description: None }
                },
                RelevantFile {
                    file_name: PathBuf::from("file.R"),
                    class: RelevantFileClass::File { justification: "A required justification".to_string() }
                }
            ]
        }
    }

    struct TestGitHelpers;

    impl GitHelpers for TestGitHelpers {
        fn file_content_url(&self, commit: &str, file: &std::path::Path) -> String {
            format!(
                "https://github.com/owner/repo/blob/{}/{}",
                commit,
                file.display()
            )
        }

        fn commit_comparison_url(
            &self,
            current_commit: &gix::ObjectId,
            previous_commit: &gix::ObjectId,
        ) -> String {
            format!(
                "https://github.com/owner/repo/compare/{}..{}",
                previous_commit, current_commit
            )
        }

        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/owner/repo/issues/{issue_number}")
        }
    }

    #[test]
    fn test_issue_body_snapshot() {
        let issue = create_test_issue();
        let git_helpers = TestGitHelpers;

        let body = issue.body(&git_helpers);
        insta::assert_snapshot!(body);
    }
}
