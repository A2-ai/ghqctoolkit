use crate::archive::ArchiveFile;
use crate::{GitCommit, RelevantFile};
use anyhow::{Result, anyhow, bail};
use clap::builder::TypedValueParser;
use clap::{Arg, Command, error::ErrorKind};
use gix::ObjectId;
use std::path::PathBuf;
use std::str::FromStr;

// Custom parser for clap
#[derive(Clone)]
pub struct RelevantFileParser;

impl TypedValueParser for RelevantFileParser {
    type Value = RelevantFile;

    fn parse_ref(
        &self,
        _cmd: &Command,
        arg: Option<&Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let s = value.to_str().ok_or_else(|| {
            clap::Error::raw(
                ErrorKind::InvalidUtf8,
                "Invalid UTF-8 in file specification",
            )
        })?;

        s.parse().map_err(|_| {
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
                clap::error::ContextValue::String("name:path".to_string()),
            );
            err
        })
    }
}

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
