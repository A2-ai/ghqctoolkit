use std::path::{Path, PathBuf};

use crate::{
    configuration::Checklist,
    git::{GitAuthor, GitFileOps, GitFileOpsError, GitHelpers, GitRepository, GitRepositoryError},
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
    ) -> Result<Self, QCIssueError> {
        Ok(Self {
            title: file.as_ref().to_path_buf(),
            commit: git_info.commit()?,
            branch: git_info.branch()?,
            authors: git_info.authors(file.as_ref())?,
            checklist,
            assignees,
            milestone_id,
        })
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
    }

    #[test]
    fn test_issue_body_snapshot() {
        let issue = create_test_issue();
        let git_helpers = TestGitHelpers;

        let body = issue.body(&git_helpers);
        insta::assert_snapshot!(body);
    }
}
