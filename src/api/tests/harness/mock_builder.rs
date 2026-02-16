use std::path::PathBuf;

use super::loader::LoadedFixtures;
use super::types::GitState;
use crate::api::tests::helpers::MockGitInfo;

/// Builds MockGitInfo from test specification
pub struct MockBuilder;

impl MockBuilder {
    /// Build a MockGitInfo instance from git state and loaded fixtures
    pub fn build(git_state: &GitState, fixtures: &LoadedFixtures) -> MockGitInfo {
        let mut builder = MockGitInfo::builder()
            .with_owner(&git_state.owner)
            .with_repo(&git_state.repo)
            .with_commit(&git_state.commit)
            .with_branch(&git_state.branch);

        // Add all issues
        for (number, issue) in &fixtures.issues {
            builder = builder.with_issue(*number, issue.clone());
        }

        // Add all milestones
        for (_number, milestone) in &fixtures.milestones {
            builder = builder.with_milestone(milestone.clone());
        }

        // Add users (for assignees endpoint)
        if !fixtures.users.is_empty() {
            builder = builder.with_users(fixtures.users.clone());
        }

        // Add blocking relationships
        for blocking in &fixtures.blocking {
            let blocked_issues: Vec<_> = blocking
                .blocks
                .iter()
                .filter_map(|num| fixtures.issues.get(num).cloned())
                .collect();
            builder = builder.with_blocked_issues(blocking.issue, blocked_issues);
        }

        // Add dirty files
        for file in &git_state.dirty_files {
            builder = builder.with_dirty_file(PathBuf::from(file));
        }

        builder.build()
    }
}
