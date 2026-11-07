//! Recorder configuration: queue bounds, body capture limit, commit cadence.

use serde::{Deserialize, Serialize};
use std::{ops::Range, time::Duration};

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

/// How a run of captured bytes is recorded: the pieces within the body
/// limit, and how many bytes past it are omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSplit {
    /// Byte ranges of the run, in order, each at most
    /// [`CaptureOptions::PIECE_BYTES`].
    pub pieces: Vec<Range<usize>>,
    /// Bytes past the body limit, omitted by policy.
    pub over_limit: u64,
}

impl CaptureOptions {
    /// The size of one queued data piece.
    pub const PIECE_BYTES: usize = 64 * 1024;

    /// How `len` bytes observed at `offset` in a body are recorded: the
    /// pieces that fit under the body limit, and how much the limit omits.
    pub fn split(&self, offset: u64, len: usize) -> CaptureSplit {
        let limit = u64::try_from(self.body_limit_bytes).unwrap_or(u64::MAX);
        let eligible = usize::try_from(limit.saturating_sub(offset))
            .unwrap_or(usize::MAX)
            .min(len);
        let mut pieces = Vec::with_capacity(eligible.div_ceil(Self::PIECE_BYTES));
        let mut from = 0;
        while from < eligible {
            let to = from.saturating_add(Self::PIECE_BYTES).min(eligible);
            pieces.push(from..to);
            from = to;
        }
        CaptureSplit {
            pieces,
            over_limit: u64::try_from(len.saturating_sub(eligible)).unwrap_or(u64::MAX),
        }
    }

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

#[cfg(test)]
mod tests {
    use super::CaptureOptions;
    use proptest::prelude::*;

    #[test]
    fn runs_split_into_pieces_under_the_body_limit() {
        let options = CaptureOptions {
            body_limit_bytes: 100 * 1024,
            ..CaptureOptions::default()
        };
        let piece = CaptureOptions::PIECE_BYTES;
        // A run straddling the limit: its tail is omitted by policy.
        let split = options.split(90 * 1024, 30 * 1024);
        assert_eq!(split.pieces, vec![0..10 * 1024; 1]);
        assert_eq!(split.over_limit, 20 * 1024);
        // A long run within the limit: whole pieces, then the rest.
        let split = options.split(0, piece + 5);
        assert_eq!(split.pieces, [0..piece, piece..piece + 5]);
        assert_eq!(split.over_limit, 0);
        // Past the limit, nothing is kept.
        let split = options.split(200 * 1024, 7);
        assert!(split.pieces.is_empty());
        assert_eq!(split.over_limit, 7);
    }

    proptest! {
        /// Every byte of a run is either in exactly one piece, in order, or
        /// counted as over the limit; no piece is empty or too large.
        #[test]
        fn a_split_accounts_for_every_byte(
            limit in 0..300_000_usize,
            offset in 0..400_000_u64,
            len in 0..300_000_usize,
        ) {
            let options = CaptureOptions { body_limit_bytes: limit, ..CaptureOptions::default() };
            let split = options.split(offset, len);
            let mut next = 0;
            for piece in &split.pieces {
                prop_assert_eq!(piece.start, next);
                prop_assert!(!piece.is_empty() && piece.len() <= CaptureOptions::PIECE_BYTES);
                next = piece.end;
            }
            prop_assert_eq!(u64::try_from(next).unwrap() + split.over_limit, u64::try_from(len).unwrap());
            prop_assert!(offset + u64::try_from(next).unwrap() <= u64::try_from(limit).unwrap().max(offset));
        }
    }
}
