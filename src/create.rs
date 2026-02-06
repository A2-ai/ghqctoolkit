use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    configuration::Checklist,
    git::{
        GitAuthor, GitFileOps, GitFileOpsError, GitHelpers, GitHubApiError, GitHubReader,
        GitHubWriter, GitRepository, GitRepositoryError,
    },
    relevant_files::{RelevantFile, relevant_files_section},
};

#[derive(Debug, thiserror::Error)]
pub enum QCIssueError {
    #[error(transparent)]
    GitRepositoryError(#[from] GitRepositoryError),
    #[error(transparent)]
    GitFileOpsError(#[from] GitFileOpsError),
    #[error(transparent)]
    GitHubApiError(#[from] GitHubApiError),
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

    /// Returns the blocking issues (GatingQC and PreviousQC) with their issue numbers and IDs.
    /// These issues must be approved before the current issue can be approved.
    /// Returns Vec<(issue_number, Option<issue_id>)>
    pub fn blocking_issues(&self) -> Vec<(u64, Option<u64>)> {
        use crate::relevant_files::RelevantFileClass;

        self.relevant_files
            .iter()
            .filter_map(|rf| match &rf.class {
                RelevantFileClass::GatingQC {
                    issue_number,
                    issue_id,
                    ..
                }
                | RelevantFileClass::PreviousQC {
                    issue_number,
                    issue_id,
                    ..
                } => Some((*issue_number, *issue_id)),
                _ => None,
            })
            .collect()
    }

    /// Posts the issue to GitHub and creates blocking relationships for GatingQC and PreviousQC issues.
    ///
    /// This function:
    /// 1. Posts the issue to GitHub via `post_issue()`
    /// 2. For each blocking issue (GatingQC/PreviousQC), creates a "blocked by" relationship
    /// 3. If issue_id is not available, fetches it via `get_issue()`
    /// 4. Blocking relationship failures are handled gracefully (logged but don't fail the operation)
    ///
    /// Returns the URL of the created issue.
    pub async fn post_with_blocking<T: GitHubWriter + GitHubReader + GitHelpers>(
        &self,
        git_info: &T,
    ) -> Result<CreateResult, QCIssueError> {
        let issue_url = git_info.post_issue(self).await?;
        let mut create_result = CreateResult {
            issue_url: issue_url.to_string(),
            parse_failed: false,
            successful_blocking: Vec::new(),
            blocking_errors: HashMap::new(),
        };

        let blocking_issues = self.blocking_issues();
        if !blocking_issues.is_empty() {
            // Parse issue number from URL (e.g., "https://github.com/owner/repo/issues/123")
            if let Some(new_issue_number) = issue_url
                .split('/')
                .last()
                .and_then(|s| s.parse::<u64>().ok())
            {
                for (issue_number, issue_id) in blocking_issues {
                    // Get the issue_id if not already available
                    let blocking_id = match issue_id {
                        Some(id) => id,
                        None => {
                            // Fetch the issue to get its internal ID
                            match git_info.get_issue(issue_number).await {
                                Ok(issue) => issue.id.0,
                                Err(e) => {
                                    create_result.blocking_errors.insert(issue_number, e);
                                    continue;
                                }
                            }
                        }
                    };

                    // Create the blocking relationship
                    // Failures are handled gracefully - GitHub Enterprise may not support this feature
                    if let Err(e) = git_info.block_issue(new_issue_number, blocking_id).await {
                        create_result.blocking_errors.insert(issue_number, e);
                    } else {
                        create_result.successful_blocking.push(issue_number);
                    }
                }
            } else {
                log::warn!(
                    "Failed to parse issue number form issue url. Skipping all issue blocking..."
                );
                create_result.parse_failed = true;
            }
        }

        Ok(create_result)
    }
}

pub struct CreateResult {
    issue_url: String,
    parse_failed: bool,
    successful_blocking: Vec<u64>,
    blocking_errors: HashMap<u64, GitHubApiError>,
}

impl fmt::Display for CreateResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.parse_failed {
            return write!(
                f,
                "⚠️ Issue created successfully. Issue URL could not be properly parsed, resulting in no blocking issues being posted\n"
            );
        }

        if self.blocking_errors.is_empty() {
            write!(f, "✅ Issue created successfully!\n")?;
        } else {
            write!(f, "⚠️ Issue created successfully.\n")?;
        }

        if !self.successful_blocking.is_empty() {
            write!(
                f,
                "  Issue blocked by issue(s): {}\n",
                self.successful_blocking
                    .iter()
                    .map(|s| format!("#{s}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }

        if !self.blocking_errors.is_empty() {
            write!(
                f,
                "  Failed to post issue blocking for:
    - {}
    Blocking Issues may not be supported by your GitHub deployment and cause errors.
    This may result in degredation of unapproval automation\n",
                self.blocking_errors
                    .iter()
                    .map(|(i, e)| format!("#{i}: {e}"))
                    .collect::<Vec<_>>()
                    .join("\n    - ")
            )?;
        }

        write!(f, "\n{}", self.issue_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        git::{GitAuthor, GitHelpers, GitHubReader, GitHubWriter},
        relevant_files::RelevantFileClass,
    };
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

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
                    class: RelevantFileClass::PreviousQC { issue_number: 1, issue_id: Some(1001), description: Some("This file has been previously QCed".to_string()) },
                },
                RelevantFile {
                    file_name: PathBuf::from("gating.R"),
                    class: RelevantFileClass::GatingQC { issue_number: 2, issue_id: Some(1002), description: Some("This file gates the approval of this QC".to_string()) }
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

    #[test]
    fn test_blocking_issues() {
        let issue = create_test_issue();
        let blocking = issue.blocking_issues();

        // Should return GatingQC (#2) and PreviousQC (#1), not RelevantQC or File
        assert_eq!(blocking.len(), 2);

        // Check that GatingQC is included
        assert!(
            blocking
                .iter()
                .any(|(num, id)| *num == 2 && *id == Some(1002)),
            "Expected GatingQC issue #2 with id 1002"
        );

        // Check that PreviousQC is included
        assert!(
            blocking
                .iter()
                .any(|(num, id)| *num == 1 && *id == Some(1001)),
            "Expected PreviousQC issue #1 with id 1001"
        );

        // Verify RelevantQC (#3) is NOT included (it's not a blocking issue)
        assert!(
            !blocking.iter().any(|(num, _)| *num == 3),
            "RelevantQC should not be included in blocking issues"
        );
    }

    #[test]
    fn test_blocking_issues_with_none_issue_id() {
        use crate::configuration::Checklist;

        let issue = QCIssue {
            milestone_id: 1,
            title: PathBuf::from("src/example.rs"),
            commit: "abc123def456789".to_string(),
            branch: "feature/new-feature".to_string(),
            authors: vec![],
            checklist: Checklist::new("Test".to_string(), None, "- [ ] item".to_string()),
            assignees: vec![],
            relevant_files: vec![RelevantFile {
                file_name: PathBuf::from("gating.R"),
                class: RelevantFileClass::GatingQC {
                    issue_number: 5,
                    issue_id: None, // CLI args mode - no issue_id available
                    description: None,
                },
            }],
        };

        let blocking = issue.blocking_issues();
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0], (5, None));
    }

    #[test]
    fn test_blocking_issues_empty() {
        use crate::configuration::Checklist;

        let issue = QCIssue {
            milestone_id: 1,
            title: PathBuf::from("src/example.rs"),
            commit: "abc123def456789".to_string(),
            branch: "feature/new-feature".to_string(),
            authors: vec![],
            checklist: Checklist::new("Test".to_string(), None, "- [ ] item".to_string()),
            assignees: vec![],
            relevant_files: vec![
                RelevantFile {
                    file_name: PathBuf::from("file.R"),
                    class: RelevantFileClass::File {
                        justification: "No QC needed".to_string(),
                    },
                },
                RelevantFile {
                    file_name: PathBuf::from("related.R"),
                    class: RelevantFileClass::RelevantQC {
                        issue_number: 10,
                        description: None,
                    },
                },
            ],
        };

        let blocking = issue.blocking_issues();
        // No GatingQC or PreviousQC, so blocking should be empty
        assert!(blocking.is_empty());
    }

    struct MockGitInfo {
        post_issue_url: String,
        fail_blocking_ids: Arc<HashSet<u64>>,
        block_calls: Arc<Mutex<Vec<(u64, u64)>>>,
        issues_by_number: Arc<HashMap<u64, octocrab::models::issues::Issue>>,
    }

    impl MockGitInfo {
        fn new(
            post_issue_url: &str,
            fail_blocking_ids: HashSet<u64>,
            issues_by_number: HashMap<u64, octocrab::models::issues::Issue>,
        ) -> Self {
            Self {
                post_issue_url: post_issue_url.to_string(),
                fail_blocking_ids: Arc::new(fail_blocking_ids),
                block_calls: Arc::new(Mutex::new(Vec::new())),
                issues_by_number: Arc::new(issues_by_number),
            }
        }
    }

    impl GitHelpers for MockGitInfo {
        fn file_content_url(&self, _git_ref: &str, _file: &std::path::Path) -> String {
            "https://example.com/file".to_string()
        }

        fn commit_comparison_url(
            &self,
            _current_commit: &gix::ObjectId,
            _previous_commit: &gix::ObjectId,
        ) -> String {
            "https://example.com/compare".to_string()
        }

        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://example.com/issues/{issue_number}")
        }
    }

    impl GitHubWriter for MockGitInfo {
        fn create_milestone(
            &self,
            _milestone_name: &str,
            _description: &Option<String>,
        ) -> impl std::future::Future<Output = Result<octocrab::models::Milestone, GitHubApiError>> + Send
        {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn post_issue(
            &self,
            _issue: &QCIssue,
        ) -> impl std::future::Future<Output = Result<String, GitHubApiError>> + Send {
            let url = self.post_issue_url.clone();
            async move { Ok(url) }
        }

        fn post_comment<T: crate::comment_system::CommentBody + 'static>(
            &self,
            _comment: &T,
        ) -> impl std::future::Future<Output = Result<String, GitHubApiError>> + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn close_issue(
            &self,
            _issue_number: u64,
        ) -> impl std::future::Future<Output = Result<(), GitHubApiError>> + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn open_issue(
            &self,
            _issue_number: u64,
        ) -> impl std::future::Future<Output = Result<(), GitHubApiError>> + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn create_label(
            &self,
            _name: &str,
            _color: &str,
        ) -> impl std::future::Future<Output = Result<(), GitHubApiError>> + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn block_issue(
            &self,
            blocked_issue_number: u64,
            blocking_issue_id: u64,
        ) -> impl std::future::Future<Output = Result<(), GitHubApiError>> + Send {
            let fail_ids = self.fail_blocking_ids.clone();
            let block_calls = self.block_calls.clone();
            async move {
                block_calls
                    .lock()
                    .expect("block_calls lock poisoned")
                    .push((blocked_issue_number, blocking_issue_id));

                if fail_ids.contains(&blocking_issue_id) {
                    Err(GitHubApiError::NoApi)
                } else {
                    Ok(())
                }
            }
        }
    }

    impl GitHubReader for MockGitInfo {
        fn get_milestones(
            &self,
        ) -> impl std::future::Future<
            Output = Result<Vec<octocrab::models::Milestone>, GitHubApiError>,
        > + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_issues(
            &self,
            _milestone: Option<u64>,
        ) -> impl std::future::Future<
            Output = Result<Vec<octocrab::models::issues::Issue>, GitHubApiError>,
        > + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_issue(
            &self,
            issue_number: u64,
        ) -> impl std::future::Future<
            Output = Result<octocrab::models::issues::Issue, GitHubApiError>,
        > + Send {
            let issues_by_number = self.issues_by_number.clone();
            async move {
                issues_by_number
                    .get(&issue_number)
                    .cloned()
                    .ok_or(GitHubApiError::NoApi)
            }
        }

        fn get_assignees(
            &self,
        ) -> impl std::future::Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_user_details(
            &self,
            _username: &str,
        ) -> impl std::future::Future<Output = Result<crate::git::RepoUser, GitHubApiError>> + Send
        {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_labels(
            &self,
        ) -> impl std::future::Future<Output = Result<Vec<String>, GitHubApiError>> + Send {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_issue_comments(
            &self,
            _issue: &octocrab::models::issues::Issue,
        ) -> impl std::future::Future<Output = Result<Vec<crate::git::GitComment>, GitHubApiError>> + Send
        {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_issue_events(
            &self,
            _issue: &octocrab::models::issues::Issue,
        ) -> impl std::future::Future<Output = Result<Vec<serde_json::Value>, GitHubApiError>> + Send
        {
            async move { Err(GitHubApiError::NoApi) }
        }

        fn get_blocked_issues(
            &self,
            _issue_number: u64,
        ) -> impl std::future::Future<
            Output = Result<Vec<octocrab::models::issues::Issue>, GitHubApiError>,
        > + Send {
            async move { Err(GitHubApiError::NoApi) }
        }
    }

    fn load_issue(issue_file: &str) -> octocrab::models::issues::Issue {
        let path = format!("src/tests/github_api/issues/{}", issue_file);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));

        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    #[tokio::test]
    async fn test_post_with_blocking_aggregates_results() {
        use crate::configuration::Checklist;

        let issue_one = load_issue("test_file_issue.json");
        let issue_two = load_issue("config_file_issue.json");
        let mut issues_by_number = HashMap::new();
        issues_by_number.insert(issue_one.number, issue_one.clone());
        issues_by_number.insert(issue_two.number, issue_two.clone());

        let issue = QCIssue {
            milestone_id: 1,
            title: PathBuf::from("src/example.rs"),
            commit: "abc123def456789".to_string(),
            branch: "feature/new-feature".to_string(),
            authors: vec![],
            checklist: Checklist::new("Test".to_string(), None, "- [ ] item".to_string()),
            assignees: vec![],
            relevant_files: vec![
                RelevantFile {
                    file_name: PathBuf::from("previous.R"),
                    class: RelevantFileClass::PreviousQC {
                        issue_number: issue_one.number,
                        issue_id: None,
                        description: None,
                    },
                },
                RelevantFile {
                    file_name: PathBuf::from("gating.R"),
                    class: RelevantFileClass::GatingQC {
                        issue_number: issue_two.number,
                        issue_id: None,
                        description: None,
                    },
                },
            ],
        };

        let mut fail_ids = HashSet::new();
        fail_ids.insert(issue_two.id.0);
        let git_info = MockGitInfo::new(
            "https://github.com/owner/repo/issues/42",
            fail_ids,
            issues_by_number,
        );

        let result = issue.post_with_blocking(&git_info).await.unwrap();

        assert_eq!(result.issue_url, "https://github.com/owner/repo/issues/42");
        assert!(!result.parse_failed);

        assert!(result.successful_blocking.contains(&issue_one.number));
        assert!(!result.successful_blocking.contains(&issue_two.number));
        assert!(result.blocking_errors.contains_key(&issue_two.number));
        assert!(!result.blocking_errors.contains_key(&issue_one.number));

        let calls = git_info
            .block_calls
            .lock()
            .expect("block_calls lock poisoned")
            .clone();
        assert_eq!(calls.len(), 2);
        assert!(calls.contains(&(42, issue_one.id.0)));
        assert!(calls.contains(&(42, issue_two.id.0)));
    }
}
