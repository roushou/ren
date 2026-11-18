//! `/api/storage`'s answer, and the storage usage `/api/metrics` also
//! reports: the gateway's own wire types, each converted once from the store's
//! or the recorder's type, so a change there cannot change the API unnoticed.
//! ts-rs writes them to the UI (`ui/apps/web/src/lib/wire`).

use ren_recorder::retention;
use ren_store::view;
use serde::Serialize;
use ts_rs::TS;

/// `/api/storage`'s answer.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct Storage {
    pub(super) policy: RetentionPolicy,
    pub(super) usage: StorageView,
    pub(super) capture: CaptureLimits,
}

/// What retention keeps, as configured.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct RetentionPolicy {
    enabled: bool,
    max_age_seconds: u64,
    max_calls: u64,
    max_storage_bytes: u64,
}

impl From<&retention::RetentionPolicy> for RetentionPolicy {
    fn from(policy: &retention::RetentionPolicy) -> Self {
        let retention::RetentionPolicy {
            enabled,
            max_age_seconds,
            max_calls,
            max_storage_bytes,
        } = *policy;
        Self {
            enabled,
            max_age_seconds,
            max_calls,
            max_storage_bytes,
        }
    }
}

/// What capture keeps of each call, so a reader can tell a body trimmed at
/// the limit (by design) from one that lost bytes.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct CaptureLimits {
    /// Bytes stored per body; past this, the rest is left out by design.
    pub(super) body_limit_bytes: usize,
}

/// What is retained, and how well chunk deduplication works on it.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct StorageView {
    disk: DiskUsage,
    calls: u64,
    open_calls: u64,
    /// Calls whose activity is derived: every finished call, once derived.
    digested_calls: u64,
    chunks: u64,
    /// Raw bytes all stored segments reference.
    referenced_bytes: u64,
    /// Raw bytes of the distinct chunks.
    unique_bytes: u64,
    /// Bytes the chunks occupy after compression.
    compressed_bytes: u64,
}

impl From<view::StorageView> for StorageView {
    fn from(view: view::StorageView) -> Self {
        let view::StorageView {
            disk,
            calls,
            open_calls,
            digested_calls,
            chunks,
            referenced_bytes,
            unique_bytes,
            compressed_bytes,
        } = view;
        Self {
            disk: disk.into(),
            calls,
            open_calls,
            digested_calls,
            chunks,
            referenced_bytes,
            unique_bytes,
            compressed_bytes,
        }
    }
}

/// Disk usage of the store's files. The search index is part of the
/// metadata file.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct DiskUsage {
    metadata: FileUsage,
    blobs: FileUsage,
}

impl From<view::DiskUsage> for DiskUsage {
    fn from(view::DiskUsage { metadata, blobs }: view::DiskUsage) -> Self {
        Self {
            metadata: metadata.into(),
            blobs: blobs.into(),
        }
    }
}

/// Real disk usage of one database file.
#[derive(Serialize, TS)]
#[ts(export)]
pub(super) struct FileUsage {
    page_size: u64,
    page_count: u64,
    freelist_count: u64,
}

impl From<view::FileUsage> for FileUsage {
    fn from(
        view::FileUsage {
            page_size,
            page_count,
            freelist_count,
        }: view::FileUsage,
    ) -> Self {
        Self {
            page_size,
            page_count,
            freelist_count,
        }
    }
}
