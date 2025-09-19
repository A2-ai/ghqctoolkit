use etcetera::BaseStrategy;
use gix::Url;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use crate::git::{GitAction, GitRepository, GitStatusOps};
use crate::utils::EnvProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ConfigurationOptions {
    // Note to prepend at the top of all checklists
    pub(crate) prepended_checklist_note: Option<String>,
    // What to call the checklist in the app. Default: checklist
    pub(crate) checklist_display_name: String,
    // Path to the logo within the configuration repo. Default: logo
    logo_path: PathBuf,
    // Path to the checklist directory within the configuration repo. Default: checklists
    checklist_directory: PathBuf,
}

impl Default for ConfigurationOptions {
    fn default() -> Self {
        Self {
            prepended_checklist_note: None,
            checklist_display_name: "checklists".to_string(),
            logo_path: PathBuf::from("logo.png"),
            checklist_directory: PathBuf::from("checklists"),
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
}

#[derive(Debug, Clone)]
pub struct Checklist {
    pub(crate) name: String,
    note: Option<String>,
    content: String,
}

impl Checklist {
    pub fn new(name: String, note: Option<String>, content: String) -> Self {
        Self {
            name,
            note,
            content,
        }
    }

    fn items(&self) -> usize {
        self.content.matches("- [ ]").count()
    }
}

impl fmt::Display for Checklist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let note = if let Some(n) = &self.note {
            format!("\n\n{n}")
        } else {
            String::new()
        };
        writeln!(f, "# {}{note}\n\n{}", self.name, self.content)
    }
}

impl Default for Checklist {
    fn default() -> Self {
        Self {
            name: "Custom".to_string(),
            note: None,
            content: "- [ ] [INSERT]".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Configuration {
    pub(crate) path: PathBuf,
    // checklist name and content
    pub(crate) checklists: HashMap<String, Checklist>,
    pub(crate) options: ConfigurationOptions,
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
                "txt" => {
                    match extract_title_from_filename(&path) {
                        Ok(key) => {
                            let checklist = Checklist {
                                name: key.to_string(),
                                note: self.options.prepended_checklist_note.clone(),
                                content,
                            };
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
                        let checklist = Checklist {
                            name: title.to_string(),
                            note: self.options.prepended_checklist_note.clone(),
                            content,
                        };
                        self.checklists.insert(title, checklist);
                    }
                    Err(e) => {
                        log::warn!(
                            "Could not parse yaml at {} as valid checklist due to: {}",
                            path.display(),
                            e
                        )
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

fn parse_yaml_checklist(content: &str) -> Result<(String, String), ConfigurationError> {
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
    config_dir: impl AsRef<Path>,
    git: Url,
    git_action: impl GitAction,
) -> Result<(), ConfigurationError> {
    let config_dir = config_dir.as_ref();

    // Check if config directory already exists
    if config_dir.exists() {
        log::debug!(
            "Config directory already exists at {}",
            config_dir.display()
        );

        // Check if it's already a git repository with the same remote
        match git_action.remote(config_dir) {
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

    log::debug!(
        "Cloning configuration repository from {} to {}",
        git,
        config_dir.display()
    );

    if let Some(parent) = config_dir.parent() {
        if !parent.is_dir() {
            fs::create_dir_all(parent)?;
        }
    }

    // Clone the repository
    git_action.clone(git, config_dir)?;

    log::debug!(
        "Successfully set up configuration at {}",
        config_dir.display()
    );
    Ok(())
}

pub fn determine_config_info(
    config_dir: Option<PathBuf>,
    env: &impl EnvProvider,
) -> Result<PathBuf, ConfigurationError> {
    if let Some(c) = config_dir {
        log::debug!("Using custom config dir: {}", c.display());
        return Ok(c);
    }

    let strategy = etcetera::choose_base_strategy()
        .map_err(|e| ConfigurationError::ConfigDir(e.to_string()))?;
    let config_dir = strategy.config_dir().join("ghqc");

    match env.var("GHQC_CONFIG_HOME") {
        Ok(url_str) => {
            log::debug!("GHQC_CONFIG_HOME found: {url_str}");
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
                .ok_or_else(|| {
                    ConfigurationError::ConfigDir(format!(
                        "Cannot extract repo name from URL: {}",
                        url
                    ))
                })?;

            let dir = config_dir.join(repo_name);
            log::debug!("Using env var directory: {}", dir.display());

            Ok(dir)
        }
        Err(_) => {
            // No env var set, use default path with no URL
            let dir = config_dir.join("config");
            log::debug!(
                "GHQC_CONFIG_HOME not set. Using default dir: {}",
                dir.display()
            );
            Ok(dir)
        }
    }
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
        format!(
            "\n📦 git repository: {}/{}{}",
            git_info.owner(),
            git_info.repo(),
            if let Ok(status) = git_info.status() {
                format!("\n{}", status.to_string())
            } else {
                String::new()
            }
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
    GitAction(#[from] crate::git::GitActionError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::MockEnvProvider;
    use tempfile::TempDir;

    #[test]
    fn test_determine_config_info_with_provided_path() {
        let provided_path = PathBuf::from("/custom/config/path");
        let mock_env = MockEnvProvider::new();

        let result = determine_config_info(Some(provided_path.clone()), &mock_env).unwrap();
        assert_eq!(result, provided_path);
    }

    #[test]
    fn test_determine_config_info_with_env_var() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_HOME"))
            .times(1)
            .returning(|_| Ok("https://github.com/owner/my-config-repo.git".to_string()));

        let result = determine_config_info(None, &mock_env).unwrap();

        // Should extract "my-config-repo.git" from the URL and append to config dir
        assert!(result.ends_with("my-config-repo.git"));
        assert!(result.to_string_lossy().contains("config")); // Should be in some config directory
    }

    #[test]
    fn test_determine_config_info_without_env_var() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_HOME"))
            .times(1)
            .returning(|_| Err(std::env::VarError::NotPresent));

        let result = determine_config_info(None, &mock_env).unwrap();

        // Should use default "ghqc" directory
        assert!(result.ends_with("config"));
        assert!(result.to_string_lossy().contains("ghqc")); // Should be in some config directory
    }

    #[test]
    fn test_determine_config_info_with_invalid_url() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_HOME"))
            .times(1)
            .returning(|_| Ok("://invalid-url-scheme".to_string()));

        let result = determine_config_info(None, &mock_env);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigurationError::InvalidGitUrl { .. }
        ));
    }

    #[test]
    fn test_determine_config_info_with_url_no_path() {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("GHQC_CONFIG_HOME"))
            .times(1)
            .returning(|_| Ok("https://github.com".to_string()));

        let result = determine_config_info(None, &mock_env);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigurationError::ConfigDir(_)
        ));
    }

    #[test]
    fn test_load_checklists_default() {
        let test_config_path = PathBuf::from("src/tests/default_configuration");

        let mut config = Configuration::from_path(&test_config_path);
        config.load_checklists();

        // Should have loaded 5 checklists and 1 default custom (ignoring .md file)
        assert_eq!(config.checklists.len(), 6);

        // Verify all expected keys are present
        assert!(config.checklists.contains_key("Custom"));
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
        assert_eq!(
            config.options.logo_path,
            PathBuf::from("assets/custom_logo.svg")
        );
        assert_eq!(
            config.options.checklist_directory,
            PathBuf::from("my_custom_checklists")
        );

        let custom_content = &config.checklists["Custom Checklist"];
        insta::assert_snapshot!("custom_directory_checklist", custom_content);
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
            status: crate::git::GitStatus,
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
        }

        impl crate::git::GitStatusOps for MockGitInfo {
            fn status(&self) -> Result<crate::git::GitStatus, crate::git::GitStatusError> {
                Ok(self.status.clone())
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
            status: crate::git::GitStatus::Clean,
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
            status: crate::GitStatus::Dirty(vec![
                PathBuf::from("src/main.rs"),
                PathBuf::from("README.md"),
            ]),
        };

        let result_dirty = configuration_status(&configuration, &Some(git_info_dirty));
        insta::assert_snapshot!("configuration_status_dirty", result_dirty);
    }
}
