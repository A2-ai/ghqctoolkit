use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    configuration::Checklist,
    git::{
        GitAuthor, GitFileOps, GitFileOpsError, GitHelpers, GitHubApiError, GitHubReader,
        GitHubWriter, GitRepository, GitRepositoryError,
    },
    relevant_files::{RelevantFile, RelevantFileClass, relevant_files_section},
};

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

        // Use up to 7 characters for short commit hash, or full length if shorter
        let commit_short = &self.commit[..self.commit.len().min(7)];
        metadata.push(format!(
            "[file contents at initial qc commit]({})",
            git_info.file_content_url(commit_short, &self.title)
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
        let issue = git_info.post_issue(self).await?;
        let issue_number = issue.number;
        let issue_id = issue.id.0;
        let issue_url = issue.html_url.to_string();

        let mut create_result = CreateResult {
            issue_url,
            issue_number,
            issue_id,
            parse_failed: false,
            successful_blocking: Vec::new(),
            blocking_errors: HashMap::new(),
        };

        let blocking_issues = self.blocking_issues();
        if !blocking_issues.is_empty() {
            let new_issue_number = issue_number;
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
            }
        }

        Ok(create_result)
    }
}

pub struct CreateResult {
    pub issue_url: String,
    pub issue_number: u64,
    pub issue_id: u64,
    pub parse_failed: bool,
    pub successful_blocking: Vec<u64>,
    pub blocking_errors: HashMap<u64, GitHubApiError>,
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

/// Entry for creating a single QC issue in a batch
#[derive(Debug, Clone)]
pub struct QCEntry {
    /// File path (used as unique identifier)
    pub title: PathBuf,
    /// Checklist to use
    pub checklist: Checklist,
    /// Assignees for this issue
    pub assignees: Vec<String>,
    /// Related files (existing or being created in this batch)
    pub relevant_files: Vec<RelevantFileEntry>,
}

/// Reference to a related file (either existing or new)
#[derive(Debug, Clone)]
pub enum RelevantFileEntry {
    /// Reference to an already-created issue
    ExistingIssue(RelevantFile),
    /// Reference to an issue being created in this batch
    NewIssue {
        file_path: PathBuf,
        relationship: QCRelationship,
        description: Option<String>,
    },
    File {
        file_path: PathBuf,
        justification: String,
    },
}

/// Type of QC relationship
#[derive(Debug, Clone, Copy)]
pub enum QCRelationship {
    PreviousQC,
    GatingQC,
    RelevantQC,
}

/// Dependency graph for batch QC entry creation using references
#[derive(Debug)]
struct DependencyGraph<'a> {
    /// Map from file path to entry
    entries: HashMap<PathBuf, &'a QCEntry>,
    /// Forward edges: file -> set of files that depend on it
    /// If A blocks B, dependents[A] contains B
    dependents: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Reverse edges: file -> set of files it depends on
    /// If A blocks B, dependencies[B] contains A
    dependencies: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl<'a> DependencyGraph<'a> {
    /// Build dependency graph from QC entries
    fn build(entries: &'a [QCEntry]) -> Self {
        let mut entry_map = HashMap::new();
        let mut dependents = HashMap::new();
        let mut dependencies = HashMap::new();

        // Build entry map and initialize dependency sets
        for entry in entries {
            entry_map.insert(entry.title.clone(), entry);
            dependents.insert(entry.title.clone(), HashSet::new());
            dependencies.insert(entry.title.clone(), HashSet::new());
        }

        // Build dependency relationships
        for entry in entries {
            for relevant_file in &entry.relevant_files {
                if let RelevantFileEntry::NewIssue { file_path, .. } = relevant_file {
                    // Track ALL New references - they all must be created first (for hyperlinks)
                    if entry_map.contains_key(file_path) {
                        // file_path blocks entry.title
                        dependents
                            .get_mut(file_path)
                            .unwrap()
                            .insert(entry.title.clone());
                        dependencies
                            .get_mut(&entry.title)
                            .unwrap()
                            .insert(file_path.clone());
                    }
                }
            }
        }

        Self {
            entries: entry_map,
            dependents,
            dependencies,
        }
    }

    /// Detect cycles in dependency graph using DFS
    fn detect_cycles(&self) -> Vec<DependencyCycle> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for file_path in self.entries.keys() {
            if !visited.contains(file_path) {
                self.detect_cycle_dfs(
                    file_path,
                    &mut visited,
                    &mut rec_stack,
                    &mut path,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    fn detect_cycle_dfs(
        &self,
        path_buf: &PathBuf,
        visited: &mut HashSet<PathBuf>,
        rec_stack: &mut HashSet<PathBuf>,
        path: &mut Vec<PathBuf>,
        cycles: &mut Vec<DependencyCycle>,
    ) {
        visited.insert(path_buf.clone());
        rec_stack.insert(path_buf.clone());
        path.push(path_buf.clone());

        if let Some(deps) = self.dependents.get(path_buf) {
            for dependent_path in deps {
                if !visited.contains(dependent_path) {
                    self.detect_cycle_dfs(dependent_path, visited, rec_stack, path, cycles);
                } else if rec_stack.contains(dependent_path) {
                    // Found cycle - extract from path
                    if let Some(cycle_start) = path.iter().position(|p| p == dependent_path) {
                        let cycle_files: Vec<PathBuf> = path[cycle_start..].to_vec();
                        cycles.push(DependencyCycle { files: cycle_files });
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(path_buf);
    }

    fn in_degree(&self) -> HashMap<&PathBuf, usize> {
        self.dependencies
            .iter()
            .map(|(file, dependencies)| (file, dependencies.len()))
            .collect()
    }
}

/// Result of dependency resolution
#[derive(Debug, Clone, Default)]
pub struct ResolutionResult {
    /// Creation order (file paths in order)
    creation_order: Vec<PathBuf>,
    /// Detected cycles (if any)
    cycles: Vec<DependencyCycle>,
    /// Validation errors
    errors: Vec<DependencyError>,
}

/// Circular dependency cycle
#[derive(Debug, Clone)]
pub struct DependencyCycle {
    /// Files involved in cycle (in order)
    files: Vec<PathBuf>,
}

/// Resolve creation order for batch of QC entries
///
/// Uses topological sort (Kahn's algorithm) to determine valid creation order.
/// Returns errors if circular dependencies or other issues detected.
fn resolve_creation_order(entries: &[QCEntry]) -> ResolutionResult {
    let mut res = ResolutionResult::default();
    let mut seen_files = HashSet::new();

    // First pass: collect all files and check for duplicates
    for entry in entries {
        if !seen_files.insert(entry.title.clone()) {
            res.errors.push(DependencyError::DuplicateFile {
                file: entry.title.clone(),
            });
        }
    }

    // Second pass: validate all references (now that we know all files in batch)
    for entry in entries {
        for rel_file in &entry.relevant_files {
            if let RelevantFileEntry::NewIssue { file_path, .. } = rel_file {
                if file_path == &entry.title {
                    // Self-reference
                    res.errors.push(DependencyError::SelfReference {
                        file: entry.title.clone(),
                    });
                } else if !seen_files.contains(file_path) {
                    // Referenced file is not in the batch
                    res.errors.push(DependencyError::MissingBatchReference {
                        referencing_file: entry.title.clone(),
                        referenced_file: file_path.clone(),
                    });
                }
            }
        }
    }

    if !res.errors.is_empty() {
        return res;
    }

    // Build dependency graph
    let graph = DependencyGraph::build(entries);

    // Perform topological sort using Kahn's algorithm
    // Calculate in-degree for each node (number of dependencies)
    let mut in_degree = graph.in_degree();

    // Start with nodes that have no dependencies
    let mut queue: VecDeque<PathBuf> = in_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(&path, _)| path.clone())
        .collect();

    while let Some(path) = queue.pop_front() {
        res.creation_order.push(path.clone());

        // Reduce in-degree for all dependents
        if let Some(deps) = graph.dependents.get(&path) {
            for dependent_path in deps {
                let degree = in_degree.get_mut(dependent_path).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(dependent_path.clone());
                }
            }
        }
    }

    // Check if all entries were processed
    if res.creation_order.len() != entries.len() {
        // Cycles detected - find them
        let cycles = graph.detect_cycles();
        res.errors = cycles
            .iter()
            .map(|c| DependencyError::CircularDependency { cycle: c.clone() })
            .collect();
        res.cycles = cycles;
    }

    res
}

/// Create multiple QC issues in resolved dependency order
pub async fn batch_post_qc_entries(
    entries: &[QCEntry],
    git_info: &(impl GitHubWriter + GitHubReader + GitHelpers + GitRepository + GitFileOps),
    milestone_id: u64,
) -> Result<Vec<CreateResult>, QCIssueError> {
    let commit = git_info.commit()?;
    let branch = git_info.branch()?;

    // Resolve creation order
    let resolution = resolve_creation_order(entries);

    // Check for errors
    if !resolution.errors.is_empty() {
        return Err(QCIssueError::DependencyResolution {
            errors: resolution.errors,
        });
    }

    // Build map from file path to entry for quick lookup
    let entry_map: HashMap<PathBuf, &QCEntry> =
        entries.iter().map(|e| (e.title.clone(), e)).collect();

    // Create issues in resolved order
    let mut results = Vec::new();
    let mut created_issues: HashMap<PathBuf, (u64, u64)> = HashMap::new();

    for file_path in &resolution.creation_order {
        let entry = entry_map
            .get(file_path)
            .ok_or(QCIssueError::EntryNotFound {
                file: file_path.clone(),
            })?;

        // Resolve RelevantFileEntry::New references to actual issue numbers
        let relevant_files: Vec<RelevantFile> = entry
            .relevant_files
            .iter()
            .map(|rf| -> Result<RelevantFile, QCIssueError> {
                match rf {
                    RelevantFileEntry::ExistingIssue(rel_file) => Ok(rel_file.clone()),
                    RelevantFileEntry::NewIssue {
                        file_path,
                        relationship,
                        description,
                    } => {
                        let &(issue_number, issue_id) =
                            created_issues.get(file_path).ok_or_else(|| {
                                QCIssueError::UnresolvedReference {
                                    file: file_path.clone(),
                                    referencing_file: entry.title.clone(),
                                }
                            })?;
                        Ok(RelevantFile {
                            file_name: file_path.clone(),
                            class: match relationship {
                                QCRelationship::PreviousQC => RelevantFileClass::PreviousQC {
                                    issue_number,
                                    issue_id: Some(issue_id),
                                    description: description.clone(),
                                },
                                QCRelationship::GatingQC => RelevantFileClass::GatingQC {
                                    issue_number,
                                    issue_id: Some(issue_id),
                                    description: description.clone(),
                                },
                                QCRelationship::RelevantQC => RelevantFileClass::RelevantQC {
                                    issue_number,
                                    description: description.clone(),
                                },
                            },
                        })
                    }
                    RelevantFileEntry::File {
                        file_path,
                        justification,
                    } => Ok(RelevantFile {
                        file_name: file_path.to_path_buf(),
                        class: RelevantFileClass::File {
                            justification: justification.clone(),
                        },
                    }),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Get authors for file
        let authors = git_info.authors(&entry.title)?;

        // Create QCIssue
        let qc_issue = QCIssue {
            milestone_id,
            title: entry.title.clone(),
            commit: commit.clone(),
            branch: branch.clone(),
            authors,
            checklist: entry.checklist.clone(),
            assignees: entry.assignees.clone(),
            relevant_files,
        };

        // Post with blocking relationships
        let create_result = qc_issue.post_with_blocking(git_info).await?;

        // Store the issue number and ID from the created issue
        created_issues.insert(
            entry.title.clone(),
            (create_result.issue_number, create_result.issue_id),
        );

        results.push(create_result);
    }

    Ok(results)
}

/// Dependency validation error
#[derive(Debug, Clone, thiserror::Error)]
pub enum DependencyError {
    #[error("Circular dependency: {}", .cycle.files.iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" -> "))]
    CircularDependency { cycle: DependencyCycle },

    #[error("File {file:?} references itself")]
    SelfReference { file: PathBuf },

    #[error("Duplicate file in batch: {file:?}")]
    DuplicateFile { file: PathBuf },

    #[error("File {referencing_file:?} references {referenced_file:?} which is not in the batch")]
    MissingBatchReference {
        referencing_file: PathBuf,
        referenced_file: PathBuf,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum QCIssueError {
    #[error(transparent)]
    GitRepositoryError(#[from] GitRepositoryError),
    #[error(transparent)]
    GitFileOpsError(#[from] GitFileOpsError),
    #[error(transparent)]
    GitHubApiError(#[from] GitHubApiError),
    #[error("Dependency resolution failed: {errors:?}")]
    DependencyResolution { errors: Vec<DependencyError> },
    #[error("Entry not found for file: {file:?}")]
    EntryNotFound { file: PathBuf },
    #[error("Unresolved NewIssue reference: {file:?} referenced by {referencing_file:?}")]
    UnresolvedReference {
        file: PathBuf,
        referencing_file: PathBuf,
    },
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
                Some("NOTE"),
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
        issues_by_number: Arc<Mutex<HashMap<u64, octocrab::models::issues::Issue>>>,
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
                issues_by_number: Arc::new(Mutex::new(issues_by_number)),
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
            issue: &QCIssue,
        ) -> impl std::future::Future<
            Output = Result<octocrab::models::issues::Issue, GitHubApiError>,
        > + Send {
            let url = self.post_issue_url.clone();
            let issues_map = self.issues_by_number.clone();
            let body = issue.body(self);
            let title = issue.title();

            async move {
                // Parse issue number from URL
                let issue_number = url.split('/').last().unwrap().parse::<u64>().unwrap();

                // Create the issue using shared test helper
                let created_issue = crate::test_utils::create_test_issue(
                    "owner",
                    "repo",
                    issue_number,
                    &title,
                    &body,
                    None,   // milestone
                    "open", // state
                );

                // Store it so get_issue can find it
                issues_map
                    .lock()
                    .unwrap()
                    .insert(issue_number, created_issue.clone());

                Ok(created_issue)
            }
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
                    .lock()
                    .unwrap()
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

        fn get_current_user(
            &self,
        ) -> impl std::future::Future<Output = Result<Option<String>, GitHubApiError>> + Send
        {
            async move { Ok(None) }
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

    fn make_entry(title: &str, relevant_files: Vec<RelevantFileEntry>) -> QCEntry {
        QCEntry {
            title: PathBuf::from(title),
            checklist: Checklist::new("Test".to_string(), None, "- [ ] test".to_string()),
            assignees: vec![],
            relevant_files,
        }
    }

    #[test]
    fn test_linear_dependency() {
        // A blocks B blocks C
        // Expected order: [A, B, C]
        let entries = vec![
            make_entry(
                "C",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("B"),
                    relationship: QCRelationship::PreviousQC,
                    description: None,
                }],
            ),
            make_entry(
                "B",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("A"),
                    relationship: QCRelationship::GatingQC,
                    description: None,
                }],
            ),
            make_entry("A", vec![]),
        ];

        let result = resolve_creation_order(&entries);

        assert!(result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert_eq!(result.creation_order.len(), 3);

        // A must come before B, B must come before C
        let a_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("A"))
            .unwrap();
        let b_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("B"))
            .unwrap();
        let c_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("C"))
            .unwrap();

        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn test_diamond_dependency() {
        // A blocks B and C, B and C block D
        // Expected: A first, D last
        let entries = vec![
            make_entry(
                "D",
                vec![
                    RelevantFileEntry::NewIssue {
                        file_path: PathBuf::from("B"),
                        relationship: QCRelationship::PreviousQC,
                        description: None,
                    },
                    RelevantFileEntry::NewIssue {
                        file_path: PathBuf::from("C"),
                        relationship: QCRelationship::GatingQC,
                        description: None,
                    },
                ],
            ),
            make_entry(
                "B",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("A"),
                    relationship: QCRelationship::PreviousQC,
                    description: None,
                }],
            ),
            make_entry(
                "C",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("A"),
                    relationship: QCRelationship::GatingQC,
                    description: None,
                }],
            ),
            make_entry("A", vec![]),
        ];

        let result = resolve_creation_order(&entries);

        assert!(result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert_eq!(result.creation_order.len(), 4);

        // A must come first
        assert_eq!(result.creation_order[0], PathBuf::from("A"));

        // D must come last
        assert_eq!(result.creation_order[3], PathBuf::from("D"));

        // B and C must come after A but before D
        let b_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("B"))
            .unwrap();
        let c_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("C"))
            .unwrap();
        assert!(b_pos > 0 && b_pos < 3);
        assert!(c_pos > 0 && c_pos < 3);
    }

    #[test]
    fn test_circular_dependency() {
        // A blocks B blocks A
        // Expected: cycle detected
        let entries = vec![
            make_entry(
                "A",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("B"),
                    relationship: QCRelationship::PreviousQC,
                    description: None,
                }],
            ),
            make_entry(
                "B",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("A"),
                    relationship: QCRelationship::GatingQC,
                    description: None,
                }],
            ),
        ];

        let result = resolve_creation_order(&entries);

        assert!(!result.errors.is_empty());
        assert!(!result.cycles.is_empty());
        assert!(result.creation_order.is_empty());

        // Verify error is CircularDependency
        assert!(matches!(
            result.errors[0],
            DependencyError::CircularDependency { .. }
        ));
    }

    #[test]
    fn test_self_reference() {
        // A references A
        // Expected: self-reference error
        let entries = vec![make_entry(
            "A",
            vec![RelevantFileEntry::NewIssue {
                file_path: PathBuf::from("A"),
                relationship: QCRelationship::PreviousQC,
                description: None,
            }],
        )];

        let result = resolve_creation_order(&entries);

        assert!(!result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert!(result.creation_order.is_empty());

        // Verify error is SelfReference
        assert!(matches!(
            result.errors[0],
            DependencyError::SelfReference { .. }
        ));
    }

    #[test]
    fn test_duplicate_file() {
        // Two entries with same file
        // Expected: duplicate error
        let entries = vec![make_entry("A", vec![]), make_entry("A", vec![])];

        let result = resolve_creation_order(&entries);

        assert!(!result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert!(result.creation_order.is_empty());

        // Verify error is DuplicateFile
        assert!(matches!(
            result.errors[0],
            DependencyError::DuplicateFile { .. }
        ));
    }

    #[test]
    fn test_missing_batch_reference() {
        // A references B which is not in the batch
        // Expected: missing batch reference error (caught before creation starts)
        let entries = vec![make_entry(
            "A",
            vec![RelevantFileEntry::NewIssue {
                file_path: PathBuf::from("B"),
                relationship: QCRelationship::GatingQC,
                description: Some("Needs B".to_string()),
            }],
        )];

        let result = resolve_creation_order(&entries);

        assert!(!result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert!(result.creation_order.is_empty());

        // Verify error is MissingBatchReference
        assert!(matches!(
            result.errors[0],
            DependencyError::MissingBatchReference { .. }
        ));
    }

    #[test]
    fn test_relevant_qc_creates_dependency() {
        // A has RelevantQC reference to B
        // Expected: B created before A (for hyperlink)
        // Note: RelevantQC doesn't block approval, but still needs to be created first
        let entries = vec![
            make_entry(
                "A",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("B"),
                    relationship: QCRelationship::RelevantQC,
                    description: Some("Just a reference".to_string()),
                }],
            ),
            make_entry("B", vec![]),
        ];

        let result = resolve_creation_order(&entries);

        assert!(result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert_eq!(result.creation_order.len(), 2);

        // B must come before A (to create the hyperlink)
        let b_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("B"))
            .unwrap();
        let a_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("A"))
            .unwrap();

        assert!(b_pos < a_pos);
    }

    #[test]
    fn test_mixed_existing_and_new_references() {
        // A references existing issue and B (new)
        // B references existing issue
        // C has only a file reference (no issue dependencies)
        // Expected: B created before A, C can be anywhere
        let existing_ref = RelevantFile {
            file_name: PathBuf::from("existing.rs"),
            class: RelevantFileClass::PreviousQC {
                issue_number: 100,
                issue_id: Some(1000),
                description: Some("Existing issue".to_string()),
            },
        };

        let entries = vec![
            make_entry(
                "A",
                vec![
                    RelevantFileEntry::ExistingIssue(existing_ref.clone()),
                    RelevantFileEntry::NewIssue {
                        file_path: PathBuf::from("B"),
                        relationship: QCRelationship::GatingQC,
                        description: None,
                    },
                ],
            ),
            make_entry("B", vec![RelevantFileEntry::ExistingIssue(existing_ref)]),
            make_entry(
                "C",
                vec![RelevantFileEntry::File {
                    file_path: PathBuf::from("C"),
                    justification: "justification".to_string(),
                }],
            ),
        ];

        let result = resolve_creation_order(&entries);

        assert!(result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert_eq!(result.creation_order.len(), 3);

        // B must come before A
        let b_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("B"))
            .unwrap();
        let a_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("A"))
            .unwrap();

        assert!(b_pos < a_pos);
    }

    #[test]
    fn test_complex_cycle_detection() {
        // A blocks B, B blocks C, C blocks A
        // Expected: cycle detected with all three files
        let entries = vec![
            make_entry(
                "A",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("B"),
                    relationship: QCRelationship::PreviousQC,
                    description: None,
                }],
            ),
            make_entry(
                "B",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("C"),
                    relationship: QCRelationship::GatingQC,
                    description: None,
                }],
            ),
            make_entry(
                "C",
                vec![RelevantFileEntry::NewIssue {
                    file_path: PathBuf::from("A"),
                    relationship: QCRelationship::RelevantQC,
                    description: None,
                }],
            ),
        ];

        let result = resolve_creation_order(&entries);

        assert!(!result.errors.is_empty());
        assert!(!result.cycles.is_empty());
        assert!(result.creation_order.is_empty());

        // Verify cycle contains all three files
        let cycle_files: HashSet<PathBuf> = result.cycles[0].files.iter().cloned().collect();
        assert_eq!(cycle_files.len(), 3);
        assert!(cycle_files.contains(&PathBuf::from("A")));
        assert!(cycle_files.contains(&PathBuf::from("B")));
        assert!(cycle_files.contains(&PathBuf::from("C")));
    }

    #[test]
    fn test_no_dependencies() {
        // Three independent entries
        // Expected: all can be created (order doesn't matter)
        let entries = vec![
            make_entry("A", vec![]),
            make_entry("B", vec![]),
            make_entry("C", vec![]),
        ];

        let result = resolve_creation_order(&entries);

        assert!(result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert_eq!(result.creation_order.len(), 3);

        // All three files should be in the result
        let result_set: HashSet<PathBuf> = result.creation_order.iter().cloned().collect();
        assert!(result_set.contains(&PathBuf::from("A")));
        assert!(result_set.contains(&PathBuf::from("B")));
        assert!(result_set.contains(&PathBuf::from("C")));
    }

    #[test]
    fn test_all_relationship_types() {
        // Test that all three relationship types create dependencies
        let entries = vec![
            make_entry(
                "main",
                vec![
                    RelevantFileEntry::NewIssue {
                        file_path: PathBuf::from("previous"),
                        relationship: QCRelationship::PreviousQC,
                        description: None,
                    },
                    RelevantFileEntry::NewIssue {
                        file_path: PathBuf::from("gating"),
                        relationship: QCRelationship::GatingQC,
                        description: None,
                    },
                    RelevantFileEntry::NewIssue {
                        file_path: PathBuf::from("relevant"),
                        relationship: QCRelationship::RelevantQC,
                        description: None,
                    },
                ],
            ),
            make_entry("previous", vec![]),
            make_entry("gating", vec![]),
            make_entry("relevant", vec![]),
        ];

        let result = resolve_creation_order(&entries);

        assert!(result.errors.is_empty());
        assert!(result.cycles.is_empty());
        assert_eq!(result.creation_order.len(), 4);

        // All dependencies must come before main
        let main_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("main"))
            .unwrap();
        let prev_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("previous"))
            .unwrap();
        let gating_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("gating"))
            .unwrap();
        let relevant_pos = result
            .creation_order
            .iter()
            .position(|p| p == &PathBuf::from("relevant"))
            .unwrap();

        assert!(prev_pos < main_pos);
        assert!(gating_pos < main_pos);
        assert!(relevant_pos < main_pos);
    }
}
