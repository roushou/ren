//! DRAFT

use ren_recorder::batch::HistoryChange;
use ren_recorder::id::RecordingId;
use ren_recorder::id::RunId;
use ren_recorder::id::SessionId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// What a history edit applies to: a recording, a run with the recording its
/// client recorded, when there is one, or an operator session with all its
/// runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
#[ts(export)]
pub enum HistorySubject {
    Recording(#[ts(as = "String")] RecordingId),
    Run(#[ts(as = "String")] RunId),
    OperatorSession(#[ts(as = "String")] SessionId),
}

impl From<ren_recorder::batch::HistorySubject> for HistorySubject {
    fn from(value: ren_recorder::batch::HistorySubject) -> Self {
        match value {
            ren_recorder::batch::HistorySubject::Recording(v0) => Self::Recording(v0),
            ren_recorder::batch::HistorySubject::Run(v0) => Self::Run(v0),
            ren_recorder::batch::HistorySubject::OperatorSession(v0) => Self::OperatorSession(v0),
        }
    }
}

/// What one backup wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BackupReport {
    /// The backup, a data directory a reader can open.
    #[ts(as = "String")]
    pub directory: PathBuf,
    pub metadata_bytes: u64,
    /// Chunks the backup holds now.
    pub chunks: u64,
    pub chunks_copied: u64,
    /// Stored (compressed) bytes copied.
    pub bytes_copied: u64,
    pub chunks_removed: u64,
}

impl From<ren_store::backup::BackupReport> for BackupReport {
    fn from(value: ren_store::backup::BackupReport) -> Self {
        let ren_store::backup::BackupReport {
            directory,
            metadata_bytes,
            chunks,
            chunks_copied,
            bytes_copied,
            chunks_removed,
        } = value;
        Self {
            directory,
            metadata_bytes,
            chunks,
            chunks_copied,
            bytes_copied,
            chunks_removed,
        }
    }
}

/// What a backup answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BackupBody {
    pub backup: BackupReport,
}

/// What a history edit answers: what it applied to, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HistoryEdit {
    pub subject: HistorySubject,
    /// `archive`, `unarchive`, or `delete`.
    #[ts(as = "String")]
    pub change: HistoryChange,
}
