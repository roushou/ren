//! The approvals a client waits on the operator for.

use crate::drive::{ApprovalId, CommandRefused, DriveEvent, Received};
use std::collections::{BTreeMap, btree_map};

/// Each approval waiting on the operator, with what answering it needs
/// (`V`: the client's request id, what it asked, the options it gave).
#[derive(Debug)]
pub(crate) struct ApprovalBook<V> {
    waiting: BTreeMap<ApprovalId, V>,
}

impl<V> ApprovalBook<V> {
    pub(crate) fn new() -> Self {
        Self {
            waiting: BTreeMap::new(),
        }
    }

    /// The client asks for `approval`.
    pub(crate) fn ask(&mut self, approval: ApprovalId, answering: V) {
        self.waiting.insert(approval, answering);
    }

    /// What answering `approval` needs, while it waits.
    pub(crate) fn get(&self, approval: &ApprovalId) -> Result<&V, CommandRefused> {
        self.waiting
            .get(approval)
            .ok_or_else(|| CommandRefused::UnknownApproval(approval.clone()))
    }

    /// The operator answers `approval`: what answering it needs, once.
    pub(crate) fn answer(&mut self, approval: ApprovalId) -> Result<V, CommandRefused> {
        match self.waiting.remove(&approval) {
            Some(answering) => Ok(answering),
            None => Err(CommandRefused::UnknownApproval(approval)),
        }
    }

    /// The client withdrew `approval` or it was answered elsewhere: whether
    /// it was waiting.
    pub(crate) fn resolve(&mut self, approval: &ApprovalId) -> Option<V> {
        self.waiting.remove(approval)
    }

    /// Every approval waiting, withdrawn: each answering need, in order.
    pub(crate) fn drain(&mut self) -> btree_map::IntoValues<ApprovalId, V> {
        std::mem::take(&mut self.waiting).into_values()
    }

    /// Every approval waiting no longer waits (its turn ended), each
    /// reported resolved.
    pub(crate) fn withdraw(&mut self, received: &mut Received) {
        received.events.extend(
            std::mem::take(&mut self.waiting)
                .into_keys()
                .map(|approval| DriveEvent::ApprovalResolved { approval }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::ApprovalBook;
    use crate::drive::{ApprovalId, CommandRefused, DriveEvent, Received};

    fn id(text: &str) -> ApprovalId {
        text.parse().unwrap()
    }

    #[test]
    fn an_approval_is_answered_once_and_withdrawn_with_its_turn() {
        let mut book = ApprovalBook::new();
        book.ask(id("1"), 'a');
        book.ask(id("2"), 'b');
        assert_eq!(book.answer(id("1")), Ok('a'));
        assert_eq!(
            book.answer(id("1")),
            Err(CommandRefused::UnknownApproval(id("1")))
        );
        let mut received = Received::default();
        book.withdraw(&mut received);
        assert_eq!(
            received.events,
            vec![DriveEvent::ApprovalResolved { approval: id("2") }]
        );
        assert_eq!(book.resolve(&id("2")), None);
    }
}
