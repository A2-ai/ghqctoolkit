use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use futures::future;
use octocrab::models::Milestone;
use tar::Builder;

use crate::{
    DiskCache, GitCommitAnalysis, GitFileOps, GitFileOpsError, GitHubApiError, GitHubReader,
    IssueError, IssueThread,
};

pub async fn get_archive_content(
    cache: Option<&DiskCache>,
    milestones: &[Milestone],
    include_unapproved: bool,
    git_info: &(impl GitHubReader + GitFileOps + GitCommitAnalysis),
) -> Result<HashMap<PathBuf, String>, ArchiveError> {
    // Collect all milestone-issue futures
    let milestone_futures: Vec<_> = milestones
        .iter()
        .map(|milestone| async move {
            let issues = git_info.get_milestone_issues(milestone).await?;
            Ok::<_, ArchiveError>(issues)
        })
        .collect();

    // Resolve all milestone issues concurrently
    let milestone_results = future::try_join_all(milestone_futures).await?;

    // Flatten all issues and create issue thread futures
    let issue_thread_futures: Vec<_> = milestone_results
        .into_iter()
        .flatten()
        .map(|issue| async move {
            let issue_thread = IssueThread::from_issue(&issue, cache, git_info).await?;

            // Filter out unapproved issues if needed
            if !include_unapproved && issue_thread.approved_commit.is_none() {
                return Ok::<Option<(PathBuf, String)>, ArchiveError>(None);
            }

            // Get file content at the latest commit
            let content = git_info
                .file_content_at_commit(&issue_thread.file, issue_thread.latest_commit())?;

            Ok(Some((issue_thread.file.clone(), content)))
        })
        .collect();

    // Resolve all issue threads and content concurrently
    let thread_results = future::try_join_all(issue_thread_futures).await?;

    // Build the final HashMap and check for conflicts
    let mut res = HashMap::new();
    for result in thread_results.into_iter().flatten() {
        let (file_path, content) = result;
        if res.insert(file_path.clone(), content).is_some() {
            return Err(ArchiveError::NonUniqueMilestoneFiles(file_path));
        }
    }

    Ok(res)
}

pub fn compress(
    archive_content: &HashMap<PathBuf, String>,
    flatten: bool,
    archive_path: impl AsRef<Path>,
) -> Result<(), ArchiveError> {
    let archive_path = archive_path.as_ref();

    // Create the output file
    let file = File::create(archive_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(encoder);

    let mut used_names = HashSet::new();

    // Add each file to the archive, checking for conflicts in a single pass
    for (path, content) in archive_content {
        let archive_path_str = if flatten {
            // Use just the basename
            let basename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unnamed");

            // Check for conflict when flattening
            if !used_names.insert(basename.to_string()) {
                return Err(ArchiveError::FlatteningConflict(basename.to_string()));
            }

            basename.to_string()
        } else {
            // Use the full path, but strip leading slash if present
            let full_path = path
                .strip_prefix("/")
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            used_names.insert(full_path.clone());
            full_path
        };

        let mut header = tar::Header::new_gnu();
        header.set_path(&archive_path_str)?;
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();

        tar.append(&header, content.as_bytes())?;
    }

    // Finish the archive
    tar.finish()?;

    log::debug!(
        "Successfully created compressed archive at {}",
        archive_path.display()
    );
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("Failed to access GitHub API due to: {0}")]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Failed to analyze issue due to: {0}")]
    IssueError(#[from] IssueError),
    #[error("Failed to get file content at commit due to: {0}")]
    GitFileOpsError(#[from] GitFileOpsError),
    #[error("Failed to create archive since multiple issues for '{0}' exists within Milestones")]
    NonUniqueMilestoneFiles(PathBuf),
    #[error("Cannot flatten archive: multiple files have the same basename '{0}'")]
    FlatteningConflict(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GitAuthor, RepoUser,
        git::{
            GitCommitAnalysis, GitCommitAnalysisError, GitFileOps, GitFileOpsError, GitHelpers,
            GitHubApiError, GitRepository, GitRepositoryError, GitStatus, GitStatusError,
            GitStatusOps,
        },
    };
    use gix::ObjectId;
    use octocrab::models::{Milestone, issues::Issue};
    use std::{collections::HashMap, path::PathBuf, str::FromStr};
    use tempfile::TempDir;

    /// Mock implementation for archive testing (reuses pattern from record tests)
    pub struct ArchiveMockGitInfo {
        pub milestones: Vec<Milestone>,
        pub milestone_issues: HashMap<String, Vec<Issue>>,
        pub repo_users: Vec<RepoUser>,
        pub git_status: GitStatus,
        pub owner: String,
        pub repo: String,
        pub current_branch: String,
        pub current_commit: String,
        pub file_commits: HashMap<PathBuf, Vec<(ObjectId, String)>>,
        pub file_content: HashMap<(PathBuf, ObjectId), String>,
    }

    impl ArchiveMockGitInfo {
        pub fn new() -> Self {
            Self {
                milestones: Vec::new(),
                milestone_issues: HashMap::new(),
                repo_users: Vec::new(),
                git_status: GitStatus::Clean,
                owner: "owner".to_string(),
                repo: "repo".to_string(),
                current_branch: "main".to_string(),
                current_commit: "abc123def456789012345678901234567890abcd".to_string(),
                file_commits: HashMap::new(),
                file_content: HashMap::new(),
            }
        }

        pub fn with_milestones(mut self, milestones: Vec<Milestone>) -> Self {
            self.milestones = milestones;
            self
        }

        pub fn with_milestone_issues(mut self, issues: HashMap<String, Vec<Issue>>) -> Self {
            self.milestone_issues = issues;
            self
        }

        pub fn with_file_commits(
            mut self,
            file: PathBuf,
            commits: Vec<(ObjectId, String)>,
        ) -> Self {
            self.file_commits.insert(file, commits);
            self
        }

        pub fn with_file_content(
            mut self,
            file: PathBuf,
            commit: ObjectId,
            content: String,
        ) -> Self {
            self.file_content.insert((file, commit), content);
            self
        }
    }

    impl GitHubReader for ArchiveMockGitInfo {
        async fn get_milestones(&self) -> Result<Vec<Milestone>, GitHubApiError> {
            Ok(self.milestones.clone())
        }

        async fn get_milestone_issues(
            &self,
            milestone: &Milestone,
        ) -> Result<Vec<Issue>, GitHubApiError> {
            Ok(self
                .milestone_issues
                .get(&milestone.title)
                .cloned()
                .unwrap_or_default())
        }

        async fn get_assignees(&self) -> Result<Vec<String>, GitHubApiError> {
            Ok(self.repo_users.iter().map(|u| u.login.clone()).collect())
        }

        async fn get_user_details(&self, username: &str) -> Result<RepoUser, GitHubApiError> {
            Ok(self
                .repo_users
                .iter()
                .find(|u| u.login == username)
                .cloned()
                .unwrap_or_else(|| RepoUser {
                    login: username.to_string(),
                    name: None,
                }))
        }

        async fn get_labels(&self) -> Result<Vec<String>, GitHubApiError> {
            Ok(vec!["ghqc".to_string(), "urgent".to_string()])
        }

        async fn get_issue_comments(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<crate::git::GitComment>, GitHubApiError> {
            Ok(vec![])
        }

        async fn get_issue_events(
            &self,
            _issue: &Issue,
        ) -> Result<Vec<serde_json::Value>, GitHubApiError> {
            Ok(vec![])
        }
    }

    impl GitRepository for ArchiveMockGitInfo {
        fn commit(&self) -> Result<String, GitRepositoryError> {
            Ok(self.current_commit.clone())
        }

        fn branch(&self) -> Result<String, GitRepositoryError> {
            Ok(self.current_branch.clone())
        }

        fn owner(&self) -> &str {
            &self.owner
        }

        fn repo(&self) -> &str {
            &self.repo
        }
    }

    impl GitStatusOps for ArchiveMockGitInfo {
        fn status(&self) -> Result<GitStatus, GitStatusError> {
            Ok(self.git_status.clone())
        }
    }

    impl GitFileOps for ArchiveMockGitInfo {
        fn file_commits(
            &self,
            file: &std::path::Path,
            _branch: &Option<String>,
        ) -> Result<Vec<(ObjectId, String)>, GitFileOpsError> {
            Ok(self.file_commits.get(file).cloned().unwrap_or_else(|| {
                vec![(
                    ObjectId::from_str(&self.current_commit).unwrap(),
                    "Test commit".to_string(),
                )]
            }))
        }

        fn authors(&self, _file: &std::path::Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(vec![GitAuthor {
                name: "Test Author".to_string(),
                email: "test@example.com".to_string(),
            }])
        }

        fn file_content_at_commit(
            &self,
            file: &std::path::Path,
            commit: &ObjectId,
        ) -> Result<String, GitFileOpsError> {
            Ok(self
                .file_content
                .get(&(file.to_path_buf(), *commit))
                .cloned()
                .unwrap_or_else(|| "test content".to_string()))
        }
    }

    impl GitCommitAnalysis for ArchiveMockGitInfo {
        fn get_all_merge_commits(&self) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(Vec::new())
        }

        fn get_commit_parents(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(Vec::new())
        }

        fn is_ancestor(
            &self,
            _ancestor: &ObjectId,
            _descendant: &ObjectId,
        ) -> Result<bool, GitCommitAnalysisError> {
            Ok(false)
        }

        fn get_branches_containing_commit(
            &self,
            _commit: &ObjectId,
        ) -> Result<Vec<String>, GitCommitAnalysisError> {
            Ok(vec![self.current_branch.clone()])
        }
    }

    impl GitHelpers for ArchiveMockGitInfo {
        fn file_content_url(&self, commit: &str, file: &std::path::Path) -> String {
            format!(
                "https://github.com/{}/{}/blob/{}/{}",
                self.owner,
                self.repo,
                commit,
                file.display()
            )
        }

        fn commit_comparison_url(
            &self,
            current_commit: &ObjectId,
            previous_commit: &ObjectId,
        ) -> String {
            format!(
                "https://github.com/{}/{}/compare/{}...{}",
                self.owner, self.repo, previous_commit, current_commit
            )
        }
    }

    // Test helper functions
    fn load_test_milestone(file_name: &str) -> Milestone {
        let path = format!("src/tests/github_api/milestones/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read milestone file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse milestone file {}: {}", path, e))
    }

    fn load_test_issue(file_name: &str) -> Issue {
        let path = format!("src/tests/github_api/issues/{}", file_name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read issue file: {}", path));
        serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse issue file {}: {}", path, e))
    }

    #[tokio::test]
    async fn test_get_archive_content_with_approved_issues() {
        let v1_milestone = load_test_milestone("v1.0.json");
        let milestones = vec![v1_milestone.clone()];

        let main_issue = load_test_issue("main_file_issue.json");
        let test_issue = load_test_issue("test_file_issue.json");

        let mut milestone_issues = HashMap::new();
        milestone_issues.insert(
            "v1.0".to_string(),
            vec![main_issue.clone(), test_issue.clone()],
        );

        let main_commit = ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let test_commit = ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap();

        let git_info = ArchiveMockGitInfo::new()
            .with_milestones(milestones.clone())
            .with_milestone_issues(milestone_issues)
            .with_file_commits(
                PathBuf::from("src/main.rs"),
                vec![(main_commit, "Main commit".to_string())],
            )
            .with_file_commits(
                PathBuf::from("src/test.rs"),
                vec![(test_commit, "Test commit".to_string())],
            )
            .with_file_content(
                PathBuf::from("src/main.rs"),
                main_commit,
                "main content".to_string(),
            )
            .with_file_content(
                PathBuf::from("src/test.rs"),
                test_commit,
                "test content".to_string(),
            );

        let result = get_archive_content(None, &milestones, true, &git_info).await;
        assert!(result.is_ok());

        let content = result.unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(
            content.get(&PathBuf::from("src/main.rs")),
            Some(&"main content".to_string())
        );
        assert_eq!(
            content.get(&PathBuf::from("src/test.rs")),
            Some(&"test content".to_string())
        );
    }

    #[tokio::test]
    async fn test_get_archive_content_duplicate_files() {
        let v1_milestone = load_test_milestone("v1.0.json");
        let v2_milestone = load_test_milestone("v2.0.json");
        let milestones = vec![v1_milestone.clone(), v2_milestone.clone()];

        let main_issue1 = load_test_issue("main_file_issue.json");
        let mut main_issue2 = main_issue1.clone();
        main_issue2.number = 999; // Different issue number but same file

        let mut milestone_issues = HashMap::new();
        milestone_issues.insert("v1.0".to_string(), vec![main_issue1.clone()]);
        milestone_issues.insert("v2.0".to_string(), vec![main_issue2.clone()]);

        let main_commit = ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = ArchiveMockGitInfo::new()
            .with_milestones(milestones.clone())
            .with_milestone_issues(milestone_issues)
            .with_file_commits(
                PathBuf::from("src/main.rs"),
                vec![(main_commit, "Main commit".to_string())],
            )
            .with_file_content(
                PathBuf::from("src/main.rs"),
                main_commit,
                "main content".to_string(),
            );

        let result = get_archive_content(None, &milestones, true, &git_info).await;
        assert!(result.is_err());

        if let Err(ArchiveError::NonUniqueMilestoneFiles(path)) = result {
            assert_eq!(path, PathBuf::from("src/main.rs"));
        } else {
            panic!("Expected NonUniqueMilestoneFiles error");
        }
    }

    #[test]
    fn test_compress_normal_paths() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.tar.gz");

        let mut content = HashMap::new();
        content.insert(PathBuf::from("src/main.rs"), "main content".to_string());
        content.insert(PathBuf::from("src/lib.rs"), "lib content".to_string());
        content.insert(
            PathBuf::from("docs/README.md"),
            "readme content".to_string(),
        );

        let result = compress(&content, false, &archive_path);
        assert!(result.is_ok());
        assert!(archive_path.exists());

        // Verify the archive contains expected files
        let file = std::fs::File::open(&archive_path).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);

        let entries: Vec<_> = archive.entries().unwrap().collect();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_compress_flattened_no_conflicts() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test_flat.tar.gz");

        let mut content = HashMap::new();
        content.insert(PathBuf::from("src/main.rs"), "main content".to_string());
        content.insert(
            PathBuf::from("tests/helper.rs"),
            "helper content".to_string(),
        );
        content.insert(
            PathBuf::from("docs/README.md"),
            "readme content".to_string(),
        );

        let result = compress(&content, true, &archive_path);
        assert!(result.is_ok());
        assert!(archive_path.exists());
    }

    #[test]
    fn test_compress_flattened_with_conflicts() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test_conflict.tar.gz");

        let mut content = HashMap::new();
        content.insert(PathBuf::from("src/main.rs"), "src main content".to_string());
        content.insert(
            PathBuf::from("tests/main.rs"),
            "test main content".to_string(),
        );

        let result = compress(&content, true, &archive_path);
        assert!(result.is_err());

        if let Err(ArchiveError::FlatteningConflict(name)) = result {
            assert_eq!(name, "main.rs");
        } else {
            panic!("Expected FlatteningConflict error");
        }
    }

    #[test]
    fn test_compress_empty_content() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("empty.tar.gz");

        let content = HashMap::new();

        let result = compress(&content, false, &archive_path);
        assert!(result.is_ok());
        assert!(archive_path.exists());

        // Verify empty archive
        let file = std::fs::File::open(&archive_path).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);

        let entries: Vec<_> = archive.entries().unwrap().collect();
        assert_eq!(entries.len(), 0);
    }
}
