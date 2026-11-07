//! The recorder task: drains the queue in order, cuts bodies into chunks,
//! inspects responses incrementally, and commits in batches.

use super::{
    item::{Item, Piece, Track},
    metrics::Counters,
    options::CaptureOptions,
    sink::{Observation, RecordSink},
};
use crate::{
    batch::{AttemptBody, Batch, BodyKey, FinishAttempt, FinishCall, Op, Segment},
    capture::{BodyEnding, Omission},
    chunk::Chunker,
    id::CallId,
    inspection::InspectionRecord,
    retention::RetentionPolicy,
    time::{Clock, Monotonic},
};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tgv_protocol::{
    CallInspector,
    inspect::pipeline::{BodyEnd as InspectedEnd, InspectionLimits, Protocol},
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
    pub(super) calls: HashMap<CallId, CallInProgress>,
    pub(super) batch: Batch,
    pub(super) held: Vec<OwnedSemaphorePermit>,
    pub(super) oldest: Option<Monotonic>,
}

/// Recorder state for a call between its open and finish records.
pub(super) struct CallInProgress {
    protocol: Option<Protocol>,
    request: BodyCapture,
    /// The open attempt's changed request, if it sends one.
    attempt_request: Option<BodyCapture>,
    response: BodyCapture,
    inspector: Option<CallInspector>,
}

/// One body being cut into chunks.
struct BodyCapture {
    key: BodyKey,
    chunker: Chunker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    Run,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Continuity {
    Contiguous,
    /// Bytes before this piece were omitted.
    AfterGap,
}

impl BodyCapture {
    fn new(key: BodyKey) -> Self {
        Self {
            key,
            chunker: Chunker::at(0),
        }
    }

    fn push(&mut self, offset: u64, bytes: &[u8], batch: &mut Batch) -> Continuity {
        let continuity = if offset == self.chunker.next_offset() {
            Continuity::Contiguous
        } else {
            self.flush(batch);
            self.chunker = Chunker::at(offset);
            Continuity::AfterGap
        };
        let mut cuts = Vec::new();
        self.chunker.push(bytes, &mut cuts);
        self.emit(cuts, batch);
        continuity
    }

    fn flush(&mut self, batch: &mut Batch) {
        let mut cuts = Vec::new();
        self.chunker.flush(&mut cuts);
        self.emit(cuts, batch);
    }

    fn emit(&self, cuts: Vec<crate::chunk::Cut>, batch: &mut Batch) {
        if cuts.is_empty() {
            return;
        }
        let segments = cuts
            .into_iter()
            .map(|cut| Segment {
                offset: cut.offset,
                chunk: batch.add_chunk(cut.bytes),
            })
            .collect();
        batch.push(Op::Segments {
            body: self.key,
            segments,
        });
    }

    /// The end of the contiguous bytes received since the last gap.
    fn received(&self) -> u64 {
        self.chunker.next_offset()
    }
}

impl<S: RecordSink> RecorderTask<S> {
    /// A batch commits at this size even before the interval elapses.
    const MAX_BATCH_OPS: usize = 512;
    const MAX_BATCH_BYTES: u64 = 4 * 1024 * 1024;
    const MAINTENANCE: Duration = Duration::from_secs(60);
    /// Checkpoint the WALs every this many maintenance passes.
    const CHECKPOINT_EVERY: u32 = 5;
    const PRUNE_ROUNDS: u32 = 8;

    pub async fn run(mut self) -> Result<(), S::Error> {
        let mut commit = tokio::time::interval(self.options.commit_interval());
        commit.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut maintenance = tokio::time::interval(Self::MAINTENANCE);
        maintenance.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut passes = 0_u32;
        loop {
            tokio::select! {
                item = self.receiver.recv() => match item {
                    Some(item) => {
                        self.accept(item);
                        if self.batch.ops() >= Self::MAX_BATCH_OPS || self.batch.data_bytes() >= Self::MAX_BATCH_BYTES {
                            self.commit().await;
                        }
                    }
                    None => break,
                },
                _ = commit.tick() => self.commit().await,
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

    fn accept(&mut self, item: Item) {
        match item {
            Item::Open {
                open,
                attempt,
                endpoint,
            } => {
                self.observe_age(open.stamp.at.monotonic);
                let call = open.stamp.call;
                self.calls.insert(
                    call,
                    CallInProgress {
                        protocol: Protocol::for_endpoint(endpoint),
                        request: BodyCapture::new(BodyKey::Request(call)),
                        attempt_request: None,
                        // Replaced when the first attempt starts.
                        response: BodyCapture::new(BodyKey::Request(call)),
                        inspector: None,
                    },
                );
                self.batch.push(Op::OpenCall(open));
                if let Some(attempt) = attempt {
                    self.attempt(attempt);
                }
            }
            Item::Attempt(attempt) => {
                self.observe_age(attempt.stamp.at.monotonic);
                self.attempt(attempt);
            }
            Item::Head(head) => {
                self.observe_age(head.stamp.at.monotonic);
                if let Some(call) = self.calls.get_mut(&head.stamp.call) {
                    call.inspector = call.protocol.map(|protocol| {
                        CallInspector::new(
                            protocol,
                            head.format,
                            head.encoding,
                            InspectionLimits::default(),
                        )
                    });
                }
                self.batch.push(Op::ResponseHead(head));
            }
            Item::Data {
                piece,
                slot,
                budget,
            } => {
                drop(slot);
                self.held.push(budget);
                self.data(piece);
            }
            Item::Close { close, _channel } => {
                self.observe_age(close.at.monotonic);
                self.batch.push(Op::CloseChannel(close));
            }
            Item::Operator { action, _slot } => {
                self.observe_age(action.at.monotonic);
                self.batch.push(Op::Operator(action));
            }
            Item::FinishAttempt {
                stamp,
                attempt,
                end,
                response,
                request,
            } => {
                self.observe_age(stamp.at.monotonic);
                let inspection = self.calls.get_mut(&stamp.call).and_then(|call| {
                    if let Some(mut changed) = call.attempt_request.take() {
                        changed.flush(&mut self.batch);
                    }
                    call.response.flush(&mut self.batch);
                    let mut inspector = call.inspector.take()?;
                    // Observed bytes that never reached the recorder are a gap.
                    if call.response.received() < response.observed_bytes {
                        inspector.gap();
                    }
                    let end = match response.ending {
                        BodyEnding::Ended => InspectedEnd::Complete,
                        BodyEnding::Open | BodyEnding::Unfinished => InspectedEnd::Unfinished,
                    };
                    Some(Box::new(InspectionRecord::from(
                        &inspector.finish(end).inspection,
                    )))
                });
                self.batch.push(Op::FinishAttempt(FinishAttempt {
                    stamp,
                    attempt,
                    end,
                    response,
                    request,
                    inspection,
                }));
            }
            Item::FinishCall {
                stamp,
                request,
                rejection,
                _call,
            } => {
                self.observe_age(stamp.at.monotonic);
                if let Some(mut call) = self.calls.remove(&stamp.call) {
                    call.request.flush(&mut self.batch);
                }
                self.batch.push(Op::FinishCall(FinishCall {
                    stamp,
                    request,
                    rejection,
                }));
            }
        }
    }

    /// An attempt started: its bodies are captured from here on.
    fn attempt(&mut self, attempt: crate::batch::StartAttempt) {
        if let Some(call) = self.calls.get_mut(&attempt.stamp.call) {
            call.response = BodyCapture::new(BodyKey::Response(attempt.attempt));
            call.attempt_request = match attempt.body {
                AttemptBody::Original => None,
                AttemptBody::Changed => {
                    Some(BodyCapture::new(BodyKey::AttemptRequest(attempt.attempt)))
                }
            };
            call.inspector = None;
        }
        self.batch.push(Op::StartAttempt(attempt));
    }

    fn data(&mut self, piece: Piece) {
        self.observe_age(piece.enqueued);
        let Some(call) = self.calls.get_mut(&piece.call) else {
            return;
        };
        let body = match piece.track {
            Track::Request => &mut call.request,
            Track::Response => &mut call.response,
            Track::AttemptRequest => match &mut call.attempt_request {
                Some(body) => body,
                None => return,
            },
        };
        if let Some(mark) = piece.message {
            self.batch.push(Op::Message {
                body: body.key,
                mark,
            });
        }
        let continuity = body.push(piece.offset, &piece.bytes, &mut self.batch);
        match piece.track {
            Track::Request | Track::AttemptRequest => {}
            Track::Response => {
                if let Some(inspector) = &mut call.inspector {
                    match continuity {
                        Continuity::Contiguous => {}
                        Continuity::AfterGap => inspector.gap(),
                    }
                    if let Some(mark) = piece.message {
                        inspector.message(mark.len);
                    }
                    inspector.feed(&piece.bytes);
                }
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
        // Streaming responses are cut at each commit, so live previews advance.
        // Requests keep content-defined cuts, where deduplication matters.
        for call in self.calls.values_mut() {
            call.response.flush(&mut self.batch);
        }
        if self.batch.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.batch);
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
