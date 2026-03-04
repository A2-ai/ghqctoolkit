use std::{
    fmt,
    path::{Path, PathBuf},
};

use octocrab::models::Milestone;
use serde::Serialize;

use crate::{
    Configuration, GitHubReader, GitInfo, GitRepository, determine_config_dir,
    utils::{EnvProvider, StdEnvProvider},
};

#[derive(Debug, Clone, Serialize)]
struct MilestoneSitRep {
    name: String,
    open: u64,
    closed: u64,
    total: u64,
}

impl From<Milestone> for MilestoneSitRep {
    fn from(milestone: Milestone) -> Self {
        let open = milestone.open_issues.unwrap_or_default() as u64;
        let closed = milestone.closed_issues.unwrap_or_default() as u64;
        Self {
            name: milestone.title,
            open,
            closed,
            total: open + closed,
        }
    }
}

impl fmt::Display for MilestoneSitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} open | {} closed",
            self.name, self.open, self.closed
        )
    }
}

#[derive(Debug, Clone, Serialize)]
struct RepoSitRep {
    path: PathBuf,
    owner: String,
    repo: String,
    remote_url: String,
    branch: Result<String, String>,
    milestones: Result<Vec<(String, MilestoneSitRep)>, String>,
}

impl RepoSitRep {
    async fn new(git_info: GitInfo) -> Self {
        let milestones = match git_info.get_milestones().await {
            Ok(milestones) => {
                let mut milestones = milestones
                    .into_iter()
                    .map(|m| (m.title.clone(), MilestoneSitRep::from(m)))
                    .collect::<Vec<(_, _)>>();

                // sort milestones by highest # open, then name alphabetically
                milestones.sort_by(|(_, a), (_, b)| {
                    a.open.cmp(&b.open).reverse().then(a.name.cmp(&b.name))
                });

                Ok(milestones)
            }
            Err(e) => Err(e.to_string()),
        };
        RepoSitRep {
            remote_url: format!("{}/{}/{}", git_info.base_url, git_info.owner, git_info.repo),
            branch: git_info.branch().map_err(|e| e.to_string()),
            path: git_info
                .repository_path
                .canonicalize()
                .unwrap_or(git_info.repository_path),
            owner: git_info.owner,
            repo: git_info.repo,
            milestones,
        }
    }
}

impl fmt::Display for RepoSitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Directory: {}", self.path.display())?;
        writeln!(
            f,
            "Repository: {}/{} ({})",
            self.owner, self.repo, self.remote_url
        )?;
        match &self.branch {
            Ok(branch) => {
                writeln!(f, "Branch: {branch}")?;
            }
            Err(e) => {
                writeln!(f, "Branch: Failed to determine branch: {e}")?;
            }
        }

        match &self.milestones {
            Ok(milestones) => {
                writeln!(
                    f,
                    "Milestones: {}\n  - {}",
                    milestones.len(),
                    milestones
                        .iter()
                        .map(|(_, m)| m.to_string())
                        .collect::<Vec<_>>()
                        .join("\n  - ")
                )
            }
            Err(e) => writeln!(f, "Milestones: Failed to determine milestones: {e}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ConfigSitRep {
    owner: Option<String>,
    repo: Option<String>,
    remote_url: Option<String>,
    path_exists: bool,
    configuration: Configuration,
}

impl ConfigSitRep {
    fn new(path: impl AsRef<Path>, git_info: Option<&GitInfo>) -> Self {
        let (remote_url, owner, repo) = match git_info {
            Some(g) => (
                Some(format!("{}/{}/{}", g.base_url, g.owner, g.repo)),
                Some(g.owner.clone()),
                Some(g.repo.clone()),
            ),
            None => (None, None, None),
        };

        let mut configuration = Configuration::from_path(path.as_ref());
        configuration.load_checklists();

        Self {
            owner,
            repo,
            remote_url,
            path_exists: path.as_ref().exists(),
            configuration,
        }
    }
}

impl fmt::Display for ConfigSitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Directory: {}{}",
            self.configuration.path.display(),
            if self.path_exists {
                ""
            } else {
                "❌ Directory not found"
            }
        )?;

        if let (Some(owner), Some(repo), Some(remote_url)) =
            (&self.owner, &self.repo, &self.remote_url)
        {
            writeln!(f, "Repository: {}/{} ({})", owner, repo, remote_url)?;
        } else {
            writeln!(f, "Repository: Not determined to be git repository")?;
        }

        let mut checklists = self
            .configuration
            .checklists
            .iter()
            .map(|(name, checklist)| format!("{name}: {} items", checklist.items()))
            .collect::<Vec<_>>();
        checklists.sort_by(|a, b| a.cmp(b));
        writeln!(
            f,
            "Checklists: {}\n  -{}",
            self.configuration.checklists.len(),
            checklists.join("\n  - ")
        )?;

        writeln!(f, "Options:")?;
        let options = &self.configuration.options;
        if let Some(note) = &options.prepended_checklist_note {
            writeln!(
                f,
                "  - Prepended Checklist Note:\n     │ {}",
                note.replace("\n", "\n     │ ")
            )?;
        }
        writeln!(
            f,
            "  - Checklist Display Name:  {}",
            options.checklist_display_name
        )?;
        writeln!(f, "  - Logo Path: {}", options.logo_path.display())?;
        writeln!(
            f,
            "  - Checklist Directory: {}",
            options.checklist_directory.display()
        )?;
        writeln!(
            f,
            "  - Record Template Path: {}",
            options.record_path.display()
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SitRep {
    directory: PathBuf,
    repository: Result<RepoSitRep, String>,
    configuration: ConfigSitRep,
}

impl SitRep {
    pub async fn new(directory: impl AsRef<Path>, config_dir: Option<impl AsRef<Path>>) -> Self {
        let env = StdEnvProvider;
        let repository = match GitInfo::from_path(directory.as_ref(), &env) {
            Ok(git_info) => Ok(RepoSitRep::new(git_info).await),
            Err(e) => Err(e.to_string()),
        };

        let config_dir = determine_config_dir(config_dir.map(|c| c.as_ref().to_path_buf()), &env)
            .unwrap_or(
                PathBuf::from(env.var("HOME").unwrap_or(".".to_string()))
                    .join(".local")
                    .join("share"),
            );
        let config_git_info = GitInfo::from_path(&config_dir, &env).ok();
        let configuration = ConfigSitRep::new(&config_dir, config_git_info.as_ref());

        Self {
            directory: directory.as_ref().to_path_buf(),
            repository,
            configuration,
        }
    }
}

impl fmt::Display for SitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== Repository =====================")?;
        match &self.repository {
            Ok(r) => {
                writeln!(f, "{r}")?;
            }
            Err(e) => {
                writeln!(
                    f,
                    "Failed to determine Git Repository Info for {}: {e}",
                    self.directory.display()
                )?;
            }
        }

        writeln!(f, "=== Configuration ==================")?;
        writeln!(f, "{}", self.configuration)
    }
}
