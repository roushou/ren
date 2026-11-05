//! Recorder configuration: queue bounds, body capture limit, commit cadence.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureOptions {
    /// Queue slots for body data. Lifecycle records have their own reserved
    /// slots, sized from the gateway's concurrency limit.
    pub queue_capacity: usize,
    /// Body bytes held between capture and commit. Must exceed several
    /// concurrent large requests.
    pub queue_bytes: usize,
    /// Bytes captured per body; the rest is counted as omitted.
    pub body_limit_bytes: usize,
    /// Longest time a record waits before its batch commits.
    pub commit_interval_ms: u64,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            queue_bytes: 64 * 1024 * 1024,
            body_limit_bytes: 16 * 1024 * 1024,
            commit_interval_ms: 50,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaptureOptionsError {
    #[error("capture.queue_capacity must be between 4 and 65536")]
    QueueCapacity,
    #[error("capture.queue_bytes must be between 64 KiB and 1 GiB")]
    QueueBytes,
    #[error("capture.body_limit_bytes must be at most 256 MiB")]
    BodyLimit,
    #[error("capture.commit_interval_ms must be between 1 and 1000")]
    CommitInterval,
}

impl CaptureOptions {
    /// The size of one queued data piece.
    pub const PIECE_BYTES: usize = 64 * 1024;

    pub fn validate(&self) -> Result<(), CaptureOptionsError> {
        if !(4..=65536).contains(&self.queue_capacity) {
            return Err(CaptureOptionsError::QueueCapacity);
        }
        if !(Self::PIECE_BYTES..=1024 * 1024 * 1024).contains(&self.queue_bytes) {
            return Err(CaptureOptionsError::QueueBytes);
        }
        if self.body_limit_bytes > 256 * 1024 * 1024 {
            return Err(CaptureOptionsError::BodyLimit);
        }
        if !(1..=1000).contains(&self.commit_interval_ms) {
            return Err(CaptureOptionsError::CommitInterval);
        }
        Ok(())
    }

    pub(super) fn commit_interval(&self) -> Duration {
        Duration::from_millis(self.commit_interval_ms)
    }
}
