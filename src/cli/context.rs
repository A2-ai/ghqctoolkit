use anyhow::{anyhow, Result};
use std::path::PathBuf;

use crate::{
    cli::interactive::{
        prompt_assignees, prompt_checklist, prompt_file, prompt_milestone, prompt_relevant_files,
    }, configuration::Checklist, create::validate_assignees, Configuration, GitHubApi, GitInfo, MilestoneStatus, RelevantFile, RepoUser
};

pub struct CliContext {
    pub file: PathBuf,
    pub milestone_status: MilestoneStatus,
    pub checklist: Checklist,
    pub assignees: Vec<String>,
    pub relevant_files: Vec<RelevantFile>,
    pub configuration: Configuration,
    pub git_info: GitInfo,
}

impl CliContext {
    pub async fn from_interactive(
        project_dir: &PathBuf,
        configuration: Configuration,
        git_info: GitInfo,
    ) -> Result<Self> {
        println!("🚀 Welcome to GHQC Interactive Mode!");
        // Fetch users once for validation and interactive prompts
        let repo_users: Vec<RepoUser> = git_info.get_users().await?;

        // Interactive prompts
        let milestone_status = prompt_milestone(&git_info).await?;
        let file = prompt_file(project_dir)?;
        let checklist = prompt_checklist(&configuration)?;
        let assignees = prompt_assignees(&repo_users)?;
        let relevant_files = prompt_relevant_files(project_dir)?;

        // Display summary
        println!("\n✨ Creating issue with:");
        println!("   📊 Milestone: {}", milestone_status);
        println!("   📁 File: {}", file.display());
        println!("   📋 Checklist: {}", checklist);
        if !assignees.is_empty() {
            println!("   👥 Assignees: {}", assignees.join(", "));
        }
        if !relevant_files.is_empty() {
            println!(
                "   🔗 Relevant files: {}",
                relevant_files
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        println!();

        Ok(Self {
            file,
            milestone_status,
            checklist,
            assignees,
            relevant_files,
            configuration,
            git_info,
        })
    }

    pub async fn from_args(
        milestone: String,
        file: PathBuf,
        checklist_name: String,
        assignees: Option<Vec<String>>,
        relevant_files: Option<Vec<RelevantFile>>,
        configuration: Configuration,
        git_info: GitInfo,
    ) -> Result<Self> {
        let final_assignees = assignees.unwrap_or_default();
        let final_relevant_files = relevant_files.unwrap_or_default();

        // Fetch users for validation
        let repo_users: Vec<RepoUser> = git_info.get_users().await?;

        // Validate assignees if provided
        validate_assignees(&final_assignees, &repo_users)?;

        // Get selected checklist
        let checklist = configuration
            .checklists
            .get(&checklist_name)
            .ok_or(anyhow!("No checklist named {checklist_name}"))?
            .clone();

        Ok(Self {
            file,
            milestone_status: MilestoneStatus::Unknown(milestone),
            checklist,
            assignees: final_assignees,
            relevant_files: final_relevant_files,
            configuration,
            git_info,
        })
    }
}
