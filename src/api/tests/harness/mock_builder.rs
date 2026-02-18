use std::path::PathBuf;
use std::str::FromStr;

use super::loader::LoadedFixtures;
use super::types::{GitState, GitStatusSpec};
use crate::GitStatus;
use crate::api::tests::helpers::MockGitInfo;
use gix::ObjectId;

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

        // Set git status
        if let Some(status_spec) = &git_state.status {
            builder = builder.with_status(Self::convert_status(status_spec));
        }

        builder.build()
    }

    /// Convert GitStatusSpec to GitStatus
    fn convert_status(spec: &GitStatusSpec) -> GitStatus {
        match spec {
            GitStatusSpec::Clean => GitStatus::Clean,
            GitStatusSpec::Ahead { commits } => GitStatus::Ahead(Self::parse_object_ids(commits)),
            GitStatusSpec::Behind { commits } => GitStatus::Behind(Self::parse_object_ids(commits)),
            GitStatusSpec::Diverged { ahead, behind } => GitStatus::Diverged {
                ahead: Self::parse_object_ids(ahead),
                behind: Self::parse_object_ids(behind),
            },
        }
    }

    /// Parse commit hash strings into ObjectIds
    fn parse_object_ids(hashes: &[String]) -> Vec<ObjectId> {
        hashes
            .iter()
            .map(|h| {
                ObjectId::from_str(h)
                    .unwrap_or_else(|_| ObjectId::empty_tree(gix::hash::Kind::Sha1))
            })
            .collect()
    }
}
