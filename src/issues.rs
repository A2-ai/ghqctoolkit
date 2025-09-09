use clap::builder::TypedValueParser;
use clap::{Arg, Command, error::ErrorKind};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

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

impl RelevantFile {
    pub fn new(name: String, path: PathBuf, notes: Option<String>) -> Self {
        Self { name, path, notes }
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

// Custom parser for clap
#[derive(Clone)]
pub struct RelevantFileParser;

impl TypedValueParser for RelevantFileParser {
    type Value = RelevantFile;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                "Invalid UTF-8 in file specification",
            )
        })?;

        s.parse().map_err(|_| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue);
            if let Some(arg) = arg {
                err.insert(
                    clap::error::ContextKind::InvalidArg,
                    clap::error::ContextValue::String(arg.to_string()),
                );
            }
            err.insert(
                clap::error::ContextKind::InvalidValue,
                clap::error::ContextValue::String(s.to_string()),
            );
            err.insert(
                clap::error::ContextKind::ValidValue,
                clap::error::ContextValue::String("name:path".to_string()),
            );
            err
        })
    }
}

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
    relevant_files: Vec<RelevantFile>,
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
        let file_contents_html =
            format!("[file contents at initial qc commit]({file_contents_url})");

        let rel_files_section = if self.relevant_files.is_empty() {
            String::new()
        } else {
            let rel_files = self
                .relevant_files
                .iter()
                .map(|r| r.as_string(git_info, &self.branch))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "\n
## Relevant files

{rel_files}"
            )
        };

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
* {file_contents_html}{rel_files_section}
        
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
        relevant_files: Vec<RelevantFile>,
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
                mockall::predicate::eq("abc123def456789"),
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
                mockall::predicate::eq("abc123def456789"),
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
                Ok(RelevantFile::new(
                    "Config File".to_string(),
                    PathBuf::from("src/config.rs"),
                    None,
                )),
                "basic parsing with name and path",
            ),
            (
                "  Test File  :  src/test.rs  ",
                Ok(RelevantFile::new(
                    "Test File".to_string(),
                    PathBuf::from("src/test.rs"),
                    None,
                )),
                "parsing with extra spaces",
            ),
            (
                "Database:db/models.rs",
                Ok(RelevantFile::new(
                    "Database".to_string(),
                    PathBuf::from("db/models.rs"),
                    None,
                )),
                "single word name",
            ),
            (
                "Very Long Config File Name:path/to/very/long/file/name.rs",
                Ok(RelevantFile::new(
                    "Very Long Config File Name".to_string(),
                    PathBuf::from("path/to/very/long/file/name.rs"),
                    None,
                )),
                "long names and paths",
            ),
            (
                "File with: colon:src/special.rs",
                Ok(RelevantFile::new(
                    "File with".to_string(),
                    PathBuf::from("colon:src/special.rs"),
                    None,
                )),
                "multiple colons (only first is separator)",
            ),
            (
                "src/config.rs",
                Ok(RelevantFile::new(
                    "config.rs".to_string(),
                    PathBuf::from("src/config.rs"),
                    None,
                )),
                "no separator - path only, name derived from filename",
            ),
            (
                "path/to/file.txt",
                Ok(RelevantFile::new(
                    "file.txt".to_string(),
                    PathBuf::from("path/to/file.txt"),
                    None,
                )),
                "no separator - derive name from file extension",
            ),
            (
                "single_file",
                Ok(RelevantFile::new(
                    "single_file".to_string(),
                    PathBuf::from("single_file"),
                    None,
                )),
                "no separator - single filename",
            ),
            (
                ":src/file.rs",
                Ok(RelevantFile::new(
                    "file.rs".to_string(),
                    PathBuf::from("src/file.rs"),
                    None,
                )),
                "empty name - derive from filename",
            ),
            (
                "   :  src/test.rs  ",
                Ok(RelevantFile::new(
                    "test.rs".to_string(),
                    PathBuf::from("src/test.rs"),
                    None,
                )),
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
