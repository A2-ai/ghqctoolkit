use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) enum RelevantFileClass {
    PreviousQC {
        issue_number: u64,
        description: Option<String>,
    },
    GatingQC {
        issue_number: u64,
        description: Option<String>,
    },
    RelevantQC {
        issue_number: u64,
        description: Option<String>,
    },
    File {
        justification: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct RelevantFile {
    pub(crate) file_name: PathBuf,
    pub(crate) class: RelevantFileClass,
}
