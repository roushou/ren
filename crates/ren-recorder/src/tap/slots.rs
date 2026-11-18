//! The single-record slots: an operator action, an archive or delete of
//! history, and a session's facts, each reserved before it is applied.

use crate::item::{Item, Shared};
use crate::{
    batch::{
        HistoryChange, HistoryEdit, HistorySubject, OperatorAction, Recorded, SessionFact,
        SessionRecord,
    },
    id::SessionId,
};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, mpsc::OwnedPermit};

/// One reserved slot for an operator action. Without one, the command is not
/// applied: every operator decision is recorded.
pub struct OperatorSlot {
    shared: Arc<Shared>,
    slot: OwnedPermit<Item>,
    permit: OwnedSemaphorePermit,
}

impl OperatorSlot {
    pub(crate) fn reserve(shared: Arc<Shared>) -> Option<Self> {
        let (slot, permit) = shared.reserve_close()?;
        Some(Self {
            shared,
            slot,
            permit,
        })
    }

    /// Record `command`, which made `state` at `revision`. Both documents are
    /// serialized here and kept as they are; if either cannot be serialized,
    /// nothing is recorded and the slot is released.
    pub fn record<C: serde::Serialize, S: serde::Serialize>(
        self,
        revision: crate::routing::OperatorRevision,
        actor: crate::batch::Actor,
        command: &C,
        state: &S,
    ) -> Result<(), serde_json::Error> {
        let action = OperatorAction {
            at: self.shared.clock.now(),
            revision,
            actor,
            command: Recorded::of(command)?,
            state: Recorded::of(state)?,
        };
        self.slot.send(Item::Operator {
            action,
            _slot: self.permit,
        });
        Ok(())
    }
}

/// One reserved slot for a history edit. Without one, the edit is not made:
/// every archive and delete is recorded with who asked for it.
pub struct HistorySlot {
    shared: Arc<Shared>,
    slot: OwnedPermit<Item>,
    permit: OwnedSemaphorePermit,
}

impl HistorySlot {
    pub(crate) fn reserve(shared: Arc<Shared>) -> Option<Self> {
        let (slot, permit) = shared.reserve_close()?;
        Some(Self {
            shared,
            slot,
            permit,
        })
    }

    /// Make `change` to `subject`, as `actor` asked. It applies when the
    /// recorder reaches it, after everything queued before.
    pub fn record(
        self,
        actor: crate::batch::Actor,
        subject: HistorySubject,
        change: HistoryChange,
    ) {
        self.slot.send(Item::History {
            edit: HistoryEdit {
                at: self.shared.clock.now(),
                actor,
                subject,
                change,
            },
            _slot: self.permit,
        });
    }
}

/// A reserved slot for one fact about a session, recorded when it is used.
pub struct SessionSlot {
    shared: Arc<Shared>,
    slot: OwnedPermit<Item>,
    permit: OwnedSemaphorePermit,
}

impl SessionSlot {
    pub(crate) fn reserve(shared: Arc<Shared>) -> Option<Self> {
        let (slot, permit) = shared.reserve_close()?;
        Some(Self {
            shared,
            slot,
            permit,
        })
    }

    /// Record `fact` about `session`, after everything queued before.
    pub fn record(self, session: SessionId, fact: SessionFact) {
        self.slot.send(Item::Session {
            record: SessionRecord {
                at: self.shared.clock.now(),
                session,
                fact,
            },
            _slot: self.permit,
        });
    }
}
