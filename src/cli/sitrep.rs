use std::{
    fmt,
    path::{Path, PathBuf},
};

use octocrab::models::Milestone;
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::{
    AuthSources, AuthStore, Configuration, GitHubReader, GitInfo, GitRepository,
    determine_config_dir, extract_host_from_base_url,
    utils::{EnvProvider, StdEnvProvider},
};

#[derive(Debug, Clone, Serialize)]
struct MilestoneSitRep {
    name: String,
    is_open: bool,
    open: u64,
    closed: u64,
    total: u64,
}

impl From<Milestone> for MilestoneSitRep {
    fn from(milestone: Milestone) -> Self {
        let open = milestone.open_issues.unwrap_or_default() as u64;
        let closed = milestone.closed_issues.unwrap_or_default() as u64;
        let is_open = milestone.state.as_deref() != Some("closed");
        Self {
            name: milestone.title,
            is_open,
            open,
            closed,
            total: open + closed,
        }
    }
}

impl fmt::Display for MilestoneSitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.is_open { "open" } else { "closed" };
        write!(
            f,
            "{} [{}]: {} open | {} closed",
            self.name, state, self.open, self.closed
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
        writeln!(f, "{} {}", "Directory:".bold(), self.path.display())?;
        writeln!(
            f,
            "{} {}/{} ({})",
            "Repository:".bold(),
            self.owner,
            self.repo,
            self.remote_url
        )?;
        match &self.branch {
            Ok(branch) => {
                writeln!(f, "{} {branch}", "Branch:".bold())?;
            }
            Err(e) => {
                writeln!(f, "{} Failed to determine branch: {e}", "Branch:".bold())?;
            }
        }

        match &self.milestones {
            Ok(milestones) => {
                writeln!(
                    f,
                    "{} {}{}",
                    "Milestones:".bold(),
                    if milestones.len() == 0 {
                        "None".to_string()
                    } else {
                        format!("{}\n  - ", milestones.len())
                    },
                    milestones
                        .iter()
                        .map(|(_, m)| m.to_string())
                        .collect::<Vec<_>>()
                        .join("\n  - ")
                )
            }
            Err(e) => writeln!(
                f,
                "{} Failed to determine milestones: {e}",
                "Milestones:".bold()
            ),
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
            "{} {}{}",
            "Directory:".bold(),
            self.configuration.path.display(),
            if self.path_exists {
                ""
            } else {
                "    ❌ Directory not found"
            }
        )?;

        if let (Some(owner), Some(repo), Some(remote_url)) =
            (&self.owner, &self.repo, &self.remote_url)
        {
            writeln!(
                f,
                "{} {}/{} ({})",
                "Repository:".bold(),
                owner,
                repo,
                remote_url
            )?;
        } else {
            writeln!(
                f,
                "{} Not determined to be git repository",
                "Repository:".bold()
            )?;
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
            "{} {}\n  - {}",
            "Checklists:".bold(),
            self.configuration.checklists.len(),
            checklists.join("\n  - ")
        )?;

        writeln!(f, "{}", "Options:".bold())?;
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
struct BinarySitRep {
    version: String,
    path: Result<PathBuf, String>,
}

impl BinarySitRep {
    fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            path: std::env::current_exe().map_err(|e| e.to_string()),
        }
    }
}

impl fmt::Display for BinarySitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {}", "Version:".bold(), self.version)?;
        match &self.path {
            Ok(p) => writeln!(f, "{} {}", "Path:".bold(), p.display()),
            Err(e) => writeln!(
                f,
                "{} Failed to determine executable path: {e}",
                "Path:".bold()
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct AuthSourceEntry {
    kind: String,
    token_preview: Option<String>,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AuthSitRep {
    store_dir: Option<PathBuf>,
    stored_tokens: String,
    host: Option<String>,
    sources: Vec<AuthSourceEntry>,
}

impl AuthSitRep {
    fn new(
        git_info: Option<&GitInfo>,
        auth_store: Option<&AuthStore>,
        env: &impl EnvProvider,
    ) -> Self {
        let host = git_info.and_then(|g| extract_host_from_base_url(&g.base_url).ok());

        let (store_dir, stored_tokens) = match auth_store {
            Some(store) => (
                Some(store.root.clone()),
                store.display_with_selected(host.as_deref()),
            ),
            None => (None, "unavailable".to_string()),
        };

        let sources = if let Some(git_info) = git_info {
            let auth_sources = AuthSources::new(&git_info.base_url, env, auth_store);
            let active = auth_sources
                .sorted()
                .into_iter()
                .next()
                .map(|(k, _)| k.to_string());
            auth_sources
                .all_by_priority()
                .into_iter()
                .map(|(kind, token)| AuthSourceEntry {
                    is_active: active.as_deref() == Some(&kind.to_string()),
                    kind: kind.to_string(),
                    token_preview: token.map(|t| crate::auth::preview_token(t)),
                })
                .collect()
        } else {
            vec![]
        };

        Self {
            store_dir,
            stored_tokens,
            host,
            sources,
        }
    }
}

impl fmt::Display for AuthSitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.store_dir {
            Some(dir) => writeln!(f, "{} {}", "store directory:".bold(), dir.display())?,
            None => writeln!(f, "{} unavailable", "store directory:".bold())?,
        }

        if self.stored_tokens == "none" || self.stored_tokens == "unavailable" {
            writeln!(f, "{} {}", "stored tokens:".bold(), self.stored_tokens)?;
        } else {
            writeln!(f, "{}", "stored tokens:".bold())?;
            for line in self.stored_tokens.lines() {
                writeln!(f, "  {line}")?;
            }
        }

        writeln!(f)?;

        match &self.host {
            Some(host) => writeln!(f, "{} {host}", "repository host:".bold())?,
            None => writeln!(f, "{} not determined", "repository host:".bold())?,
        }

        if self.sources.is_empty() {
            writeln!(f, "{} unknown", "available auth sources:".bold())?;
        } else {
            writeln!(f, "{}", "available auth sources".bold())?;
            for entry in &self.sources {
                let marker = if entry.is_active {
                    "▶ ".green().to_string()
                } else {
                    "  ".to_string()
                };
                match &entry.token_preview {
                    Some(preview) => writeln!(
                        f,
                        "  {}{} {} ({})",
                        marker,
                        "✓".green(),
                        if entry.is_active {
                            format!("{:<26}", entry.kind).bold().to_string()
                        } else {
                            format!("{:<26}", entry.kind)
                        },
                        preview
                    )?,
                    None => writeln!(f, "    {} {}", "✗".red(), entry.kind)?,
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SitRep {
    binary: BinarySitRep,
    directory: PathBuf,
    repository: Result<RepoSitRep, String>,
    configuration: ConfigSitRep,
    auth: AuthSitRep,
}

impl SitRep {
    pub async fn new(
        directory: impl AsRef<Path>,
        config_dir: Option<impl AsRef<Path>>,
        auth_store: Option<&AuthStore>,
    ) -> Self {
        let env = StdEnvProvider;
        let git_info_result = GitInfo::from_path(directory.as_ref(), &env, auth_store);
        let auth = AuthSitRep::new(git_info_result.as_ref().ok(), auth_store, &env);
        let repository = match git_info_result {
            Ok(git_info) => Ok(RepoSitRep::new(git_info).await),
            Err(e) => Err(e.to_string()),
        };

        let config_dir = determine_config_dir(config_dir.map(|c| c.as_ref().to_path_buf()), &env)
            .unwrap_or(
                PathBuf::from(env.var("HOME").unwrap_or(".".to_string()))
                    .join(".local")
                    .join("share"),
            );
        let config_git_info = GitInfo::from_path(&config_dir, &env, None).ok();
        let configuration = ConfigSitRep::new(&config_dir, config_git_info.as_ref());

        Self {
            binary: BinarySitRep::new(),
            directory: directory.as_ref().to_path_buf(),
            repository,
            configuration,
            auth,
        }
    }
}

impl fmt::Display for SitRep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", super::section_header("Binary"))?;
        writeln!(f, "{}", self.binary)?;
        writeln!(f, "{}", super::section_header("Repository"))?;
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

        writeln!(f, "{}", super::section_header("Auth"))?;
        writeln!(f, "{}", self.auth)?;
        writeln!(f, "{}", super::section_header("Configuration"))?;
        writeln!(f, "{}", self.configuration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    fn load_milestone_json(name: &str) -> Value {
        let text =
            std::fs::read_to_string(format!("src/tests/github_api/milestones/{}.json", name))
                .unwrap_or_else(|_| panic!("Failed to load milestone fixture: {}", name));
        serde_json::from_str(&text).expect("Failed to parse milestone fixture JSON")
    }

    fn parse_milestone(json: Value) -> octocrab::models::Milestone {
        serde_json::from_value(json).expect("Failed to deserialize milestone")
    }

    // v1.0: open=4, closed=8, state=open  |  v2.0: open=2, closed=3, state=open

    #[test]
    fn test_from_milestone_totals() {
        let rep = MilestoneSitRep::from(parse_milestone(load_milestone_json("v1.0")));
        assert_eq!(rep.name, "v1.0");
        assert_eq!(rep.open, 4);
        assert_eq!(rep.closed, 8);
        assert_eq!(rep.total, 12);
        assert!(rep.is_open);
    }

    #[test]
    fn test_from_milestone_closed_state() {
        let mut json = load_milestone_json("v1.0");
        json["state"] = serde_json::json!("closed");
        let rep = MilestoneSitRep::from(parse_milestone(json));
        assert!(!rep.is_open);
    }

    #[test]
    fn test_milestone_display() {
        let rep = MilestoneSitRep::from(parse_milestone(load_milestone_json("v2.0")));
        assert_eq!(rep.to_string(), "v2.0 [open]: 2 open | 3 closed");
    }

    #[test]
    fn test_milestone_sort_highest_open_first() {
        // v1.0 has more open issues (4) than v2.0 (2), so v1.0 should sort first
        let v1 = MilestoneSitRep::from(parse_milestone(load_milestone_json("v1.0")));
        let v2 = MilestoneSitRep::from(parse_milestone(load_milestone_json("v2.0")));
        let mut milestones: Vec<(String, MilestoneSitRep)> =
            vec![("v2.0".to_string(), v2), ("v1.0".to_string(), v1)];
        milestones
            .sort_by(|(_, a), (_, b)| a.open.cmp(&b.open).reverse().then(a.name.cmp(&b.name)));
        let names: Vec<&str> = milestones.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["v1.0", "v2.0"]);
    }

    #[test]
    fn test_milestone_sort_tiebreak_alphabetical() {
        // When open counts are equal, sort alphabetically by name.
        // Use v1.0 as the base fixture and patch title + open_issues for each variant.
        let make = |title: &str, open: u64| {
            let mut json = load_milestone_json("v1.0");
            json["title"] = serde_json::json!(title);
            json["open_issues"] = serde_json::json!(open);
            MilestoneSitRep::from(parse_milestone(json))
        };

        let mut milestones: Vec<(String, MilestoneSitRep)> = vec![
            ("zebra".to_string(), make("zebra", 3)),
            ("alpha".to_string(), make("alpha", 3)),
            ("beta".to_string(), make("beta", 3)),
        ];
        milestones
            .sort_by(|(_, a), (_, b)| a.open.cmp(&b.open).reverse().then(a.name.cmp(&b.name)));
        let names: Vec<&str> = milestones.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "zebra"]);
    }

    #[test]
    fn test_repo_sitrep_display_branch_error() {
        let rep = RepoSitRep {
            path: PathBuf::from("/some/path"),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            remote_url: "https://github.com/owner/repo".to_string(),
            branch: Err("detached HEAD".to_string()),
            milestones: Ok(vec![]),
        };
        assert!(
            rep.to_string()
                .contains("Failed to determine branch: detached HEAD")
        );
    }

    fn fixed_binary() -> BinarySitRep {
        BinarySitRep {
            version: "0.0.0-test".to_string(),
            path: Ok(PathBuf::from("/usr/local/bin/ghqc")),
        }
    }

    fn make_sitrep(repository: Result<RepoSitRep, String>) -> SitRep {
        SitRep {
            binary: fixed_binary(),
            directory: PathBuf::from("/projects/myrepo"),
            repository,
            configuration: ConfigSitRep {
                owner: Some("owner".to_string()),
                repo: Some("repo".to_string()),
                remote_url: Some("https://github.com/owner/repo".to_string()),
                path_exists: false,
                configuration: Configuration::from_path(PathBuf::from("/config/path")),
            },
            auth: AuthSitRep {
                store_dir: Some(PathBuf::from("/home/user/.local/share/ghqc/auth")),
                stored_tokens: "none".to_string(),
                host: Some("github.com".to_string()),
                sources: vec![],
            },
        }
    }

    #[test]
    fn test_sitrep_display_ok() {
        let v1 = MilestoneSitRep::from(parse_milestone(load_milestone_json("v1.0")));
        let v2 = MilestoneSitRep::from(parse_milestone(load_milestone_json("v2.0")));
        // pre-sorted: v1.0 (4 open) before v2.0 (2 open)
        let repo = RepoSitRep {
            path: PathBuf::from("/projects/myrepo"),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            remote_url: "https://github.com/owner/repo".to_string(),
            branch: Ok("main".to_string()),
            milestones: Ok(vec![("v1.0".to_string(), v1), ("v2.0".to_string(), v2)]),
        };
        insta::assert_snapshot!(make_sitrep(Ok(repo)).to_string());
    }

    #[test]
    fn test_sitrep_display_repo_error() {
        insta::assert_snapshot!(make_sitrep(Err("not a git repository".to_string())).to_string());
    }

    #[test]
    fn test_sitrep_display_custom_configuration() {
        let v1 = MilestoneSitRep::from(parse_milestone(load_milestone_json("v1.0")));
        let repo = RepoSitRep {
            path: PathBuf::from("/projects/myrepo"),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            remote_url: "https://github.com/owner/repo".to_string(),
            branch: Ok("main".to_string()),
            milestones: Ok(vec![("v1.0".to_string(), v1)]),
        };
        let sitrep = SitRep {
            binary: fixed_binary(),
            directory: PathBuf::from("/projects/myrepo"),
            repository: Ok(repo),
            configuration: ConfigSitRep::new("src/tests/custom_configuration", None),
            auth: AuthSitRep {
                store_dir: Some(PathBuf::from("/home/user/.local/share/ghqc/auth")),
                stored_tokens: "none".to_string(),
                host: Some("github.com".to_string()),
                sources: vec![],
            },
        };
        insta::assert_snapshot!(sitrep.to_string());
    }
}
