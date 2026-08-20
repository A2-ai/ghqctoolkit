use crate::GitCommit;
use crate::archive::{ArchiveFile, ArchiveTarget};
use anyhow::{Result, anyhow, bail};
use clap::builder::TypedValueParser;
use clap::{Arg, Command, error::ErrorKind};
use gix::ObjectId;
use std::collections::HashMap;
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

/// Represents a GitHub issue URL with an optional description and diff flag.
/// Format: "<GITHUB_URL>/issues/<NUMBER>[::description][::no_diff]"
/// Examples:
///   "https://github.com/owner/repo/issues/123"
///   "https://github.com/owner/repo/issues/123::My description"
///   "https://github.com/owner/repo/issues/123::no_diff"
///   "https://github.com/owner/repo/issues/123::My description::no_diff"
#[derive(Debug, Clone)]
pub struct IssueUrlArg {
    /// The original URL provided (without description)
    pub url: String,
    /// The parsed issue number
    pub issue_number: u64,
    /// Optional description for this reference
    pub description: Option<String>,
    /// Whether to include a diff comment (only meaningful for previous_qc; default true)
    pub include_diff: bool,
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
        // Split on "::" into segments: [url, optional_description, optional_no_diff]
        let segments: Vec<&str> = s.splitn(3, "::").collect();

        let url_part = segments[0].trim();

        // Check if the last segment is "no_diff"
        let (description, include_diff) = match segments.len() {
            1 => (None, true),
            2 => {
                let seg = segments[1].trim();
                if seg == "no_diff" {
                    (None, false)
                } else if seg.is_empty() {
                    (None, true)
                } else {
                    (Some(seg.to_string()), true)
                }
            }
            3 => {
                let desc = segments[1].trim();
                let last = segments[2].trim();
                let include_diff = last != "no_diff";
                let description = if desc.is_empty() {
                    None
                } else {
                    Some(desc.to_string())
                };
                (description, include_diff)
            }
            _ => (None, true),
        };

        let url = url_part;

        // Parse issue number from URL - expected format ends with /issues/<NUMBER>
        // e.g., https://github.com/owner/repo/issues/123
        let parts: Vec<&str> = url.rsplitn(2, '/').collect();
        if parts.len() < 2 {
            return Err(format!(
                "Invalid issue URL format: {}. Expected format: <url>/issues/<number>[::description][::no_diff]",
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
            include_diff,
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
                    "<url>/issues/<number>[::description][::no_diff]".to_string(),
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

/// A per-issue archive round selection, from `--round <issue#>=<n>`.
///
/// The round is a *selection*, 1-based with `1` being Initial QC — never a claim that
/// the round was approved. Two things this deliberately does not check, because neither
/// is knowable here: whether the issue is in the selected milestones, and whether the
/// round exists on its thread. Both are reported where they are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueRoundArg {
    pub issue_number: u64,
    pub round: u32,
}

impl IssueRoundArg {
    /// The archive target this selection addresses.
    pub fn target(self) -> ArchiveTarget {
        ArchiveTarget::Round(self.round)
    }
}

impl FromStr for IssueRoundArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const FORMAT: &str = "Format must be '<issue#>=<round>'. Example: 42=1";

        // `split_once` rather than `split`: a second `=` lands in the round half and is
        // rejected there by name, instead of being reported as a shapeless format error.
        let (issue, round) = s.split_once('=').ok_or_else(|| FORMAT.to_string())?;
        let (issue, round) = (issue.trim(), round.trim());

        if issue.is_empty() || round.is_empty() {
            return Err(FORMAT.to_string());
        }

        let issue_number = issue
            .parse::<u64>()
            .map_err(|_| format!("Issue number '{issue}' is not a number. {FORMAT}"))?;
        if issue_number == 0 {
            return Err("Issue numbers start at 1".to_string());
        }

        let round = round
            .parse::<u32>()
            .map_err(|_| format!("Round '{round}' is not a number. {FORMAT}"))?;
        // Rejected here rather than left to the thread: round 0 names no round on any
        // thread, so nothing about the issue's history is needed to know it is wrong.
        if round == 0 {
            return Err(
                "Rounds are 1-based and round 1 is Initial QC, so round 0 does not exist"
                    .to_string(),
            );
        }

        Ok(IssueRoundArg {
            issue_number,
            round,
        })
    }
}

// Custom parser for clap
#[derive(Clone)]
pub struct IssueRoundArgParser;

impl TypedValueParser for IssueRoundArgParser {
    type Value = IssueRoundArg;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                "Invalid UTF-8 in issue#=round specification",
            )
        })?;

        // Raw rather than the context-inserting shape used above for `file:commit`:
        // clap drops a `Usage` context on a value error, so that shape reports only
        // "invalid value", and which half of `issue#=round` is wrong is the whole of
        // what the user needs to know here.
        s.parse().map_err(|err_msg: String| {
            clap::Error::raw(
                ErrorKind::ValueValidation,
                format!(
                    "invalid value '{s}' for '{}': {err_msg}\n",
                    arg.map(|arg| arg.to_string())
                        .unwrap_or_else(|| "--round <issue#=round>".to_string())
                ),
            )
        })
    }
}

/// Collapse repeated `--round` arguments into one target per issue.
///
/// An issue named twice is reported rather than resolved last-one-wins: a file appears
/// at most once in an archive, so two rounds for one file is a contradiction in the
/// request, not a preference.
pub fn round_targets(rounds: &[IssueRoundArg]) -> Result<HashMap<u64, ArchiveTarget>> {
    let mut targets: HashMap<u64, ArchiveTarget> = HashMap::new();
    for selection in rounds {
        if let Some(ArchiveTarget::Round(existing)) =
            targets.insert(selection.issue_number, selection.target())
        {
            bail!(
                "--round names issue #{} more than once (round {existing} and round {}): \
                 a file is archived at exactly one round",
                selection.issue_number,
                selection.round
            );
        }
    }
    Ok(targets)
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
        assert!(arg.include_diff);

        // URL with description
        let arg: IssueUrlArg = "https://github.com/owner/repo/issues/456::This is a description"
            .parse()
            .unwrap();
        assert_eq!(arg.url, "https://github.com/owner/repo/issues/456");
        assert_eq!(arg.issue_number, 456);
        assert_eq!(arg.description, Some("This is a description".to_string()));
        assert!(arg.include_diff);

        // URL with no_diff suffix (no description)
        let arg: IssueUrlArg = "https://github.com/owner/repo/issues/789::no_diff"
            .parse()
            .unwrap();
        assert_eq!(arg.url, "https://github.com/owner/repo/issues/789");
        assert_eq!(arg.issue_number, 789);
        assert!(arg.description.is_none());
        assert!(!arg.include_diff);

        // URL with description and no_diff suffix
        let arg: IssueUrlArg = "https://github.com/owner/repo/issues/012::My desc::no_diff"
            .parse()
            .unwrap();
        assert_eq!(arg.url, "https://github.com/owner/repo/issues/012");
        assert_eq!(arg.description, Some("My desc".to_string()));
        assert!(!arg.include_diff);

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

    #[test]
    fn test_issue_round_arg_parsing() {
        let arg: IssueRoundArg = "42=1".parse().unwrap();
        assert_eq!(arg.issue_number, 42);
        assert_eq!(arg.round, 1);
        assert_eq!(arg.target(), ArchiveTarget::Round(1));

        // Whitespace around either half is tolerated
        let arg: IssueRoundArg = " 7 = 3 ".parse().unwrap();
        assert_eq!(arg.issue_number, 7);
        assert_eq!(arg.round, 3);

        // Missing separator
        assert!("42".parse::<IssueRoundArg>().is_err());
        // Missing either half
        assert!("42=".parse::<IssueRoundArg>().is_err());
        assert!("=1".parse::<IssueRoundArg>().is_err());
        // Non-numeric halves
        assert!("abc=1".parse::<IssueRoundArg>().is_err());
        assert!("42=abc".parse::<IssueRoundArg>().is_err());
        // A second separator lands in the round half and is rejected there
        assert!("42=1=2".parse::<IssueRoundArg>().is_err());
        // Rounds are 1-based; round 0 names no round on any thread
        assert!("42=0".parse::<IssueRoundArg>().is_err());
        assert!("0=1".parse::<IssueRoundArg>().is_err());
        // Negative numbers are not issue numbers or rounds
        assert!("-42=1".parse::<IssueRoundArg>().is_err());
        assert!("42=-1".parse::<IssueRoundArg>().is_err());
    }

    #[test]
    fn test_round_targets_maps_each_issue_once() {
        let targets = round_targets(&[
            IssueRoundArg {
                issue_number: 42,
                round: 2,
            },
            IssueRoundArg {
                issue_number: 43,
                round: 1,
            },
        ])
        .unwrap();

        assert_eq!(targets.len(), 2);
        assert_eq!(targets.get(&42), Some(&ArchiveTarget::Round(2)));
        assert_eq!(targets.get(&43), Some(&ArchiveTarget::Round(1)));
        // Unlisted issues are absent, which is how the caller reads "latest round"
        assert_eq!(targets.get(&44), None);
    }

    #[test]
    fn test_round_targets_rejects_a_duplicate_issue() {
        let err = round_targets(&[
            IssueRoundArg {
                issue_number: 42,
                round: 1,
            },
            IssueRoundArg {
                issue_number: 42,
                round: 2,
            },
        ])
        .unwrap_err()
        .to_string();

        assert!(err.contains("#42"), "error should name the issue: {err}");
        assert!(
            err.contains("more than once"),
            "error should say what is wrong: {err}"
        );

        // Repeating the *same* round is still a duplicate: the request says one thing
        // twice, and honouring it silently is how a contradiction goes unnoticed.
        assert!(
            round_targets(&[
                IssueRoundArg {
                    issue_number: 42,
                    round: 1,
                },
                IssueRoundArg {
                    issue_number: 42,
                    round: 1,
                },
            ])
            .is_err()
        );
    }
}
