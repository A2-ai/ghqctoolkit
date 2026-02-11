//! Git provider trait combining all git operations.

use super::{GitCommitAnalysis, GitFileOps, GitHelpers, GitRepository, GitStatusOps};
use super::api::{GitHubReader, GitHubWriter};

/// Super-trait combining all git/GitHub operations.
///
/// This trait enables dependency injection for testing by allowing both
/// GitInfo (production) and MockGitInfo (tests) to be used interchangeably.
pub trait GitProvider:
    GitHubReader
    + GitHubWriter
    + GitHelpers
    + GitRepository
    + GitFileOps
    + GitStatusOps
    + GitCommitAnalysis
    + Clone
    + Send
    + Sync
{}

// Blanket implementation: any type implementing all traits automatically implements GitProvider
impl<T> GitProvider for T
where
    T: GitHubReader
        + GitHubWriter
        + GitHelpers
        + GitRepository
        + GitFileOps
        + GitStatusOps
        + GitCommitAnalysis
        + Clone
        + Send
        + Sync
{}
