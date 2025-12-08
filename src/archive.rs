use std::{
    collections::HashMap,
    fmt,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use gix::ObjectId;
use serde::{Deserialize, Serialize};

use crate::{GitFileOps, GitFileOpsError, IssueError, IssueThread, utils::EnvProvider};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveQC {
    pub milestone: String,
    pub approved: bool,
}

fn display_as_string<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: fmt::Display,
{
    serializer.serialize_str(&value.to_string())
}

fn parse_from_string<'de, D>(deserializer: D) -> Result<ObjectId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    ObjectId::from_hex(s.as_bytes()).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveFile {
    pub repository_file: PathBuf,
    pub archive_file: PathBuf,
    #[serde(
        serialize_with = "display_as_string",
        deserialize_with = "parse_from_string"
    )]
    pub commit: ObjectId,
    // archive file will only ever have a milestone AND approval status or neither
    #[serde(flatten)]
    pub qc: Option<ArchiveQC>,
}

impl ArchiveFile {
    pub fn file_content(&self, git_info: &impl GitFileOps) -> Result<Vec<u8>, GitFileOpsError> {
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

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
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

            let error_message =
                format!("Conflicts detected:\n{}", conflict_descriptions.join("\n"));

            return Err(ArchiveError::FileConflict(error_message));
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
    #[error("Cannot create archive: multiple files have the same archive name '{0}'")]
    FileConflict(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Failed to determine commit for {0}")]
    CommitDetermination(PathBuf),
    #[error("Failed to serialize metadata: {0}")]
    Serde(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IssueCommit, IssueThread, git::MockGitFileOps, issue::CommitState, utils::MockEnvProvider,
    };
    use flate2::read::GzDecoder;
    use gix::ObjectId;
    use std::collections::HashMap;
    use tar::Archive;
    use tempfile::TempDir;

    fn create_test_object_id(suffix: &str) -> ObjectId {
        // Create a valid 40-character hex string for SHA-1
        let hex_str = format!("{:0<40}", format!("deadbeef{}", suffix));
        ObjectId::from_hex(hex_str.as_bytes()).unwrap()
    }

    fn create_test_issue_thread() -> IssueThread {
        IssueThread {
            file: PathBuf::from("src/test.rs"),
            branch: "main".to_string(),
            open: false,
            commits: vec![
                IssueCommit {
                    hash: create_test_object_id("123"),
                    message: "Initial commit".to_string(),
                    state: CommitState::Initial,
                    file_changed: true,
                    reviewed: false,
                },
                IssueCommit {
                    hash: create_test_object_id("456"),
                    message: "Fix bug".to_string(),
                    state: CommitState::Approved,
                    file_changed: true,
                    reviewed: true,
                },
            ],
            milestone: "v1.0".to_string(),
        }
    }

    fn setup_mock_env_with_user() -> MockEnvProvider {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USER"))
            .returning(|_| Ok("test_user".to_string()));
        mock_env
    }

    fn setup_mock_env_no_user() -> MockEnvProvider {
        let mut mock_env = MockEnvProvider::new();
        mock_env
            .expect_var()
            .with(mockall::predicate::eq("USER"))
            .returning(|_| Err(std::env::VarError::NotPresent));
        mock_env
    }

    #[test]
    fn test_archive_metadata_new_success() {
        let mock_env = setup_mock_env_with_user();

        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("src/main.rs"),
                commit: create_test_object_id("123"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    approved: true,
                }),
            },
            ArchiveFile {
                repository_file: PathBuf::from("src/lib.rs"),
                archive_file: PathBuf::from("src/lib.rs"),
                commit: create_test_object_id("456"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    approved: false,
                }),
            },
        ];

        let result = ArchiveMetadata::new(files.clone(), &mock_env);
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert_eq!(metadata.creator, Some("test_user".to_string()));
        assert_eq!(metadata.files.len(), 2);
        assert_eq!(
            metadata.files[0].repository_file,
            PathBuf::from("src/main.rs")
        );
        assert_eq!(
            metadata.files[1].repository_file,
            PathBuf::from("src/lib.rs")
        );
    }

    #[test]
    fn test_archive_metadata_new_no_user() {
        let mock_env = setup_mock_env_no_user();

        let files = vec![ArchiveFile {
            repository_file: PathBuf::from("src/main.rs"),
            archive_file: PathBuf::from("main.rs"),
            commit: create_test_object_id("123"),
            qc: None,
        }];

        let result = ArchiveMetadata::new(files, &mock_env);
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert_eq!(metadata.creator, None);
        assert_eq!(metadata.files.len(), 1);
    }

    #[test]
    fn test_archive_metadata_new_duplicate_paths_error() {
        let mock_env = setup_mock_env_with_user();

        // Create files that will conflict in the archive (same archive_file path)
        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("main.rs"), // Flattened path
                commit: create_test_object_id("123"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("tests/main.rs"),
                archive_file: PathBuf::from("main.rs"), // Same flattened path!
                commit: create_test_object_id("456"),
                qc: None,
            },
        ];

        let result = ArchiveMetadata::new(files, &mock_env);
        assert!(result.is_err());

        match result.unwrap_err() {
            ArchiveError::FileConflict(msg) => {
                assert!(msg.contains("Conflicts detected"));
                assert!(msg.contains("src/main.rs + tests/main.rs -> main.rs"));
            }
            _ => panic!("Expected FileConflict error"),
        }
    }

    #[test]
    fn test_archive_metadata_new_multiple_conflicts() {
        let mock_env = setup_mock_env_with_user();

        let files = vec![
            // First conflict: main.rs
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("main.rs"),
                commit: create_test_object_id("123"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("tests/main.rs"),
                archive_file: PathBuf::from("main.rs"),
                commit: create_test_object_id("456"),
                qc: None,
            },
            // Second conflict: config.rs
            ArchiveFile {
                repository_file: PathBuf::from("src/config.rs"),
                archive_file: PathBuf::from("config.rs"),
                commit: create_test_object_id("789"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("lib/config.rs"),
                archive_file: PathBuf::from("config.rs"),
                commit: create_test_object_id("abc"),
                qc: None,
            },
        ];

        let result = ArchiveMetadata::new(files, &mock_env);
        assert!(result.is_err());

        match result.unwrap_err() {
            ArchiveError::FileConflict(msg) => {
                assert!(msg.contains("Conflicts detected"));
                // Should contain both conflicts
                assert!(msg.contains("main.rs"));
                assert!(msg.contains("config.rs"));
            }
            _ => panic!("Expected FileConflict error"),
        }
    }

    #[test]
    fn test_archive_file_from_issue_thread_approved() {
        let issue_thread = create_test_issue_thread();

        let result = ArchiveFile::from_issue_thread(&issue_thread, false);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.repository_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.archive_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.commit, create_test_object_id("456")); // Approved commit

        let qc = archive_file.qc.unwrap();
        assert_eq!(qc.milestone, "v1.0");
        assert!(qc.approved);
    }

    #[test]
    fn test_archive_file_from_issue_thread_flattened() {
        let issue_thread = create_test_issue_thread();

        let result = ArchiveFile::from_issue_thread(&issue_thread, true);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.repository_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.archive_file, PathBuf::from("test.rs")); // Flattened
        assert_eq!(archive_file.commit, create_test_object_id("456"));
    }

    #[test]
    fn test_archive_file_from_issue_thread_not_approved() {
        let mut issue_thread = create_test_issue_thread();
        // Remove the approved commit, should use latest instead
        // Commits are stored newest first, so 789 should be first to be "latest"
        issue_thread.commits = vec![
            IssueCommit {
                hash: create_test_object_id("789"),
                message: "Latest commit".to_string(),
                state: CommitState::Notification,
                file_changed: true,
                reviewed: false,
            },
            IssueCommit {
                hash: create_test_object_id("123"),
                message: "Initial commit".to_string(),
                state: CommitState::Initial,
                file_changed: true,
                reviewed: false,
            },
        ];

        let result = ArchiveFile::from_issue_thread(&issue_thread, false);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.commit, create_test_object_id("789")); // Latest commit

        let qc = archive_file.qc.unwrap();
        assert!(!qc.approved);
    }

    #[test]
    fn test_archive_file_from_issue_thread_no_commits() {
        let mut issue_thread = create_test_issue_thread();
        issue_thread.commits = vec![];

        let result = ArchiveFile::from_issue_thread(&issue_thread, false);
        assert!(result.is_err());

        match result.unwrap_err() {
            ArchiveError::CommitDetermination(path) => {
                assert_eq!(path, PathBuf::from("src/test.rs"));
            }
            _ => panic!("Expected CommitDetermination error"),
        }
    }

    #[test]
    fn test_archive_file_from_file() {
        let file_path = PathBuf::from("src/example.rs");
        let commit = create_test_object_id("123");

        let archive_file = ArchiveFile::from_file(&file_path, commit.clone(), false);

        assert_eq!(archive_file.repository_file, file_path);
        assert_eq!(archive_file.archive_file, PathBuf::from("src/example.rs"));
        assert_eq!(archive_file.commit, commit);
        assert!(archive_file.qc.is_none());
    }

    #[test]
    fn test_archive_file_from_file_flattened() {
        let file_path = PathBuf::from("src/example.rs");
        let commit = create_test_object_id("123");

        let archive_file = ArchiveFile::from_file(&file_path, commit.clone(), true);

        assert_eq!(archive_file.repository_file, file_path);
        assert_eq!(archive_file.archive_file, PathBuf::from("example.rs")); // Flattened
        assert_eq!(archive_file.commit, commit);
    }

    #[test]
    fn test_archive_file_content() {
        let mut mock_git = MockGitFileOps::new();
        let file_content = b"fn main() { println!(\"Hello\"); }";
        let commit = create_test_object_id("123");

        mock_git
            .expect_file_bytes_at_commit()
            .with(
                mockall::predicate::eq(PathBuf::from("src/main.rs")),
                mockall::predicate::eq(commit.clone()),
            )
            .returning(move |_, _| Ok(file_content.to_vec()));

        let archive_file = ArchiveFile {
            repository_file: PathBuf::from("src/main.rs"),
            archive_file: PathBuf::from("main.rs"),
            commit: commit.clone(),
            qc: None,
        };

        let result = archive_file.file_content(&mock_git);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), file_content.to_vec());
    }

    #[test]
    fn test_archive_creates_valid_tar_gz() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test_archive.tar.gz");

        let mut mock_git = MockGitFileOps::new();
        let file1_content = b"content of file1";
        let file2_content = b"content of file2";

        mock_git
            .expect_file_bytes_at_commit()
            .with(
                mockall::predicate::eq(PathBuf::from("src/file1.rs")),
                mockall::predicate::eq(create_test_object_id("123")),
            )
            .returning(move |_, _| Ok(file1_content.to_vec()));

        mock_git
            .expect_file_bytes_at_commit()
            .with(
                mockall::predicate::eq(PathBuf::from("src/file2.rs")),
                mockall::predicate::eq(create_test_object_id("456")),
            )
            .returning(move |_, _| Ok(file2_content.to_vec()));

        let mock_env = setup_mock_env_with_user();

        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/file1.rs"),
                archive_file: PathBuf::from("file1.rs"),
                commit: create_test_object_id("123"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    approved: true,
                }),
            },
            ArchiveFile {
                repository_file: PathBuf::from("src/file2.rs"),
                archive_file: PathBuf::from("file2.rs"),
                commit: create_test_object_id("456"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    approved: false,
                }),
            },
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let result = archive(metadata, &mock_git, &archive_path);

        assert!(result.is_ok());
        assert!(archive_path.exists());

        // Verify the archive can be read and contains expected files
        let file = std::fs::File::open(&archive_path).unwrap();
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_string_lossy().to_string();
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
            entries.insert(path, contents);
        }

        // Should contain metadata file + 2 source files
        assert_eq!(entries.len(), 3);
        assert!(entries.contains_key("ghqc_archive_metadata.json"));
        assert!(entries.contains_key("file1.rs"));
        assert!(entries.contains_key("file2.rs"));

        // Verify file contents
        assert_eq!(entries["file1.rs"], file1_content);
        assert_eq!(entries["file2.rs"], file2_content);

        // Verify metadata file contains valid JSON
        let metadata_content =
            String::from_utf8(entries["ghqc_archive_metadata.json"].clone()).unwrap();
        let parsed_metadata: ArchiveMetadata = serde_json::from_str(&metadata_content).unwrap();
        assert_eq!(parsed_metadata.creator, Some("test_user".to_string()));
        assert_eq!(parsed_metadata.files.len(), 2);
    }

    #[test]
    fn test_archive_creates_directory_structure() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir
            .path()
            .join("nested")
            .join("directory")
            .join("archive.tar.gz");

        let mut mock_git = MockGitFileOps::new();
        let file_content = b"test content";

        mock_git
            .expect_file_bytes_at_commit()
            .returning(move |_, _| Ok(file_content.to_vec()));

        let mock_env = setup_mock_env_with_user();

        let files = vec![ArchiveFile {
            repository_file: PathBuf::from("src/test.rs"),
            archive_file: PathBuf::from("test.rs"),
            commit: create_test_object_id("123"),
            qc: None,
        }];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let result = archive(metadata, &mock_git, &nested_path);

        assert!(result.is_ok());
        assert!(nested_path.exists());
        assert!(nested_path.parent().unwrap().is_dir());
    }

    #[test]
    fn test_archive_preserves_directory_structure() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("structured_archive.tar.gz");

        let mut mock_git = MockGitFileOps::new();
        let file_content = b"content";

        mock_git
            .expect_file_bytes_at_commit()
            .returning(move |_, _| Ok(file_content.to_vec()));

        let mock_env = setup_mock_env_with_user();

        let files = vec![
            ArchiveFile {
                repository_file: PathBuf::from("src/main.rs"),
                archive_file: PathBuf::from("src/main.rs"), // Keep directory structure
                commit: create_test_object_id("123"),
                qc: None,
            },
            ArchiveFile {
                repository_file: PathBuf::from("tests/integration.rs"),
                archive_file: PathBuf::from("tests/integration.rs"), // Keep directory structure
                commit: create_test_object_id("456"),
                qc: None,
            },
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let result = archive(metadata, &mock_git, &archive_path);

        assert!(result.is_ok());

        // Verify directory structure is preserved in archive
        let file = std::fs::File::open(&archive_path).unwrap();
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        let paths: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"tests/integration.rs".to_string()));
        assert!(paths.contains(&"ghqc_archive_metadata.json".to_string()));
    }
}
