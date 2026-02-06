use crate::GitCommit;
use crate::archive::ArchiveFile;
use anyhow::{Result, anyhow, bail};
use clap::builder::TypedValueParser;
use clap::{Arg, Command, error::ErrorKind};
use gix::ObjectId;
use std::path::PathBuf;
use std::str::FromStr;

/// Represents a file path paired with a specific commit hash
#[derive(Debug, Clone)]
pub struct FileCommitPair {
    pub file: PathBuf,
    pub commit: String,
}

impl FileCommitPair {
    pub fn into_archive_file(&self, commits: &[GitCommit], flatten: bool) -> Result<ArchiveFile> {
        let commit = if let Ok(commit) = ObjectId::from_str(&self.commit) {
            commit
        } else {
            if let Some(commit) = commits
                .iter()
                .find(|c| c.commit.to_string().starts_with(&self.commit))
            {
                commit.commit.clone()
            } else {
                bail!(
                    "Specified commit {} could not be parsed or found in local commits",
                    self.commit
                );
            }
        };

        let archive_file = if flatten {
            self.file.file_name().map(PathBuf::from).ok_or(anyhow!(
                "Provided file ({}) does not have a valid file name",
                self.file.display()
            ))?
        } else {
            self.file
                .strip_prefix("/")
                .unwrap_or(&self.file)
                .to_path_buf()
        };

        Ok(ArchiveFile {
            repository_file: self.file.clone(),
            archive_file,
            commit,
            qc: None,
        })
    }
}

impl FromStr for FileCommitPair {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err("Format must be 'file:commit'. Example: src/main.rs:abc123".to_string());
        }

        let file = PathBuf::from(parts[0]);
        let commit_str = parts[1];

        if commit_str.len() < 6 {
            return Err("commit must be at least 6 characters".to_string());
        }

        Ok(FileCommitPair {
            file,
            commit: commit_str.to_string(),
        })
    }
}

// Custom parser for clap
#[derive(Clone)]
pub struct FileCommitPairParser;

impl TypedValueParser for FileCommitPairParser {
    type Value = FileCommitPair;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                "Invalid UTF-8 in file:commit specification",
            )
        })?;

        s.parse().map_err(|err_msg: String| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue);
            if let Some(arg) = arg {
                err.insert(
                    clap::error::ContextKind::InvalidArg,
                    clap::error::ContextValue::String(arg.to_string()),
                );
            }
            err.insert(
                clap::error::ContextKind::InvalidValue,
                clap::error::ContextValue::String(s.to_string()),
            );
            err.insert(
                clap::error::ContextKind::ValidValue,
                clap::error::ContextValue::String("file:commit".to_string()),
            );
            // Include the specific error message from parsing
            err.insert(
                clap::error::ContextKind::Usage,
                clap::error::ContextValue::String(err_msg),
            );
            err
        })
    }
}

/// Represents a GitHub issue URL with an optional description
/// Format: "<GITHUB_URL>/issues/<NUMBER>[::description]"
/// Example: "https://github.com/owner/repo/issues/123" or "https://github.com/owner/repo/issues/123::This is the description"
#[derive(Debug, Clone)]
pub struct IssueUrlArg {
    /// The original URL provided (without description)
    pub url: String,
    /// The parsed issue number
    pub issue_number: u64,
    /// Optional description for this reference
    pub description: Option<String>,
}

impl IssueUrlArg {
    /// Validates that this issue URL belongs to the expected repository by comparing
    /// against a generated issue URL from the GitHelpers trait
    pub fn validate_repo(&self, expected_issue_url: &str) -> Result<()> {
        if self.url != expected_issue_url {
            bail!(
                "Issue URL '{}' does not match expected repository issue URL '{}'",
                self.url,
                expected_issue_url
            );
        }
        Ok(())
    }
}

impl FromStr for IssueUrlArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Split on "::" to separate URL from optional description
        let (url_part, description) = if let Some(idx) = s.find("::") {
            let desc = s[idx + 2..].trim();
            let desc = if desc.is_empty() {
                None
            } else {
                Some(desc.to_string())
            };
            (&s[..idx], desc)
        } else {
            (s, None)
        };

        let url = url_part.trim();

        // Parse issue number from URL - expected format ends with /issues/<NUMBER>
        // e.g., https://github.com/owner/repo/issues/123
        let parts: Vec<&str> = url.rsplitn(2, '/').collect();
        if parts.len() < 2 {
            return Err(format!(
                "Invalid issue URL format: {}. Expected format: <url>/issues/<number>[::description]",
                url
            ));
        }

        let issue_number_str = parts[0];
        let prefix = parts[1];

        // Verify the URL contains /issues/ before the number
        if !prefix.ends_with("/issues") {
            return Err(format!(
                "Invalid issue URL format: {}. URL must contain '/issues/<number>'",
                url
            ));
        }

        let issue_number: u64 = issue_number_str
            .parse()
            .map_err(|_| format!("Invalid issue number: {}", issue_number_str))?;

        Ok(IssueUrlArg {
            url: url.to_string(),
            issue_number,
            description,
        })
    }
}

/// Custom parser for IssueUrlArg
#[derive(Clone)]
pub struct IssueUrlArgParser;

impl TypedValueParser for IssueUrlArgParser {
    type Value = IssueUrlArg;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(ErrorKind::InvalidUtf8, "Invalid UTF-8 in issue URL")
        })?;

        s.parse().map_err(|err_msg: String| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue);
            if let Some(arg) = arg {
                err.insert(
                    clap::error::ContextKind::InvalidArg,
                    clap::error::ContextValue::String(arg.to_string()),
                );
            }
            err.insert(
                clap::error::ContextKind::InvalidValue,
                clap::error::ContextValue::String(s.to_string()),
            );
            err.insert(
                clap::error::ContextKind::ValidValue,
                clap::error::ContextValue::String(
                    "<url>/issues/<number>[::description]".to_string(),
                ),
            );
            err.insert(
                clap::error::ContextKind::Usage,
                clap::error::ContextValue::String(err_msg),
            );
            err
        })
    }
}

/// Represents a file path with a required justification string
/// Format: "file_path::justification"
/// Example: "src/main.rs::This file contains the main entry point"
#[derive(Debug, Clone)]
pub struct RelevantFileArg {
    pub file: PathBuf,
    pub justification: String,
}

impl FromStr for RelevantFileArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Split on "::" to separate file path from justification
        let idx = s.find("::").ok_or_else(|| {
            "Format must be 'file::justification'. The justification is required.".to_string()
        })?;

        let file_part = &s[..idx];
        let justification = s[idx + 2..].trim();

        if file_part.is_empty() {
            return Err("File path cannot be empty".to_string());
        }

        if justification.is_empty() {
            return Err("Justification is required and cannot be empty".to_string());
        }

        Ok(RelevantFileArg {
            file: PathBuf::from(file_part),
            justification: justification.to_string(),
        })
    }
}

/// Custom parser for RelevantFileArg
#[derive(Clone)]
pub struct RelevantFileArgParser;

impl TypedValueParser for RelevantFileArgParser {
    type Value = RelevantFileArg;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                "Invalid UTF-8 in file::justification specification",
            )
        })?;

        s.parse().map_err(|err_msg: String| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue);
            if let Some(arg) = arg {
                err.insert(
                    clap::error::ContextKind::InvalidArg,
                    clap::error::ContextValue::String(arg.to_string()),
                );
            }
            err.insert(
                clap::error::ContextKind::InvalidValue,
                clap::error::ContextValue::String(s.to_string()),
            );
            err.insert(
                clap::error::ContextKind::ValidValue,
                clap::error::ContextValue::String("file::justification".to_string()),
            );
            err.insert(
                clap::error::ContextKind::Usage,
                clap::error::ContextValue::String(err_msg),
            );
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_url_arg_parsing() {
        // Basic URL without description
        let arg: IssueUrlArg = "https://github.com/owner/repo/issues/123".parse().unwrap();
        assert_eq!(arg.url, "https://github.com/owner/repo/issues/123");
        assert_eq!(arg.issue_number, 123);
        assert!(arg.description.is_none());

        // URL with description
        let arg: IssueUrlArg = "https://github.com/owner/repo/issues/456::This is a description"
            .parse()
            .unwrap();
        assert_eq!(arg.url, "https://github.com/owner/repo/issues/456");
        assert_eq!(arg.issue_number, 456);
        assert_eq!(arg.description, Some("This is a description".to_string()));

        // GitHub Enterprise URL
        let arg: IssueUrlArg = "https://github.enterprise.com/org/project/issues/789"
            .parse()
            .unwrap();
        assert_eq!(
            arg.url,
            "https://github.enterprise.com/org/project/issues/789"
        );
        assert_eq!(arg.issue_number, 789);

        // Invalid URL - missing issues path
        assert!(
            "https://github.com/owner/repo/123"
                .parse::<IssueUrlArg>()
                .is_err()
        );

        // Invalid URL - non-numeric issue number
        assert!(
            "https://github.com/owner/repo/issues/abc"
                .parse::<IssueUrlArg>()
                .is_err()
        );
    }

    #[test]
    fn test_issue_url_arg_validate_repo() {
        let arg: IssueUrlArg = "https://github.com/owner/repo/issues/123".parse().unwrap();

        // Matching URL
        assert!(
            arg.validate_repo("https://github.com/owner/repo/issues/123")
                .is_ok()
        );

        // Non-matching URL (different issue number doesn't matter for this validation
        // since we're comparing the full URL)
        assert!(
            arg.validate_repo("https://github.com/other/repo/issues/123")
                .is_err()
        );
    }

    #[test]
    fn test_relevant_file_arg_parsing() {
        // Valid file with justification
        let arg: RelevantFileArg = "src/main.rs::This is the main entry point".parse().unwrap();
        assert_eq!(arg.file, PathBuf::from("src/main.rs"));
        assert_eq!(arg.justification, "This is the main entry point");

        // File path with spaces in justification
        let arg: RelevantFileArg = "data/config.yaml::Configuration file for the application"
            .parse()
            .unwrap();
        assert_eq!(arg.file, PathBuf::from("data/config.yaml"));
        assert_eq!(arg.justification, "Configuration file for the application");

        // Missing justification
        assert!("src/main.rs::".parse::<RelevantFileArg>().is_err());

        // Missing separator
        assert!("src/main.rs".parse::<RelevantFileArg>().is_err());

        // Empty file path
        assert!("::some justification".parse::<RelevantFileArg>().is_err());
    }
}
