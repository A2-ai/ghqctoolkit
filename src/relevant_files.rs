use std::path::{Path, PathBuf};

use gix::ObjectId;
use octocrab::models::issues::Issue;

use crate::{
    comment_system::CommentBody,
    diff_utils,
    git::{GitFileOps, GitHelpers},
};

#[derive(Debug, Clone)]
pub enum RelevantFileClass {
    /// A QC that was done previously on this file or a closely related one (re-QC of an analysis)
    /// Must be approved before approving the current QC
    PreviousQC {
        issue_number: u64,
        /// GitHub internal issue ID (needed for creating blocking relationships)
        issue_id: Option<u64>,
        description: Option<String>,
        /// Whether to post a diff comment for this entry
        include_diff: bool,
    },
    /// A QC which the issue of interest is developed based on.
    /// Must be approved before approving the current QC
    GatingQC {
        issue_number: u64,
        /// GitHub internal issue ID (needed for creating blocking relationships)
        issue_id: Option<u64>,
        description: Option<String>,
    },
    /// A QC which provides previous context to the current QC but does not directly impact the analysis
    /// Approval status has no baring on the ability to approve the current QC
    RelevantQC {
        issue_number: u64,
        description: Option<String>,
    },
    /// A file which has no associated issue that is relevant to the current QC.
    /// A justification for the lack of QC is required
    File { justification: String },
}

#[derive(Debug, Clone)]
pub struct RelevantFile {
    pub(crate) file_name: PathBuf,
    pub(crate) class: RelevantFileClass,
}

pub(crate) fn relevant_files_section(
    relevant_files: &[RelevantFile],
    git_info: &impl GitHelpers,
) -> String {
    let mut previous = Vec::new();
    let mut gating_qc = Vec::new();
    let mut non_gating_qc = Vec::new();
    let mut rel_file = Vec::new();

    let make_issue_bullet = |issue_number: &u64, description: &Option<String>, file_name: &Path| {
        format!(
            "[{}]({}){}",
            file_name.display(),
            git_info.issue_url(*issue_number),
            description
                .as_ref()
                .map(|d| format!(" - {d}"))
                .unwrap_or_default()
        )
    };

    for file in relevant_files {
        match &file.class {
            RelevantFileClass::PreviousQC {
                issue_number,
                description,
                ..
            } => {
                previous.push(make_issue_bullet(
                    issue_number,
                    description,
                    &file.file_name,
                ));
            }
            RelevantFileClass::GatingQC {
                issue_number,
                description,
                ..
            } => {
                gating_qc.push(make_issue_bullet(
                    issue_number,
                    description,
                    &file.file_name,
                ));
            }
            RelevantFileClass::RelevantQC {
                issue_number,
                description,
                ..
            } => {
                non_gating_qc.push(make_issue_bullet(
                    issue_number,
                    description,
                    &file.file_name,
                ));
            }
            RelevantFileClass::File { justification } => {
                rel_file.push(format!(
                    "**{}** - {justification}",
                    file.file_name.display()
                ));
            }
        }
    }

    let mut res = vec!["## Relevant Files".to_string()];

    if !previous.is_empty() {
        res.push(format!("### Previous QC\n- {}", previous.join("\n- ")));
    }

    if !gating_qc.is_empty() {
        res.push(format!("### Gating QC\n- {}", gating_qc.join("\n- ")));
    }

    if !non_gating_qc.is_empty() {
        res.push(format!("### Relevant QC\n- {}", non_gating_qc.join("\n- ")));
    }

    if !rel_file.is_empty() {
        res.push(format!("### Relevant File\n- {}", rel_file.join("\n- ")));
    }

    if res.len() > 1 {
        res.join("\n\n")
    } else {
        String::new()
    }
}

/// A GitHub comment posted on a newly-created QC issue, showing the diff between
/// the previous QC file at its latest/approved commit and the current QC file at its initial commit.
pub struct PreviousQCDiffComment {
    pub issue: Issue,
    pub prev_file: PathBuf,
    pub current_file: PathBuf,
    pub prev_commit: ObjectId,
    pub current_commit: ObjectId,
    pub prev_issue_number: u64,
}

impl CommentBody for PreviousQCDiffComment {
    fn generate_body(&self, git_info: &(impl GitHelpers + GitFileOps)) -> String {
        let prev_short = &self.prev_commit.to_string()[..7];

        let metadata = vec![
            "## Metadata".to_string(),
            format!(
                "previous qc issue: [#{}: {}]({})",
                self.prev_issue_number,
                self.prev_file.display(),
                git_info.issue_url(self.prev_issue_number)
            ),
            format!(
                "[file at latest qc commit]({})",
                git_info.file_content_url(prev_short, &self.prev_file)
            ),
            format!("latest qc commit: {}", self.prev_commit),
            format!("new qc initial qc commit: {}", self.current_commit),
        ];

        let mut body = vec!["# Previous QC".to_string(), metadata.join("\n* ")];

        // Build the diff section
        let diff_section = self.build_diff(git_info);
        body.push(format!("## File Difference\n{}", diff_section));

        body.join("\n\n")
    }

    fn issue(&self) -> &Issue {
        &self.issue
    }
}

impl PreviousQCDiffComment {
    pub(crate) fn build_diff(&self, git_info: &impl GitFileOps) -> String {
        let prev_bytes = match git_info.file_bytes_at_commit(&self.prev_file, &self.prev_commit) {
            Ok(b) => b,
            Err(e) => {
                log::warn!(
                    "Could not read prev file {:?} at commit {}: {e}",
                    self.prev_file,
                    self.prev_commit
                );
                return String::new();
            }
        };
        let curr_bytes =
            match git_info.file_bytes_at_commit(&self.current_file, &self.current_commit) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!(
                        "Could not read current file {:?} at commit {}: {e}",
                        self.current_file,
                        self.current_commit
                    );
                    return String::new();
                }
            };

        match diff_utils::file_diff(prev_bytes, curr_bytes, &self.current_file) {
            Some(diff) => format!(
                "<details>\n<summary>View diff</summary>\n\n{}\n\n</details>",
                diff
            ),
            None => {
                log::warn!(
                    "Could not generate diff between {:?} and {:?}",
                    self.prev_file,
                    self.current_file
                );
                String::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        GitAuthor,
        comment_system::CommentBody,
        git::{GitCommit, GitFileOpsError},
    };
    use gix::ObjectId;
    use std::{collections::HashMap, str::FromStr};

    const PREV_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CURR_COMMIT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    struct MockGitInfo {
        file_contents: HashMap<(PathBuf, String), Vec<u8>>,
    }

    impl MockGitInfo {
        fn new() -> Self {
            Self {
                file_contents: HashMap::new(),
            }
        }

        fn with_file(mut self, file: &str, commit: &str, content: &str) -> Self {
            self.file_contents.insert(
                (PathBuf::from(file), commit.to_string()),
                content.as_bytes().to_vec(),
            );
            self
        }
    }

    impl GitHelpers for MockGitInfo {
        fn file_content_url(&self, commit: &str, file: &Path) -> String {
            format!(
                "https://github.com/owner/repo/blob/{}/{}",
                &commit[..7],
                file.display()
            )
        }
        fn commit_comparison_url(&self, _current: &ObjectId, _previous: &ObjectId) -> String {
            "https://github.com/owner/repo/compare/prev..current".to_string()
        }
        fn issue_url(&self, issue_number: u64) -> String {
            format!("https://github.com/owner/repo/issues/{issue_number}")
        }
    }

    impl GitFileOps for MockGitInfo {
        fn commits(
            &self,
            _branch: &Option<String>,
            _stop_at: Option<ObjectId>,
        ) -> Result<Vec<GitCommit>, GitFileOpsError> {
            Ok(Vec::new())
        }
        fn authors(&self, _file: &Path) -> Result<Vec<GitAuthor>, GitFileOpsError> {
            Ok(Vec::new())
        }
        fn file_bytes_at_commit(
            &self,
            file: &Path,
            commit: &ObjectId,
        ) -> Result<Vec<u8>, GitFileOpsError> {
            self.file_contents
                .get(&(file.to_path_buf(), commit.to_string()))
                .cloned()
                .ok_or_else(|| GitFileOpsError::FileNotFoundAtCommit(file.to_path_buf()))
        }
        fn branch_tip(&self, _branch: &Option<String>) -> Result<ObjectId, GitFileOpsError> {
            Err(GitFileOpsError::BranchNotFound("mock".to_string()))
        }

        fn file_touching_commits(
            &self,
            _branch: Option<String>,
            _file: &Path,
        ) -> Result<std::collections::HashSet<String>, GitFileOpsError> {
            Ok(std::collections::HashSet::new())
        }

        fn list_tree_entries(&self, _path: &str) -> Result<Vec<(String, bool)>, GitFileOpsError> {
            Ok(Vec::new())
        }
    }

    fn load_issue() -> octocrab::models::issues::Issue {
        let content = std::fs::read_to_string("src/tests/github_api/issues/test_file_issue.json")
            .expect("Failed to read issue fixture");
        serde_json::from_str(&content).expect("Failed to parse issue fixture")
    }

    fn make_diff_comment(prev_file: &str, curr_file: &str) -> PreviousQCDiffComment {
        PreviousQCDiffComment {
            issue: load_issue(),
            prev_file: PathBuf::from(prev_file),
            current_file: PathBuf::from(curr_file),
            prev_commit: ObjectId::from_str(PREV_COMMIT).unwrap(),
            current_commit: ObjectId::from_str(CURR_COMMIT).unwrap(),
            prev_issue_number: 42,
        }
    }

    // ─── relevant_files_section ───────────────────────────────────────────────

    fn git() -> MockGitInfo {
        MockGitInfo::new()
    }

    #[test]
    fn test_section_empty() {
        assert_eq!(relevant_files_section(&[], &git()), "");
    }

    #[test]
    fn test_section_previous_qc_only() {
        let files = vec![RelevantFile {
            file_name: PathBuf::from("src/old.R"),
            class: RelevantFileClass::PreviousQC {
                issue_number: 5,
                issue_id: None,
                description: None,
                include_diff: true,
            },
        }];
        let result = relevant_files_section(&files, &git());
        assert!(result.contains("### Previous QC"));
        assert!(result.contains("[src/old.R](https://github.com/owner/repo/issues/5)"));
        assert!(!result.contains("### Gating QC"));
        assert!(!result.contains("### Relevant QC"));
        assert!(!result.contains("### Relevant File"));
    }

    #[test]
    fn test_section_previous_qc_with_description() {
        let files = vec![RelevantFile {
            file_name: PathBuf::from("src/old.R"),
            class: RelevantFileClass::PreviousQC {
                issue_number: 5,
                issue_id: None,
                description: Some("Re-QC of updated model".to_string()),
                include_diff: false,
            },
        }];
        let result = relevant_files_section(&files, &git());
        assert!(result.contains("Re-QC of updated model"));
    }

    #[test]
    fn test_section_gating_qc_only() {
        let files = vec![RelevantFile {
            file_name: PathBuf::from("src/dep.R"),
            class: RelevantFileClass::GatingQC {
                issue_number: 7,
                issue_id: Some(700),
                description: None,
            },
        }];
        let result = relevant_files_section(&files, &git());
        assert!(result.contains("### Gating QC"));
        assert!(result.contains("[src/dep.R](https://github.com/owner/repo/issues/7)"));
        assert!(!result.contains("### Previous QC"));
    }

    #[test]
    fn test_section_relevant_qc_only() {
        let files = vec![RelevantFile {
            file_name: PathBuf::from("src/ctx.R"),
            class: RelevantFileClass::RelevantQC {
                issue_number: 9,
                description: Some("provides context".to_string()),
            },
        }];
        let result = relevant_files_section(&files, &git());
        assert!(result.contains("### Relevant QC"));
        assert!(result.contains("provides context"));
    }

    #[test]
    fn test_section_file_only() {
        let files = vec![RelevantFile {
            file_name: PathBuf::from("data/raw.csv"),
            class: RelevantFileClass::File {
                justification: "No QC required for raw input".to_string(),
            },
        }];
        let result = relevant_files_section(&files, &git());
        assert!(result.contains("### Relevant File"));
        assert!(result.contains("**data/raw.csv**"));
        assert!(result.contains("No QC required for raw input"));
    }

    #[test]
    fn test_section_all_types_present() {
        let files = vec![
            RelevantFile {
                file_name: PathBuf::from("src/old.R"),
                class: RelevantFileClass::PreviousQC {
                    issue_number: 1,
                    issue_id: None,
                    description: None,
                    include_diff: true,
                },
            },
            RelevantFile {
                file_name: PathBuf::from("src/gate.R"),
                class: RelevantFileClass::GatingQC {
                    issue_number: 2,
                    issue_id: None,
                    description: None,
                },
            },
            RelevantFile {
                file_name: PathBuf::from("src/ctx.R"),
                class: RelevantFileClass::RelevantQC {
                    issue_number: 3,
                    description: None,
                },
            },
            RelevantFile {
                file_name: PathBuf::from("data/raw.csv"),
                class: RelevantFileClass::File {
                    justification: "raw input".to_string(),
                },
            },
        ];
        let result = relevant_files_section(&files, &git());
        assert!(result.contains("### Previous QC"));
        assert!(result.contains("### Gating QC"));
        assert!(result.contains("### Relevant QC"));
        assert!(result.contains("### Relevant File"));
    }

    #[test]
    fn test_section_heading_always_present() {
        let files = vec![RelevantFile {
            file_name: PathBuf::from("src/old.R"),
            class: RelevantFileClass::PreviousQC {
                issue_number: 1,
                issue_id: None,
                description: None,
                include_diff: true,
            },
        }];
        let result = relevant_files_section(&files, &git());
        assert!(result.starts_with("## Relevant Files"));
    }

    // ─── PreviousQCDiffComment::generate_body ─────────────────────────────────

    #[test]
    fn test_diff_comment_body_with_changes() {
        let comment = make_diff_comment("src/old.R", "src/new.R");
        let git_info = MockGitInfo::new()
            .with_file("src/old.R", PREV_COMMIT, "line1\nline2\nline3\n")
            .with_file("src/new.R", CURR_COMMIT, "line1\nline2 changed\nline3\n");

        let body = comment.generate_body(&git_info);
        insta::assert_snapshot!("previous_qc_diff_with_changes", body);
    }

    #[test]
    fn test_diff_comment_body_identical_files() {
        let comment = make_diff_comment("src/analysis.R", "src/analysis.R");
        let content = "x <- 1\ny <- 2\n";
        let git_info = MockGitInfo::new()
            .with_file("src/analysis.R", PREV_COMMIT, content)
            .with_file("src/analysis.R", CURR_COMMIT, content);

        let body = comment.generate_body(&git_info);
        assert!(body.contains("No difference between file versions."));
        assert!(body.contains("## File Difference"));
    }

    #[test]
    fn test_diff_comment_body_prev_file_missing() {
        let comment = make_diff_comment("src/missing.R", "src/new.R");
        // Only curr file registered — prev read will fail
        let git_info = MockGitInfo::new().with_file("src/new.R", CURR_COMMIT, "content\n");

        let body = comment.generate_body(&git_info);
        // Diff section should be empty when prev file can't be read
        let diff_section = body.split("## File Difference").nth(1).unwrap_or("");
        assert!(diff_section.trim().is_empty());
    }

    #[test]
    fn test_diff_comment_body_curr_file_missing() {
        let comment = make_diff_comment("src/old.R", "src/missing.R");
        // Only prev file registered — curr read will fail
        let git_info = MockGitInfo::new().with_file("src/old.R", PREV_COMMIT, "content\n");

        let body = comment.generate_body(&git_info);
        let diff_section = body.split("## File Difference").nth(1).unwrap_or("");
        assert!(diff_section.trim().is_empty());
    }

    #[test]
    fn test_diff_comment_metadata_structure() {
        let comment = make_diff_comment("src/old.R", "src/new.R");
        let git_info = MockGitInfo::new()
            .with_file("src/old.R", PREV_COMMIT, "a\n")
            .with_file("src/new.R", CURR_COMMIT, "b\n");

        let body = comment.generate_body(&git_info);
        assert!(body.starts_with("# Previous QC"));
        assert!(body.contains("## Metadata"));
        assert!(body.contains("## File Difference"));
        assert!(body.contains(&format!("latest qc commit: {PREV_COMMIT}")));
        assert!(body.contains(&format!("new qc initial qc commit: {CURR_COMMIT}")));
        assert!(body.contains("https://github.com/owner/repo/issues/42"));
        // File content URL uses short commit (first 7 chars)
        assert!(body.contains(&PREV_COMMIT[..7]));
    }

    #[test]
    fn test_build_diff_wraps_in_details() {
        let comment = make_diff_comment("src/a.R", "src/b.R");
        let git_info = MockGitInfo::new()
            .with_file("src/a.R", PREV_COMMIT, "old\n")
            .with_file("src/b.R", CURR_COMMIT, "new\n");

        let diff = comment.build_diff(&git_info);
        assert!(diff.starts_with("<details>"));
        assert!(diff.contains("<summary>View diff</summary>"));
        assert!(diff.ends_with("</details>"));
    }

    #[test]
    fn test_build_diff_empty_on_missing_file() {
        let comment = make_diff_comment("src/gone.R", "src/b.R");
        let git_info = MockGitInfo::new(); // no files registered

        let diff = comment.build_diff(&git_info);
        assert!(diff.is_empty());
    }
}
