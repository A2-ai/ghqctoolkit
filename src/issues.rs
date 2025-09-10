use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::configuration::Checklist;
use crate::git::{GitHelpers, LocalGitError, LocalGitInfo, local::GitAuthor};

#[derive(Debug, Clone, PartialEq)]
pub struct RelevantFile {
    pub name: String,
    pub path: PathBuf,
    pub notes: Option<String>,
}

impl fmt::Display for RelevantFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

impl FromStr for RelevantFile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((name, path)) = s.split_once(':') {
            if path.trim().is_empty() {
                return Err("Path cannot be empty".to_string());
            }
            let path_trimmed = path.trim();
            let path_buf = PathBuf::from(path_trimmed);

            // If name is empty or just whitespace, use the file name from path as the name
            let final_name = if name.trim().is_empty() {
                path_buf
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path_trimmed)
                    .to_string()
            } else {
                name.trim().to_string()
            };

            Ok(Self {
                name: final_name,
                path: path_buf,
                notes: None,
            })
        } else {
            // No colon separator - treat the whole string as a path and derive name from it
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("Path cannot be empty".to_string());
            }

            let path_buf = PathBuf::from(trimmed);
            let name = path_buf
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(trimmed)
                .to_string();

            Ok(Self {
                name,
                path: path_buf,
                notes: None,
            })
        }
    }
}

impl RelevantFile {
    fn as_string(&self, git_info: &impl GitHelpers, branch: &str) -> String {
        let note = if let Some(n) = &self.notes {
            // Convert literal \n sequences to actual newlines, then format with proper indentation
            let converted_notes = n.replace("\\n", "\n");
            format!("\n\t> {}", converted_notes.replace("\n", "\n\t> "))
        } else {
            String::new()
        };

        format!(
            "- **{}**\n\t- [`{}`]({}){}",
            self.name,
            self.path.display(),
            git_info.file_content_url(branch, &self.path),
            note
        )
    }
}

#[derive(Debug, Clone)]
pub struct QCIssue {
    pub(crate) milestone_id: u64,
    title: PathBuf,
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

        if !self.relevant_files.is_empty() {
            let rel_files = self
                .relevant_files
                .iter()
                .map(|r| r.as_string(git_info, &self.branch))
                .collect::<Vec<_>>()
                .join("\n");
            metadata.push(format!("## Relevant files\n\n{rel_files}"));
        };

        body.push(self.checklist.to_string());

        body.join("\n\n")
    }

    pub(crate) fn title(&self) -> String {
        self.title.to_string_lossy().to_string()
    }

    pub(crate) fn new(
        file: impl AsRef<Path>,
        git_info: &impl LocalGitInfo,
        milestone_id: u64,
        assignees: Vec<String>,
        relevant_files: Vec<RelevantFile>,
        checklist: Checklist,
    ) -> Result<Self, LocalGitError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{helpers::MockGitHelpers, local::GitAuthor};
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
                    name: "rel file".to_string(),
                    path: PathBuf::from("path/to/file.rel"),
                    notes: Some("this\nis\na note".to_string())
                },
                RelevantFile {
                    name: "rel file2".to_string(),
                    path: PathBuf::from("path/to/file2.rel"),
                    notes: None,
                }
            ]
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
    fn test_issue_body_snapshot_with_rel_files() {
        let issue = create_test_issue();

        let mut mock_git_info = MockGitInfo {
            helpers: MockGitHelpers::new(),
        };

        mock_git_info
            .helpers
            .expect_file_content_url()
            .with(
                mockall::predicate::eq("abc123d"),
                mockall::predicate::eq(PathBuf::from("src/example.rs")),
            )
            .returning(|_, _| {
                "https://github.com/owner/repo/blob/abc123d/src/example.rs".to_string()
            });

        // Mock expectation for the relevant file
        mock_git_info
            .helpers
            .expect_file_content_url()
            .with(
                mockall::predicate::eq("feature/new-feature"),
                mockall::predicate::eq(PathBuf::from("path/to/file.rel")),
            )
            .returning(|_, _| {
                "https://github.com/owner/repo/blob/feature/new-feature/path/to/file.rel"
                    .to_string()
            });
        mock_git_info
            .helpers
            .expect_file_content_url()
            .with(
                mockall::predicate::eq("feature/new-feature"),
                mockall::predicate::eq(PathBuf::from("path/to/file2.rel")),
            )
            .returning(|_, _| {
                "https://github.com/owner/repo/blob/feature/new-feature/path/to/file2.rel"
                    .to_string()
            });

        let body = issue.body(&mock_git_info);
        insta::assert_snapshot!(body);
    }

    #[test]
    fn test_issue_body_snapshot_without_rel_files() {
        let mut issue = create_test_issue();
        issue.relevant_files = Vec::new();

        let mut mock_git_info = MockGitInfo {
            helpers: MockGitHelpers::new(),
        };

        mock_git_info
            .helpers
            .expect_file_content_url()
            .with(
                mockall::predicate::eq("abc123d"),
                mockall::predicate::eq(PathBuf::from("src/example.rs")),
            )
            .returning(|_, _| {
                "https://github.com/owner/repo/blob/abc123d/src/example.rs".to_string()
            });

        let body = issue.body(&mock_git_info);
        insta::assert_snapshot!(body);
    }

    #[test]
    fn test_named_file_parsing_matrix() {
        let test_cases = vec![
            // (input, expected_result, test_description)
            (
                "Config File:src/config.rs",
                Ok(RelevantFile {
                    name: "Config File".to_string(),
                    path: PathBuf::from("src/config.rs"),
                    notes: None,
                }),
                "basic parsing with name and path",
            ),
            (
                "  Test File  :  src/test.rs  ",
                Ok(RelevantFile {
                    name: "Test File".to_string(),
                    path: PathBuf::from("src/test.rs"),
                    notes: None,
                }),
                "parsing with extra spaces",
            ),
            (
                "Database:db/models.rs",
                Ok(RelevantFile {
                    name: "Database".to_string(),
                    path: PathBuf::from("db/models.rs"),
                    notes: None,
                }),
                "single word name",
            ),
            (
                "Very Long Config File Name:path/to/very/long/file/name.rs",
                Ok(RelevantFile {
                    name: "Very Long Config File Name".to_string(),
                    path: PathBuf::from("path/to/very/long/file/name.rs"),
                    notes: None,
                }),
                "long names and paths",
            ),
            (
                "File with: colon:src/special.rs",
                Ok(RelevantFile {
                    name: "File with".to_string(),
                    path: PathBuf::from("colon:src/special.rs"),
                    notes: None,
                }),
                "multiple colons (only first is separator)",
            ),
            (
                "src/config.rs",
                Ok(RelevantFile {
                    name: "config.rs".to_string(),
                    path: PathBuf::from("src/config.rs"),
                    notes: None,
                }),
                "no separator - path only, name derived from filename",
            ),
            (
                "path/to/file.txt",
                Ok(RelevantFile {
                    name: "file.txt".to_string(),
                    path: PathBuf::from("path/to/file.txt"),
                    notes: None,
                }),
                "no separator - derive name from file extension",
            ),
            (
                "single_file",
                Ok(RelevantFile {
                    name: "single_file".to_string(),
                    path: PathBuf::from("single_file"),
                    notes: None,
                }),
                "no separator - single filename",
            ),
            (
                ":src/file.rs",
                Ok(RelevantFile {
                    name: "file.rs".to_string(),
                    path: PathBuf::from("src/file.rs"),
                    notes: None,
                }),
                "empty name - derive from filename",
            ),
            (
                "   :  src/test.rs  ",
                Ok(RelevantFile {
                    name: "test.rs".to_string(),
                    path: PathBuf::from("src/test.rs"),
                    notes: None,
                }),
                "whitespace name - derive from filename",
            ),
            (
                "Name:",
                Err("Path cannot be empty".to_string()),
                "empty path with colon",
            ),
            (
                "",
                Err("Path cannot be empty".to_string()),
                "completely empty",
            ),
            (
                "   ",
                Err("Path cannot be empty".to_string()),
                "only whitespace",
            ),
        ];

        for (input, expected, description) in test_cases {
            let result: Result<RelevantFile, String> = input.parse();
            match (result, expected) {
                (Ok(actual), Ok(expected)) => {
                    assert_eq!(actual, expected, "Test case failed: {}", description);
                }
                (Err(actual_err), Err(expected_err)) => {
                    assert!(
                        actual_err.contains(&expected_err),
                        "Test case '{}' failed: expected error containing '{}', got '{}'",
                        description,
                        expected_err,
                        actual_err
                    );
                }
                (Ok(actual), Err(expected_err)) => {
                    panic!(
                        "Test case '{}' failed: expected error '{}', but got success: {:?}",
                        description, expected_err, actual
                    );
                }
                (Err(actual_err), Ok(expected)) => {
                    panic!(
                        "Test case '{}' failed: expected success {:?}, but got error '{}'",
                        description, expected, actual_err
                    );
                }
            }
        }
    }
}
