//! Recorder counters. They are atomics read without the queue, so overload is
//! reported even while the queue itself is saturated.

use crate::capture::Omission;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub(super) struct Counters {
    omitted_policy: AtomicU64,
    omitted_queue_bytes: AtomicU64,
    omitted_queue_slots: AtomicU64,
    omitted_storage: AtomicU64,
    omitted_unattributed: AtomicU64,
    unrecorded_calls: AtomicU64,
    pub storage_errors: AtomicU64,
    pub retention_errors: AtomicU64,
    pub pruned_calls: AtomicU64,
    pub pruned_chunks: AtomicU64,
    pub commits: AtomicU64,
    pub last_commit_ms: AtomicU64,
    pub max_commit_ms: AtomicU64,
    pub lag_ms: AtomicU64,
    pub referenced_bytes: AtomicU64,
    pub new_bytes: AtomicU64,
    pub compressed_bytes: AtomicU64,
    pub recovered_calls: AtomicU64,
}

impl Counters {
    /// Count omitted bytes, or for [`Omission::Unrecorded`], one call.
    pub(super) fn omit(&self, reason: Omission, amount: u64) {
        if amount == 0 {
            return;
        }
        let counter = match reason {
            Omission::PolicyLimit => &self.omitted_policy,
            Omission::QueueBytes => &self.omitted_queue_bytes,
            Omission::QueueSlots => &self.omitted_queue_slots,
            Omission::StorageFailure => &self.omitted_storage,
            Omission::Unrecorded => &self.unrecorded_calls,
            Omission::Unattributed => &self.omitted_unattributed,
        };
        counter.fetch_add(amount, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self, queue: QueueMetrics) -> RecorderMetrics {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        RecorderMetrics {
            queue,
            omissions: OmissionMetrics {
                policy_limit_bytes: load(&self.omitted_policy),
                queue_bytes_bytes: load(&self.omitted_queue_bytes),
                queue_slots_bytes: load(&self.omitted_queue_slots),
                storage_failure_bytes: load(&self.omitted_storage),
                unattributed_bytes: load(&self.omitted_unattributed),
                unrecorded_calls: load(&self.unrecorded_calls),
            },
            storage_errors: load(&self.storage_errors),
            retention_errors: load(&self.retention_errors),
            pruned_calls: load(&self.pruned_calls),
            pruned_chunks: load(&self.pruned_chunks),
            commits: load(&self.commits),
            last_commit_ms: load(&self.last_commit_ms),
            max_commit_ms: load(&self.max_commit_ms),
            lag_ms: load(&self.lag_ms),
            dedup: DedupMetrics {
                referenced_bytes: load(&self.referenced_bytes),
                new_bytes: load(&self.new_bytes),
                compressed_bytes: load(&self.compressed_bytes),
            },
            recovered_calls: load(&self.recovered_calls),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QueueMetrics {
    pub items: usize,
    pub capacity: usize,
    pub bytes: usize,
    pub byte_capacity: usize,
}

/// Omitted capture, by [`Omission`] reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OmissionMetrics {
    pub policy_limit_bytes: u64,
    pub queue_bytes_bytes: u64,
    pub queue_slots_bytes: u64,
    pub storage_failure_bytes: u64,
    pub unattributed_bytes: u64,
    pub unrecorded_calls: u64,
}

/// Bytes written since start: referenced by segments, new chunks, and their
/// compressed size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DedupMetrics {
    pub referenced_bytes: u64,
    pub new_bytes: u64,
    pub compressed_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RecorderMetrics {
    pub queue: QueueMetrics,
    pub omissions: OmissionMetrics,
    pub storage_errors: u64,
    pub retention_errors: u64,
    pub pruned_calls: u64,
    pub pruned_chunks: u64,
    pub commits: u64,
    pub last_commit_ms: u64,
    pub max_commit_ms: u64,
    /// Time from the oldest record in the last batch to its commit.
    pub lag_ms: u64,
    pub dedup: DedupMetrics,
    pub recovered_calls: u64,
}

impl RecorderMetrics {
    /// Persistence is working: no storage or retention failures.
    pub fn healthy(&self) -> bool {
        self.storage_errors == 0 && self.retention_errors == 0
    }
}
