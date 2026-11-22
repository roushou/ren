//! Capture of an agent run: its start, opening, settings, conversation
//! entries, and end.

use crate::item::{Item, Shared};
use crate::{
    batch::{RunEntry, RunFact, RunRecord, SessionFact, SessionRecord},
    capture::{Omission, ProcessExit, RunEnd},
    id::{RunId, SessionId},
};
use ren_harness::{Harness, drive::DriveEvent, settings::RunSettings};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, mpsc::OwnedPermit};

/// The recorder had no room for a run's records, so the run was not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the recorder has no room to record a run")]
pub struct RunUnrecorded;

/// The session a run serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunSession {
    /// A session created with the run, recorded before it.
    New(SessionId),
    /// A session already recorded.
    Existing(SessionId),
}

impl RunSession {
    pub fn id(self) -> SessionId {
        match self {
            Self::New(session) | Self::Existing(session) => session,
        }
    }
}

/// Capture of one run's lifecycle. Its start is recorded when it is made, and
/// its end's slot is reserved until [`RunTap::end`]. Dropping it without an
/// end records nothing more: the next startup marks the run recovered,
/// unless its run host still holds it. A run its host holds is detached
/// ([`RunTap::detach`]) as the daemon stops, and rejoined
/// ([`crate::Recorder::rejoin`]) by the next.
pub struct RunTap {
    shared: Arc<Shared>,
    run: RunId,
    end: Option<(OwnedPermit<Item>, OwnedSemaphorePermit)>,
}

impl RunTap {
    pub(crate) fn start(
        shared: Arc<Shared>,
        run: RunId,
        session: RunSession,
        harness: Harness,
        workdir: String,
        settings: RunSettings,
        toolset: Option<ren_depot::package_set::Generation>,
    ) -> Result<Self, RunUnrecorded> {
        // Every slot first: a run starts recorded whole, or not at all.
        let created = match session {
            RunSession::New(_) => Some(shared.reserve_close().ok_or(RunUnrecorded)?),
            RunSession::Existing(_) => None,
        };
        let start = shared.reserve_close().ok_or(RunUnrecorded)?;
        let end = shared.reserve_close().ok_or(RunUnrecorded)?;
        if let Some((slot, permit)) = created {
            slot.send(Item::Session {
                record: SessionRecord {
                    at: shared.clock.now(),
                    session: session.id(),
                    fact: SessionFact::Created {
                        harness,
                        workdir: workdir.clone(),
                        settings: settings.clone(),
                    },
                },
                _slot: permit,
            });
        }
        let tap = Self {
            shared,
            run,
            end: Some(end),
        };
        tap.send(
            start,
            RunFact::Started {
                session: session.id(),
                harness,
                workdir,
                settings,
                toolset,
            },
        );
        Ok(tap)
    }

    /// Take up `run` again after a daemon restart: recorded as rejoined,
    /// with its end's slot reserved again.
    pub(crate) fn rejoin(shared: Arc<Shared>, run: RunId) -> Result<Self, RunUnrecorded> {
        let rejoined = shared.reserve_close().ok_or(RunUnrecorded)?;
        let end = shared.reserve_close().ok_or(RunUnrecorded)?;
        let tap = Self {
            shared,
            run,
            end: Some(end),
        };
        tap.send(rejoined, RunFact::Rejoined);
        Ok(tap)
    }

    pub fn run(&self) -> RunId {
        self.run
    }

    /// The daemon stops while the run goes on under its run host: recorded
    /// in its end's slot, as the end of what this daemon saw.
    pub fn detach(mut self) {
        if let Some(slot) = self.end.take() {
            self.send(slot, RunFact::Detached);
        }
    }

    /// The client opened its conversation as `thread`. Without room this is
    /// counted as unrecorded.
    pub fn opened(&self, thread: NativeThreadId) {
        match self.shared.reserve_close() {
            Some(slot) => self.send(slot, RunFact::Opened { thread }),
            None => self.shared.counters.omit(Omission::Unrecorded, 1),
        }
    }

    /// The run moved to `settings` mid-run. Without room this is counted
    /// as unrecorded.
    pub fn configured(&self, settings: RunSettings) {
        match self.shared.reserve_close() {
            Some(slot) => self.send(slot, RunFact::Configured { settings }),
            None => self.shared.counters.omit(Omission::Unrecorded, 1),
        }
    }

    /// Record one entry of the run's conversation, in `turn`. Never blocks:
    /// without room for its bytes it is counted as omitted, by reason.
    pub fn entry(&self, turn: Option<NativeTurnId>, event: DriveEvent) {
        let len = serde_json::to_vec(&event).map_or(0, |bytes| bytes.len());
        match self.shared.admit(len) {
            Ok(admission) => admission.send_entry(RunEntry {
                at: self.shared.clock.now(),
                run: self.run,
                turn,
                event,
            }),
            Err(omission) => self.shared.counters.omit(omission, 1),
        }
    }

    /// Record how the run ended.
    pub fn end(mut self, end: RunEnd, exit: Option<ProcessExit>) {
        if let Some(slot) = self.end.take() {
            self.send(slot, RunFact::Ended { end, exit });
        }
    }

    fn send(&self, (slot, permit): (OwnedPermit<Item>, OwnedSemaphorePermit), fact: RunFact) {
        slot.send(Item::Run {
            record: RunRecord {
                at: self.shared.clock.now(),
                run: self.run,
                fact,
            },
            _slot: permit,
        });
    }
}
