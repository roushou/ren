//! Recording: the tap the relay feeds, one ordered queue with reserved
//! lifecycle slots, and the task that turns records into batches for a
//! [`RecordSink`]. The primary type is [`Recorder`], the handle forwarding
//! uses; [`RecorderTask`] runs it.
//!
//! The crate also owns what is recorded: TGV's identities (`id`) and the
//! `Clock` and `IdSource` capabilities (`time`, `id`), attempt and call
//! terminals (`capture`), routing facts (`routing`), batches and their chunks
//! (`batch`, `chunk`), the stored inspection format (`inspection`), and the
//! retention policy the sink applies (`retention`).
//!
//! Recording never stalls forwarding. The tap only uses non-blocking queue
//! admission; data that does not fit is omitted with a reason, and lifecycle
//! records cannot be starved because their slots are reserved when a call is
//! admitted. Counters are atomics, readable while the queue is saturated.
//!
//! The task is returned to the caller to run, because components do not spawn.

pub mod batch;
pub mod capture;
pub mod chunk;
pub mod digest;
pub mod id;
pub mod inspection;
mod item;
pub mod metrics;
pub mod options;
pub mod retention;
pub mod routing;
pub mod secrets;
mod sink;
mod tap;
mod task;
pub mod time;

pub use metrics::RecorderMetrics;
pub use options::{CaptureOptions, CaptureOptionsError};
pub use sink::{Committed, Observation, Pruned, RecordSink};
pub use tap::{AttemptStart, CallStart, CallTap, ChannelTap, OperatorSlot};
pub use task::RecorderTask;

use crate::{
    id::{ChannelId, IdSource},
    retention::RetentionPolicy,
    secrets::SecretScanner,
    time::Clock,
};
use item::Shared;
use metrics::{Counters, QueueMetrics};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
};
use tokio::sync::broadcast;

/// The recorder's counters, readable after every [`Recorder`] handle is
/// dropped. Queue figures are the caller's last reading, since the queue
/// itself closes with the handles.
#[derive(Clone)]
pub struct RecorderMeter {
    counters: Arc<Counters>,
}

impl RecorderMeter {
    pub fn snapshot(&self, queue: QueueMetrics) -> RecorderMetrics {
        self.counters.snapshot(queue)
    }
}

/// What the recorder is sized for.
#[derive(Debug, Clone)]
pub struct RecorderSetup {
    pub options: CaptureOptions,
    pub retention: RetentionPolicy,
    /// Calls open at once: the gateway's admission limit.
    pub max_calls: usize,
    /// Committed observations kept for live subscribers that fall behind.
    pub live_buffer: usize,
    /// Redacts the text of every call's digest before it is stored.
    pub scanner: Arc<SecretScanner>,
}

/// A handle to the recorder: starts call taps, reads metrics, and subscribes
/// to committed observations.
#[derive(Clone)]
pub struct Recorder {
    shared: Arc<Shared>,
    live: broadcast::Sender<Observation>,
}

impl Recorder {
    /// A recorder writing to `sink`. `recovered_calls` is how many calls the
    /// sink found left open by the previous run, reported in the metrics.
    pub fn new<S: RecordSink>(
        sink: S,
        recovered_calls: u64,
        setup: RecorderSetup,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdSource>,
    ) -> Result<(Self, RecorderTask<S>), CaptureOptionsError> {
        setup.options.validate()?;
        let counters = Arc::new(Counters::default());
        counters
            .recovered_calls
            .store(recovered_calls, Ordering::Relaxed);
        let (shared, receiver) = Shared::new(
            setup.options.clone(),
            setup.max_calls,
            counters.clone(),
            clock.clone(),
            ids,
        );
        let (live, _) = broadcast::channel(setup.live_buffer.max(16));
        let task = RecorderTask {
            receiver,
            sink,
            live: live.clone(),
            counters,
            clock,
            options: setup.options,
            retention: setup.retention,
            calls: HashMap::new(),
            batch: Default::default(),
            held: Vec::new(),
            oldest: None,
            scanner: setup.scanner,
        };
        Ok((
            Self {
                shared: Arc::new(shared),
                live,
            },
            task,
        ))
    }

    /// Admit a call and its first attempt for capture. Never blocks; without
    /// a free reservation the call is forwarded unrecorded and counted.
    pub fn start(&self, start: CallStart, attempt: AttemptStart) -> Arc<CallTap> {
        let channel = ChannelId::generate(self.shared.ids.as_ref());
        Arc::new(CallTap::start(
            self.shared.clone(),
            channel,
            start,
            Some(attempt),
        ))
    }

    /// Admit a call on a channel that already exists: a WebSocket message call.
    pub fn start_in(
        &self,
        channel: ChannelId,
        start: CallStart,
        attempt: AttemptStart,
    ) -> Arc<CallTap> {
        Arc::new(CallTap::start(
            self.shared.clone(),
            channel,
            start,
            Some(attempt),
        ))
    }

    /// Record a call TGV answered without forwarding, with its request as
    /// far as it was read. `channel` is the WebSocket's, for a message call.
    pub fn reject(
        &self,
        channel: Option<ChannelId>,
        start: CallStart,
        request: Option<&bytes::Bytes>,
        rejection: crate::capture::CallRejection,
    ) {
        let channel = channel.unwrap_or_else(|| ChannelId::generate(self.shared.ids.as_ref()));
        let tap = CallTap::start(self.shared.clone(), channel, start, None);
        if let Some(request) = request {
            tap.request(request);
        }
        tap.reject(rejection);
    }

    /// A slot to record one operator action, or `None` when the recorder has
    /// no room.
    pub fn operator_slot(&self) -> Option<OperatorSlot> {
        OperatorSlot::reserve(self.shared.clone())
    }

    /// Reserve the end record of a WebSocket channel whose handshake call
    /// already opened it. Never blocks; without a reservation the end is not
    /// recorded and the channel is counted as unrecorded.
    pub fn channel(&self, channel: ChannelId) -> ChannelTap {
        ChannelTap::open(self.shared.clone(), channel)
    }

    /// A reader of this recorder's counters that does not keep its queue
    /// open, so they can be read once the task has drained.
    pub fn meter(&self) -> RecorderMeter {
        RecorderMeter {
            counters: self.shared.counters.clone(),
        }
    }

    pub fn metrics(&self) -> RecorderMetrics {
        let sender = &self.shared.sender;
        let byte_capacity = self.shared.options.queue_bytes;
        self.shared.counters.snapshot(QueueMetrics {
            items: sender.max_capacity().saturating_sub(sender.capacity()),
            capacity: sender.max_capacity(),
            bytes: byte_capacity.saturating_sub(self.shared.bytes.available_permits()),
            byte_capacity,
        })
    }

    /// Committed observations from now on. A subscriber that falls more than
    /// the live buffer behind is told it lagged and should catch up from the
    /// store.
    pub fn subscribe(&self) -> broadcast::Receiver<Observation> {
        self.live.subscribe()
    }
}
