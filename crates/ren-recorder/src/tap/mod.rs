//! The capture side: what the relay calls while forwarding. Nothing here
//! blocks or awaits. Admission to the queue is decided with `try_` operations,
//! and anything that does not fit is counted as omitted.
//!
//! A call's capture is a typestate, one value per state, each owned by
//! whoever drives that state: [`AttemptTap`] while an attempt awaits its
//! response head, [`ResponseTap`] while its response streams, and
//! [`CallTap`] between attempts. Every transition consumes the state it
//! leaves, so a second terminal, an attempt over an open one, or a head
//! without an attempt cannot be written. No state holds a lock: whoever owns
//! it writes its bodies through `&mut self`.
//!
//! Dropping any state records what is still open as dropped: the attempt's
//! terminal first, then the call's. An outcome is never rounded up.
//!
//! A WebSocket channel's end is [`ChannelTap`], here too. An agent run's
//! capture is in `run`, and the single-record slots (an operator action,
//! a history edit, a session's facts) are in `slots`.

mod run;
mod slots;

pub use run::{RunSession, RunTap, RunUnrecorded};
pub use slots::{HistorySlot, OperatorSlot, ProjectSlot, SessionSlot, WorkSlot};

use super::item::{AttemptSlots, Item, Piece, Shared, Track};
use crate::caller::Caller;
use crate::{
    batch::{
        AttemptBody, BodyEnd, CloseChannel, MessageMark, OpenCall, Recorded, ResponseHead, Stamp,
        StartAttempt, Unattributed,
    },
    capture::{BodyEnding, CallRejection, ChannelEnd, Omission, TerminalRecord},
    id::{AttemptId, CallId, ChannelId},
    routing::{ConfigRevision, OperatorRevision, UpstreamId},
};
use bytes::Bytes;
use ren_harness::attribution::Attribution;
use ren_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
    route::{Api, Endpoint, Method, Provider, Transport},
    websocket::Payload,
};
use std::{collections::VecDeque, fmt, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, mpsc::OwnedPermit};

/// What the gateway knows when a call is admitted.
#[derive(Debug, Clone)]
pub struct CallStart {
    pub transport: Transport,
    /// The provider and wire format of the entry the call was routed to.
    pub api: Api,
    pub endpoint: Endpoint,
    pub method: Method,
    pub config_revision: ConfigRevision,
    pub attribution: Attribution,
    /// Who made the call.
    pub caller: Caller,
    /// The operator state the call was planned against, once planned.
    pub operator_revision: Option<OperatorRevision>,
    /// The routing plan, once planned.
    pub plan: Option<Recorded>,
    /// Attempts the plan allows; their records are reserved at admission,
    /// each as one of the recorder's units (`RecorderSetup::max_calls`).
    pub attempts: usize,
}

/// What is known when an attempt starts.
#[derive(Debug, Clone)]
pub struct AttemptStart {
    pub upstream: UpstreamId,
    /// The provider of the upstream entry.
    pub provider: Provider,
    /// What Ren changed in the request, if anything.
    pub changes: Option<Recorded>,
    /// The request body the attempt sends.
    pub body: AttemptRequest,
}

impl AttemptStart {
    /// An attempt that sends the client's request unchanged.
    pub fn unchanged(upstream: UpstreamId, provider: Provider) -> Self {
        Self {
            upstream,
            provider,
            changes: None,
            body: AttemptRequest::Original,
        }
    }
}

/// The request body an attempt sends. A changed body is recorded with the
/// attempt when it starts, so an attempt that sends the original has no
/// changed body to write.
#[derive(Clone)]
pub enum AttemptRequest {
    /// The call's request as the client sent it.
    Original,
    /// A body a transform changed, whole.
    Changed(Bytes),
}

impl AttemptRequest {
    /// The recorded label.
    pub fn label(&self) -> AttemptBody {
        match self {
            Self::Original => AttemptBody::Original,
            Self::Changed(_) => AttemptBody::Changed,
        }
    }
}

/// Request bodies can carry secrets: only their length is shown.
impl fmt::Debug for AttemptRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Original => f.write_str("Original"),
            Self::Changed(bytes) => write!(f, "Changed({} bytes)", bytes.len()),
        }
    }
}

/// The head of an attempt's response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    pub format: ResponseFormat,
    pub encoding: ResponseEncoding,
}

/// A call between attempts: its last attempt ended, and the plan may allow
/// another. Dropping it closes the call.
#[must_use = "dropping a call's tap closes the call"]
pub struct CallTap {
    call: CallCapture,
}

/// A call whose attempt awaits its response head. Dropping it records the
/// attempt as dropped, then closes the call.
///
/// Each transition consumes the state it leaves:
///
/// ```
/// # use ren_recorder::{AttemptStart, AttemptTap, Head, capture::TerminalRecord};
/// fn fall_over(tap: AttemptTap, head: Head, next: AttemptStart, end: TerminalRecord) {
///     let call = tap.head(head).end(end);
///     call.attempt(next).finish(end);
/// }
/// ```
///
/// so a head after the attempt's end does not compile,
///
/// ```compile_fail
/// # use ren_recorder::{AttemptTap, Head, capture::TerminalRecord};
/// fn late_head(tap: AttemptTap, head: Head, end: TerminalRecord) {
///     let _call = tap.end(end);
///     let _response = tap.head(head);
/// }
/// ```
///
/// nor a second terminal,
///
/// ```compile_fail
/// # use ren_recorder::{ResponseTap, capture::TerminalRecord};
/// fn twice(tap: ResponseTap, end: TerminalRecord) {
///     tap.finish(end);
///     tap.finish(end);
/// }
/// ```
///
/// nor an attempt over an open one,
///
/// ```compile_fail
/// # use ren_recorder::{AttemptStart, AttemptTap};
/// fn over(tap: AttemptTap, next: AttemptStart) {
///     let _second = tap.attempt(next);
/// }
/// ```
///
/// nor a head with no attempt open.
///
/// ```compile_fail
/// # use ren_recorder::{CallTap, Head};
/// fn headless(call: CallTap, head: Head) {
///     let _response = call.head(head);
/// }
/// ```
#[must_use = "dropping an attempt's tap records it as dropped"]
pub struct AttemptTap {
    // Fields drop in order: the attempt's terminal is queued before the call's.
    attempt: AttemptCapture,
    /// The response head's slot, when the attempt is recorded.
    head: Option<OwnedPermit<Item>>,
    call: CallCapture,
}

/// A call whose attempt's response is streaming. Dropping it records the
/// attempt as dropped, then closes the call.
#[must_use = "dropping a response's tap records its attempt as dropped"]
pub struct ResponseTap {
    // Fields drop in order: the attempt's terminal is queued before the call's.
    attempt: AttemptCapture,
    call: CallCapture,
}

/// What every state of a call's tap holds: its identity, its reservations,
/// and its request's tally. Dropping it records the call's finish.
struct CallCapture {
    shared: Arc<Shared>,
    stamp: Stamp,
    slots: CallSlots,
    request: Tally,
    /// Attempts started so far.
    attempts: u32,
}

/// The call's reservations.
enum CallSlots {
    /// Reserved at admission. The finish slot is always reserved, so the
    /// call's terminal is never lost to a full queue.
    Held {
        /// Slots of the attempts the plan still allows.
        spare: VecDeque<AttemptSlots>,
        finish: OwnedPermit<Item>,
        units: OwnedSemaphorePermit,
    },
    /// Nothing is reserved: the call is unrecorded, or its finish is sent.
    Released,
}

/// One attempt's identity, tallies, and finish slot. Dropping it records the
/// attempt as dropped.
struct AttemptCapture {
    shared: Arc<Shared>,
    stamp: Stamp,
    id: AttemptId,
    /// The finish slot: none when the attempt is unrecorded or has finished.
    finish: Option<OwnedPermit<Item>>,
    request: ChangedTally,
    response: Tally,
}

/// The attempt's changed request body.
#[derive(Debug, Clone, Copy)]
enum ChangedTally {
    Original,
    Changed(Tally),
}

#[derive(Debug, Clone, Copy)]
struct Tally {
    observed: u64,
    ending: BodyEnding,
}

impl Tally {
    const OPEN: Self = Self {
        observed: 0,
        ending: BodyEnding::Open,
    };

    fn end(self, ending: BodyEnding) -> BodyEnd {
        BodyEnd {
            observed_bytes: self.observed,
            ending: match self.ending {
                BodyEnding::Open => ending,
                BodyEnding::Ended | BodyEnding::Unfinished => self.ending,
            },
        }
    }

    fn ended(&mut self) {
        self.ending = BodyEnding::Ended;
    }

    /// Count `source` and, when `feed` queues, admit its pieces.
    fn feed(&mut self, feed: Feed<'_>, track: Track, source: Source<'_>, message: Option<Payload>) {
        let bytes = source.bytes();
        if bytes.is_empty() {
            return;
        }
        let offset = self.observed;
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.observed = offset.saturating_add(len);
        let Feed::Queue { shared, call } = feed else {
            return;
        };
        let counters = &shared.counters;
        let split = shared.options.split(offset, bytes.len());
        counters.omit(Omission::PolicyLimit, split.over_limit);
        let mut mark = message.map(|payload| MessageMark {
            offset,
            len,
            payload,
            at: shared.clock.now().wall,
        });
        for piece in split.pieces {
            let piece_len = u64::try_from(piece.len()).unwrap_or(u64::MAX);
            let piece_offset =
                offset.saturating_add(u64::try_from(piece.start).unwrap_or(u64::MAX));
            match shared.admit(piece.len()) {
                Err(reason) => counters.omit(reason, piece_len),
                Ok(admission) => admission.send(Piece {
                    call,
                    track,
                    offset: piece_offset,
                    bytes: source.piece(piece.start, piece.end),
                    enqueued: shared.clock.now().monotonic,
                    message: mark.take(),
                }),
            }
            mark = None;
        }
    }
}

/// Where a body's bytes go.
#[derive(Clone, Copy)]
enum Feed<'a> {
    /// Into the queue, as far as it admits them.
    Queue { shared: &'a Shared, call: CallId },
    /// Nowhere: the body is unrecorded, and only its length is kept.
    Count,
}

/// Bytes to capture: copied when borrowed from a network buffer, shared when
/// the gateway already owns exactly these bytes.
#[derive(Clone, Copy)]
enum Source<'a> {
    Borrowed(&'a [u8]),
    Shared(&'a Bytes),
}

impl Source<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }

    fn piece(&self, from: usize, to: usize) -> Bytes {
        match self {
            Self::Borrowed(bytes) => {
                Bytes::copy_from_slice(bytes.get(from..to).unwrap_or_default())
            }
            Self::Shared(bytes) => bytes.slice(from..to),
        }
    }
}

impl CallCapture {
    /// Admit a call, reserving the records of `attempts` attempts, and send
    /// its open record with its first attempt, if it makes one. Without a
    /// reservation the call is unrecorded and counted.
    fn admit(
        shared: Arc<Shared>,
        stamp: Stamp,
        start: CallStart,
        first: Option<(AttemptId, &AttemptStart)>,
        attempts: usize,
    ) -> (Self, Option<AttemptSlots>) {
        let (slots, current) = match shared.reserve(attempts) {
            None => {
                shared.counters.omit(Omission::Unrecorded, 1);
                (CallSlots::Released, None)
            }
            Some(mut reservation) => {
                let endpoint = start.endpoint;
                let current = reservation.attempts.pop_front();
                reservation.open.send(Item::Open {
                    open: Box::new(OpenCall {
                        stamp,
                        transport: start.transport,
                        provider: start.api.provider(),
                        wire: start.api.wire(),
                        endpoint: start.endpoint,
                        method: start.method,
                        config_revision: start.config_revision,
                        attribution: start.attribution,
                        caller: start.caller,
                        operator_revision: start.operator_revision,
                        plan: start.plan,
                    }),
                    attempt: first.map(|(id, attempt)| attempt.record(stamp, id, 0)),
                    endpoint,
                });
                (
                    CallSlots::Held {
                        spare: reservation.attempts,
                        finish: reservation.finish,
                        units: reservation.units,
                    },
                    current,
                )
            }
        };
        let call = Self {
            shared,
            stamp,
            slots,
            request: Tally::OPEN,
            attempts: u32::from(first.is_some()),
        };
        (call, current)
    }

    /// A new call's stamp on `channel`. Its identity is drawn before its
    /// first attempt's.
    fn stamp(shared: &Shared, channel: ChannelId) -> Stamp {
        Stamp {
            at: shared.clock.now(),
            channel,
            call: CallId::generate(shared.ids.as_ref()),
        }
    }

    fn stamped(&self) -> Stamp {
        Stamp {
            at: self.shared.clock.now(),
            ..self.stamp
        }
    }

    fn request(&mut self, source: Source<'_>, message: Option<Payload>) {
        let feed = match self.slots {
            CallSlots::Held { .. } => Feed::Queue {
                shared: &self.shared,
                call: self.stamp.call,
            },
            CallSlots::Released => Feed::Count,
        };
        self.request.feed(feed, Track::Request, source, message);
    }

    /// Start the call's next attempt with the slots the plan reserved. An
    /// attempt beyond the plan is a gateway bug: it is forwarded unrecorded,
    /// and counted.
    fn attempt(mut self, start: AttemptStart) -> AttemptTap {
        let id = AttemptId::generate(self.shared.ids.as_ref());
        let ordinal = self.attempts;
        self.attempts = self.attempts.saturating_add(1);
        let stamp = self.stamped();
        let slots = match &mut self.slots {
            CallSlots::Held { spare, .. } => match spare.pop_front() {
                Some(AttemptSlots {
                    start: Some(slot),
                    head,
                    finish,
                }) => {
                    slot.send(Item::Attempt(start.record(stamp, id, ordinal)));
                    Some(AttemptSlots {
                        start: None,
                        head,
                        finish,
                    })
                }
                Some(AttemptSlots { start: None, .. }) | None => {
                    tracing::warn!("an attempt beyond the call's plan is not recorded");
                    self.shared.counters.omit(Omission::Unrecorded, 1);
                    None
                }
            },
            CallSlots::Released => None,
        };
        AttemptTap::open(self, id, start.body, slots)
    }

    /// Record the call's finish. It runs once: the slots are released after.
    fn close(&mut self, rejection: Option<CallRejection>) {
        let slots = std::mem::replace(&mut self.slots, CallSlots::Released);
        let CallSlots::Held { finish, units, .. } = slots else {
            return;
        };
        finish.send(Item::FinishCall {
            stamp: self.stamped(),
            request: self.request.end(BodyEnding::Unfinished),
            rejection,
            _units: units,
        });
    }
}

impl Drop for CallCapture {
    fn drop(&mut self) {
        self.close(None);
    }
}

impl AttemptCapture {
    fn response(&mut self, source: Source<'_>, message: Option<Payload>) {
        let feed = match self.finish {
            Some(_) => Feed::Queue {
                shared: &self.shared,
                call: self.stamp.call,
            },
            None => Feed::Count,
        };
        self.response.feed(feed, Track::Response, source, message);
    }

    /// Record the attempt's terminal; bodies whose end was not observed are
    /// recorded as unfinished. It runs once: the slot is used after.
    fn close(&mut self, end: TerminalRecord) {
        let Some(finish) = self.finish.take() else {
            return;
        };
        finish.send(Item::FinishAttempt {
            stamp: Stamp {
                at: self.shared.clock.now(),
                ..self.stamp
            },
            attempt: self.id,
            end,
            response: self.response.end(BodyEnding::Unfinished),
            request: match self.request {
                ChangedTally::Original => None,
                ChangedTally::Changed(tally) => Some(tally.end(BodyEnding::Unfinished)),
            },
        });
    }
}

impl Drop for AttemptCapture {
    fn drop(&mut self) {
        self.close(TerminalRecord::DROPPED);
    }
}

impl AttemptTap {
    /// Admit a call and its first attempt.
    pub(super) fn start(
        shared: Arc<Shared>,
        channel: ChannelId,
        start: CallStart,
        attempt: AttemptStart,
    ) -> Self {
        let stamp = CallCapture::stamp(&shared, channel);
        let id = AttemptId::generate(shared.ids.as_ref());
        let attempts = start.attempts.max(1);
        let (call, slots) =
            CallCapture::admit(shared, stamp, start, Some((id, &attempt)), attempts);
        Self::open(call, id, attempt.body, slots)
    }

    /// The attempt `id` of `call` is open: record its changed body, if it
    /// sends one.
    fn open(
        call: CallCapture,
        id: AttemptId,
        body: AttemptRequest,
        slots: Option<AttemptSlots>,
    ) -> Self {
        let (head, finish) = match slots {
            Some(slots) => (Some(slots.head), Some(slots.finish)),
            None => (None, None),
        };
        let feed = match finish {
            Some(_) => Feed::Queue {
                shared: &call.shared,
                call: call.stamp.call,
            },
            None => Feed::Count,
        };
        let request = match &body {
            AttemptRequest::Original => ChangedTally::Original,
            AttemptRequest::Changed(bytes) => {
                let mut tally = Tally::OPEN;
                tally.feed(feed, Track::AttemptRequest, Source::Shared(bytes), None);
                tally.ended();
                ChangedTally::Changed(tally)
            }
        };
        let attempt = AttemptCapture {
            shared: call.shared.clone(),
            stamp: call.stamp,
            id,
            finish,
            request,
            response: Tally::OPEN,
        };
        Self {
            attempt,
            head,
            call,
        }
    }

    pub fn call(&self) -> CallId {
        self.call.stamp.call
    }

    pub fn channel(&self) -> ChannelId {
        self.call.stamp.channel
    }

    /// The whole request as the client sent it, buffered by the gateway. The
    /// recorder shares these bytes instead of copying them; the request
    /// ends with them.
    pub fn request(&mut self, bytes: &Bytes) {
        self.call.request(Source::Shared(bytes), None);
        self.call.request.ended();
    }

    /// Request bytes as they stream. They are copied only once admitted, so
    /// a slice never pins a larger network buffer than the budget accounts
    /// for.
    pub fn request_data(&mut self, bytes: &[u8]) {
        self.call.request(Source::Borrowed(bytes), None);
    }

    /// One whole WebSocket message from the client, as in
    /// [`ResponseTap::message`].
    pub fn request_message(&mut self, payload: Payload, bytes: &[u8]) {
        self.call.request(Source::Borrowed(bytes), Some(payload));
    }

    /// The end of the request was observed.
    pub fn request_ended(&mut self) {
        self.call.request.ended();
    }

    /// The response head arrived: its body streams from here.
    pub fn head(self, head: Head) -> ResponseTap {
        let Self {
            attempt,
            head: slot,
            call,
        } = self;
        if let Some(slot) = slot {
            slot.send(Item::Head(ResponseHead {
                stamp: call.stamped(),
                attempt: attempt.id,
                status: head.status,
                format: head.format,
                encoding: head.encoding,
            }));
        }
        ResponseTap { attempt, call }
    }

    /// The attempt ended before its head; the call stays open for its next
    /// attempt.
    pub fn end(self, end: TerminalRecord) -> CallTap {
        let Self {
            mut attempt, call, ..
        } = self;
        attempt.close(end);
        CallTap { call }
    }

    /// The attempt ended before its head, and with it the call.
    pub fn finish(self, end: TerminalRecord) {
        self.end(end).finish();
    }
}

impl ResponseTap {
    pub fn call(&self) -> CallId {
        self.call.stamp.call
    }

    pub fn channel(&self) -> ChannelId {
        self.call.stamp.channel
    }

    /// Bytes forwarded in `direction`, copied only once admitted.
    pub fn data(&mut self, direction: Direction, bytes: &[u8]) {
        match direction {
            Direction::Request => self.call.request(Source::Borrowed(bytes), None),
            Direction::Response => self.attempt.response(Source::Borrowed(bytes), None),
        }
    }

    /// One whole WebSocket message forwarded in `direction`. Its boundary is
    /// recorded with its first stored piece; a message whose first piece is
    /// omitted keeps no boundary, and the body shows the gap.
    pub fn message(&mut self, direction: Direction, payload: Payload, bytes: &[u8]) {
        let source = Source::Borrowed(bytes);
        match direction {
            Direction::Request => self.call.request(source, Some(payload)),
            Direction::Response => self.attempt.response(source, Some(payload)),
        }
    }

    /// The end of the body in `direction` was observed.
    pub fn ended(&mut self, direction: Direction) {
        match direction {
            Direction::Request => self.call.request.ended(),
            Direction::Response => self.attempt.response.ended(),
        }
    }

    /// The attempt ended; the call stays open for its next attempt.
    pub fn end(self, end: TerminalRecord) -> CallTap {
        let Self { mut attempt, call } = self;
        attempt.close(end);
        CallTap { call }
    }

    /// The attempt ended, and with it the call.
    pub fn finish(self, end: TerminalRecord) {
        self.end(end).finish();
    }
}

impl CallTap {
    /// Admit a call that makes no attempt, to record its refusal.
    pub(super) fn refused(shared: Arc<Shared>, channel: ChannelId, start: CallStart) -> Self {
        let stamp = CallCapture::stamp(&shared, channel);
        let (call, _) = CallCapture::admit(shared, stamp, start, None, 0);
        Self { call }
    }

    pub fn call(&self) -> CallId {
        self.call.stamp.call
    }

    pub fn channel(&self) -> ChannelId {
        self.call.stamp.channel
    }

    /// Start the call's next attempt. The plan reserved its records at
    /// admission; an attempt beyond the plan is forwarded unrecorded and
    /// counted.
    pub fn attempt(self, start: AttemptStart) -> AttemptTap {
        self.call.attempt(start)
    }

    /// Close the call after its last attempt.
    pub fn finish(mut self) {
        self.call.close(None);
    }

    /// The request as far as it was read, for a refused call.
    pub(super) fn request(&mut self, bytes: &Bytes) {
        self.call.request(Source::Shared(bytes), None);
        self.call.request.ended();
    }

    /// Close a call that made no attempt: Ren answered it.
    pub(super) fn reject(mut self, rejection: CallRejection) {
        self.call.close(Some(rejection));
    }
}

impl AttemptStart {
    fn record(&self, stamp: Stamp, attempt: AttemptId, ordinal: u32) -> StartAttempt {
        StartAttempt {
            stamp,
            attempt,
            ordinal,
            upstream: self.upstream.clone(),
            provider: self.provider,
            changes: self.changes.clone(),
            body: self.body.label(),
        }
    }
}

/// Capture of one WebSocket channel's end. Dropping it without
/// [`ChannelTap::close`] records the channel as dropped.
#[must_use = "dropping a channel's tap records it as dropped"]
pub struct ChannelTap {
    shared: Arc<Shared>,
    channel: ChannelId,
    /// The end's slot: none when the channel is unrecorded or has closed.
    reservation: Option<(OwnedPermit<Item>, OwnedSemaphorePermit)>,
    unattributed: Unattributed,
}

impl ChannelTap {
    pub(super) fn open(shared: Arc<Shared>, channel: ChannelId) -> Self {
        let reservation = shared.reserve_close();
        if reservation.is_none() {
            shared.counters.omit(Omission::Unrecorded, 1);
        }
        Self {
            shared,
            channel,
            reservation,
            unattributed: Unattributed::default(),
        }
    }

    pub fn channel(&self) -> ChannelId {
        self.channel
    }

    /// A message no call owns was forwarded. Its bytes are counted, not stored.
    pub fn unattributed(&mut self, bytes: &[u8]) {
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.unattributed.messages = self.unattributed.messages.saturating_add(1);
        self.unattributed.bytes = self.unattributed.bytes.saturating_add(len);
        self.shared.counters.omit(Omission::Unattributed, len);
    }

    /// Record the channel's end.
    pub fn close(mut self, end: ChannelEnd, close_code: Option<u16>) {
        self.send(end, close_code);
    }

    fn send(&mut self, end: ChannelEnd, close_code: Option<u16>) {
        let Some((slot, channel)) = self.reservation.take() else {
            return;
        };
        slot.send(Item::Close {
            close: CloseChannel {
                at: self.shared.clock.now(),
                channel: self.channel,
                end,
                close_code,
                unattributed: self.unattributed,
            },
            _channel: channel,
        });
    }
}

impl Drop for ChannelTap {
    fn drop(&mut self) {
        self.send(ChannelEnd::Dropped, None);
    }
}

#[cfg(test)]
mod tests;
