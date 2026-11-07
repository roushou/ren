//! Persistent capture: the observation log, its query tables, and
//! deduplicated body chunks.
//!
//! `tgv.sqlite3` holds metadata, the log, and segment rows. `blobs.sqlite3`
//! holds content-defined chunks keyed by blake3 hash and compressed with zstd.
//! One [`Store`] owns the directory and is the only writer; [`Reader`] serves
//! queries through a read-only pool. Chunks commit before the segments that
//! reference them, and retention removes unreferenced chunks by mark and
//! sweep, so no reference count spans the two files.
//!
//! This module is shell: it performs I/O. Its batch, query, and view types are
//! plain data, and chunking is sans-IO.

pub mod activity;
pub mod backup;
pub mod chunk;
pub mod query;
pub mod read;
mod reader;
pub mod search;
pub mod view;
mod writer;

pub use reader::{PrefixLimit, Reader};
pub use writer::{Recovery, Store, StoredOperatorState, SweepGate, SweepHold};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("another daemon owns this data directory")]
    Locked,
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("invalid stored JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("a storage worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[error("data directory path is not UTF-8")]
    NonUtf8Path,
    #[error("stored data is corrupt: {0}")]
    Corrupt(&'static str),
    #[error("call, session, or body not found")]
    NotFound,
    #[error("body capture is incomplete; inspect its metadata instead")]
    IncompleteBody,
}

#[cfg(test)]
mod tests;
