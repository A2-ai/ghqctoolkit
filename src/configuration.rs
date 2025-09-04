use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ConfigurationOptions {
    // Note to prepend at the top of all checklists
    pub(crate) prepended_checklist_notes: Option<String>,
    // What to call the checklist in the app. Default: checklist
    checklist_display_name: String,
    // Path to the logo within the configuration repo. Default: logo
    logo_path: PathBuf,
    // Path to the checklist directory within the configuration repo. Default: checklists
    checklist_directory: PathBuf,
}

impl Default for ConfigurationOptions {
    fn default() -> Self {
        Self {
            prepended_checklist_notes: None,
            checklist_display_name: "checklist".to_string(),
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
pub struct Configuration {
    path: PathBuf,
    // checklist name and content
    pub(crate) checklists: HashMap<String, String>,
    pub(crate) options: ConfigurationOptions,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            path: PathBuf::default(),
            checklists: HashMap::from([("Custom".to_string(), "- [ ] [INSERT]".to_string())]),
            options: ConfigurationOptions::default(),
        }
    }
}

impl Configuration {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigurationError> {
        let path = path.as_ref();
        let options =
            ConfigurationOptions::from_path(path.join("options.yaml")).unwrap_or_default();
        Ok(Configuration {
            path: path.to_path_buf(),
            options,
            ..Default::default()
        })
    }

    pub fn load_checklists(&mut self) -> Result<(), ConfigurationError> {
        let checklist_dir = self.path.join(&self.options.checklist_directory);

        if !checklist_dir.exists() {
            log::debug!("Checklist directory does not exist. Nothing to load");
            return Ok(()); // No checklists directory, nothing to load
        }

        for entry in fs::read_dir(&checklist_dir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };

            let content = fs::read_to_string(&path)?;

            match extension.to_lowercase().as_str() {
                "txt" => {
                    let key = extract_title_from_filename(&path)?;
                    self.checklists.insert(key, content);
                }
                "yaml" | "yml" => {
                    let (title, parsed_content) = parse_yaml_checklist(&content)?;
                    self.checklists.insert(title, parsed_content);
                }
                _ => continue, // Skip other file types
            }
        }

        log::debug!("Found checklists with titles: {:?}", self.checklists.keys());

        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_checklists_default() {
        let test_config_path = PathBuf::from("src/tests/default_configuration");

        let mut config = Configuration::from_path(&test_config_path).unwrap();
        config.load_checklists().unwrap();

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

        let mut config = Configuration::from_path(&test_config_path).unwrap();
        config.load_checklists().unwrap();

        assert_eq!(config.checklists.len(), 2);
        assert!(config.checklists.contains_key("Custom Checklist"));

        // Verify the custom options were loaded
        assert_eq!(
            config.options.prepended_checklist_notes,
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
        let mut config = Configuration::from_path(temp_dir).unwrap();

        // Should not error when checklist directory doesn't exist
        config.load_checklists().unwrap();
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
}
