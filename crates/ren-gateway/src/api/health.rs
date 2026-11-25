//! `/health`'s answer, and the recorder metrics `/api/metrics` also reports:
//! the gateway's own wire types, each converted once from the type it reports,
//! so a change there cannot change the API unnoticed. ts-rs writes them to the
//! UI (`ui/apps/web/src/lib/wire`).
//!
//! Labels and other values written as text (`HealthStatus`, `Environment`,
//! `Revision`) are `string` here: the UI reads them as `Stored<T>` or parses
//! them, and each crate's `tests/fixtures/labels.txt` pins the label sets.

use super::HealthStatus;
use crate::{build, operator};
use ren_git::SourceRevision;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `/health`'s answer.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct Health {
    #[ts(type = "string")]
    pub(super) status: HealthStatus,
    pub(super) build: Build,
    /// Production or development, as configured.
    pub(super) instance: InstanceOptions,
    /// Whether new calls are admitted, read-only: changing it takes the
    /// control API.
    pub(super) operator: OperatorSummary,
    pub(super) recorder: RecorderMetrics,
}

/// A package version and the source revision it was compiled from.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct Build {
    version: String,
    /// `<commit>`, `<commit>-dirty`, or `unknown`.
    #[ts(type = "string")]
    revision: SourceRevision,
}

impl From<&crate::release::Build> for Build {
    fn from(build: &crate::release::Build) -> Self {
        Self {
            version: build.version().to_owned(),
            revision: build.revision().clone(),
        }
    }
}

/// What kind of instance the gateway is.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct InstanceOptions {
    #[ts(type = "string")]
    environment: build::Environment,
}

impl From<build::InstanceOptions> for InstanceOptions {
    fn from(build::InstanceOptions { environment }: build::InstanceOptions) -> Self {
        Self { environment }
    }
}

/// The operator state: the pause, and how many session and user blocks are
/// in force now.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct OperatorSummary {
    pub(super) pause: Pause,
    pub(super) blocked_sessions: usize,
    pub(super) blocked_users: usize,
}

/// Whether new calls are admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "state")]
#[ts(export)]
pub enum Pause {
    Running,
    /// Every new call gets `503` with this `retry-after`, in seconds.
    Paused {
        retry_after: u32,
    },
}

impl From<operator::Pause> for Pause {
    fn from(pause: operator::Pause) -> Self {
        match pause {
            operator::Pause::Running => Self::Running,
            operator::Pause::Paused { retry_after } => Self::Paused {
                retry_after: retry_after.get(),
            },
        }
    }
}

/// The recorder's counters since the daemon started.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct RecorderMetrics {
    queue: QueueMetrics,
    omissions: OmissionMetrics,
    storage_errors: u64,
    retention_errors: u64,
    pruned_calls: u64,
    pruned_chunks: u64,
    commits: u64,
    last_commit_ms: u64,
    max_commit_ms: u64,
    /// Time from the oldest record in the last batch to its commit.
    lag_ms: u64,
    dedup: DedupMetrics,
    recovered_calls: u64,
}

impl From<ren_recorder::RecorderMetrics> for RecorderMetrics {
    fn from(metrics: ren_recorder::RecorderMetrics) -> Self {
        let ren_recorder::RecorderMetrics {
            queue,
            omissions,
            storage_errors,
            retention_errors,
            pruned_calls,
            pruned_chunks,
            commits,
            last_commit_ms,
            max_commit_ms,
            lag_ms,
            dedup,
            recovered_calls,
        } = metrics;
        Self {
            queue: queue.into(),
            omissions: omissions.into(),
            storage_errors,
            retention_errors,
            pruned_calls,
            pruned_chunks,
            commits,
            last_commit_ms,
            max_commit_ms,
            lag_ms,
            dedup: dedup.into(),
            recovered_calls,
        }
    }
}

/// The capture queue's fill, by items and by bytes.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct QueueMetrics {
    items: usize,
    capacity: usize,
    bytes: usize,
    byte_capacity: usize,
}

impl From<ren_recorder::QueueMetrics> for QueueMetrics {
    fn from(
        ren_recorder::QueueMetrics {
            items,
            capacity,
            bytes,
            byte_capacity,
        }: ren_recorder::QueueMetrics,
    ) -> Self {
        Self {
            items,
            capacity,
            bytes,
            byte_capacity,
        }
    }
}

/// Omitted capture, by reason.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct OmissionMetrics {
    policy_limit_bytes: u64,
    queue_bytes_bytes: u64,
    queue_slots_bytes: u64,
    storage_failure_bytes: u64,
    unattributed_bytes: u64,
    unrecorded_calls: u64,
}

impl From<ren_recorder::OmissionMetrics> for OmissionMetrics {
    fn from(
        ren_recorder::OmissionMetrics {
            policy_limit_bytes,
            queue_bytes_bytes,
            queue_slots_bytes,
            storage_failure_bytes,
            unattributed_bytes,
            unrecorded_calls,
        }: ren_recorder::OmissionMetrics,
    ) -> Self {
        Self {
            policy_limit_bytes,
            queue_bytes_bytes,
            queue_slots_bytes,
            storage_failure_bytes,
            unattributed_bytes,
            unrecorded_calls,
        }
    }
}

/// Bytes written since start: referenced by segments, new chunks, and their
/// compressed size.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct DedupMetrics {
    referenced_bytes: u64,
    new_bytes: u64,
    compressed_bytes: u64,
}

impl From<ren_recorder::DedupMetrics> for DedupMetrics {
    fn from(
        ren_recorder::DedupMetrics {
            referenced_bytes,
            new_bytes,
            compressed_bytes,
        }: ren_recorder::DedupMetrics,
    ) -> Self {
        Self {
            referenced_bytes,
            new_bytes,
            compressed_bytes,
        }
    }
}
