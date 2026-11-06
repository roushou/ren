//! The port the recorder writes through, and what it reports back.
//!
//! The recorder decides what to record and when to commit, prune, and
//! checkpoint; a sink makes it durable. The store is the sink the daemon uses.

use crate::{
    batch::{Batch, ObservationKind},
    id::{AttemptId, CallId, ChannelId},
    retention::RetentionPolicy,
    time::UnixMillis,
};
use serde::Serialize;
use std::future::Future;
use tgv_protocol::label::Stored;

/// Where recorded batches go. Each method runs on the recorder task, one at a
/// time, between the batches it commits.
pub trait RecordSink: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Commit one batch: all of it, or nothing.
    fn commit(
        &mut self,
        batch: &Batch,
    ) -> impl Future<Output = Result<Committed, Self::Error>> + Send;

    /// One retention pass under `policy` at `now`.
    fn prune(
        &mut self,
        policy: &RetentionPolicy,
        now: UnixMillis,
    ) -> impl Future<Output = Result<Pruned, Self::Error>> + Send;

    /// Move buffered writes into durable files.
    fn checkpoint(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Close after the last commit.
    fn close(self) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        Self: Sized;
}

/// What one commit wrote.
#[derive(Debug, Default)]
pub struct Committed {
    pub observations: Vec<Observation>,
    /// Raw bytes the batch's segments reference.
    pub referenced_bytes: u64,
    /// Raw bytes of chunks not already stored.
    pub new_bytes: u64,
    /// Bytes those new chunks occupy on disk.
    pub compressed_bytes: u64,
}

/// What one retention pass removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Pruned {
    pub calls: u64,
    pub chunks: u64,
    /// More eligible calls remain; run again soon.
    pub more: bool,
}

/// One committed row of the observation log, as published to live
/// subscribers and read back by the store, and as `/api/live` clients read
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Observation {
    pub seq: u64,
    pub at: UnixMillis,
    pub kind: Stored<ObservationKind>,
    pub channel: ChannelId,
    pub call: Option<CallId>,
    pub attempt: Option<AttemptId>,
}
