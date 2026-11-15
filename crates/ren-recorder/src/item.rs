//! The ordered queue between the relay and the recorder task.
//!
//! One queue carries every record, so each call's records stay in order. Its
//! capacity is the data slots plus the lifecycle slots of every admitted call:
//! open, the call's finish, and per attempt a start (after the first), a
//! response head, and the attempt's finish. A call reserves the slots of every
//! attempt its plan allows at admission and holds one unit per attempt until
//! its finish record is dequeued, so data can never take the slots a terminal
//! needs. The caller sizes the units (`RecorderSetup::max_calls`) for the
//! attempts its plans allow; the recorder holds no plan of its own.

use super::{metrics::Counters, options::CaptureOptions};
pub(super) use crate::recording::{Piece, Record, Track};
use crate::{
    batch::{
        BodyEnd, CloseChannel, HistoryEdit, OpenCall, OperatorAction, ResponseHead, RunEntry,
        RunRecord, SessionRecord, Stamp, StartAttempt,
    },
    capture::{CallRejection, Omission, TerminalRecord},
    id::{AttemptId, IdSource},
    time::Clock,
};
use ren_protocol::route::Endpoint;
use std::sync::Arc;
use tokio::sync::{
    OwnedSemaphorePermit, Semaphore,
    mpsc::{self, OwnedPermit},
};

pub(super) enum Item {
    Open {
        open: Box<OpenCall>,
        /// The first attempt; absent for a call refused before dispatch.
        attempt: Option<StartAttempt>,
        endpoint: Endpoint,
    },
    /// A later attempt of an open call (failover).
    Attempt(StartAttempt),
    Head(ResponseHead),
    Data {
        piece: Piece,
        /// Released when dequeued: frees a data slot.
        slot: OwnedSemaphorePermit,
        /// Released after commit: the bytes stay accounted until persisted.
        budget: OwnedSemaphorePermit,
    },
    /// A WebSocket channel ended.
    Close {
        close: CloseChannel,
        /// Released when dequeued, returning the channel's slot.
        _channel: OwnedSemaphorePermit,
    },
    FinishAttempt {
        stamp: Stamp,
        attempt: AttemptId,
        end: TerminalRecord,
        response: BodyEnd,
        /// The changed request body's end, if the attempt sent one.
        request: Option<BodyEnd>,
    },
    FinishCall {
        stamp: Stamp,
        request: BodyEnd,
        rejection: Option<CallRejection>,
        /// Released when dequeued, returning this call's lifecycle units.
        _units: OwnedSemaphorePermit,
    },
    Operator {
        action: OperatorAction,
        /// Released when dequeued.
        _slot: OwnedSemaphorePermit,
    },
    Session {
        record: SessionRecord,
        /// Released when dequeued.
        _slot: OwnedSemaphorePermit,
    },
    Run {
        record: RunRecord,
        /// Released when dequeued.
        _slot: OwnedSemaphorePermit,
    },
    History {
        edit: HistoryEdit,
        /// Released when dequeued.
        _slot: OwnedSemaphorePermit,
    },
    RunEntry {
        entry: Box<RunEntry>,
        /// Released when dequeued: frees a data slot.
        slot: OwnedSemaphorePermit,
        /// Released after commit: its bytes stay accounted until persisted.
        budget: OwnedSemaphorePermit,
    },
}

impl Item {
    /// The record for the core, and the byte budget a data piece holds until
    /// its bytes commit. Every other permit is released here, as the item is
    /// dequeued.
    pub(super) fn into_record(self) -> (Record, Option<OwnedSemaphorePermit>) {
        match self {
            Self::Open {
                open,
                attempt,
                endpoint,
            } => (
                Record::Open {
                    open,
                    attempt,
                    endpoint,
                },
                None,
            ),
            Self::Attempt(attempt) => (Record::Attempt(attempt), None),
            Self::Head(head) => (Record::Head(head), None),
            Self::Data {
                piece,
                slot,
                budget,
            } => {
                drop(slot);
                (Record::Data(piece), Some(budget))
            }
            Self::Close { close, _channel } => (Record::Close(close), None),
            Self::FinishAttempt {
                stamp,
                attempt,
                end,
                response,
                request,
            } => (
                Record::FinishAttempt {
                    stamp,
                    attempt,
                    end,
                    response,
                    request,
                },
                None,
            ),
            Self::FinishCall {
                stamp,
                request,
                rejection,
                _units,
            } => (
                Record::FinishCall {
                    stamp,
                    request,
                    rejection,
                },
                None,
            ),
            Self::Operator { action, _slot } => (Record::Operator(action), None),
            Self::Session { record, _slot } => (Record::Session(record), None),
            Self::Run { record, _slot } => (Record::Run(record), None),
            Self::History { edit, _slot } => (Record::History(edit), None),
            Self::RunEntry {
                entry,
                slot,
                budget,
            } => {
                drop(slot);
                (Record::RunEntry(entry), Some(budget))
            }
        }
    }
}

/// State shared by every tap.
pub(super) struct Shared {
    pub sender: mpsc::Sender<Item>,
    /// Lifecycle units: one per attempt of an open call, per channel end,
    /// and per pending lifecycle record of a run or the operator.
    units: Arc<Semaphore>,
    slots: Arc<Semaphore>,
    pub bytes: Arc<Semaphore>,
    pub counters: Arc<Counters>,
    pub options: CaptureOptions,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdSource>,
}

/// A call's lifecycle slots and its units.
pub(super) struct Reservation {
    pub open: OwnedPermit<Item>,
    pub finish: OwnedPermit<Item>,
    /// One entry per attempt the call may make, in order.
    pub attempts: std::collections::VecDeque<AttemptSlots>,
    pub units: OwnedSemaphorePermit,
}

/// One attempt's lifecycle slots. The first attempt starts with the call's
/// open record, so it has no start slot.
pub(super) struct AttemptSlots {
    pub start: Option<OwnedPermit<Item>>,
    pub head: OwnedPermit<Item>,
    pub finish: OwnedPermit<Item>,
}

/// Room for one data piece.
pub(super) struct Admission {
    permit: OwnedPermit<Item>,
    slot: OwnedSemaphorePermit,
    budget: OwnedSemaphorePermit,
}

impl Admission {
    pub fn send(self, piece: Piece) {
        self.permit.send(Item::Data {
            piece,
            slot: self.slot,
            budget: self.budget,
        });
    }

    pub fn send_entry(self, entry: RunEntry) {
        self.permit.send(Item::RunEntry {
            entry: Box::new(entry),
            slot: self.slot,
            budget: self.budget,
        });
    }
}

impl Shared {
    /// Queue slots per unit: a one-attempt call's open and finish, and its
    /// attempt's start, response head, and finish. A call of `n` attempts
    /// needs `3n + 1` slots and holds `n` units, so the units it holds
    /// always cover its slots.
    const UNIT_SLOTS: usize = 5;

    pub fn new(
        options: CaptureOptions,
        units: usize,
        counters: Arc<Counters>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdSource>,
    ) -> (Self, mpsc::Receiver<Item>) {
        let units = units.max(1);
        let capacity = options
            .queue_capacity
            .saturating_add(units.saturating_mul(Self::UNIT_SLOTS));
        let (sender, receiver) = mpsc::channel(capacity);
        let shared = Self {
            sender,
            units: Arc::new(Semaphore::new(units)),
            slots: Arc::new(Semaphore::new(options.queue_capacity)),
            bytes: Arc::new(Semaphore::new(options.queue_bytes)),
            counters,
            options,
            clock,
            ids,
        };
        (shared, receiver)
    }

    /// Lifecycle slots for a new call that may make up to `attempts`
    /// attempts, or `None` when the units the recorder was sized for are
    /// taken, or the recorder has stopped.
    pub fn reserve(&self, attempts: usize) -> Option<Reservation> {
        let units = u32::try_from(attempts.max(1)).ok()?;
        let units = self.units.clone().try_acquire_many_owned(units).ok()?;
        let slot = || self.sender.clone().try_reserve_owned().ok();
        let mut reserved = std::collections::VecDeque::new();
        for ordinal in 0..attempts {
            reserved.push_back(AttemptSlots {
                start: if ordinal == 0 { None } else { Some(slot()?) },
                head: slot()?,
                finish: slot()?,
            });
        }
        Some(Reservation {
            open: slot()?,
            finish: slot()?,
            attempts: reserved,
            units,
        })
    }

    /// One lifecycle slot, such as a WebSocket channel's end. It takes a
    /// unit, so the recorder's units must include the channel limit.
    pub fn reserve_close(&self) -> Option<(OwnedPermit<Item>, OwnedSemaphorePermit)> {
        let call = self.units.clone().try_acquire_owned().ok()?;
        let slot = self.sender.clone().try_reserve_owned().ok()?;
        Some((slot, call))
    }

    /// Room for `len` bytes of data, or the reason there is none.
    pub fn admit(&self, len: usize) -> Result<Admission, Omission> {
        let budget = u32::try_from(len)
            .ok()
            .and_then(|len| self.bytes.clone().try_acquire_many_owned(len).ok())
            .ok_or(Omission::QueueBytes)?;
        let slot = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Omission::QueueSlots)?;
        let permit = self
            .sender
            .clone()
            .try_reserve_owned()
            .map_err(|_| Omission::QueueSlots)?;
        Ok(Admission {
            permit,
            slot,
            budget,
        })
    }
}
