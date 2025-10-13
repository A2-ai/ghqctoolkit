use gix::ObjectId;
#[cfg(test)]
use mockall::automock;

use crate::GitInfo;

#[derive(thiserror::Error, Debug)]
pub enum GitCommitAnalysisError {
    #[error("Failed to walk revision history: {0}")]
    RevWalkError(gix::revision::walk::Error),
    #[error("Failed to traverse commits: {0}")]
    TraverseError(gix::revision::walk::iter::Error),
    #[error("Failed to find git object: {0}")]
    ObjectError(gix::object::find::existing::Error),
    #[error("Failed to parse commit: {0}")]
    CommitError(gix::object::try_into::Error),
    #[error("Failed to access repository: {0}")]
    RepositoryError(#[from] crate::git::GitInfoError),
}

/// Advanced commit analysis and branch operations
#[cfg_attr(test, automock)]
pub trait GitCommitAnalysis {
    /// Get all merge commits in the repository
    fn get_all_merge_commits(&self) -> Result<Vec<ObjectId>, GitCommitAnalysisError>;

    /// Get parent commits for a specific commit
    fn get_commit_parents(
        &self,
        commit: &ObjectId,
    ) -> Result<Vec<ObjectId>, GitCommitAnalysisError>;

    /// Check if one commit is an ancestor of another
    fn is_ancestor(
        &self,
        ancestor: &ObjectId,
        descendant: &ObjectId,
    ) -> Result<bool, GitCommitAnalysisError>;

    /// Get all branches that contain a specific commit
    fn get_branches_containing_commit(
        &self,
        commit: &ObjectId,
    ) -> Result<Vec<String>, GitCommitAnalysisError>;
}

impl GitCommitAnalysis for GitInfo {
    fn get_all_merge_commits(&self) -> Result<Vec<gix::ObjectId>, GitCommitAnalysisError> {
        log::debug!("Finding all merge commits in repository");

        let repo = self.repository()?;
        let mut merge_commits = Vec::new();
        // Get all references and walk from all of them to ensure we see all merge commits
        let mut start_points: Vec<gix::ObjectId> = Vec::new();

        // Add HEAD
        if let Ok(head_id) = repo.head_id() {
            start_points.push(head_id.into());
        }

        // Add all local and remote branch tips
        if let Ok(refs) = repo.references() {
            if let Ok(all_refs) = refs.all() {
                for reference_result in all_refs {
                    if let Ok(reference) = reference_result {
                        if let Some(target) = reference.target().try_id() {
                            start_points.push(target.to_owned());
                        }
                    }
                }
            }
        }

        // Ensure we have at least HEAD to walk from
        if start_points.is_empty() {
            let head_id = repo.head_id().map_err(|_| {
                GitCommitAnalysisError::ObjectError(gix::object::find::existing::Error::NotFound {
                    oid: gix::ObjectId::empty_tree(gix::hash::Kind::Sha1),
                })
            })?;
            start_points.push(head_id.into());
        }

        let revwalk = repo.rev_walk(start_points);

        for commit_info in revwalk
            .all()
            .map_err(GitCommitAnalysisError::RevWalkError)?
        {
            let commit_info = commit_info.map_err(GitCommitAnalysisError::TraverseError)?;
            let commit_id = commit_info.id;

            let commit = repo
                .find_object(commit_id)
                .map_err(GitCommitAnalysisError::ObjectError)?
                .try_into_commit()
                .map_err(GitCommitAnalysisError::CommitError)?;

            // Check if this is a merge commit (has multiple parents)
            if commit.parent_ids().count() > 1 {
                merge_commits.push(commit_id);
            }
        }

        log::debug!("Found {} merge commits", merge_commits.len());
        Ok(merge_commits)
    }

    fn get_commit_parents(
        &self,
        commit: &gix::ObjectId,
    ) -> Result<Vec<gix::ObjectId>, GitCommitAnalysisError> {
        let repo = self.repository()?;
        let commit_obj = repo
            .find_object(*commit)
            .map_err(GitCommitAnalysisError::ObjectError)?
            .try_into_commit()
            .map_err(GitCommitAnalysisError::CommitError)?;

        Ok(commit_obj.parent_ids().map(|id| id.detach()).collect())
    }

    fn is_ancestor(
        &self,
        ancestor: &gix::ObjectId,
        descendant: &gix::ObjectId,
    ) -> Result<bool, GitCommitAnalysisError> {
        // Walk from descendant to see if we can reach ancestor
        let repo = self.repository()?;
        let revwalk = repo.rev_walk([*descendant]);

        for commit_info in revwalk
            .all()
            .map_err(GitCommitAnalysisError::RevWalkError)?
        {
            let commit_info = commit_info.map_err(GitCommitAnalysisError::TraverseError)?;
            if commit_info.id == *ancestor {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn get_branches_containing_commit(
        &self,
        commit: &gix::ObjectId,
    ) -> Result<Vec<String>, GitCommitAnalysisError> {
        log::debug!("Finding branches containing commit {}", commit);

        let repo = self.repository()?;
        let Ok(refs) = repo.references() else {
            return Ok(Vec::new());
        };

        let Ok(all_refs) = refs.all() else {
            return Ok(Vec::new());
        };

        let branches = all_refs
            .filter_map(Result::ok)
            .map(|r| (r.name().as_bstr().to_string(), r))
            .filter(|(name, _)| {
                name.starts_with("refs/heads/") || name.starts_with("refs/remotes/")
            })
            .filter(|(_, r)| {
                if let Some(id) = r.target().try_id() {
                    self.is_ancestor(commit, &id.into()).unwrap_or_default()
                } else {
                    false
                }
            })
            .map(|(name, _)| {
                name.strip_prefix("refs/heads/")
                    .or(name.strip_prefix("refs/remotes/"))
                    .unwrap_or(&name)
                    .to_string()
            })
            .collect::<Vec<_>>();

        log::debug!(
            "Found {} branches containing commit {}",
            branches.len(),
            commit
        );
        Ok(branches)
    }
}
