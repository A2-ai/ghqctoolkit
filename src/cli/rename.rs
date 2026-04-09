use anyhow::Result;
use inquire::Confirm;
use octocrab::models::issues::Issue;
use octocrab::models::Milestone;
use std::path::PathBuf;

use crate::cli::interactive::prompt_existing_milestone;
use crate::comment_system::CommentBody;
use crate::git::{GitFileOps, GitHelpers, GitHubApiError};
use crate::{
    FileRenameEvent, GitProvider, detect_renames, file_history_section, head_commit_hash,
    parse_file_history, splice_file_history,
};

/// CommentBody for posting a rename confirmation to the issue timeline.
struct RenameComment {
    issue: Issue,
    old_path: String,
    new_path: String,
    commit: String,
}

impl CommentBody for RenameComment {
    fn generate_body(&self, _git_info: &(impl GitHelpers + GitFileOps)) -> String {
        format!(
            "# QC File Rename\n`{}` \u{2192} `{}` (commit: {})",
            self.old_path, self.new_path, self.commit
        )
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }

    fn title(&self) -> &str {
        "QC File Rename"
    }
}

/// Alert-only: detect renamed files across the given issues and print a warning
/// if any are found, directing the user to run `ghqc issue rename`.
/// Returns the number of detected renames (without confirming any).
pub async fn alert_renames<G: GitProvider + 'static>(
    git_info: &G,
    issues: &[Issue],
) -> Result<usize> {
    let repo_path = git_info.path().to_path_buf();

    let open_issues: Vec<&Issue> = issues
        .iter()
        .filter(|i| matches!(i.state, octocrab::models::IssueState::Open))
        .collect();

    if open_issues.is_empty() {
        return Ok(0);
    }

    let issue_paths: Vec<PathBuf> = open_issues
        .iter()
        .map(|i| PathBuf::from(&i.title))
        .collect();

    let renames = tokio::task::spawn_blocking({
        let repo_path = repo_path.clone();
        move || detect_renames(&repo_path, &issue_paths)
    })
    .await?;

    if renames.is_empty() {
        return Ok(0);
    }

    println!();
    println!("⚠️  Detected {} file rename(s):", renames.len());
    for (old_path, new_path) in &renames {
        if let Some(issue) = open_issues.iter().find(|i| PathBuf::from(&i.title) == *old_path) {
            println!(
                "  `{}` → `{}` (issue #{})",
                old_path.display(),
                new_path.display(),
                issue.number
            );
        }
    }
    println!("  Run `ghqc issue rename` to confirm.");

    Ok(renames.len())
}

/// Interactive rename command: prompt for a milestone, detect renames, and
/// confirm each one interactively. Returns the number of renames confirmed.
pub async fn interactive_rename<G: GitProvider + 'static>(
    milestones: &[Milestone],
    git_info: &G,
) -> Result<()> {
    let milestone = if milestones.len() == 1 {
        milestones[0].clone()
    } else {
        prompt_existing_milestone(milestones)?
    };

    let issues = git_info.get_issues(Some(milestone.number as u64)).await?;
    if issues.is_empty() {
        println!("No issues found in milestone '{}'.", milestone.title);
        return Ok(());
    }

    let repo_path = git_info.path().to_path_buf();

    let open_issues: Vec<&Issue> = issues
        .iter()
        .filter(|i| matches!(i.state, octocrab::models::IssueState::Open))
        .collect();

    if open_issues.is_empty() {
        println!("No open issues found in milestone '{}'.", milestone.title);
        return Ok(());
    }

    let issue_paths: Vec<PathBuf> = open_issues
        .iter()
        .map(|i| PathBuf::from(&i.title))
        .collect();

    let renames = tokio::task::spawn_blocking({
        let repo_path = repo_path.clone();
        move || detect_renames(&repo_path, &issue_paths)
    })
    .await?;

    if renames.is_empty() {
        println!("No file renames detected for open issues in '{}'.", milestone.title);
        return Ok(());
    }

    println!();
    println!("⚠️  Detected {} file rename(s):", renames.len());

    let mut confirmed = 0;
    for (old_path, new_path) in &renames {
        let issue = match open_issues
            .iter()
            .find(|i| PathBuf::from(&i.title) == *old_path)
        {
            Some(i) => *i,
            None => continue,
        };

        println!(
            "  `{}` → `{}` (issue #{})",
            old_path.display(),
            new_path.display(),
            issue.number
        );

        let answer = Confirm::new(&format!(
            "Update issue #{} title and record rename in body?",
            issue.number
        ))
        .with_default(true)
        .prompt()?;

        if !answer {
            println!("  Skipped.");
            continue;
        }

        if let Err(e) = confirm_rename(git_info, issue, old_path, new_path, &repo_path).await {
            eprintln!("  ✗ Failed to confirm rename: {e}");
        } else {
            println!("  ✓ Issue #{} updated.", issue.number);
            confirmed += 1;
        }
    }

    if confirmed > 0 {
        println!();
        println!("✅ Confirmed {confirmed} rename(s).");
    }

    Ok(())
}

/// Non-interactive rename: auto-detect where `old_path` was renamed to and confirm without prompting.
pub async fn confirm_rename_noninteractive<G: GitProvider + 'static>(
    git_info: &G,
    raw_issue: &Issue,
    old_path: &PathBuf,
) -> Result<()> {
    let repo_path = git_info.path().to_path_buf();
    let old_path_clone = old_path.clone();
    let renames = tokio::task::spawn_blocking(move || {
        detect_renames(&repo_path, &[old_path_clone])
    })
    .await?;

    let new_path = renames
        .into_iter()
        .find(|(old, _)| old == old_path)
        .map(|(_, new)| new)
        .ok_or_else(|| anyhow::anyhow!("No rename detected for '{}'", old_path.display()))?;

    let repo_path = git_info.path().to_path_buf();
    confirm_rename(git_info, raw_issue, old_path, &new_path, &repo_path)
        .await
        .map_err(anyhow::Error::from)
}

async fn confirm_rename<G: GitProvider>(
    git_info: &G,
    raw_issue: &Issue,
    old_path: &PathBuf,
    new_path: &PathBuf,
    repo_path: &std::path::Path,
) -> Result<(), GitHubApiError> {
    let old_path_str = old_path.to_string_lossy().to_string();
    let new_path_str = new_path.to_string_lossy().to_string();
    let current_body = raw_issue.body.as_deref().unwrap_or("").to_string();

    let repo_path = repo_path.to_path_buf();
    let commit_hash = tokio::task::spawn_blocking(move || {
        head_commit_hash(&repo_path).unwrap_or_else(|| "unknown".to_string())
    })
    .await
    .unwrap_or_else(|_| "unknown".to_string());

    let mut events = parse_file_history(&current_body);
    events.push(FileRenameEvent {
        old_path: old_path_str.clone(),
        new_path: new_path_str.clone(),
        commit: commit_hash.clone(),
    });
    let history_section = file_history_section(&events);
    let new_body = splice_file_history(&current_body, &history_section);

    git_info
        .update_issue(raw_issue.number as u64, Some(new_path_str.clone()), Some(new_body))
        .await?;

    log::info!(
        "Renamed issue #{}: {:?} → {:?} (commit {})",
        raw_issue.number,
        old_path_str,
        new_path_str,
        commit_hash
    );

    let comment = RenameComment {
        issue: raw_issue.clone(),
        old_path: old_path_str,
        new_path: new_path_str,
        commit: commit_hash,
    };
    if let Err(e) = git_info.post_comment(&comment).await {
        log::warn!(
            "Failed to post rename comment to issue #{}: {e}",
            raw_issue.number
        );
    }

    Ok(())
}
