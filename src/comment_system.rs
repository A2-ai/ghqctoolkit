use octocrab::models::issues::Issue;

use crate::git::{GitFileOps, GitHelpers};

/// Trait for generating comment bodies for GitHub issues
///
/// This trait abstracts comment generation across all comment types (notifications,
/// approvals, unapprovals, reviews) providing a consistent interface for the
/// unified comment posting system.
pub trait CommentBody {
    /// Generate the comment body text that will be posted to GitHub
    ///
    /// All comment types require GitHelpers + GitFileOps for generating metadata like
    /// commit links and file URLs, and for accessing file contents at commits.
    fn generate_body(&self, git_info: &(impl GitHelpers + GitFileOps)) -> String;

    /// Get the GitHub issue this comment is associated with
    fn issue(&self) -> &Issue;
}
