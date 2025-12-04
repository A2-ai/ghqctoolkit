use std::{
    collections::HashMap,
    fmt,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use gix::ObjectId;
use serde::Serialize;

use crate::{GitFileOps, GitFileOpsError, IssueError, IssueThread, utils::EnvProvider};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ArchiveQC {
    milestone: String,
    approved: bool,
}

fn display_as_string<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: fmt::Display,
{
    serializer.serialize_str(&value.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveFile {
    pub(crate) repository_file: PathBuf,
    pub(crate) archive_file: PathBuf,
    #[serde(serialize_with = "display_as_string")]
    pub(crate) commit: ObjectId,
    // archive file will only ever have a milestone AND approval status or neither
    #[serde(flatten)]
    pub(crate) qc: Option<ArchiveQC>,
}

impl ArchiveFile {
    fn file_content(&self, git_info: &impl GitFileOps) -> Result<Vec<u8>, GitFileOpsError> {
        git_info.file_bytes_at_commit(&self.repository_file, &self.commit)
    }

    pub fn from_issue_thread(
        issue_thread: &IssueThread,
        flatten: bool,
    ) -> Result<Self, ArchiveError> {
        let (commit, approved) = if let Some(approved_commit) = issue_thread.approved_commit() {
            (approved_commit.hash, true)
        } else if let Some(latest_commit) = issue_thread.latest_commit() {
            (latest_commit.clone(), false)
        } else {
            return Err(ArchiveError::CommitDetermination(issue_thread.file.clone()));
        };

        let archive_file = if flatten {
            issue_thread
                .file
                .file_name()
                .map(PathBuf::from)
                .expect("File to have file name")
        } else {
            issue_thread
                .file
                .strip_prefix("/")
                .unwrap_or(&issue_thread.file)
                .to_path_buf()
        };

        Ok(Self {
            repository_file: issue_thread.file.clone(),
            archive_file,
            commit,
            qc: Some(ArchiveQC {
                milestone: issue_thread.milestone.to_string(),
                approved,
            }),
        })
    }

    pub fn from_file(file: impl AsRef<Path>, commit: ObjectId, flatten: bool) -> Self {
        let file = file.as_ref();
        let archive_file = if flatten {
            file.file_name()
                .map(PathBuf::from)
                .expect("File to have file name")
        } else {
            file.strip_prefix("/").unwrap_or(file).to_path_buf()
        };
        Self {
            repository_file: file.to_path_buf(),
            archive_file,
            commit,
            qc: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveMetadata {
    creator: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    files: Vec<ArchiveFile>,
}

impl ArchiveMetadata {
    pub fn new(files: Vec<ArchiveFile>, env: &impl EnvProvider) -> Result<Self, ArchiveError> {
        // Check for duplicate archive paths and collect ALL conflicts
        let mut path_to_sources = HashMap::new();

        for file in &files {
            let archive_path = &file.archive_file;
            let source_path = &file.repository_file;
            path_to_sources
                .entry(archive_path.clone())
                .or_insert_with(Vec::new)
                .push(source_path.clone());
        }

        // Find all conflicts (archive paths with multiple sources)
        let conflicts: Vec<_> = path_to_sources
            .into_iter()
            .filter(|(_, sources)| sources.len() > 1)
            .collect();

        if !conflicts.is_empty() {
            // Create well-formatted error message showing all conflicts
            let conflict_descriptions: Vec<String> = conflicts
                .into_iter()
                .map(|(archive_path, sources)| {
                    let sources_str = sources
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" + ");
                    format!("{} -> {}", sources_str, archive_path.display())
                })
                .collect();

            let error_message = format!(
                "Flattening conflicts detected:\n{}",
                conflict_descriptions.join("\n")
            );

            return Err(ArchiveError::FlatteningConflict(error_message));
        }

        let creator = env.var("USER").ok();
        if creator.is_none() {
            log::warn!("Failed to determine creator using environment variable USER");
        }
        Ok(Self {
            creator,
            created_at: chrono::Utc::now(),
            files,
        })
    }
}

pub fn archive(
    archive_metadata: ArchiveMetadata,
    git_info: &impl GitFileOps,
    path: impl AsRef<Path>,
) -> Result<(), ArchiveError> {
    let path = path.as_ref();
    log::debug!(
        "Writing {} files to archive at {}",
        archive_metadata.files.len(),
        path.display()
    );
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(encoder);

    log::trace!("Writing metadata file to archive at ghqc_archive_metadata.json");
    let metadata = serde_json::to_string_pretty(&archive_metadata)?;
    write_content(&mut tar, "ghqc_archive_metadata.json", metadata.as_bytes())?;

    for archive_file in archive_metadata.files {
        log::trace!(
            "Writing {} at {} to archive at {}",
            archive_file.repository_file.display(),
            archive_file.commit.to_string(),
            archive_file.archive_file.display()
        );
        let content = archive_file.file_content(git_info)?;
        write_content(&mut tar, &archive_file.archive_file, &content)?;
    }

    tar.finish()?;
    log::debug!(
        "Successfully created compressed archive at {}",
        path.display()
    );

    Ok(())
}

fn write_content(
    tar: &mut tar::Builder<GzEncoder<File>>,
    path: impl AsRef<Path>,
    content: &[u8],
) -> io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_path(path)?;
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();

    tar.append(&header, content)
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("Failed to analyze issue due to: {0}")]
    IssueError(#[from] IssueError),
    #[error("Failed to get file content at commit due to: {0}")]
    GitFileOpsError(#[from] GitFileOpsError),
    #[error("Cannot flatten archive: multiple files have the same basename '{0}'")]
    FlatteningConflict(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Failed to determine commit for {0}")]
    CommitDetermination(PathBuf),
    #[error("Failed to serialize metadata: {0}")]
    Serde(#[from] serde_json::Error),
}
