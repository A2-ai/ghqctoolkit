use core::fmt;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use octocrab::models::Milestone;

use crate::{
    RelevantFile,
    configuration::Checklist,
    git::{GitHubApi, GitHubApiError, LocalGitError, LocalGitInfo, RepoUser},
    issues::QCIssue,
};

#[derive(Debug, Clone)]
pub enum MilestoneStatus {
    Existing(Milestone),
    New(String),
}

impl<'a> fmt::Display for MilestoneStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New(name) => write!(f, "{name} (new)"),
            Self::Existing(milestone) => {
                write!(f, "{} (existing: #{})", milestone.title, milestone.number)
            }
        }
    }
}

impl MilestoneStatus {
    async fn determine_milestone<'a>(
        &'a self,
        file: impl AsRef<Path>,
        git_info: &impl GitHubApi,
    ) -> Result<Cow<'a, Milestone>, CreateError> {
        let file = file.as_ref();
        match self {
            Self::Existing(milestone) => {
                if issue_exists(git_info, milestone, file).await? {
                    return Err(CreateError::IssueExists(file.to_path_buf()));
                } else {
                    Ok(Cow::Borrowed(milestone))
                }
            }
            Self::New(milestone_name) => {
                let m = git_info.create_milestone(milestone_name).await?;
                log::debug!(
                    "Created milestone '{}' with ID: {}",
                    milestone_name,
                    m.number
                );
                Ok(Cow::Owned(m))
            }
        }
    }
}

pub fn validate_assignees(
    assignees: &[String],
    repo_users: &[RepoUser],
) -> Result<(), CreateError> {
    if assignees.is_empty() {
        return Ok(());
    }

    log::debug!("Validating {} assignees", assignees.len());
    let valid_logins: Vec<String> = repo_users.iter().map(|u| u.login.clone()).collect();

    for assignee in assignees {
        if !valid_logins.contains(assignee) {
            return Err(CreateError::InvalidAssignee(assignee.clone()));
        }
    }

    log::debug!("All assignees are valid repository users");
    Ok(())
}

pub async fn create_issue(
    file: impl AsRef<Path>,
    milestone_status: &MilestoneStatus,
    checklist: &Checklist,
    assignees: Vec<String>,
    git_info: &(impl LocalGitInfo + GitHubApi),
    relevant_files: Vec<RelevantFile>,
) -> Result<String, CreateError> {
    let file = file.as_ref();

    let milestone = milestone_status.determine_milestone(file, git_info).await?;

    let issue = QCIssue::new(
        file,
        git_info,
        milestone.number as u64,
        assignees,
        relevant_files,
        checklist.clone(),
    )?;

    git_info.create_labels_if_needed(&issue.branch).await?;

    let issue_url = git_info.post_issue(&issue).await?;

    Ok(issue_url)
}

async fn issue_exists(
    git_info: &impl GitHubApi,
    milestone: &Milestone,
    file: impl AsRef<Path>,
) -> Result<bool, GitHubApiError> {
    let issues = git_info.get_milestone_issues(milestone).await?;
    log::debug!("Found {} existing issues in milestone", issues.len());
    Ok(issues
        .iter()
        .any(|i| i.title == file.as_ref().to_string_lossy()))
}

#[derive(thiserror::Error, Debug)]
pub enum CreateError {
    #[error("Failed to access GitHub API: {0}")]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Failed to perform git action: {0}")]
    LocalGitError(#[from] LocalGitError),
    #[error("Issue already exists within milestone for {0:?}")]
    IssueExists(PathBuf),
    #[error("Invalid assignee: {0} is not a valid user in this repository")]
    InvalidAssignee(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Configuration,
        git::{
            api::{MockGitHubApi, RepoUser},
            local::{GitAuthor, MockLocalGitInfo},
        },
    };
    use mockall::predicate::*;
    use octocrab::models::{Milestone, issues::Issue};
    use std::fs;

    // Mock implementation that combines both traits
    struct MockGitInfo {
        local: MockLocalGitInfo,
        github: MockGitHubApi,
    }

    impl LocalGitInfo for MockGitInfo {
        fn commit(&self) -> Result<String, crate::git::LocalGitError> {
            self.local.commit()
        }

        fn branch(&self) -> Result<String, crate::git::LocalGitError> {
            self.local.branch()
        }

        fn file_commits(
            &self,
            file: &Path,
        ) -> Result<Vec<(gix::ObjectId, String)>, crate::git::LocalGitError> {
            self.local.file_commits(file)
        }

        fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, crate::git::LocalGitError> {
            self.local.authors(file)
        }

        fn file_content_at_commit(
            &self,
            file: &Path,
            commit: &gix::ObjectId,
        ) -> Result<String, crate::git::LocalGitError> {
            self.local.file_content_at_commit(file, commit)
        }
    }

    impl GitHubApi for MockGitInfo {
        async fn get_milestones(&self) -> Result<Vec<Milestone>, GitHubApiError> {
            self.github.get_milestones().await
        }

        async fn get_milestone_issues(
            &self,
            milestone: &Milestone,
        ) -> Result<Vec<Issue>, GitHubApiError> {
            self.github.get_milestone_issues(milestone).await
        }

        async fn create_milestone(
            &self,
            milestone_name: &str,
        ) -> Result<Milestone, GitHubApiError> {
            self.github.create_milestone(milestone_name).await
        }

        async fn post_issue(&self, issue: &QCIssue) -> Result<String, GitHubApiError> {
            self.github.post_issue(issue).await
        }
        async fn post_comment(&self, comment: &crate::QCComment) -> Result<String, GitHubApiError> {
            self.github.post_comment(comment).await
        }
        async fn get_users(&self) -> Result<Vec<RepoUser>, GitHubApiError> {
            self.github.get_users().await
        }
        async fn create_labels_if_needed(&self, branch: &str) -> Result<(), GitHubApiError> {
            self.github.create_labels_if_needed(branch).await
        }
    }

    // Test scenario struct for matrix testing
    #[derive(Clone)]
    struct CreateIssueTestCase {
        name: &'static str,
        milestone_status: MilestoneStatus,
        checklist_name: &'static str,
        assignees: Vec<&'static str>,
        existing_issues: Vec<&'static str>,      // fixture names
        created_milestone: Option<&'static str>, // fixture name for new milestone
    }

    // Helper functions to load test fixtures
    fn load_milestone(name: &str) -> Milestone {
        let json_str =
            fs::read_to_string(format!("src/tests/github_api/milestones/{}.json", name)).unwrap();
        serde_json::from_str(&json_str).unwrap()
    }

    fn load_issue(name: &str) -> Issue {
        let json_str =
            fs::read_to_string(format!("src/tests/github_api/issues/{}.json", name)).unwrap();
        serde_json::from_str(&json_str).unwrap()
    }

    fn create_test_configuration() -> Configuration {
        let mut config = Configuration::from_path("src/tests/default_configuration");
        config.load_checklists();
        config
    }

    fn setup_mock_git_info(test_case: &CreateIssueTestCase) -> MockGitInfo {
        let mut mock_git_info = MockGitInfo {
            local: MockLocalGitInfo::new(),
            github: MockGitHubApi::new(),
        };

        // Set up expectations based on the milestone status
        {
            match &test_case.milestone_status {
                MilestoneStatus::Existing(milestone) => {
                    // For existing milestones, we need to check if issue already exists
                    let issues: Vec<Issue> = test_case
                        .existing_issues
                        .iter()
                        .map(|&name| load_issue(name))
                        .collect();

                    let expected_milestone_number = milestone.number;
                    mock_git_info
                        .github
                        .expect_get_milestone_issues()
                        .withf(move |m: &Milestone| m.number == expected_milestone_number)
                        .times(1)
                        .returning(move |_| {
                            let issues = issues.clone();
                            Box::pin(async move { Ok(issues) })
                        });
                }
                MilestoneStatus::New(_) => {
                    // For new milestones, expect create_milestone call
                    if let Some(created_milestone_name) = test_case.created_milestone {
                        let created_milestone = load_milestone(created_milestone_name);
                        let milestone_name = match &test_case.milestone_status {
                            MilestoneStatus::New(name) => name.clone(),
                            _ => unreachable!(),
                        };

                        mock_git_info
                            .github
                            .expect_create_milestone()
                            .with(eq(milestone_name))
                            .times(1)
                            .returning(move |_| {
                                let created_milestone = created_milestone.clone();
                                Box::pin(async move { Ok(created_milestone) })
                            });
                    }
                }
            }
        }

        // Set up get_users mock for validation
        {
            let valid_users = get_test_repo_users();

            mock_git_info
                .github
                .expect_get_users()
                .times(0..=1) // May not be called if validation fails early
                .returning({
                    let users = valid_users.clone();
                    move || {
                        let users = users.clone();
                        Box::pin(async move { Ok(users) })
                    }
                });
        }

        // Set up git and post expectations
        {
            mock_git_info
                .local
                .expect_commit()
                .times(1)
                .returning(|| Ok("abc123".to_string()));

            mock_git_info
                .local
                .expect_branch()
                .times(1)
                .returning(|| Ok("main".to_string()));

            mock_git_info
                .local
                .expect_authors()
                .times(1)
                .returning(|_| {
                    Ok(vec![GitAuthor {
                        name: "Test Author".to_string(),
                        email: "test@example.com".to_string(),
                    }])
                });

            mock_git_info
                .github
                .expect_post_issue()
                .times(1)
                .returning(|_| {
                    Box::pin(
                        async move { Ok("https://github.com/owner/repo/issues/123".to_string()) },
                    )
                });

            mock_git_info
                .github
                .expect_create_labels_if_needed()
                .with(eq("main"))
                .times(1)
                .returning(|_| Box::pin(async move { Ok(()) }));
        }

        mock_git_info
    }

    // Helper function to get test repo users
    fn get_test_repo_users() -> Vec<RepoUser> {
        vec![
            RepoUser {
                login: "user1".to_string(),
                name: Some("User One".to_string()),
            },
            RepoUser {
                login: "user2".to_string(),
                name: Some("User Two".to_string()),
            },
            RepoUser {
                login: "admin".to_string(),
                name: None,
            },
        ]
    }

    #[tokio::test]
    async fn test_create_issue_matrix() {
        // Load milestone fixtures that will be referenced by the test cases
        let v1_milestone = load_milestone("v1.0");

        let test_cases = vec![
            CreateIssueTestCase {
                name: "success_with_existing_milestone",
                milestone_status: MilestoneStatus::Existing(v1_milestone),
                checklist_name: "Simple Tasks",
                assignees: vec!["user1", "user2"],
                existing_issues: vec![],
                created_milestone: None,
            },
            CreateIssueTestCase {
                name: "success_with_new_milestone",
                milestone_status: MilestoneStatus::New("v2.0".to_string()),
                checklist_name: "NCA Analysis",
                assignees: vec!["admin"],
                existing_issues: vec![],
                created_milestone: Some("v2.0"),
            },
        ];

        let config = create_test_configuration();

        for test_case in test_cases {
            println!("Running test case: {}", test_case.name);

            let mock_git_info = setup_mock_git_info(&test_case);
            let repo_users = get_test_repo_users();
            let assignees: Vec<String> =
                test_case.assignees.iter().map(|s| s.to_string()).collect();

            // Validate assignees before calling create_issue (simulating main.rs logic)
            let validation_result = validate_assignees(&assignees, &repo_users);

            let result = if validation_result.is_err() {
                validation_result.map(|_| ())
            } else {
                let checklist = &config.checklists[test_case.checklist_name];
                create_issue(
                    PathBuf::from("src/test.rs"),
                    &test_case.milestone_status,
                    checklist,
                    assignees,
                    &mock_git_info,
                    vec![],
                )
                .await
                .map(|_| ()) // Ignore the URL, just check for success
            };

            // All test cases should succeed
            assert!(
                result.is_ok(),
                "Test case '{}' should succeed",
                test_case.name
            );
        }
    }

    #[test]
    fn test_validate_assignees() {
        let repo_users = get_test_repo_users();

        // Test valid assignees
        assert!(
            validate_assignees(&["user1".to_string(), "user2".to_string()], &repo_users).is_ok()
        );
        assert!(validate_assignees(&["admin".to_string()], &repo_users).is_ok());
        assert!(validate_assignees(&[], &repo_users).is_ok()); // Empty is valid

        // Test invalid assignees
        let result = validate_assignees(&["invalid_user".to_string()], &repo_users);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CreateError::InvalidAssignee(_)
        ));

        // Test mixed valid and invalid
        let result = validate_assignees(
            &["user1".to_string(), "invalid_user".to_string()],
            &repo_users,
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CreateError::InvalidAssignee(_)
        ));
    }
}
