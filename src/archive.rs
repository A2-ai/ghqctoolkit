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

/// The frozen QC facts for one archived file (M10).
///
/// A **detached audit snapshot** (D28.3): `approved`, `round`, and
/// `subsequent_file_changes` are all derivable from a live `IssueThread`, but no thread
/// is present when this is read back, so freezing them here is correct and explicitly
/// exempted from D26. D15 marks the line — freeze *observations*, never *claims that go
/// stale*, which is why there is deliberately **no total round count**: any later round
/// would invalidate it.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveQC {
    pub milestone: String,
    pub approved: bool,
    /// The 1-based index of the archived round (D15).
    pub round: u32,
    /// Whether file-changing **commits** exist after the archived commit (R11: a dirty
    /// working tree is a separate signal and never feeds this field).
    pub subsequent_file_changes: bool,
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

    /// Archive one QC-attached file at the selected round (D15).
    ///
    /// `round` is the 1-based round index; `None` selects the latest round (C4/U5
    /// default). The commit is that round's, via the single definition in
    /// `Round::latest_commit()` (M8) — never restated here.
    pub fn from_issue_thread(
        issue_thread: &IssueThread,
        round: Option<u32>,
        flatten: bool,
    ) -> Result<Self, ArchiveError> {
        let round = match round {
            Some(index) => {
                issue_thread
                    .round(index)
                    .ok_or_else(|| ArchiveError::RoundNotFound {
                        file: issue_thread.file.clone(),
                        round: index,
                    })?
            }
            None => issue_thread.latest_round(),
        };
        // D55: refuse, never substitute. `latest_commit()` is `None` when the round
        // could not be placed (D54.1) or when its approval was force-pushed off its
        // branch (D54.2); freezing any other commit here would archive content the
        // metadata then claims was approved — the audit lie D15/D28.3 forbid.
        let commit = round
            .latest_commit()
            .ok_or_else(|| ArchiveError::RoundCommitUnresolved {
                file: issue_thread.file.clone(),
                round: round.index,
                branch: round.branch.clone(),
            })?
            .hash;
        let approved = round.is_closed();
        let subsequent_file_changes = subsequent_file_changes(issue_thread, &commit);

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
                round: round.index,
                subsequent_file_changes,
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

/// W3: are there file-changing commits after `commit`?
///
/// The walk itself lives in `IssueThread::file_commits_after`, which goes over
/// `segments()` newest→oldest. R11: committed changes only — the working tree never
/// enters here.
///
/// **D39.3:** when **any** gap in the thread is `divergent`, segment order no longer
/// implies commit order (a divergent gap is walked with no `stop_at`, so it can hold
/// commits older than the round before it), which makes W3's ordering unsound. In that
/// case the answer is conservatively `true`: for an audit artefact the safe direction is
/// to claim "changes may exist after this commit", never the reverse.
fn subsequent_file_changes(issue_thread: &IssueThread, commit: &ObjectId) -> bool {
    let divergent = issue_thread.drift.divergent
        || issue_thread
            .rounds
            .iter()
            .any(|round| round.preceding_gap.divergent);
    if divergent {
        log::debug!(
            "Divergent gap in {}: reporting subsequent_file_changes conservatively as true (D39.3)",
            issue_thread.file.display()
        );
        return true;
    }

    !issue_thread.file_commits_after(commit).is_empty()
}

/// One file the milestone archive does **not** contain, and why (D61).
///
/// A frozen observation at archive time, like [`ArchiveQC`] and equally exempt from D26:
/// it records what was true when the archive was written and never becomes a claim that
/// goes stale. It exists because a terminal warning scrolls away and the archive
/// outlives it — a partial archive whose own manifest does not say it is partial is the
/// audit hazard this design keeps guarding against (cf. D39.3, D45).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct SkippedFile {
    pub repository_file: PathBuf,
    /// The 1-based index of the round that was selected but could not be resolved.
    pub round: u32,
    pub branch: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ArchiveMetadata {
    creator: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    files: Vec<ArchiveFile>,
    /// D61: the files this archive omits. `default` + `skip_serializing_if` keep a
    /// **complete** archive's manifest byte-for-byte what it was — the key is absent, not
    /// `[]` — and let archives written before this field deserialize unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skipped: Vec<SkippedFile>,
}

impl ArchiveMetadata {
    /// A **complete** archive: nothing was skipped, so the manifest keeps its existing
    /// shape (D61). Paths that can skip must use [`ArchiveMetadata::new_with_skipped`].
    pub fn new(files: Vec<ArchiveFile>, env: &impl EnvProvider) -> Result<Self, ArchiveError> {
        Self::new_with_skipped(files, Vec::new(), env)
    }

    /// D61: an archive that carries its own record of what it omits.
    pub fn new_with_skipped(
        files: Vec<ArchiveFile>,
        skipped: Vec<SkippedFile>,
        env: &impl EnvProvider,
    ) -> Result<Self, ArchiveError> {
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
            skipped,
        })
    }
}

/// Build the archive entries for a set of QC-attached threads, **skipping** any whose
/// selected round has no resolvable commit and returning the record of what was skipped
/// (D61).
///
/// This is the multi-file (`ghqc milestone archive`) path, and it **replaces** D55's
/// abort-the-run behaviour there: the user's remedy is often "select a different round
/// that resolves" rather than "fetch the branch", and aborting a fifty-file milestone
/// over one stale branch denies them both the archive and the choice. The round picker
/// already renders `fetch <branch>` per round (D53.5), so an unresolvable round is
/// visible before selection.
///
/// The single-file API path keeps D55's error — see [`ArchiveFile::from_issue_thread`],
/// which still refuses. **No path substitutes a commit.** Every other archive error
/// (a round that does not exist, for instance) still aborts.
pub fn archive_files_for_threads<'a, I>(
    selections: I,
    flatten: bool,
) -> Result<(Vec<ArchiveFile>, Vec<SkippedFile>), ArchiveError>
where
    I: IntoIterator<Item = (&'a IssueThread, Option<u32>)>,
{
    let mut files = Vec::new();
    let mut skipped = Vec::new();

    for (issue_thread, round) in selections {
        match ArchiveFile::from_issue_thread(issue_thread, round, flatten) {
            Ok(file) => files.push(file),
            Err(ArchiveError::RoundCommitUnresolved {
                file,
                round,
                branch,
            }) => {
                // D60: the reason distinguishes an unfetched branch from an approval
                // rewritten off a branch the user already has, so the record says which
                // remedy applies.
                let reason = issue_thread
                    .round(round)
                    .and_then(|selected| selected.unresolved_commit_error())
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| format!("round {round} has no resolvable commit"));
                log::warn!(
                    "Skipping {} from the archive: round {round} on '{branch}' has no \
                     resolvable commit ({reason})",
                    file.display()
                );
                skipped.push(SkippedFile {
                    repository_file: file,
                    round,
                    branch,
                    reason,
                });
            }
            Err(other) => return Err(other),
        }
    }

    Ok((files, skipped))
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
    #[error("Round {round} does not exist for {}", file.display())]
    RoundNotFound { file: PathBuf, round: u32 },
    /// D55: the selected round's commit does not resolve — the round could not be
    /// placed, or its approval is no longer on its branch. Naming the branch is the
    /// whole point: it is the user's remedy.
    #[error("fetch '{branch}' to archive round {round} of {}", file.display())]
    RoundCommitUnresolved {
        file: PathBuf,
        round: u32,
        branch: String,
    },
    #[error("Failed to serialize metadata: {0}")]
    Serde(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IssueCommit, IssueThread,
        git::MockGitFileOps,
        issue::{Approval, CommitStatus, Gap, Round, RoundChecklist, RoundPlacement, RoundState},
        utils::MockEnvProvider,
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

    /// A single-round thread whose round is approved at its newest commit.
    fn create_test_issue_thread() -> IssueThread {
        let start = create_test_object_id("123");
        let approval = create_test_object_id("456");

        IssueThread {
            file: PathBuf::from("src/test.rs"),
            milestone: "v1.0".to_string(),
            open: false,
            blocking_qcs: vec![],
            rounds: vec![Round {
                index: 1,
                branch: "main".to_string(),
                branch_inherited: false,
                placement: RoundPlacement::Placed,
                start_commit: start,
                preceding_gap: Gap::default(),
                checklist: RoundChecklist::default(),
                // newest-first, per D8
                commits: vec![
                    IssueCommit {
                        hash: approval,
                        message: "Fix bug".to_string(),
                        statuses: {
                            let mut set = std::collections::HashSet::new();
                            set.insert(CommitStatus::Reviewed);
                            set
                        },
                        file_changed: true,
                    },
                    IssueCommit {
                        hash: start,
                        message: "Initial commit".to_string(),
                        statuses: {
                            let mut set = std::collections::HashSet::new();
                            set.insert(CommitStatus::Initial);
                            set
                        },
                        file_changed: true,
                    },
                ],
                // D33: approvedness lives here and nowhere else.
                state: RoundState::Approved(Approval {
                    commit: approval,
                    comment_id: None,
                }),
            }],
            drift: Gap::default(),
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
                    round: 1,
                    subsequent_file_changes: false,
                }),
            },
            ArchiveFile {
                repository_file: PathBuf::from("src/lib.rs"),
                archive_file: PathBuf::from("src/lib.rs"),
                commit: create_test_object_id("456"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    approved: false,
                    round: 1,
                    subsequent_file_changes: false,
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

        let result = ArchiveFile::from_issue_thread(&issue_thread, None, false);
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

        let result = ArchiveFile::from_issue_thread(&issue_thread, None, true);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.repository_file, PathBuf::from("src/test.rs"));
        assert_eq!(archive_file.archive_file, PathBuf::from("test.rs")); // Flattened
        assert_eq!(archive_file.commit, create_test_object_id("456"));
    }

    /// Newest-first commit with the given statuses.
    fn commit(suffix: &str, status: Option<CommitStatus>, file_changed: bool) -> IssueCommit {
        let mut statuses = std::collections::HashSet::new();
        if let Some(status) = status {
            statuses.insert(status);
        }
        IssueCommit {
            hash: create_test_object_id(suffix),
            message: format!("commit {suffix}"),
            statuses,
            file_changed,
        }
    }

    /// A two-round thread, both rounds approved:
    /// `R1[c1..c2 approved] G2[] R2[c3..c4 approved] S[]`.
    fn create_two_round_issue_thread() -> IssueThread {
        let round = |index: u32, start: &str, approval: &str| Round {
            index,
            branch: "main".to_string(),
            branch_inherited: false,
            placement: RoundPlacement::Placed,
            start_commit: create_test_object_id(start),
            preceding_gap: Gap::default(),
            checklist: RoundChecklist::default(),
            commits: vec![
                commit(approval, Some(CommitStatus::Reviewed), true),
                commit(start, Some(CommitStatus::Initial), true),
            ],
            state: RoundState::Approved(Approval {
                commit: create_test_object_id(approval),
                comment_id: None,
            }),
        };

        IssueThread {
            file: PathBuf::from("src/test.rs"),
            milestone: "v1.0".to_string(),
            open: false,
            blocking_qcs: vec![],
            rounds: vec![round(1, "c1", "c2"), round(2, "c3", "c4")],
            drift: Gap::default(),
        }
    }

    #[test]
    fn test_archive_file_from_issue_thread_selects_requested_round() {
        let issue_thread = create_two_round_issue_thread();

        // D15: selection is per round, and the commit is that round's
        // `latest_commit()` (M8) — round 1's approval, not the thread's newest commit.
        let archive_file = ArchiveFile::from_issue_thread(&issue_thread, Some(1), false).unwrap();

        assert_eq!(archive_file.commit, create_test_object_id("c2"));
        let qc = archive_file.qc.unwrap();
        assert_eq!(qc.round, 1);
        assert!(qc.approved);

        // The latest round remains the default.
        let latest = ArchiveFile::from_issue_thread(&issue_thread, None, false).unwrap();
        assert_eq!(latest.commit, create_test_object_id("c4"));
        assert_eq!(latest.qc.unwrap().round, 2);
    }

    #[test]
    fn test_archive_file_from_issue_thread_unknown_round_errors() {
        let issue_thread = create_two_round_issue_thread();

        match ArchiveFile::from_issue_thread(&issue_thread, Some(3), false) {
            Err(ArchiveError::RoundNotFound { file, round }) => {
                assert_eq!(file, PathBuf::from("src/test.rs"));
                assert_eq!(round, 3);
            }
            other => panic!("Expected RoundNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_subsequent_file_changes_true_when_a_later_segment_touches_the_file() {
        let issue_thread = create_two_round_issue_thread();

        // W3: round 2 owns file-changing commits newer than round 1's approval.
        let round_one = ArchiveFile::from_issue_thread(&issue_thread, Some(1), false).unwrap();
        assert!(round_one.qc.unwrap().subsequent_file_changes);
    }

    #[test]
    fn test_subsequent_file_changes_false_when_nothing_follows() {
        let issue_thread = create_two_round_issue_thread();

        // Nothing at all after round 2's approval: empty drift, no later round.
        let round_two = ArchiveFile::from_issue_thread(&issue_thread, Some(2), false).unwrap();
        assert!(!round_two.qc.unwrap().subsequent_file_changes);
    }

    #[test]
    fn test_subsequent_file_changes_true_when_drift_touches_the_file() {
        let mut issue_thread = create_two_round_issue_thread();
        issue_thread.drift.commits = vec![commit("c5", None, true)];

        let round_two = ArchiveFile::from_issue_thread(&issue_thread, Some(2), false).unwrap();
        assert!(round_two.qc.unwrap().subsequent_file_changes);
    }

    #[test]
    fn test_subsequent_file_changes_false_when_drift_leaves_the_file_alone() {
        let mut issue_thread = create_two_round_issue_thread();
        // A post-approval commit that does not touch the QC'd file.
        issue_thread.drift.commits = vec![commit("c5", None, false)];

        let round_two = ArchiveFile::from_issue_thread(&issue_thread, Some(2), false).unwrap();
        assert!(!round_two.qc.unwrap().subsequent_file_changes);
    }

    #[test]
    fn test_divergent_gap_forces_subsequent_file_changes_true() {
        // D39.3: a divergent gap is walked with no `stop_at`, so segment order no longer
        // implies commit order and W3's ordering is unsound. Conservative `true` claims
        // "changes may exist after this commit", never the reverse.
        let mut divergent_preceding_gap = create_two_round_issue_thread();
        divergent_preceding_gap.rounds[1].preceding_gap.divergent = true;

        // Without the divergence this exact selection is `false`
        // (test_subsequent_file_changes_false_when_nothing_follows).
        let archive_file =
            ArchiveFile::from_issue_thread(&divergent_preceding_gap, Some(2), false).unwrap();
        assert!(archive_file.qc.unwrap().subsequent_file_changes);

        // Same fact in the other gap position: an approval rewritten off its branch
        // (D31).
        let mut divergent_drift = create_two_round_issue_thread();
        divergent_drift.drift.divergent = true;

        let archive_file =
            ArchiveFile::from_issue_thread(&divergent_drift, Some(2), false).unwrap();
        assert!(archive_file.qc.unwrap().subsequent_file_changes);
    }

    #[test]
    fn test_subsequent_file_changes_ignores_working_tree_dirtiness() {
        // R11 (committed changes only) is enforced **structurally, not by this test**:
        // `subsequent_file_changes` is computed by `from_issue_thread`, which takes only
        // an `&IssueThread`, a round index and a flag — no git handle and no dirty bit are
        // in scope, so a dirty working tree cannot reach the field. Nothing below
        // constructs a dirty-tree signal, because there is nothing to construct it with.
        // What the assertion actually covers is the committed path, and it is the same
        // case as `test_subsequent_file_changes_false_when_drift_leaves_the_file_alone`:
        // a post-approval commit that does not touch the file reports `false`.
        let mut issue_thread = create_two_round_issue_thread();
        issue_thread.drift.commits = vec![commit("c5", None, false)];

        let archive_file = ArchiveFile::from_issue_thread(&issue_thread, Some(2), false).unwrap();
        assert!(!archive_file.qc.unwrap().subsequent_file_changes);
    }

    #[test]
    fn test_archive_qc_records_no_total_round_count() {
        // D15: the metadata records the round index, never a total — a total is a claim
        // any later round invalidates.
        let issue_thread = create_two_round_issue_thread();
        let archive_file = ArchiveFile::from_issue_thread(&issue_thread, Some(1), false).unwrap();

        let json = serde_json::to_value(&archive_file).unwrap();
        let mut keys = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "approved",
                "archive_file",
                "commit",
                "milestone",
                "repository_file",
                "round",
                "subsequent_file_changes",
            ]
        );
        assert_eq!(json["round"], 1);
    }

    /// D55: selecting a round whose commit does not resolve is an **error naming the
    /// branch** — never a substituted commit. This is the off-branch-approval case
    /// (D54.2): the approval was force-pushed away, so the round owns no commit that was
    /// approved. Freezing the newest commit instead would make `ArchiveQC.approved: true`
    /// a claim about content nobody approved (D15/D28.3).
    #[test]
    fn test_archive_refuses_a_round_whose_approval_is_off_branch() {
        let mut issue_thread = create_two_round_issue_thread();
        issue_thread.rounds[1].state = RoundState::Approved(Approval {
            commit: create_test_object_id("f00"),
            comment_id: None,
        });

        match ArchiveFile::from_issue_thread(&issue_thread, Some(2), false) {
            Err(ArchiveError::RoundCommitUnresolved {
                file,
                round,
                branch,
            }) => {
                assert_eq!(file, PathBuf::from("src/test.rs"));
                assert_eq!(round, 2);
                assert_eq!(branch, "main");
            }
            other => panic!("expected RoundCommitUnresolved, got {other:?}"),
        }
    }

    /// D55 for the other `None` case (D54.1): an unplaceable round cannot be archived,
    /// and the message is the remedy — fetch the branch.
    #[test]
    fn test_archive_refuses_an_unplaceable_round() {
        let mut issue_thread = create_two_round_issue_thread();
        let second = &mut issue_thread.rounds[1];
        second.commits.clear();
        second.branch = "feature".to_string();
        second.placement = RoundPlacement::Unplaceable {
            branch: "feature".to_string(),
        };

        let error = ArchiveFile::from_issue_thread(&issue_thread, Some(2), false)
            .expect_err("an unplaceable round has no commit to freeze");
        assert!(
            matches!(&error, ArchiveError::RoundCommitUnresolved { branch, round, .. }
                if branch == "feature" && *round == 2),
            "got {error:?}"
        );
        // The branch is in the message, because that is the user's fix.
        assert!(error.to_string().contains("feature"), "got {error}");

        // D15/D10: the *other* round is unaffected — refusal is per selection.
        assert!(ArchiveFile::from_issue_thread(&issue_thread, Some(1), false).is_ok());
    }

    /// D61: the multi-file path **skips** the file whose selected round does not
    /// resolve and keeps going — aborting a fifty-file milestone over one stale branch
    /// denies the user both the archive and the choice of another round.
    #[test]
    fn test_archive_files_for_threads_skips_an_unresolvable_round() {
        let resolvable = create_two_round_issue_thread();
        let mut unresolvable = create_two_round_issue_thread();
        unresolvable.file = PathBuf::from("src/other.rs");
        let second = &mut unresolvable.rounds[1];
        second.commits.clear();
        second.branch = "feature".to_string();
        second.placement = RoundPlacement::Unplaceable {
            branch: "feature".to_string(),
        };

        let (files, skipped) =
            archive_files_for_threads(vec![(&resolvable, None), (&unresolvable, None)], false)
                .expect("one unresolvable round must not abort the run");

        // The resolvable file is archived at its own round's commit — nothing is
        // substituted for the skipped one.
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].repository_file, PathBuf::from("src/test.rs"));
        assert_eq!(files[0].commit, create_test_object_id("c4"));

        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].repository_file, PathBuf::from("src/other.rs"));
        assert_eq!(skipped[0].round, 2);
        assert_eq!(skipped[0].branch, "feature");
        // D60: the reason distinguishes the two causes, so it names the remedy.
        assert!(
            skipped[0].reason.contains("not checked out locally"),
            "got {}",
            skipped[0].reason
        );
    }

    /// D61 + D60: the off-branch-approval case is skipped too, and its recorded reason
    /// does **not** tell the user to fetch a branch they already have.
    #[test]
    fn test_archive_files_for_threads_records_an_off_branch_approval_reason() {
        let mut thread = create_two_round_issue_thread();
        thread.rounds[1].state = RoundState::Approved(Approval {
            commit: create_test_object_id("f00"),
            comment_id: None,
        });

        let (files, skipped) = archive_files_for_threads(vec![(&thread, None)], false).unwrap();

        assert!(files.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("no longer reachable"),
            "got {}",
            skipped[0].reason
        );
        assert!(
            !skipped[0].reason.contains("not checked out locally"),
            "got {}",
            skipped[0].reason
        );
    }

    /// Only the unresolvable-commit case is skipped: a selected round that does not
    /// exist is a caller bug, not a stale branch, and still aborts (D61).
    #[test]
    fn test_archive_files_for_threads_still_aborts_on_other_errors() {
        let thread = create_two_round_issue_thread();

        match archive_files_for_threads(vec![(&thread, Some(7))], false) {
            Err(ArchiveError::RoundNotFound { round, .. }) => assert_eq!(round, 7),
            other => panic!("expected RoundNotFound, got {other:?}"),
        }
    }

    /// D61: the skip lives in the archive, not only on stderr — a terminal warning
    /// scrolls away and a partial archive whose manifest does not say it is partial is
    /// the audit hazard this design guards against.
    #[test]
    fn test_metadata_records_skipped_files_in_the_manifest() {
        let mock_env = setup_mock_env_with_user();
        let skipped = vec![SkippedFile {
            repository_file: PathBuf::from("src/other.rs"),
            round: 2,
            branch: "feature".to_string(),
            reason: "Branch 'feature' is not checked out locally".to_string(),
        }];

        let metadata = ArchiveMetadata::new_with_skipped(Vec::new(), skipped, &mock_env).unwrap();
        let json = serde_json::to_value(&metadata).unwrap();

        let recorded = json["skipped"].as_array().expect("the skip is recorded");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["repository_file"], "src/other.rs");
        assert_eq!(recorded[0]["round"], 2);
        assert_eq!(recorded[0]["branch"], "feature");
        assert_eq!(
            recorded[0]["reason"],
            "Branch 'feature' is not checked out locally"
        );

        // And it round-trips, so a reader of an old or new archive sees the same shape.
        let parsed: ArchiveMetadata = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.skipped.len(), 1);
    }

    /// D61: a **complete** archive's manifest keeps its existing shape byte-for-byte —
    /// `skipped` is absent, not `[]` — and a manifest written before the field still
    /// deserializes.
    #[test]
    fn test_complete_archive_manifest_shape_is_unchanged() {
        let mock_env = setup_mock_env_with_user();
        let files = vec![
            ArchiveFile::from_issue_thread(&create_two_round_issue_thread(), None, false).unwrap(),
        ];

        let metadata = ArchiveMetadata::new(files, &mock_env).unwrap();
        let json = serde_json::to_value(&metadata).unwrap();

        let mut keys = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(keys, vec!["created_at", "creator", "files"]);

        // An archive written before D61 has no `skipped` key at all.
        let legacy = serde_json::json!({
            "creator": "test_user",
            "created_at": "2024-01-01T00:00:00Z",
            "files": [],
        });
        let parsed: ArchiveMetadata = serde_json::from_value(legacy).unwrap();
        assert!(parsed.skipped.is_empty());
    }

    #[test]
    fn test_archive_file_from_issue_thread_not_approved() {
        let mut issue_thread = create_test_issue_thread();
        // Revoke the approval: the archive commit falls back to the newest
        // status-bearing commit the round owns (M8).
        // Commits are stored newest first, so 789 should be first to be "latest"
        let round = &mut issue_thread.rounds[0];
        round.state = RoundState::Unapproved;
        round.commits = vec![
            IssueCommit {
                hash: create_test_object_id("789"),
                message: "Latest commit".to_string(),
                statuses: {
                    let mut set = std::collections::HashSet::new();
                    set.insert(CommitStatus::Notification);
                    set
                },
                file_changed: true,
            },
            IssueCommit {
                hash: create_test_object_id("123"),
                message: "Initial commit".to_string(),
                statuses: {
                    let mut set = std::collections::HashSet::new();
                    set.insert(CommitStatus::Initial);
                    set
                },
                file_changed: true,
            },
        ];

        let result = ArchiveFile::from_issue_thread(&issue_thread, None, false);
        assert!(result.is_ok());

        let archive_file = result.unwrap();
        assert_eq!(archive_file.commit, create_test_object_id("789")); // Latest commit

        let qc = archive_file.qc.unwrap();
        assert!(!qc.approved);
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
                    round: 1,
                    subsequent_file_changes: false,
                }),
            },
            ArchiveFile {
                repository_file: PathBuf::from("src/file2.rs"),
                archive_file: PathBuf::from("file2.rs"),
                commit: create_test_object_id("456"),
                qc: Some(ArchiveQC {
                    milestone: "v1.0".to_string(),
                    approved: false,
                    round: 1,
                    subsequent_file_changes: false,
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
