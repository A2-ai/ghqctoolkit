use core::fmt;
use std::path::{Path, PathBuf};

use crate::{
    Configuration, RelevantFile,
    git::{GitHubApi, GitHubApiError, LocalGitError, LocalGitInfo, RepoUser},
    issues::QCIssue,
};

#[derive(Debug, Clone)]
pub enum MilestoneStatus {
    Existing { name: String, number: u64 },
    New(String),
    Unknown(String),
}

impl fmt::Display for MilestoneStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::New(name) => write!(f, "{name} (new)"),
            Self::Existing { name, number } => write!(f, "{name} (existing: #{number})"),
            Self::Unknown(name) => write!(f, "{name} (unknown)"),
        }
    }
}

impl MilestoneStatus {
    async fn determine_milestone(
        &self,
        file: impl AsRef<Path>,
        git_info: &impl GitHubApi,
    ) -> Result<u64, CreateError> {
        let file = file.as_ref();
        match self {
            Self::Existing { number, .. } => {
                if issue_exists(git_info, *number, file).await? {
                    return Err(CreateError::IssueExists(file.to_path_buf()));
                } else {
                    Ok(*number)
                }
            }
            Self::New(milestone_name) => {
                let m = git_info.create_milestone(milestone_name).await?;
                log::debug!(
                    "Created milestone '{}' with ID: {}",
                    milestone_name,
                    m.number
                );
                Ok(m.number as u64)
            }
            Self::Unknown(milestone_name) => {
                find_or_create_milestone(file, milestone_name, git_info).await
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
    checklist_name: &str,
    assignees: Vec<String>,
    configuration: &Configuration,
    git_info: &(impl LocalGitInfo + GitHubApi),
    relevant_files: Vec<RelevantFile>,
) -> Result<(), CreateError> {
    let file = file.as_ref();

    let milestone_id = milestone_status.determine_milestone(file, git_info).await?;

    let checklist_content = match configuration.checklists.get(checklist_name) {
        Some(content) => {
            log::debug!("Found checklist in configuration");
            content
        }
        None => return Err(CreateError::NoChecklist(checklist_name.to_string())),
    };

    let issue = QCIssue::new(
        file,
        git_info,
        milestone_id,
        assignees,
        relevant_files,
        checklist_name.to_string(),
        configuration.options.prepended_checklist_notes.clone(),
        checklist_content.to_string(),
    )?;

    log::debug!("Posting issue to GitHub: {}", issue.title());
    git_info.post_issue(&issue).await?;

    Ok(())
}

async fn issue_exists(
    git_info: &impl GitHubApi,
    milestone_num: u64,
    file: impl AsRef<Path>,
) -> Result<bool, GitHubApiError> {
    let issues = git_info.get_milestone_issues(milestone_num).await?;
    log::debug!("Found {} existing issues in milestone", issues.len());
    Ok(issues
        .iter()
        .any(|i| i.title == file.as_ref().to_string_lossy()))
}

async fn find_or_create_milestone(
    file: impl AsRef<Path>,
    milestone_name: &str,
    git_info: &impl GitHubApi,
) -> Result<u64, CreateError> {
    let file = file.as_ref();
    let milestones = git_info.get_milestones().await?;
    log::debug!("Found {} existing milestones", milestones.len());

    let id = if let Some(m) = milestones.iter().find(|m| m.title == milestone_name) {
        log::debug!(
            "Found existing milestone '{}' with ID: {}",
            milestone_name,
            m.number
        );

        if issue_exists(git_info, m.number as u64, file).await? {
            return Err(CreateError::IssueExists(file.to_path_buf()));
        }

        m.number
    } else {
        let m = git_info.create_milestone(milestone_name).await?;
        log::debug!(
            "Created milestone '{}' with ID: {}",
            milestone_name,
            m.number
        );
        m.number
    };
    Ok(id as u64)
}

#[derive(thiserror::Error, Debug)]
pub enum CreateError {
    #[error("Failed to access GitHub API: {0}")]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Failed to perform git action: {0}")]
    LocalGitError(#[from] LocalGitError),
    #[error("Issue already exists within milestone for {0:?}")]
    IssueExists(PathBuf),
    #[error("Checklist name {0} does not exist in configuration directory")]
    NoChecklist(String),
    #[error("Invalid assignee: {0} is not a valid user in this repository")]
    InvalidAssignee(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{
        api::{MockGitHubApi, RepoUser},
        local::{GitAuthor, MockLocalGitInfo},
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
        ) -> Result<Vec<gix::ObjectId>, crate::git::LocalGitError> {
            self.local.file_commits(file)
        }

        fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, crate::git::LocalGitError> {
            self.local.authors(file)
        }
    }

    impl GitHubApi for MockGitInfo {
        async fn get_milestones(&self) -> Result<Vec<Milestone>, GitHubApiError> {
            self.github.get_milestones().await
        }

        async fn get_milestone_issues(
            &self,
            milestone_id: u64,
        ) -> Result<Vec<Issue>, GitHubApiError> {
            self.github.get_milestone_issues(milestone_id).await
        }

        async fn create_milestone(
            &self,
            milestone_name: &str,
        ) -> Result<Milestone, GitHubApiError> {
            self.github.create_milestone(milestone_name).await
        }

        async fn post_issue(&self, issue: &QCIssue) -> Result<(), GitHubApiError> {
            self.github.post_issue(issue).await
        }
        async fn get_users(&self) -> Result<Vec<RepoUser>, GitHubApiError> {
            self.github.get_users().await
        }
    }

    // Test scenario struct for matrix testing
    #[derive(Clone)]
    struct CreateIssueTestCase {
        name: &'static str,
        milestone_status: MilestoneStatus,
        checklist_name: &'static str,
        assignees: Vec<&'static str>,
        existing_milestones: Vec<&'static str>, // fixture names
        existing_issues: Vec<&'static str>,     // fixture names
        created_milestone: Option<&'static str>, // fixture name for new milestone
        expected_result: TestResult,
    }

    #[derive(Clone, Debug, PartialEq)]
    enum TestResult {
        Success,
        IssueExists,
        NoChecklist,
        InvalidAssignee,
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
        let mut config = Configuration::from_path("src/tests/default_configuration").unwrap();
        config.load_checklists().unwrap();
        config
    }

    fn setup_mock_git_info(test_case: &CreateIssueTestCase) -> MockGitInfo {
        let mut mock_git_info = MockGitInfo {
            local: MockLocalGitInfo::new(),
            github: MockGitHubApi::new(),
        };

        // Load milestones from fixtures
        let milestones: Vec<Milestone> = test_case
            .existing_milestones
            .iter()
            .map(|&name| load_milestone(name))
            .collect();

        // Set up expectations based on the milestone status (skip for InvalidAssignee)
        if !matches!(test_case.expected_result, TestResult::InvalidAssignee) {
            match &test_case.milestone_status {
                MilestoneStatus::Existing { number, .. } => {
                    // For existing milestones, we need to check if issue already exists
                    let issues: Vec<Issue> = test_case
                        .existing_issues
                        .iter()
                        .map(|&name| load_issue(name))
                        .collect();

                    mock_git_info
                        .github
                        .expect_get_milestone_issues()
                        .with(eq(*number))
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
                MilestoneStatus::Unknown(_) => {
                    // For unknown milestones, expect get_milestones and potentially get_milestone_issues calls
                    mock_git_info
                        .github
                        .expect_get_milestones()
                        .times(1)
                        .returning(move || {
                            let milestones = milestones.clone();
                            Box::pin(async move { Ok(milestones) })
                        });

                    // Set up milestone issues expectations for find_or_create_milestone
                    for milestone_fixture in &test_case.existing_milestones {
                        let milestone = load_milestone(milestone_fixture);
                        let milestone_name = match &test_case.milestone_status {
                            MilestoneStatus::Unknown(name) => name.as_str(),
                            _ => unreachable!(),
                        };
                        let issues: Vec<Issue> = if milestone.title == milestone_name {
                            // For the matching milestone, load the expected issues
                            test_case
                                .existing_issues
                                .iter()
                                .map(|&name| load_issue(name))
                                .collect()
                        } else {
                            vec![]
                        };

                        mock_git_info
                            .github
                            .expect_get_milestone_issues()
                            .with(eq(milestone.number as u64))
                            .times(1)
                            .returning(move |_| {
                                let issues = issues.clone();
                                Box::pin(async move { Ok(issues) })
                            });
                    }

                    // Handle milestone creation for Unknown that becomes New
                    if let Some(created_milestone_name) = test_case.created_milestone {
                        let created_milestone = load_milestone(created_milestone_name);
                        let milestone_name = match &test_case.milestone_status {
                            MilestoneStatus::Unknown(name) => name.clone(),
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

        // Set up get_users mock for validation (only needed for cases that don't fail early)
        if !matches!(test_case.expected_result, TestResult::InvalidAssignee) {
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

        // Only set up git and post expectations for success cases
        if test_case.expected_result == TestResult::Success {
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
        let test_cases = vec![
            CreateIssueTestCase {
                name: "success_with_existing_milestone",
                milestone_status: MilestoneStatus::Existing {
                    name: "v1.0".to_string(),
                    number: 1,
                },
                checklist_name: "Simple Tasks",
                assignees: vec!["user1", "user2"],
                existing_milestones: vec!["v1.0"],
                existing_issues: vec![],
                created_milestone: None,
                expected_result: TestResult::Success,
            },
            CreateIssueTestCase {
                name: "success_with_new_milestone",
                milestone_status: MilestoneStatus::New("v2.0".to_string()),
                checklist_name: "NCA Analysis",
                assignees: vec!["admin"],
                existing_milestones: vec![],
                existing_issues: vec![],
                created_milestone: Some("v2.0"),
                expected_result: TestResult::Success,
            },
            CreateIssueTestCase {
                name: "success_with_unknown_milestone",
                milestone_status: MilestoneStatus::Unknown("v1.0".to_string()),
                checklist_name: "Simple Tasks",
                assignees: vec![],
                existing_milestones: vec!["v1.0"],
                existing_issues: vec![],
                created_milestone: None,
                expected_result: TestResult::Success,
            },
            CreateIssueTestCase {
                name: "fails_when_issue_exists",
                milestone_status: MilestoneStatus::Unknown("v1.0".to_string()),
                checklist_name: "Simple Tasks",
                assignees: vec!["user1"],
                existing_milestones: vec!["v1.0"],
                existing_issues: vec!["test_file_issue"],
                created_milestone: None,
                expected_result: TestResult::IssueExists,
            },
            CreateIssueTestCase {
                name: "fails_with_nonexistent_checklist",
                milestone_status: MilestoneStatus::Unknown("v1.0".to_string()),
                checklist_name: "Nonexistent Checklist",
                assignees: vec!["user2"],
                existing_milestones: vec!["v1.0"],
                existing_issues: vec![],
                created_milestone: None,
                expected_result: TestResult::NoChecklist,
            },
            CreateIssueTestCase {
                name: "fails_with_invalid_assignee",
                milestone_status: MilestoneStatus::Unknown("v1.0".to_string()),
                checklist_name: "Simple Tasks",
                assignees: vec!["invalid_user"],
                existing_milestones: vec!["v1.0"],
                existing_issues: vec![],
                created_milestone: None,
                expected_result: TestResult::InvalidAssignee,
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
                create_issue(
                    PathBuf::from("src/test.rs"),
                    &test_case.milestone_status,
                    test_case.checklist_name,
                    assignees,
                    &config,
                    &mock_git_info,
                    vec![],
                )
                .await
            };

            match test_case.expected_result {
                TestResult::Success => {
                    assert!(
                        result.is_ok(),
                        "Test case '{}' should succeed",
                        test_case.name
                    );
                }
                TestResult::IssueExists => {
                    assert!(
                        result.is_err(),
                        "Test case '{}' should fail",
                        test_case.name
                    );
                    assert!(matches!(result.unwrap_err(), CreateError::IssueExists(_)));
                }
                TestResult::NoChecklist => {
                    assert!(
                        result.is_err(),
                        "Test case '{}' should fail",
                        test_case.name
                    );
                    assert!(matches!(result.unwrap_err(), CreateError::NoChecklist(_)));
                }
                TestResult::InvalidAssignee => {
                    assert!(
                        result.is_err(),
                        "Test case '{}' should fail",
                        test_case.name
                    );
                    assert!(matches!(
                        result.unwrap_err(),
                        CreateError::InvalidAssignee(_)
                    ));
                }
            }
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

    #[tokio::test]
    async fn test_find_or_create_milestone_matrix() {
        let test_cases: Vec<(
            &str,
            &str,
            Vec<&str>,
            Option<&str>,
            Result<u64, CreateError>,
        )> = vec![
            (
                "finds_existing_v1.0",
                "v1.0",
                vec!["v1.0", "v2.0"],
                None,
                Ok(1u64),
            ),
            (
                "finds_existing_v2.0",
                "v2.0",
                vec!["v1.0", "v2.0"],
                None,
                Ok(2u64),
            ),
            (
                "creates_new_v3.0",
                "v3.0",
                vec!["v1.0"],
                Some("v2.0"),
                Ok(2u64),
            ),
        ];

        for (name, milestone_name, existing_fixtures, created_fixture, expected) in test_cases {
            println!("Running milestone test: {}", name);

            let mut mock_api = MockGitHubApi::new();
            let milestones: Vec<Milestone> = existing_fixtures
                .iter()
                .map(|&fixture| load_milestone(fixture))
                .collect();

            // Set up get_milestones expectation
            mock_api.expect_get_milestones().times(1).returning({
                let milestones = milestones.clone();
                move || {
                    let milestones = milestones.clone();
                    Box::pin(async move { Ok(milestones) })
                }
            });

            // For existing milestones, expect get_milestone_issues call
            if let Some(found_milestone) = milestones.iter().find(|m| m.title == milestone_name) {
                mock_api
                    .expect_get_milestone_issues()
                    .with(eq(found_milestone.number as u64))
                    .times(1)
                    .returning(|_| Box::pin(async move { Ok(vec![]) }));
            }

            if let Some(created_fixture_name) = created_fixture {
                let created_milestone = load_milestone(created_fixture_name);
                let milestone_name_clone = milestone_name.to_string();

                mock_api
                    .expect_create_milestone()
                    .with(eq(milestone_name_clone))
                    .times(1)
                    .returning(move |_| {
                        let created_milestone = created_milestone.clone();
                        Box::pin(async move { Ok(created_milestone) })
                    });
            }

            let result =
                find_or_create_milestone(PathBuf::from("test.rs"), milestone_name, &mock_api).await;

            match expected {
                Ok(expected_id) => {
                    assert!(result.is_ok(), "Test '{}' should succeed", name);
                    assert_eq!(
                        result.unwrap(),
                        expected_id,
                        "Test '{}' wrong milestone ID",
                        name
                    );
                }
                Err(_) => {
                    assert!(result.is_err(), "Test '{}' should fail", name);
                }
            }
        }
    }
}
