//! The recorder task, the shell around [`RecorderCore`]: it drains the queue
//! in order, holds each piece's byte budget until it commits, commits the
//! core's batches through the sink, prunes, derives missing digests, and
//! keeps the metrics.

use super::{
    item::Item,
    metrics::Counters,
    options::CaptureOptions,
    recording::RecorderCore,
    sink::{Observation, RecordSink},
};
use crate::{
    capture::Omission,
    digest::CallDigest,
    retention::RetentionPolicy,
    time::{Clock, Monotonic},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, broadcast, mpsc},
    time::MissedTickBehavior,
};

/// Run it in the shell; it returns once every tap and handle is dropped and
/// the last batch is committed.
#[must_use = "the recorder records nothing until its task runs"]
pub struct RecorderTask<S> {
    pub(super) receiver: mpsc::Receiver<Item>,
    pub(super) sink: S,
    pub(super) live: broadcast::Sender<Observation>,
    pub(super) counters: Arc<Counters>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) options: CaptureOptions,
    pub(super) retention: RetentionPolicy,
    pub(super) core: RecorderCore,
    pub(super) held: Vec<OwnedSemaphorePermit>,
    pub(super) oldest: Option<Monotonic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    Run,
    Skip,
}

impl<S: RecordSink> RecorderTask<S> {
    /// A batch commits at this size even before the interval elapses.
    const MAX_BATCH_OPS: usize = 512;
    const MAX_BATCH_BYTES: u64 = 4 * 1024 * 1024;
    const MAINTENANCE: Duration = Duration::from_secs(60);
    /// Checkpoint the WALs every this many maintenance passes.
    const CHECKPOINT_EVERY: u32 = 5;
    /// Calls whose digest is derived again from storage per pass.
    const REDERIVE_BATCH: usize = 25;
    /// The pause between passes while calls wait, and once none do.
    const REDERIVE_BUSY: Duration = Duration::from_millis(250);
    const REDERIVE_IDLE: Duration = Duration::from_secs(30);
    const PRUNE_ROUNDS: u32 = 8;

    pub async fn run(mut self) -> Result<(), S::Error> {
        let mut commit = tokio::time::interval(self.options.commit_interval());
        commit.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut maintenance = tokio::time::interval(Self::MAINTENANCE);
        maintenance.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut passes = 0_u32;
        let mut rederive = Box::pin(tokio::time::sleep(Duration::ZERO));
        loop {
            tokio::select! {
                item = self.receiver.recv() => match item {
                    Some(item) => {
                        self.accept(item);
                        let pending = self.core.pending();
                        if pending.ops() >= Self::MAX_BATCH_OPS || pending.data_bytes() >= Self::MAX_BATCH_BYTES {
                            self.commit().await;
                        }
                    }
                    None => break,
                },
                _ = commit.tick() => self.commit().await,
                () = &mut rederive => {
                    let pause = self.rederive().await;
                    rederive.set(tokio::time::sleep(pause));
                }
                _ = maintenance.tick() => {
                    self.commit().await;
                    passes = passes.wrapping_add(1);
                    self.maintain(if passes.is_multiple_of(Self::CHECKPOINT_EVERY) {
                        Checkpoint::Run
                    } else {
                        Checkpoint::Skip
                    })
                    .await;
                }
            }
        }
        self.commit().await;
        self.sink.close().await
    }

    /// Hand a record to the core; keep the bytes' budget until they commit.
    fn accept(&mut self, item: Item) {
        let (record, budget) = item.into_record();
        self.observe_age(record.at());
        if let Some(budget) = budget {
            self.held.push(budget);
        }
        self.core.accept(record);
    }

    /// Derive digests again from stored bodies: for calls recorded before
    /// digests existed or by an older version. Returns the pause before the
    /// next pass: short while calls wait, long once none do.
    async fn rederive(&mut self) -> Duration {
        match self
            .sink
            .undigested(CallDigest::VERSION, Self::REDERIVE_BATCH)
            .await
        {
            Ok(exchanges) => {
                let full = exchanges.len() >= Self::REDERIVE_BATCH;
                self.core.rederived(exchanges);
                self.commit().await;
                if full {
                    Self::REDERIVE_BUSY
                } else {
                    Self::REDERIVE_IDLE
                }
            }
            Err(error) => {
                tracing::warn!(%error, "deriving digests from stored bodies failed; retrying");
                Self::REDERIVE_IDLE
            }
        }
    }

    fn observe_age(&mut self, at: Monotonic) {
        self.oldest = Some(self.oldest.map_or(at, |oldest| oldest.min(at)));
    }

    /// Commit the current batch. On failure, retry its lifecycle records alone
    /// so terminals survive; the lost bytes are counted and the bodies' stored
    /// byte counts show the loss.
    async fn commit(&mut self) {
        let batch = self.core.take_batch();
        if batch.is_empty() {
            return;
        }
        let held = std::mem::take(&mut self.held);
        let started = self.clock.now().monotonic;
        let committed = match self.sink.commit(&batch).await {
            Ok(committed) => Ok(committed),
            Err(error) => {
                self.counters.storage_errors.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .omit(Omission::StorageFailure, batch.data_bytes());
                tracing::error!(%error, "capture commit failed; retrying lifecycle records without body data");
                self.sink.commit(&batch.without_data()).await
            }
        };
        drop(held);
        let now = self.clock.now().monotonic;
        let millis = |duration: Duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        if let Some(oldest) = self.oldest.take() {
            self.counters
                .lag_ms
                .store(millis(now.saturating_since(oldest)), Ordering::Relaxed);
        }
        match committed {
            Ok(committed) => {
                let elapsed = millis(now.saturating_since(started));
                self.counters.commits.fetch_add(1, Ordering::Relaxed);
                self.counters
                    .last_commit_ms
                    .store(elapsed, Ordering::Relaxed);
                self.counters
                    .max_commit_ms
                    .fetch_max(elapsed, Ordering::Relaxed);
                self.counters
                    .referenced_bytes
                    .fetch_add(committed.referenced_bytes, Ordering::Relaxed);
                self.counters
                    .new_bytes
                    .fetch_add(committed.new_bytes, Ordering::Relaxed);
                self.counters
                    .compressed_bytes
                    .fetch_add(committed.compressed_bytes, Ordering::Relaxed);
                for observation in committed.observations {
                    // No subscribers is not an error.
                    let _ = self.live.send(observation);
                }
            }
            Err(error) => {
                self.counters.storage_errors.fetch_add(1, Ordering::Relaxed);
                tracing::error!(%error, "capture lifecycle commit failed; open calls will be recovered on restart");
            }
        }
    }

    async fn maintain(&mut self, checkpoint: Checkpoint) {
        for _ in 0..Self::PRUNE_ROUNDS {
            match self
                .sink
                .prune(&self.retention, self.clock.now().wall)
                .await
            {
                Ok(pruned) => {
                    self.counters
                        .pruned_calls
                        .fetch_add(pruned.calls, Ordering::Relaxed);
                    self.counters
                        .pruned_chunks
                        .fetch_add(pruned.chunks, Ordering::Relaxed);
                    if !pruned.more {
                        break;
                    }
                }
                Err(error) => {
                    self.counters
                        .retention_errors
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::error!(%error, "retention failed; forwarding is unaffected");
                    break;
                }
            }
        }
        match checkpoint {
            Checkpoint::Skip => {}
            Checkpoint::Run => {
                if let Err(error) = self.sink.checkpoint().await {
                    tracing::warn!(%error, "WAL checkpoint failed");
                }
            }
        }
    }
}
