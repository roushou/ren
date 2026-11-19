//! The single-record slots: an operator action, an archive or delete of
//! history, and a session's, work item's, or project's facts, each reserved
//! before it is applied.

use crate::item::{Item, Shared};
use crate::{
    batch::{
        HistoryChange, HistoryEdit, HistorySubject, OperatorAction, ProjectFact, ProjectRecord,
        Recorded, SessionFact, SessionRecord, WorkFact, WorkRecord,
    },
    id::{ProjectId, SessionId, WorkItemId},
    project::ProjectRegistration,
    work::WorkOrder,
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

/// A reserved slot for one fact about a work item, recorded when it is used.
pub struct WorkSlot {
    shared: Arc<Shared>,
    slot: OwnedPermit<Item>,
    permit: OwnedSemaphorePermit,
}

impl WorkSlot {
    pub(crate) fn reserve(shared: Arc<Shared>) -> Option<Self> {
        let (slot, permit) = shared.reserve_close()?;
        Some(Self {
            shared,
            slot,
            permit,
        })
    }

    /// Queue `order` as a new work item, and return its id.
    pub fn queue(self, order: WorkOrder) -> WorkItemId {
        let item = WorkItemId::generate(self.shared.ids.as_ref());
        self.record(
            item,
            WorkFact::Queued {
                order: Box::new(order),
            },
        );
        item
    }

    /// Record `fact` about `item`, after everything queued before.
    pub fn record(self, item: WorkItemId, fact: WorkFact) {
        self.slot.send(Item::Work {
            record: WorkRecord {
                at: self.shared.clock.now(),
                item,
                fact,
            },
            _slot: self.permit,
        });
    }
}

/// A reserved slot for one fact about a project, recorded when it is used.
pub struct ProjectSlot {
    shared: Arc<Shared>,
    slot: OwnedPermit<Item>,
    permit: OwnedSemaphorePermit,
}

impl ProjectSlot {
    pub(crate) fn reserve(shared: Arc<Shared>) -> Option<Self> {
        let (slot, permit) = shared.reserve_close()?;
        Some(Self {
            shared,
            slot,
            permit,
        })
    }

    /// Register `registration` as a new project, and return its id.
    pub fn register(self, registration: ProjectRegistration) -> ProjectId {
        let project = ProjectId::generate(self.shared.ids.as_ref());
        self.record(project, ProjectFact::Registered { registration });
        project
    }

    /// Record `fact` about `project`, after everything queued before.
    pub fn record(self, project: ProjectId, fact: ProjectFact) {
        self.slot.send(Item::Project {
            record: ProjectRecord {
                at: self.shared.clock.now(),
                project,
                fact,
            },
            _slot: self.permit,
        });
    }
}
