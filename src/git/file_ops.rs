use std::{
    fmt,
    path::{Path, PathBuf},
};

use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

use crate::{GitInfo, git::GitCommitAnalysis};

#[derive(Debug, Clone)]
pub struct GitAuthor {
    pub(crate) name: String,
    pub(crate) email: String,
}

impl fmt::Display for GitAuthor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.email)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GitCommit {
    pub commit: ObjectId,
    pub message: String,
    pub files: Vec<PathBuf>,
}

#[derive(thiserror::Error, Debug)]
pub enum GitFileOpsError {
    #[error("Failed to walk revision history: {0}")]
    RevWalkError(gix::revision::walk::Error),
    #[error("Failed to traverse commits: {0}")]
    TraverseError(gix::revision::walk::iter::Error),
    #[error("Failed to find git object: {0}")]
    ObjectError(gix::object::find::existing::Error),
    #[error("Failed to parse commit: {0}")]
    CommitError(gix::object::try_into::Error),
    #[error("Failed to get commit tree: {0}")]
    TreeError(gix::object::commit::Error),
    #[error("Failed to convert object to tree: {0}")]
    ObjectToTreeError(gix::object::try_into::Error),
    #[error("Failed to get signature: {0}")]
    SignatureError(gix::objs::decode::Error),
    #[error("Author not found for file: {0:?}")]
    AuthorNotFound(PathBuf),
    #[error("File not found at commit: {0:?}")]
    FileNotFoundAtCommit(PathBuf),
    #[error("Failed to read file content: {0}")]
    BlobError(gix::object::try_into::Error),
    #[error("Failed to decode file content: {0:?}")]
    EncodingError(PathBuf),
    #[error("Branch not found: {0}")]
    BranchNotFound(String),
    #[error("Failed to get HEAD ID: {0}")]
    HeadIdError(gix::reference::head_id::Error),
    #[error("Failed to access repository: {0}")]
    RepositoryError(#[from] crate::git::GitInfoError),
    #[error("Directory not found in git tree: {0}")]
    DirectoryNotFound(String),
    #[error("Path is not a directory: {0}")]
    NotADirectory(String),
}

/// File-specific git operations
#[cfg_attr(test, automock)]
pub trait GitFileOps {
    /// Get all commits for a branch/reference with the files they touch
    fn commits(&self, branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError>;

    /// Get all authors who have modified a file
    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError>;

    /// Get file bytes at a specific commit
    /// Return bytes to either use in excel reader or convert to string
    fn file_bytes_at_commit(
        &self,
        file: &Path,
        commit: &ObjectId,
    ) -> Result<Vec<u8>, GitFileOpsError>;

    /// List immediate children of `path` in the HEAD commit tree.
    /// `path` is repo-relative, slash-separated, no leading/trailing slash.
    /// Empty string = repo root.
    /// Returns `(name, is_directory)` pairs sorted: dirs first, then files, alpha within each.
    fn list_tree_entries(&self, path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError>;
}

impl GitFileOps for GitInfo {
    fn commits(&self, branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
        log::debug!("Getting all commits for branch: {:?}", branch);
        let repo = self.repository()?;
        let mut commits = Vec::new();

        let start_id = if let Some(branch_name) = branch.as_ref() {
            // Look up the specific branch
            let branch_ref_name = format!("refs/heads/{}", branch_name);
            let branch_ref = repo
                .find_reference(&branch_ref_name)
                .map_err(|_| GitFileOpsError::BranchNotFound(branch_name.clone()))?;
            branch_ref.id()
        } else {
            // Use HEAD as default
            repo.head_id().map_err(GitFileOpsError::HeadIdError)?
        };

        let revwalk = repo.rev_walk([start_id]);

        for commit_info in revwalk.all().map_err(GitFileOpsError::RevWalkError)? {
            let commit_info = commit_info.map_err(GitFileOpsError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit_obj = repo
                .find_object(commit_id)
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_commit()
                .map_err(GitFileOpsError::CommitError)?;

            // Get commit message
            let commit_message = commit_obj
                .message_raw()
                .map(|msg| msg.to_string())
                .unwrap_or(String::new());

            // Get files changed in this commit by comparing with parents
            let mut changed_files = Vec::new();

            if commit_obj.parent_ids().count() == 0 {
                // Initial commit - get all files in the tree recursively
                if let Ok(tree) = commit_obj.tree() {
                    collect_tree_files_recursive(&tree, &mut changed_files, "")?;
                }
            } else {
                // Compare this commit's tree with each parent to find changed files
                let current_tree = commit_obj.tree().map_err(GitFileOpsError::TreeError)?;

                // For merge commits, we'll collect files from all parent comparisons
                for parent_id in commit_obj.parent_ids() {
                    let parent_commit = repo
                        .find_object(parent_id)
                        .map_err(GitFileOpsError::ObjectError)?
                        .try_into_commit()
                        .map_err(GitFileOpsError::CommitError)?;

                    let parent_tree = parent_commit.tree().map_err(GitFileOpsError::TreeError)?;

                    // Find differences between trees recursively
                    let current_files = collect_tree_files_with_oids(&current_tree)?;
                    let parent_files = collect_tree_files_with_oids(&parent_tree)?;

                    // Find differences
                    for (path, oid) in &current_files {
                        match parent_files.get(path) {
                            Some(parent_oid) if parent_oid != oid => {
                                // File modified
                                let file_path = PathBuf::from(path);
                                if !changed_files.contains(&file_path) {
                                    changed_files.push(file_path);
                                }
                            }
                            None => {
                                // File added
                                let file_path = PathBuf::from(path);
                                if !changed_files.contains(&file_path) {
                                    changed_files.push(file_path);
                                }
                            }
                            _ => {
                                // File unchanged
                            }
                        }
                    }

                    // Check for deleted files
                    for (path, _) in &parent_files {
                        if !current_files.contains_key(path) {
                            let file_path = PathBuf::from(path);
                            if !changed_files.contains(&file_path) {
                                changed_files.push(file_path);
                            }
                        }
                    }
                }
            }

            commits.push(GitCommit {
                commit: commit_id,
                message: commit_message,
                files: changed_files,
            });
        }

        log::debug!("Found {} commits for branch: {:?}", commits.len(), branch);
        Ok(commits)
    }

    fn authors(&self, file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
        let repo = self.repository()?;
        let all_commits = self.commits(&None)?;

        // Find commits that touch this file
        let file_commits = find_file_commits(file, &all_commits);

        let mut res: Vec<GitAuthor> = Vec::new();

        for commit in file_commits {
            let commit_obj = repo
                .find_object(commit.commit)
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_commit()
                .map_err(GitFileOpsError::CommitError)?;

            let signature = commit_obj
                .author()
                .map_err(GitFileOpsError::SignatureError)?;
            if !res.iter().any(|author| author.email == signature.email) {
                res.push(GitAuthor {
                    name: signature.name.to_string(),
                    email: signature.email.to_string(),
                });
            }
        }

        if res.is_empty() {
            log::warn!("No authors found for file: {:?}", file);
            Err(GitFileOpsError::AuthorNotFound(file.to_path_buf()))
        } else {
            log::debug!("Found {} unique authors for file: {:?}", res.len(), file);
            Ok(res)
        }
    }

    fn file_bytes_at_commit(
        &self,
        file: &Path,
        commit: &gix::ObjectId,
    ) -> Result<Vec<u8>, GitFileOpsError> {
        let file_path = file;
        log::debug!(
            "Getting file content for {:?} at commit {}",
            file_path,
            commit
        );

        let repo = self.repository()?;

        // Get the commit object
        let commit_obj = repo
            .find_object(*commit)
            .map_err(GitFileOpsError::ObjectError)?
            .try_into_commit()
            .map_err(GitFileOpsError::CommitError)?;

        // Get the tree for this commit
        let tree = commit_obj.tree().map_err(GitFileOpsError::TreeError)?;

        // Look up the file in the tree
        let entry = tree
            .lookup_entry_by_path(file_path)
            .map_err(|_| GitFileOpsError::FileNotFoundAtCommit(file_path.to_path_buf()))?
            .ok_or_else(|| GitFileOpsError::FileNotFoundAtCommit(file_path.to_path_buf()))?;

        // Get the blob object for the file
        let blob = repo
            .find_object(entry.oid())
            .map_err(GitFileOpsError::ObjectError)?
            .try_into_blob()
            .map_err(GitFileOpsError::BlobError)?;

        log::debug!(
            "Successfully read {} bytes from file {:?} at commit {}",
            blob.data.len(),
            file_path,
            commit
        );

        Ok(blob.data.clone())
    }

    fn list_tree_entries(&self, path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
        let repo = self.repository()?;
        let head_id = repo.head_id().map_err(GitFileOpsError::HeadIdError)?;
        let commit_obj = repo
            .find_object(head_id)
            .map_err(GitFileOpsError::ObjectError)?
            .try_into_commit()
            .map_err(GitFileOpsError::CommitError)?;
        let tree = commit_obj.tree().map_err(GitFileOpsError::TreeError)?;

        let mut all_files: Vec<PathBuf> = Vec::new();
        collect_tree_files_recursive(&tree, &mut all_files, "")?;

        let mut components: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();

        for file in &all_files {
            let file_str = file.to_string_lossy();
            let remainder = if path.is_empty() {
                Some(file_str.as_ref().to_string())
            } else {
                let prefix = format!("{}/", path);
                file_str.strip_prefix(prefix.as_str()).map(|s| s.to_string())
            };

            if let Some(rest) = remainder {
                if let Some(slash_pos) = rest.find('/') {
                    let dir_name = rest[..slash_pos].to_string();
                    components.insert(dir_name, true);
                } else {
                    components.entry(rest).or_insert(false);
                }
            }
        }

        if !path.is_empty() && components.is_empty() {
            if all_files.contains(&PathBuf::from(path)) {
                return Err(GitFileOpsError::NotADirectory(path.to_string()));
            } else {
                return Err(GitFileOpsError::DirectoryNotFound(path.to_string()));
            }
        }

        let mut result: Vec<(String, bool)> = components.into_iter().collect();
        result.sort_by(|(name_a, is_dir_a), (name_b, is_dir_b)| match (is_dir_a, is_dir_b) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => name_a.cmp(name_b),
        });

        Ok(result)
    }
}

/// Find commits that touch a specific file
pub fn find_file_commits<P: AsRef<Path>>(file: P, commits: &[GitCommit]) -> Vec<&GitCommit> {
    let file_path = file.as_ref();
    commits
        .iter()
        .filter(|commit| commit.files.iter().any(|f| f == file_path))
        .collect()
}

/// Find which branch a commit was merged into using merge commit analysis
/// Based on the R algorithm: looks for merge commits where the target commit
/// is an ancestor of the second parent (merged-in branch)
fn find_merged_into_branch(
    git_info: &(impl GitFileOps + GitCommitAnalysis),
    target_commit: &gix::ObjectId,
) -> Result<Option<String>, GitFileOpsError> {
    let merge_commits = git_info.get_all_merge_commits().map_err(|e| {
        GitFileOpsError::BranchNotFound(format!("Failed to get merge commits: {}", e))
    })?;

    for merge_commit in merge_commits {
        let parents = git_info.get_commit_parents(&merge_commit).map_err(|e| {
            GitFileOpsError::BranchNotFound(format!("Failed to get commit parents: {}", e))
        })?;

        if parents.len() >= 2 {
            let _parent1 = parents[0]; // Branch that received the merge
            let parent2 = parents[1]; // Branch that was merged in

            // Check if target_commit is ancestor of parent2 (the merged-in branch)
            if git_info.is_ancestor(target_commit, &parent2).map_err(|e| {
                GitFileOpsError::BranchNotFound(format!("Failed to check ancestry: {}", e))
            })? {
                // Find branches that contain the merge commit
                let candidate_branches = git_info
                    .get_branches_containing_commit(&merge_commit)
                    .map_err(|e| {
                        GitFileOpsError::BranchNotFound(format!(
                            "Failed to get branches containing commit: {}",
                            e
                        ))
                    })?;

                // Filter to branches where parent1 is in their ancestry
                for branch in candidate_branches {
                    // Skip remote HEAD references
                    if branch.contains("HEAD") {
                        continue;
                    }

                    // We found a candidate branch, return it
                    // (In a more sophisticated implementation, we might validate further)
                    return Ok(Some(branch));
                }
            }
        }
    }

    Ok(None)
}

/// Recursively collect all files in a tree
fn collect_tree_files_recursive(
    tree: &gix::Tree<'_>,
    files: &mut Vec<PathBuf>,
    path_prefix: &str,
) -> Result<(), GitFileOpsError> {
    for entry in tree.iter() {
        let entry = entry.map_err(|e| GitFileOpsError::TreeError(e.into()))?;

        let entry_name = entry.filename().to_string();
        let full_path = if path_prefix.is_empty() {
            entry_name.clone()
        } else {
            format!("{}/{}", path_prefix, entry_name)
        };

        if entry.mode().is_blob() {
            // This is a file
            files.push(PathBuf::from(full_path));
        } else if entry.mode().is_tree() {
            // This is a directory, recurse into it
            let sub_tree = entry
                .object()
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_tree()
                .map_err(GitFileOpsError::ObjectToTreeError)?;
            collect_tree_files_recursive(&sub_tree, files, &full_path)?;
        }
    }
    Ok(())
}

/// Recursively collect all files in a tree with their OIDs
fn collect_tree_files_with_oids(
    tree: &gix::Tree<'_>,
) -> Result<std::collections::HashMap<String, gix::ObjectId>, GitFileOpsError> {
    let mut files = std::collections::HashMap::new();
    collect_tree_files_with_oids_recursive(tree, &mut files, "")?;
    Ok(files)
}

/// Helper for recursive OID collection
fn collect_tree_files_with_oids_recursive(
    tree: &gix::Tree<'_>,
    files: &mut std::collections::HashMap<String, gix::ObjectId>,
    path_prefix: &str,
) -> Result<(), GitFileOpsError> {
    for entry in tree.iter() {
        let entry = entry.map_err(|e| GitFileOpsError::TreeError(e.into()))?;

        let entry_name = entry.filename().to_string();
        let full_path = if path_prefix.is_empty() {
            entry_name.clone()
        } else {
            format!("{}/{}", path_prefix, entry_name)
        };

        if entry.mode().is_blob() {
            // This is a file
            files.insert(full_path, entry.oid().into());
        } else if entry.mode().is_tree() {
            // This is a directory, recurse into it
            let sub_tree = entry
                .object()
                .map_err(GitFileOpsError::ObjectError)?
                .try_into_tree()
                .map_err(GitFileOpsError::ObjectToTreeError)?;
            collect_tree_files_with_oids_recursive(&sub_tree, files, &full_path)?;
        }
    }
    Ok(())
}

/// Get commits with robust branch handling
/// 1. Try the specified branch first
/// 2. If commit is provided and branch not found, find merged branch using commit analysis
/// 3. Fall back to searching all branches containing the commit
pub fn get_commits_robust(
    git_info: &(impl GitFileOps + GitCommitAnalysis),
    branch: &Option<String>,
    commit: Option<&ObjectId>,
) -> Result<Vec<GitCommit>, GitFileOpsError> {
    // First, try to get commits from the specified branch
    match git_info.commits(branch) {
        Ok(commits) => {
            log::debug!("Found {} commits for branch {:?}", commits.len(), branch);
            return Ok(commits);
        }
        Err(GitFileOpsError::BranchNotFound(_)) if branch.is_some() => {
            log::debug!(
                "Branch {:?} not found locally, searching for merged commits",
                branch
            );
        }
        Err(e) => {
            return Err(e);
        }
    }

    // If we have a commit, try to find which branch it was merged into
    if let Some(commit) = commit {
        log::debug!("Using commit {} to find merged branch for commits", commit);

        // Try to find which branch this commit was merged into
        if let Some(target_branch) = find_merged_into_branch(git_info, commit)? {
            log::debug!(
                "Found that commit {} was merged into branch {}",
                commit,
                target_branch
            );

            // Try to get commits from the target branch
            match git_info.commits(&Some(target_branch.clone())) {
                Ok(commits) => {
                    log::debug!(
                        "Found {} commits for merged target branch {}",
                        commits.len(),
                        target_branch
                    );
                    return Ok(commits);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to get commits from target branch {}: {}",
                        target_branch,
                        e
                    );
                }
            }
        }

        // Fallback: Get commits from branches containing the commit
        let branches_containing_commit =
            git_info
                .get_branches_containing_commit(commit)
                .map_err(|e| {
                    GitFileOpsError::BranchNotFound(format!(
                        "Failed to get branches containing commit: {}",
                        e
                    ))
                })?;

        if !branches_containing_commit.is_empty() {
            log::debug!(
                "Found {} branches containing commit {}: {:?}",
                branches_containing_commit.len(),
                commit,
                branches_containing_commit
            );

            // Try each branch until we find one that works
            for branch_name in branches_containing_commit {
                match git_info.commits(&Some(branch_name.clone())) {
                    Ok(commits) if !commits.is_empty() => {
                        log::debug!(
                            "Found {} commits for branch {} (contains commit)",
                            commits.len(),
                            branch_name
                        );
                        return Ok(commits);
                    }
                    Ok(_) => {
                        log::debug!("Branch {} contains commit but has no commits?", branch_name);
                    }
                    Err(e) => {
                        log::debug!("Failed to get commits from branch {}: {}", branch_name, e);
                    }
                }
            }
        }
    }

    // Final fallback: return error that branch couldn't be found
    if let Some(branch_name) = branch {
        Err(GitFileOpsError::BranchNotFound(format!(
            "Could not find branch '{}' or determine alternative branch from commit",
            branch_name
        )))
    } else {
        Err(GitFileOpsError::BranchNotFound(
            "Could not determine branch from commit".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitCommitAnalysis, GitCommitAnalysisError};
    use std::{collections::HashMap, path::PathBuf, str::FromStr};

    fn create_test_commits() -> Vec<(ObjectId, String)> {
        vec![
            (
                ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap(),
                "Initial commit".to_string(),
            ),
            (
                ObjectId::from_str("def456789abc012345678901234567890123abcd").unwrap(),
                "Second commit".to_string(),
            ),
            (
                ObjectId::from_str("456def789abc012345678901234567890123cdef").unwrap(),
                "Third commit".to_string(),
            ),
            (
                ObjectId::from_str("789abc12def345678901234567890123456789ef").unwrap(),
                "Fourth commit".to_string(),
            ),
            (
                ObjectId::from_str("890cdef123abc456789012345678901234567890").unwrap(),
                "Fifth commit".to_string(),
            ),
            (
                ObjectId::from_str("123abcdef456789012345678901234567890abcd").unwrap(),
                "Sixth commit".to_string(),
            ),
            (
                ObjectId::from_str("abc123456789012345678901234567890123abcd").unwrap(),
                "Seventh commit".to_string(),
            ),
        ]
    }

    // Enhanced MockGitInfo for testing robust branch handling
    struct RobustMockGitInfo {
        file_commits_responses:
            HashMap<Option<String>, Result<Vec<(ObjectId, String)>, GitFileOpsError>>,
        merge_commits: Vec<ObjectId>,
        commit_parents: HashMap<ObjectId, Vec<ObjectId>>,
        ancestor_relationships: HashMap<(ObjectId, ObjectId), bool>,
        branches_containing_commits: HashMap<ObjectId, Vec<String>>,
    }

    impl RobustMockGitInfo {
        fn new() -> Self {
            Self {
                file_commits_responses: HashMap::new(),
                merge_commits: Vec::new(),
                commit_parents: HashMap::new(),
                ancestor_relationships: HashMap::new(),
                branches_containing_commits: HashMap::new(),
            }
        }

        fn with_file_commits_result(
            mut self,
            branch: Option<String>,
            result: Result<Vec<(ObjectId, String)>, GitFileOpsError>,
        ) -> Self {
            self.file_commits_responses.insert(branch, result);
            self
        }

        fn with_merge_commits(mut self, commits: Vec<ObjectId>) -> Self {
            self.merge_commits = commits;
            self
        }

        fn with_commit_parents(mut self, commit: ObjectId, parents: Vec<ObjectId>) -> Self {
            self.commit_parents.insert(commit, parents);
            self
        }

        fn with_ancestor_relationship(
            mut self,
            ancestor: ObjectId,
            descendant: ObjectId,
            is_ancestor: bool,
        ) -> Self {
            self.ancestor_relationships
                .insert((ancestor, descendant), is_ancestor);
            self
        }

        fn with_branches_containing_commit(
            mut self,
            commit: ObjectId,
            branches: Vec<String>,
        ) -> Self {
            self.branches_containing_commits.insert(commit, branches);
            self
        }
    }

    impl GitFileOps for RobustMockGitInfo {
        fn commits(&self, branch: &Option<String>) -> Result<Vec<GitCommit>, GitFileOpsError> {
            match self.file_commits_responses.get(branch) {
                Some(Ok(commits)) => Ok(commits
                    .iter()
                    .map(|(commit, message)| GitCommit {
                        commit: *commit,
                        message: message.clone(),
                        files: vec![PathBuf::from("test_file.rs")], // Default test file
                    })
                    .collect()),
                Some(Err(GitFileOpsError::BranchNotFound(branch_name))) => {
                    Err(GitFileOpsError::BranchNotFound(branch_name.clone()))
                }
                Some(Err(_e)) => Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))), // Fallback error for testing
                None => Ok(Vec::new()),
            }
        }

        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn file_bytes_at_commit(
            &self,
            _file: &Path,
            _commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            Ok(Vec::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    impl GitCommitAnalysis for RobustMockGitInfo {
        fn get_all_merge_commits(&self) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(self.merge_commits.clone())
        }

        fn get_commit_parents(
            &self,
            commit: &ObjectId,
        ) -> Result<Vec<ObjectId>, GitCommitAnalysisError> {
            Ok(self.commit_parents.get(commit).cloned().unwrap_or_default())
        }

        fn is_ancestor(
            &self,
            ancestor: &ObjectId,
            descendant: &ObjectId,
        ) -> Result<bool, GitCommitAnalysisError> {
            Ok(self
                .ancestor_relationships
                .get(&(*ancestor, *descendant))
                .copied()
                .unwrap_or(false))
        }

        fn get_branches_containing_commit(
            &self,
            commit: &ObjectId,
        ) -> Result<Vec<String>, GitCommitAnalysisError> {
            Ok(self
                .branches_containing_commits
                .get(commit)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn test_get_commits_robust_success_on_first_try() {
        let test_commits = create_test_commits();
        let branch = Some("feature-branch".to_string());
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            .with_file_commits_result(branch.clone(), Ok(test_commits.clone()));

        let result = get_commits_robust(&git_info, &branch, Some(&initial_commit)).unwrap();

        assert_eq!(result.len(), test_commits.len());
        // Convert result to expected format for comparison
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }

    #[tokio::test]
    async fn test_get_commits_robust_branch_not_found_uses_merge_detection() {
        let test_commits = create_test_commits();
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let merge_commit = ObjectId::from_str("1234567890abcdef123456789012345678901234").unwrap();
        let parent1 = ObjectId::from_str("2345678901234567890123456789012345678901").unwrap();
        let parent2 = ObjectId::from_str("3456789012345678901234567890123456789012").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // Merge detection finds the target branch
            .with_merge_commits(vec![merge_commit])
            .with_commit_parents(merge_commit, vec![parent1, parent2])
            .with_ancestor_relationship(initial_commit, parent2, true) // initial_commit is ancestor of parent2 (merged branch)
            .with_branches_containing_commit(merge_commit, vec!["main".to_string()])
            // Target branch has the commits
            .with_file_commits_result(Some("main".to_string()), Ok(test_commits.clone()));

        let result =
            get_commits_robust(&git_info, &Some(branch.to_string()), Some(&initial_commit))
                .unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }

    #[tokio::test]
    async fn test_get_commits_robust_fallback_to_branches_containing_commit() {
        let test_commits = create_test_commits();
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // No merge commits found
            .with_merge_commits(vec![])
            // But initial commit is found in some branches
            .with_branches_containing_commit(
                initial_commit,
                vec!["main".to_string(), "develop".to_string()],
            )
            // First branch with file commits wins
            .with_file_commits_result(Some("main".to_string()), Ok(test_commits.clone()))
            .with_file_commits_result(Some("develop".to_string()), Ok(Vec::new()));

        let result =
            get_commits_robust(&git_info, &Some(branch.to_string()), Some(&initial_commit))
                .unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }

    #[tokio::test]
    async fn test_get_commits_robust_final_fallback_fails() {
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // No merge commits found
            .with_merge_commits(vec![])
            // No branches contain the commit
            .with_branches_containing_commit(initial_commit, vec![]);

        let result =
            get_commits_robust(&git_info, &Some(branch.to_string()), Some(&initial_commit));

        // Should fail when no branch can be found
        assert!(matches!(result, Err(GitFileOpsError::BranchNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_commits_robust_no_initial_commit_in_issue_body() {
        let branch = "deleted-branch";
        let invalid_commit =
            ObjectId::from_str("0000000000000000000000000000000000000000").unwrap();

        let git_info = RobustMockGitInfo::new().with_file_commits_result(
            Some(branch.to_string()),
            Err(GitFileOpsError::BranchNotFound(branch.to_string())),
        );

        let result =
            get_commits_robust(&git_info, &Some(branch.to_string()), Some(&invalid_commit));

        // Should fail since no branches can be found and all fallbacks fail
        assert!(matches!(result, Err(GitFileOpsError::BranchNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_commits_robust_git_error_propagated() {
        let branch = "test-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();

        let git_info = RobustMockGitInfo::new().with_file_commits_result(
            Some(branch.to_string()),
            Err(GitFileOpsError::AuthorNotFound(PathBuf::from("test"))),
        );

        let result =
            get_commits_robust(&git_info, &Some(branch.to_string()), Some(&initial_commit));

        assert!(matches!(result, Err(GitFileOpsError::AuthorNotFound(_))));
    }

    #[tokio::test]
    async fn test_get_commits_robust_multiple_merge_commits() {
        let test_commits = create_test_commits();
        let branch = "deleted-branch";
        let initial_commit =
            ObjectId::from_str("abc123def456789012345678901234567890abcd").unwrap();
        let merge_commit1 = ObjectId::from_str("1111111111111111111111111111111111111111").unwrap();
        let merge_commit2 = ObjectId::from_str("2222222222222222222222222222222222222222").unwrap();
        let parent1_1 = ObjectId::from_str("3333333333333333333333333333333333333333").unwrap();
        let parent2_1 = ObjectId::from_str("4444444444444444444444444444444444444444").unwrap();
        let parent1_2 = ObjectId::from_str("5555555555555555555555555555555555555555").unwrap();
        let parent2_2 = ObjectId::from_str("6666666666666666666666666666666666666666").unwrap();

        let git_info = RobustMockGitInfo::new()
            // Original branch fails
            .with_file_commits_result(
                Some(branch.to_string()),
                Err(GitFileOpsError::BranchNotFound(branch.to_string())),
            )
            // Multiple merge commits
            .with_merge_commits(vec![merge_commit1, merge_commit2])
            .with_commit_parents(merge_commit1, vec![parent1_1, parent2_1])
            .with_commit_parents(merge_commit2, vec![parent1_2, parent2_2])
            // First merge commit doesn't match
            .with_ancestor_relationship(initial_commit, parent2_1, false)
            // Second merge commit matches
            .with_ancestor_relationship(initial_commit, parent2_2, true)
            .with_branches_containing_commit(merge_commit2, vec!["develop".to_string()])
            // Target branch has the commits
            .with_file_commits_result(Some("develop".to_string()), Ok(test_commits.clone()));

        let result =
            get_commits_robust(&git_info, &Some(branch.to_string()), Some(&initial_commit))
                .unwrap();

        assert_eq!(result.len(), test_commits.len());
        let result_tuples: Vec<(ObjectId, String)> = result
            .iter()
            .map(|c| (c.commit, c.message.clone()))
            .collect();
        assert_eq!(result_tuples, test_commits);
    }
}
