use etcetera::BaseStrategy;
use gix::Url;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use crate::git::{GitCli, GitRepository, GitState, GitStatusOps, PullOutcome, get_git_status};
use crate::utils::EnvProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConfigurationOptions {
    // Note to prepend at the top of all checklists
    pub prepended_checklist_note: Option<String>,
    // What to call the checklist in the app. Default: checklist
    pub checklist_display_name: String,
    // Whether collaborator metadata should be detected and included. Default: true
    pub include_collaborators: bool,
    // Path to the logo within the configuration repo. Default: logo
    pub logo_path: PathBuf,
    // Path to the checklist directory within the configuration repo. Default: checklists
    pub checklist_directory: PathBuf,
    // Path to the record template within the configuration repo. Default: record.typ
    pub record_path: PathBuf,
    // UI repo refresh rate in seconds. Falls back to env var/default if not set or invalid
    #[serde(default, deserialize_with = "deserialize_optional_positive_seconds")]
    pub ui_repo_refresh_rate_seconds: Option<u64>,
    // Whether the web UI may fast-forward the configuration repository. Falls back to
    // env var/default if not set. Default: true
    pub allow_ui_config_update: Option<bool>,
}

impl Default for ConfigurationOptions {
    fn default() -> Self {
        Self {
            prepended_checklist_note: None,
            checklist_display_name: "checklists".to_string(),
            include_collaborators: true,
            logo_path: PathBuf::from("logo.png"),
            checklist_directory: PathBuf::from("checklists"),
            record_path: PathBuf::from("record.typ"),
            ui_repo_refresh_rate_seconds: None,
            allow_ui_config_update: None,
        }
    }
}

impl ConfigurationOptions {
    fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigurationError> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;
        let options = serde_yaml::from_str(&content)?;
        Ok(options)
    }

    pub fn resolved_ui_repo_refresh_rate_seconds(&self, env: &impl EnvProvider) -> u64 {
        self.ui_repo_refresh_rate_seconds
            .or_else(|| {
                env.var("GHQC_UI_REFRESH_RATE")
                    .ok()
                    .and_then(|value| parse_positive_seconds(&value))
            })
            .unwrap_or(15)
    }

    pub fn resolved_allow_ui_config_update(&self, env: &impl EnvProvider) -> bool {
        self.allow_ui_config_update
            .or_else(|| {
                env.var("GHQC_ALLOW_CONFIG_UPDATE")
                    .ok()
                    .and_then(|value| parse_bool(&value))
            })
            .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checklist {
    pub name: String,
    pub content: String,
}

impl Checklist {
    pub fn new(name: String, note: Option<&str>, content: String) -> Self {
        let content = format!(
            "{}{content}",
            note.map(|n| format!("{n}\n\n")).unwrap_or_default()
        );
        Self { name, content }
    }

    pub fn items(&self) -> usize {
        self.content.matches("- [ ]").count()
    }
}

impl fmt::Display for Checklist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "# {}\n\n{}", self.name, self.content)
    }
}

impl Default for Checklist {
    fn default() -> Self {
        Self {
            name: "Custom".to_string(),
            content: "- [ ] [INSERT]".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    pub path: PathBuf,
    // checklist name and content
    pub checklists: HashMap<String, Checklist>,
    pub options: ConfigurationOptions,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            path: PathBuf::default(),
            checklists: HashMap::from([("Custom".to_string(), Checklist::default())]),
            options: ConfigurationOptions::default(),
        }
    }
}

impl Configuration {
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let options = match ConfigurationOptions::from_path(&path.join("options.yaml")) {
            Ok(o) => o,
            Err(e) => {
                log::warn!(
                    "Could not load configuration options at {} due to: {e}. Using default.",
                    path.display()
                );
                ConfigurationOptions::default()
            }
        };
        log::debug!("checklist note: {:#?}", options.prepended_checklist_note);

        Configuration {
            path: path.to_path_buf(),
            options,
            ..Default::default()
        }
    }

    pub fn load_checklists(&mut self) {
        let checklist_dir = self.path.join(&self.options.checklist_directory);

        if !checklist_dir.exists() {
            log::debug!(
                "Checklist directory {} does not exist. Nothing to load",
                checklist_dir.display()
            );
            return;
        }

        let Ok(read_dir) = fs::read_dir(&checklist_dir) else {
            log::debug!("Could not read {}", checklist_dir.display());
            return;
        };

        for entry in read_dir {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };

            let Ok(content) = fs::read_to_string(&path) else {
                log::debug!("Could not read content at {}", path.display());
                continue;
            };

            match extension.to_lowercase().as_str() {
                // Markdown and plain text are both used verbatim as the
                // checklist body; only the title comes from the filename.
                "txt" | "md" | "markdown" => {
                    match extract_title_from_filename(&path) {
                        Ok(key) => {
                            let checklist = Checklist::new(
                                key.to_string(),
                                self.options.prepended_checklist_note.as_deref(),
                                content,
                            );
                            self.checklists.insert(key, checklist);
                        }
                        Err(e) => {
                            log::warn!(
                                "Could not extract title from filename for {} due to: {}. Skipping...",
                                path.display(),
                                e
                            );
                            continue;
                        }
                    };
                }
                "yaml" | "yml" => match parse_yaml_checklist(&content) {
                    Ok((title, content)) => {
                        let checklist = Checklist::new(
                            title.to_string(),
                            self.options.prepended_checklist_note.as_deref(),
                            content,
                        );
                        self.checklists.insert(title, checklist);
                    }
                    Err(e) => {
                        log::warn!(
                            "Could not parse yaml at {} as valid checklist due to: {}",
                            path.display(),
                            e
                        );
                    }
                },
                _ => continue, // Skip other file types
            }
        }

        log::debug!("Found checklists with titles: {:?}", self.checklists.keys());
    }

    pub fn logo_path(&self) -> PathBuf {
        self.path.join(&self.options.logo_path)
    }

    pub fn record_path(&self) -> PathBuf {
        self.path.join(&self.options.record_path)
    }

    pub fn checklist_display_name(&self) -> &str {
        &self.options.checklist_display_name
    }

    pub fn prepended_checklist_note(&self) -> Option<&str> {
        self.options
            .prepended_checklist_note
            .as_ref()
            .map(|s| s.as_str())
    }

    pub fn include_collaborators(&self) -> bool {
        self.options.include_collaborators
    }

    pub fn ui_repo_refresh_rate_seconds(&self, env: &impl EnvProvider) -> u64 {
        self.options.resolved_ui_repo_refresh_rate_seconds(env)
    }
}

fn deserialize_optional_positive_seconds<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(parse_positive_seconds_yaml))
}

fn parse_positive_seconds_yaml(value: Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64().filter(|seconds| *seconds > 0).or_else(|| {
            number
                .as_i64()
                .filter(|seconds| *seconds > 0)
                .map(|seconds| seconds as u64)
        }),
        Value::String(value) => parse_positive_seconds(&value),
        _ => None,
    }
}

fn parse_positive_seconds(value: &str) -> Option<u64> {
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds as u64)
}

/// Tolerantly parses a boolean, accepting the spellings commonly used in shell
/// environments. Unrecognized values yield `None` so the caller falls back to
/// the next source rather than silently choosing a value.
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        other => {
            log::warn!("Ignoring unrecognized boolean value: {other:?}");
            None
        }
    }
}

fn extract_title_from_filename(path: &Path) -> Result<String, ConfigurationError> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ConfigurationError::InvalidFilename(path.to_path_buf()))?;

    // Handle backtick-wrapped titles for spaces
    if stem.starts_with('`') && stem.ends_with('`') && stem.len() > 2 {
        Ok(stem[1..stem.len() - 1].to_string())
    } else {
        Ok(stem.to_string())
    }
}

pub(crate) fn parse_yaml_checklist(content: &str) -> Result<(String, String), ConfigurationError> {
    use serde_yaml::Value;

    let yaml: Value = serde_yaml::from_str(content)?;

    // The root should be a mapping with a single key (the checklist name)
    let mapping = yaml.as_mapping().ok_or_else(|| {
        ConfigurationError::InvalidYamlStructure("Root must be a mapping".to_string())
    })?;

    if mapping.len() != 1 {
        return Err(ConfigurationError::InvalidYamlStructure(
            "Root mapping must have exactly one key (the checklist name)".to_string(),
        ));
    }

    let (title_key, checklist_content) = mapping.iter().next().unwrap();
    let title = title_key
        .as_str()
        .ok_or_else(|| {
            ConfigurationError::InvalidYamlStructure("Checklist name must be a string".to_string())
        })?
        .to_string();

    let formatted_content = format_checklist_items_with_level(checklist_content, 3)?; // start at header 3 (###)

    Ok((title, formatted_content))
}

fn format_checklist_items_with_level(
    checklist: &Value,
    header_level: usize,
) -> Result<String, ConfigurationError> {
    match checklist {
        // If it's a sequence, format as plain items without subheaders
        Value::Sequence(items) => Ok(format_items(items)),
        // If it's a mapping, format with subheaders
        Value::Mapping(sections) => {
            let mut formatted_sections = Vec::new();

            for (section_key, section_value) in sections {
                let section_name = section_key.as_str().ok_or_else(|| {
                    ConfigurationError::InvalidYamlStructure(
                        "Section name must be a string".to_string(),
                    )
                })?;

                match section_value {
                    // If the section contains a list, format it as items
                    Value::Sequence(items) => {
                        let formatted_section =
                            format_section_list_with_level(section_name, items, header_level);
                        formatted_sections.push(formatted_section);
                    }
                    // If the section contains nested mappings, recurse
                    Value::Mapping(_) => {
                        let header = format_header(section_name, header_level);
                        let nested_content =
                            format_checklist_items_with_level(section_value, header_level + 1)?;
                        formatted_sections.push(format!("{}\n\n{}", header, nested_content));
                    }
                    _ => {
                        return Err(ConfigurationError::InvalidYamlStructure(
                            "Section content must be either a list or nested sections".to_string(),
                        ));
                    }
                }
            }

            Ok(formatted_sections.join("\n"))
        }
        _ => Err(ConfigurationError::InvalidYamlStructure(
            "Checklist content must be either a list or a mapping".to_string(),
        )),
    }
}

fn format_items(items: &[Value]) -> String {
    let formatted_items: Vec<String> = items
        .iter()
        .filter_map(|item| item.as_str())
        .map(|item| format!("- [ ] {}", item))
        .collect();

    formatted_items.join("\n")
}

fn format_section_list_with_level(
    section_name: &str,
    items: &[Value],
    header_level: usize,
) -> String {
    let formatted_items = format_items(items);
    let header = format_header(section_name, header_level);
    format!("{}\n\n{}\n\n", header, formatted_items)
}

fn format_header(name: &str, level: usize) -> String {
    let hashes = "#".repeat(level);
    format!("{} {}", hashes, name)
}

pub async fn setup_configuration(
    git: Url,
    git_action: &(impl GitCli + ?Sized),
) -> Result<(), ConfigurationError> {
    // Check if config directory already exists
    if git_action.path().exists() {
        log::debug!(
            "Config directory already exists at {}",
            git_action.path().display()
        );

        // Check if it's already a git repository with the same remote
        match git_action.remote() {
            Ok(existing_url) => {
                if existing_url == git {
                    log::debug!("Config directory already has correct remote URL");
                    return Ok(());
                } else {
                    log::warn!(
                        "Config directory exists with different remote URL: {} (expected: {})",
                        existing_url,
                        git
                    );
                    return Err(ConfigurationError::Io(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!(
                            "Config directory exists with different remote: {}",
                            existing_url
                        ),
                    )));
                }
            }
            Err(_) => {
                // Directory exists but is not a git repository
                return Err(ConfigurationError::Io(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "Config directory exists but is not a git repository",
                )));
            }
        }
    }

    if let Some(parent) = git_action.path().parent() {
        if !parent.is_dir() {
            fs::create_dir_all(parent)?;
        }
    }

    // Clone the repository
    git_action.clone(git)?;

    log::debug!(
        "Successfully set up configuration at {}",
        git_action.path().display()
    );
    Ok(())
}

/// Reason an update of the configuration repository was refused.
///
/// Every refusal is decided *before* the repository is touched, so a refused
/// update never leaves the configuration repository in a modified state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigUpdateRefusal {
    /// The repository has uncommitted (staged or unstaged) changes.
    Dirty(Vec<PathBuf>),
    /// The repository has local commits that are not on the remote.
    Ahead(usize),
    /// The repository has both local-only and remote-only commits.
    Diverged { ahead: usize, behind: usize },
}

impl ConfigUpdateRefusal {
    /// Human readable, actionable explanation naming the repository `path`.
    ///
    /// Shared by the CLI and the API so both report the same wording.
    pub fn message(&self, path: &Path) -> String {
        match self {
            Self::Dirty(files) => format!(
                "Cannot update: {} uncommitted change(s) in {}:\n  - {}\nCommit or stash them, then retry.",
                files.len(),
                path.display(),
                files
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n  - ")
            ),
            Self::Ahead(commits) => format!(
                "Cannot update: {} local commit(s) not on the remote in {}. Push or reset them, then retry.",
                commits,
                path.display()
            ),
            Self::Diverged { ahead, behind } => format!(
                "Cannot update: {} has diverged from its remote ({} ahead, {} behind). Resolve manually.",
                path.display(),
                ahead,
                behind
            ),
        }
    }
}

/// Result of attempting to update the configuration repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigUpdateResult {
    /// The pull ran successfully (possibly a no-op, see [`PullOutcome::UpToDate`]).
    Updated(PullOutcome),
    /// The update was refused; the repository was not modified.
    Refused(ConfigUpdateRefusal),
}

/// Fast-forward the configuration repository onto its remote.
///
/// Shared by `ghqc configuration update` and `POST /api/configuration/update`
/// so the two can never drift apart.
///
/// Safety: all refusal conditions are checked *before* any mutation, and the
/// pull itself is `--ff-only`, so this never rewrites or discards local work.
///
/// Returns the (re-loaded) [`Configuration`] alongside the result so callers
/// always see fresh checklists. In the refusal case nothing changed on disk, so
/// the reloaded configuration is simply equivalent to the previous one.
pub fn update_configuration(
    git_action: &(impl GitCli + ?Sized),
    git_info: &(impl GitRepository + GitStatusOps),
) -> Result<(ConfigUpdateResult, Configuration), ConfigurationError> {
    let path = git_info.path().to_path_buf();

    let reload = |path: &Path| {
        let mut configuration = Configuration::from_path(path);
        configuration.load_checklists();
        configuration
    };

    // 1. Refuse if there is any uncommitted work to lose.
    let dirty = git_info.dirty()?;
    if !dirty.is_empty() {
        log::debug!(
            "Refusing configuration update: {} dirty files in {}",
            dirty.len(),
            path.display()
        );
        return Ok((
            ConfigUpdateResult::Refused(ConfigUpdateRefusal::Dirty(dirty)),
            reload(&path),
        ));
    }

    // 2. Fetch + compute a fresh state, and refuse anything that is not a
    //    pure fast-forward.
    let status = get_git_status(git_info)?;
    match status.state {
        GitState::Ahead(commits) => {
            log::debug!(
                "Refusing configuration update: {} local commits in {}",
                commits.len(),
                path.display()
            );
            return Ok((
                ConfigUpdateResult::Refused(ConfigUpdateRefusal::Ahead(commits.len())),
                reload(&path),
            ));
        }
        GitState::Diverged { ahead, behind } => {
            log::debug!(
                "Refusing configuration update: {} diverged from its remote",
                path.display()
            );
            return Ok((
                ConfigUpdateResult::Refused(ConfigUpdateRefusal::Diverged {
                    ahead: ahead.len(),
                    behind: behind.len(),
                }),
                reload(&path),
            ));
        }
        // Clean still pulls: it reports UpToDate and keeps a single code path.
        GitState::Clean | GitState::Behind(_) => {}
    }

    // 3. Pull, then reload so callers get refreshed checklists.
    let outcome = git_action.pull_ff_only(git_info.remote_name())?;
    log::debug!("Configuration update outcome: {:?}", outcome);

    Ok((ConfigUpdateResult::Updated(outcome), reload(&path)))
}

/// Determine directory for config:
///     1. Use provided config_dir
///     2. If `GHQC_CONFIG_REPO` set, use $XDG_DATA_HOME/ghqc/{repo_name}
///     3. If `GHQC_CONFIG_DIR` set, use that directory
///     4. If none of the above, use $XDG_DATA_HOME/ghqc/config
pub fn determine_config_dir(
    config_dir: Option<PathBuf>,
    env: &impl EnvProvider,
) -> Result<PathBuf, ConfigurationError> {
    if let Some(c) = config_dir {
        log::debug!("Using custom config dir: {}", c.display());
        return Ok(c);
    }

    let strategy =
        etcetera::choose_base_strategy().map_err(|e| ConfigurationError::ConfigDir(e.to_string()));

    let config_dir = strategy.map(|xdg| xdg.data_dir().join("ghqc"));

    if let Ok(url_str) = env.var("GHQC_CONFIG_REPO") {
        log::debug!("GHQC_CONFIG_REPO found: {url_str}");
        let url = gix::url::parse(url_str.as_str().into()).map_err(|error| {
            ConfigurationError::InvalidGitUrl {
                url: url_str,
                error,
            }
        })?;

        // Extract repo name from URL path (last segment)
        let url_path: PathBuf = url.path.to_string().into();
        let repo_name = url_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.trim_end_matches(".git"))
            .ok_or(ConfigurationError::ConfigDir(format!(
                "Cannot extract repo name from URL: {}",
                url
            )))?;

        let dir = config_dir?.join(repo_name);
        log::debug!("Using env var directory: {}", dir.display());

        return Ok(dir);
    }

    if let Ok(env_dir) = env.var("GHQC_CONFIG_DIR") {
        log::debug!("GHQC_CONFIG_DIR found: {env_dir}");
        return Ok(PathBuf::from(env_dir));
    }

    // No env var set, use default path with no URL
    let dir = config_dir?.join("config");
    log::debug!(
        "GHQC_CONFIG_REPO not set. Using default dir: {}",
        dir.display()
    );
    Ok(dir)
}

pub fn configuration_status(
    configuration: &Configuration,
    git_info: &Option<impl GitRepository + GitStatusOps>,
) -> String {
    let checklist_name = &configuration
        .options
        .checklist_display_name
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>())
                .unwrap_or_default()
                + chars.as_str()
        })
        .collect::<Vec<_>>()
        .join(" ");

    let git_str = if let Some(git_info) = git_info {
        let status = get_git_status(git_info).ok();
        let status_detail = status
            .as_ref()
            .map(|s| format!("\n{}", s.state))
            .unwrap_or_default();
        let dirty_detail = status
            .as_ref()
            .filter(|s| !s.dirty.is_empty())
            .map(|s| {
                format!(
                    "\n⚠️ {} files with uncommitted changes:\n  - {}",
                    s.dirty.len(),
                    s.dirty
                        .iter()
                        .map(|p| format!("{}", p.display()))
                        .collect::<Vec<_>>()
                        .join("\n  - ")
                )
            })
            .unwrap_or_default();
        format!(
            "\n📦 git repository: {}/{}{}{}",
            git_info.owner(),
            git_info.repo(),
            status_detail,
            dirty_detail
        )
    } else {
        String::new()
    };

    let checklist_sum = format!(
        "📋 {checklist_name} available in '{}': {}",
        configuration.options.checklist_directory.display(),
        configuration.checklists.len()
    );

    let logo_note = if configuration
        .path
        .join(&configuration.options.logo_path)
        .exists()
    {
        format!(
            "\n✅ Logo found at {}",
            configuration.options.logo_path.display()
        )
    } else if configuration.options.logo_path == PathBuf::from("logo.png") {
        // if logo path is the default and the file does not exist, no need to warn
        String::new()
    } else {
        // warn if the logo is not found at the specified path
        format!(
            "\n⚠️ Logo was not found at the specified path {}",
            configuration.options.logo_path.display()
        )
    };

    let checklist_note = if let Some(note) = &configuration.options.prepended_checklist_note {
        let note = note
            .lines()
            .map(|l| format!("│  {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("\n📌 checklist note: \n{note}\n")
    } else {
        String::new()
    };

    let mut checklist_vec = configuration
        .checklists
        .iter()
        .map(|(name, checklist)| format!("- {name}: {} checklist items", checklist.items()))
        .collect::<Vec<_>>();
    checklist_vec.sort_by(|a, b| a.cmp(b));
    let checklists_str = checklist_vec.join("\n");

    format!(
        "\
== Directory Information ==
📁 directory: {}{git_str}
{checklist_sum}{logo_note}
        
== {checklist_name} Summary =={checklist_note}
{checklists_str}
",
        configuration.path.display()
    )
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse YAML: {0}")]
    YamlParser(#[from] serde_yaml::Error),
    #[error("Invalid filename: {0:?}")]
    InvalidFilename(PathBuf),
    #[error("Invalid YAML structure: {0}")]
    InvalidYamlStructure(String),
    #[error("Failed to determine config dir: {0}")]
    ConfigDir(String),
    #[error("Invalid git url {url}: {error}")]
    InvalidGitUrl {
        url: String,
        error: gix::url::parse::Error,
    },
    #[error("Git action failed: {0}")]
    GitAction(#[from] crate::git::GitCliError),
    #[error("Failed to determine git status: {0}")]
    GitStatus(#[from] crate::git::GitStatusError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::MockEnvProvider;
    use mockall::Sequence;
    use tempfile::TempDir;

    #[test]
    fn test_determine_config_dir_with_provided_path() {
        let provided_path = PathBuf::from("/custom/config/path");
        let mock_env = MockEnvProvider::new();

        let result = determine_config_dir(Some(provided_path.clone()), &mock_env).unwrap();
        assert_eq!(result, provided_path);
    }

    #[test]
    fn test_determine_config_dir_with_env_var() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_REPO"))
            .times(1)
            .returning(|_| Ok("https://github.com/owner/my-config-repo".to_string()));

        let result = determine_config_dir(None, &mock_env).unwrap();

        // Should extract "my-config-repo.git" from the URL and append to config dir
        assert!(result.ends_with("my-config-repo"));
        assert!(result.to_string_lossy().contains("config")); // Should be in some config directory
    }

    #[test]
    fn test_determine_config_dir_without_env_var() {
        let mut mock_env = MockEnvProvider::new();
        let mut sequence = Sequence::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_REPO"))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_DIR"))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Err(std::env::VarError::NotPresent));

        let result = determine_config_dir(None, &mock_env).unwrap();

        // Should use default "ghqc" directory
        assert!(result.ends_with("config"));
        assert!(result.to_string_lossy().contains("ghqc")); // Should be in some config directory
    }

    #[test]
    fn test_determine_config_dir_with_config_dir_env_var() {
        let mut mock_env = MockEnvProvider::new();
        let mut sequence = Sequence::new();
        let env_path = "/env/config/path";

        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_REPO"))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_DIR"))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(move |_| Ok(env_path.to_string()));

        let result = determine_config_dir(None, &mock_env).unwrap();

        assert_eq!(result, PathBuf::from(env_path));
    }

    #[test]
    fn test_determine_config_dir_with_invalid_url() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_REPO"))
            .times(1)
            .returning(|_| Ok("://invalid-url-scheme".to_string()));

        let result = determine_config_dir(None, &mock_env);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigurationError::InvalidGitUrl { .. }
        ));
    }

    #[test]
    fn test_determine_config_dir_with_url_no_path() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_REPO"))
            .times(1)
            .returning(|_| Ok("https://github.com".to_string()));

        let result = determine_config_dir(None, &mock_env);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigurationError::ConfigDir(_)
        ));
    }

    #[test]
    fn test_determine_config_dir_with_git_url() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_REPO"))
            .times(1)
            .returning(|_| Ok("https://github.com/org/repo.git".to_string()));

        let result = determine_config_dir(None, &mock_env);
        assert!(result.is_ok());
        assert!(
            result
                .unwrap()
                .components()
                .last()
                .map(|dir| dir.as_os_str().to_string_lossy() == "repo")
                .unwrap_or_default()
        );
    }

    #[test]
    fn test_load_checklists_default() {
        let test_config_path = PathBuf::from("src/tests/default_configuration");

        let mut config = Configuration::from_path(&test_config_path);
        config.load_checklists();

        // Should have loaded 6 checklists and 1 default custom (ignoring the
        // .rst file, whose extension the loader does not read)
        assert_eq!(config.checklists.len(), 7);

        // Verify all expected keys are present
        assert!(config.checklists.contains_key("Custom"));
        assert!(config.checklists.contains_key("markdown_checklist"));
        assert!(config.checklists.contains_key("simple_checklist"));
        assert!(config.checklists.contains_key("Complex Checklist Name"));
        assert!(config.checklists.contains_key("Simple Tasks"));
        assert!(config.checklists.contains_key("NCA Analysis"));
        assert!(config.checklists.contains_key("Complex Analysis"));

        // Verify all content is as expected
        insta::assert_snapshot!("default_custom", &config.checklists["Custom"]);
        insta::assert_snapshot!(
            "simple_txt_checklist",
            &config.checklists["simple_checklist"]
        );
        insta::assert_snapshot!(
            "backtick_txt_checklist",
            &config.checklists["Complex Checklist Name"]
        );
        insta::assert_snapshot!("simple_yaml_checklist", &config.checklists["Simple Tasks"]);
        insta::assert_snapshot!(
            "hierarchical_yaml_checklist",
            &config.checklists["NCA Analysis"]
        );
        insta::assert_snapshot!(
            "deeply_nested_yaml_checklist",
            &config.checklists["Complex Analysis"]
        );
    }

    #[test]
    fn test_configuration_options_with_custom_directory() {
        let test_config_path = PathBuf::from("src/tests/custom_configuration");

        let mut config = Configuration::from_path(&test_config_path);
        config.load_checklists();

        assert_eq!(config.checklists.len(), 2);
        assert!(config.checklists.contains_key("Custom Checklist"));

        // Verify the custom options were loaded
        assert_eq!(
            config.options.prepended_checklist_note,
            Some("Please review carefully".to_string())
        );
        assert_eq!(
            config.options.checklist_display_name,
            "Custom Quality Check"
        );
        assert!(!config.options.include_collaborators);
        assert_eq!(
            config.options.logo_path,
            PathBuf::from("assets/custom_logo.svg")
        );
        assert_eq!(
            config.options.checklist_directory,
            PathBuf::from("my_custom_checklists")
        );
        assert_eq!(config.options.ui_repo_refresh_rate_seconds, Some(22));

        let custom_content = &config.checklists["Custom Checklist"];
        insta::assert_snapshot!("custom_directory_checklist", custom_content);
    }

    #[test]
    fn test_ui_repo_refresh_rate_prefers_configuration_option() {
        let mut mock_env = MockEnvProvider::new();
        mock_env.expect_var().times(0);

        let options = ConfigurationOptions {
            ui_repo_refresh_rate_seconds: Some(22),
            ..ConfigurationOptions::default()
        };

        assert_eq!(options.resolved_ui_repo_refresh_rate_seconds(&mock_env), 22);
    }

    #[test]
    fn test_ui_repo_refresh_rate_uses_env_var_when_option_missing() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_UI_REFRESH_RATE"))
            .times(1)
            .returning(|_| Ok("45".to_string()));

        let options = ConfigurationOptions::default();
        assert_eq!(options.resolved_ui_repo_refresh_rate_seconds(&mock_env), 45);
    }

    #[test]
    fn test_ui_repo_refresh_rate_defaults_when_missing() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_UI_REFRESH_RATE"))
            .times(1)
            .returning(|_| Err(std::env::VarError::NotPresent));

        let options = ConfigurationOptions::default();
        assert_eq!(options.resolved_ui_repo_refresh_rate_seconds(&mock_env), 15);
    }

    #[test]
    fn test_ui_repo_refresh_rate_uses_env_when_config_value_invalid() {
        let options: ConfigurationOptions =
            serde_yaml::from_str("ui_repo_refresh_rate_seconds: invalid").unwrap();

        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_UI_REFRESH_RATE"))
            .times(1)
            .returning(|_| Ok("12".to_string()));

        assert_eq!(options.ui_repo_refresh_rate_seconds, None);
        assert_eq!(options.resolved_ui_repo_refresh_rate_seconds(&mock_env), 12);
    }

    #[test]
    fn test_ui_repo_refresh_rate_defaults_when_all_values_invalid() {
        let options: ConfigurationOptions =
            serde_yaml::from_str("ui_repo_refresh_rate_seconds: 0").unwrap();

        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_UI_REFRESH_RATE"))
            .times(1)
            .returning(|_| Ok("-5".to_string()));

        assert_eq!(options.ui_repo_refresh_rate_seconds, None);
        assert_eq!(options.resolved_ui_repo_refresh_rate_seconds(&mock_env), 15);
    }

    #[test]
    fn test_allow_ui_config_update_prefers_configuration_option() {
        let mut mock_env = MockEnvProvider::new();
        mock_env.expect_var().times(0);

        let options = ConfigurationOptions {
            allow_ui_config_update: Some(false),
            ..ConfigurationOptions::default()
        };

        assert!(!options.resolved_allow_ui_config_update(&mock_env));
    }

    #[test]
    fn test_allow_ui_config_update_uses_env_var_when_option_missing() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_ALLOW_CONFIG_UPDATE"))
            .times(1)
            .returning(|_| Ok("false".to_string()));

        let options = ConfigurationOptions::default();
        assert!(!options.resolved_allow_ui_config_update(&mock_env));
    }

    #[test]
    fn test_allow_ui_config_update_defaults_to_true_when_missing() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_ALLOW_CONFIG_UPDATE"))
            .times(1)
            .returning(|_| Err(std::env::VarError::NotPresent));

        let options = ConfigurationOptions::default();
        assert!(options.resolved_allow_ui_config_update(&mock_env));
    }

    #[test]
    fn test_allow_ui_config_update_accepted_env_spellings() {
        let truthy = ["true", "TRUE", " True ", "1", "yes", "YES", "on", "On"];
        let falsy = ["false", "FALSE", " False ", "0", "no", "NO", "off", "Off"];

        for value in truthy {
            let mut mock_env = MockEnvProvider::new();
            let owned = value.to_string();
            mock_env
                .expect_var()
                .with(mockall::predicate::eq("GHQC_ALLOW_CONFIG_UPDATE"))
                .times(1)
                .returning(move |_| Ok(owned.clone()));

            let options = ConfigurationOptions::default();
            assert!(
                options.resolved_allow_ui_config_update(&mock_env),
                "expected {value:?} to parse as true"
            );
        }

        for value in falsy {
            let mut mock_env = MockEnvProvider::new();
            let owned = value.to_string();
            mock_env
                .expect_var()
                .with(mockall::predicate::eq("GHQC_ALLOW_CONFIG_UPDATE"))
                .times(1)
                .returning(move |_| Ok(owned.clone()));

            let options = ConfigurationOptions::default();
            assert!(
                !options.resolved_allow_ui_config_update(&mock_env),
                "expected {value:?} to parse as false"
            );
        }
    }

    /// An unparseable env value must never be read as `false` — a typo should not
    /// silently disable configuration updates.
    #[test]
    fn test_allow_ui_config_update_invalid_env_value_defaults_to_true() {
        for value in ["nope", "", "2", "disabled"] {
            let mut mock_env = MockEnvProvider::new();
            let owned = value.to_string();
            mock_env
                .expect_var()
                .with(mockall::predicate::eq("GHQC_ALLOW_CONFIG_UPDATE"))
                .times(1)
                .returning(move |_| Ok(owned.clone()));

            let options = ConfigurationOptions::default();
            assert!(
                options.resolved_allow_ui_config_update(&mock_env),
                "expected invalid value {value:?} to fall back to true"
            );
        }
    }

    /// Existing options.yaml files predate the key, so they must still allow updates.
    #[test]
    fn test_allow_ui_config_update_missing_from_options_yaml() {
        let config = Configuration::from_path(PathBuf::from("src/tests/custom_configuration"));
        assert_eq!(config.options.allow_ui_config_update, None);

        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_ALLOW_CONFIG_UPDATE"))
            .times(1)
            .returning(|_| Err(std::env::VarError::NotPresent));

        assert!(config.options.resolved_allow_ui_config_update(&mock_env));
    }

    #[test]
    fn test_allow_ui_config_update_parsed_from_options_yaml() {
        let options: ConfigurationOptions =
            serde_yaml::from_str("allow_ui_config_update: false").unwrap();
        assert_eq!(options.allow_ui_config_update, Some(false));
    }

    #[test]
    fn test_include_collaborators_defaults_to_true() {
        let options = ConfigurationOptions::default();
        assert!(options.include_collaborators);
    }

    #[test]
    fn test_include_collaborators_loads_from_yaml() {
        let options: ConfigurationOptions =
            serde_yaml::from_str("include_collaborators: false").unwrap();

        assert!(!options.include_collaborators);
    }

    #[test]
    fn test_missing_checklist_directory() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Configuration::from_path(temp_dir);

        // Should not error when checklist directory doesn't exist
        config.load_checklists();
        assert_eq!(config.checklists.len(), 1);
    }

    #[test]
    fn test_invalid_yaml_structures() {
        // Test YAML with multiple root keys (should fail)
        let invalid_yaml = r#"First Checklist:
  - Item 1
Second Checklist:
  - Item 2"#;

        let result = parse_yaml_checklist(invalid_yaml);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigurationError::InvalidYamlStructure(_)
        ));

        // Test YAML that's not a mapping (should fail)
        let invalid_yaml2 = "- Just a list\n- Not a mapping";
        let result2 = parse_yaml_checklist(invalid_yaml2);
        assert!(result2.is_err());
    }

    #[test]
    fn test_configuration_status() {
        // Create a mock GitInfo
        struct MockGitInfo {
            owner: String,
            repo: String,
            status: crate::git::GitState,
            dirty_files: Vec<PathBuf>,
        }

        impl crate::git::GitRepository for MockGitInfo {
            fn commit(&self) -> Result<String, crate::git::GitRepositoryError> {
                Ok("abc123".to_string())
            }

            fn branch(&self) -> Result<String, crate::git::GitRepositoryError> {
                Ok("main".to_string())
            }

            fn owner(&self) -> &str {
                &self.owner
            }

            fn repo(&self) -> &str {
                &self.repo
            }

            fn remote_name(&self) -> &str {
                "origin"
            }

            fn path(&self) -> &std::path::Path {
                std::path::Path::new(".")
            }

            fn fetch(&self) -> Result<bool, crate::git::GitRepositoryError> {
                Ok(false) // Mock: no changes fetched
            }

            fn stash_file(
                &self,
                _file: &std::path::Path,
                _message: &str,
            ) -> Result<crate::git::FileStashOutcome, crate::git::GitRepositoryError> {
                Ok(crate::git::FileStashOutcome::NoChanges)
            }

            fn configured_author(&self) -> Option<crate::GitAuthor> {
                None
            }
        }

        impl crate::git::GitStatusOps for MockGitInfo {
            fn state(
                &self,
            ) -> Result<(gix::ObjectId, crate::git::GitState), crate::git::GitStatusError>
            {
                Ok((
                    gix::ObjectId::empty_tree(gix::hash::Kind::Sha1),
                    self.status.clone(),
                ))
            }
            fn dirty(&self) -> Result<Vec<PathBuf>, crate::GitStatusError> {
                Ok(self.dirty_files.clone())
            }
        }

        // Load the custom configuration
        let config_path = PathBuf::from("src/tests/custom_configuration");
        let mut configuration = Configuration::from_path(&config_path);
        configuration.load_checklists();

        // Test with git info (clean status)
        let git_info = MockGitInfo {
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            status: crate::git::GitState::Clean,
            dirty_files: Vec::new(),
        };

        let result_with_git = configuration_status(&configuration, &Some(git_info));
        insta::assert_snapshot!("configuration_status_with_git", result_with_git);

        // Test without git info
        let result_without_git: String = configuration_status(&configuration, &None::<MockGitInfo>);
        insta::assert_snapshot!("configuration_status_without_git", result_without_git);

        // Test with dirty status
        let git_info_dirty = MockGitInfo {
            owner: "test-owner".to_string(),
            repo: "test-repo".to_string(),
            status: crate::git::GitState::Clean,
            dirty_files: vec![PathBuf::from("src/main.rs"), PathBuf::from("README.md")],
        };

        let result_dirty = configuration_status(&configuration, &Some(git_info_dirty));
        insta::assert_snapshot!("configuration_status_dirty", result_dirty);
    }

    // ---- update_configuration ----------------------------------------------

    /// Minimal git info stub implementing the two traits `update_configuration`
    /// requires.
    struct StubGitInfo {
        path: PathBuf,
        state: crate::git::GitState,
        dirty_files: Vec<PathBuf>,
    }

    impl StubGitInfo {
        fn new(path: impl Into<PathBuf>, state: crate::git::GitState) -> Self {
            Self {
                path: path.into(),
                state,
                dirty_files: Vec::new(),
            }
        }

        fn with_dirty(mut self, files: Vec<PathBuf>) -> Self {
            self.dirty_files = files;
            self
        }
    }

    impl crate::git::GitRepository for StubGitInfo {
        fn commit(&self) -> Result<String, crate::git::GitRepositoryError> {
            Ok("abc123".to_string())
        }
        fn branch(&self) -> Result<String, crate::git::GitRepositoryError> {
            Ok("main".to_string())
        }
        fn owner(&self) -> &str {
            "test-owner"
        }
        fn repo(&self) -> &str {
            "test-repo"
        }
        fn remote_name(&self) -> &str {
            "origin"
        }
        fn path(&self) -> &Path {
            &self.path
        }
        fn fetch(&self) -> Result<bool, crate::git::GitRepositoryError> {
            Ok(false)
        }
        fn stash_file(
            &self,
            _file: &Path,
            _message: &str,
        ) -> Result<crate::git::FileStashOutcome, crate::git::GitRepositoryError> {
            Ok(crate::git::FileStashOutcome::NoChanges)
        }
        fn configured_author(&self) -> Option<crate::GitAuthor> {
            None
        }
    }

    impl crate::git::GitStatusOps for StubGitInfo {
        fn state(
            &self,
        ) -> Result<(gix::ObjectId, crate::git::GitState), crate::git::GitStatusError> {
            Ok((
                gix::ObjectId::empty_tree(gix::hash::Kind::Sha1),
                self.state.clone(),
            ))
        }
        fn dirty(&self) -> Result<Vec<PathBuf>, crate::git::GitStatusError> {
            Ok(self.dirty_files.clone())
        }
    }

    fn object_ids(n: usize) -> Vec<gix::ObjectId> {
        (0..n)
            .map(|_| gix::ObjectId::empty_tree(gix::hash::Kind::Sha1))
            .collect()
    }

    #[test]
    fn test_update_configuration_refuses_when_dirty() {
        let temp = TempDir::new().unwrap();
        let mut cli = crate::git::MockGitCli::default();
        // Refusal must happen before any repository mutation.
        cli.expect_pull_ff_only().never();

        let git_info = StubGitInfo::new(temp.path(), crate::git::GitState::Behind(object_ids(2)))
            .with_dirty(vec![PathBuf::from("checklists/a.yaml")]);

        let (result, _config) = update_configuration(&cli, &git_info).unwrap();
        match result {
            ConfigUpdateResult::Refused(ConfigUpdateRefusal::Dirty(files)) => {
                assert_eq!(files, vec![PathBuf::from("checklists/a.yaml")]);
            }
            other => panic!("expected dirty refusal, got {other:?}"),
        }
    }

    #[test]
    fn test_update_configuration_refuses_when_ahead() {
        let temp = TempDir::new().unwrap();
        let mut cli = crate::git::MockGitCli::default();
        cli.expect_pull_ff_only().never();

        let git_info = StubGitInfo::new(temp.path(), crate::git::GitState::Ahead(object_ids(3)));

        let (result, _config) = update_configuration(&cli, &git_info).unwrap();
        assert_eq!(
            result,
            ConfigUpdateResult::Refused(ConfigUpdateRefusal::Ahead(3))
        );
    }

    #[test]
    fn test_update_configuration_refuses_when_diverged() {
        let temp = TempDir::new().unwrap();
        let mut cli = crate::git::MockGitCli::default();
        cli.expect_pull_ff_only().never();

        let git_info = StubGitInfo::new(
            temp.path(),
            crate::git::GitState::Diverged {
                ahead: object_ids(1),
                behind: object_ids(2),
            },
        );

        let (result, _config) = update_configuration(&cli, &git_info).unwrap();
        assert_eq!(
            result,
            ConfigUpdateResult::Refused(ConfigUpdateRefusal::Diverged {
                ahead: 1,
                behind: 2
            })
        );
    }

    #[test]
    fn test_update_configuration_clean_reports_up_to_date() {
        let temp = TempDir::new().unwrap();
        let mut cli = crate::git::MockGitCli::default();
        cli.expect_pull_ff_only()
            .times(1)
            .withf(|remote| remote == "origin")
            .returning(|_| Ok(PullOutcome::UpToDate));

        let git_info = StubGitInfo::new(temp.path(), crate::git::GitState::Clean);

        let (result, config) = update_configuration(&cli, &git_info).unwrap();
        assert_eq!(result, ConfigUpdateResult::Updated(PullOutcome::UpToDate));
        assert_eq!(config.path, temp.path());
    }

    #[test]
    fn test_update_configuration_behind_fast_forwards_and_reloads() {
        let temp = TempDir::new().unwrap();
        let checklist_dir = temp.path().join("checklists");
        fs::create_dir_all(&checklist_dir).unwrap();
        fs::write(
            checklist_dir.join("simple.yaml"),
            "Simple Checklist:\n  - first item\n  - second item\n",
        )
        .unwrap();

        let mut cli = crate::git::MockGitCli::default();
        cli.expect_pull_ff_only().times(1).returning(|_| {
            Ok(PullOutcome::FastForwarded {
                from: "aaaaaaa".to_string(),
                to: "bbbbbbb".to_string(),
                commits: 3,
            })
        });

        let git_info = StubGitInfo::new(temp.path(), crate::git::GitState::Behind(object_ids(3)));

        let (result, config) = update_configuration(&cli, &git_info).unwrap();
        assert_eq!(
            result,
            ConfigUpdateResult::Updated(PullOutcome::FastForwarded {
                from: "aaaaaaa".to_string(),
                to: "bbbbbbb".to_string(),
                commits: 3,
            })
        );
        // Checklists are reloaded from disk so callers see fresh content.
        assert!(config.checklists.contains_key("Simple Checklist"));
    }

    #[test]
    fn test_config_update_refusal_messages_name_the_path() {
        let path = Path::new("/tmp/ghqc-config");

        let dirty = ConfigUpdateRefusal::Dirty(vec![PathBuf::from("checklists/a.yaml")]);
        assert_eq!(
            dirty.message(path),
            "Cannot update: 1 uncommitted change(s) in /tmp/ghqc-config:\n  - checklists/a.yaml\nCommit or stash them, then retry."
        );

        assert_eq!(
            ConfigUpdateRefusal::Ahead(2).message(path),
            "Cannot update: 2 local commit(s) not on the remote in /tmp/ghqc-config. Push or reset them, then retry."
        );

        assert_eq!(
            ConfigUpdateRefusal::Diverged {
                ahead: 1,
                behind: 4
            }
            .message(path),
            "Cannot update: /tmp/ghqc-config has diverged from its remote (1 ahead, 4 behind). Resolve manually."
        );
    }
}
