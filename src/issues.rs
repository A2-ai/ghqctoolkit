use std::path::{Path, PathBuf};

use crate::git::{GitHelpers, LocalGitError, LocalGitInfo, local::GitAuthor};

pub struct QCIssue {
    pub(crate) milestone_id: u64,
    title: PathBuf,
    commit: String,
    pub(crate) branch: String,
    authors: Vec<GitAuthor>,
    checklist_name: String,
    checklist_note: Option<String>,
    checklist_content: String,
    pub(crate) assignees: Vec<String>,
}

impl QCIssue {
    pub(crate) fn body(&self, git_info: &impl GitHelpers) -> String {
        let author = self
            .authors
            .first()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let collaborators = if self.authors.len() > 1 {
            format!(
                "\n* collaborators: {}",
                self.authors
                    .iter()
                    .skip(1)
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };

        let file_contents_url = git_info.file_content_url(&self.commit, &self.title);
        let file_contents_html = format!(
            "<a href=\"{file_contents_url}\" target=\"_blank\">file contents at initial qc commit</a>"
        );

        let checklist_note = if let Some(note) = &self.checklist_note {
            format!("\n\n{note}")
        } else {
            String::new()
        };

        format!(
            "\
## Metadata
* initial qc commit: {}
* git branch: {}
* author: {author}{collaborators}
* {file_contents_html}
        
# {}{checklist_note}

{}
",
            self.commit, self.branch, self.checklist_name, self.checklist_content,
        )
    }

    pub(crate) fn title(&self) -> String {
        self.title.to_string_lossy().to_string()
    }

    pub(crate) fn new(
        file: impl AsRef<Path>,
        git_info: &impl LocalGitInfo,
        milestone_id: u64,
        assignees: Vec<String>,
        checklist_name: String,
        checklist_note: Option<String>,
        checklist_content: String,
    ) -> Result<Self, LocalGitError> {
        Ok(Self {
            title: file.as_ref().to_path_buf(),
            commit: git_info.commit()?,
            branch: git_info.branch()?,
            authors: git_info.authors(file.as_ref())?,
            checklist_name,
            checklist_note,
            checklist_content,
            assignees,
            milestone_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{helpers::MockGitHelpers, local::GitAuthor};
    use std::path::PathBuf;

    fn create_test_issue() -> QCIssue {
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
            checklist_name: "Code Review Checklist".to_string(),
            checklist_note: Some("NOTE".to_string()),
            checklist_content: "- [ ] Code compiles without warnings\n- [ ] Tests pass\n- [ ] Documentation updated".to_string(),
            assignees: vec!["reviewer1".to_string(), "reviewer2".to_string()],
        }
    }

    struct MockGitInfo {
        helpers: MockGitHelpers,
    }

    impl GitHelpers for MockGitInfo {
        fn file_content_url(&self, commit: &str, file: &std::path::Path) -> String {
            self.helpers.file_content_url(commit, file)
        }
    }

    #[test]
    fn test_issue_body_snapshot() {
        let issue = create_test_issue();

        let mut mock_git_info = MockGitInfo {
            helpers: MockGitHelpers::new(),
        };

        mock_git_info
            .helpers
            .expect_file_content_url()
            .with(
                mockall::predicate::eq("abc123def456789"),
                mockall::predicate::eq(PathBuf::from("src/example.rs")),
            )
            .returning(|_, _| {
                "https://github.com/owner/repo/blob/abc123d/src/example.rs".to_string()
            });

        let body = issue.body(&mock_git_info);
        insta::assert_snapshot!(body);
    }
}
