//! The requests Ren sent a client and waits on.

use std::{borrow::Borrow, collections::BTreeMap};

/// Each request Ren sent and has no answer to yet, by the id it was sent
/// with, and what it was for (`K`). Ids count up from zero, named as the
/// client's protocol names them (`Id`), and never repeat.
#[derive(Debug)]
pub(crate) struct RequestLedger<Id, K> {
    next: u64,
    pending: BTreeMap<Id, K>,
    name: fn(u64) -> Id,
}

impl<Id: Ord + Clone, K> RequestLedger<Id, K> {
    /// A ledger whose `n`th request is named `name(n)`.
    pub(crate) fn new(name: fn(u64) -> Id) -> Self {
        Self {
            next: 0,
            pending: BTreeMap::new(),
            name,
        }
    }

    /// A new request, for `kind`: its id.
    pub(crate) fn send(&mut self, kind: K) -> Id {
        let id = (self.name)(self.next);
        self.next += 1;
        self.pending.insert(id.clone(), kind);
        id
    }

    /// What the request `id` answers was for, once; none for an id Ren
    /// never sent or that was answered already.
    pub(crate) fn answer<Q>(&mut self, id: &Q) -> Option<K>
    where
        Id: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.pending.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::RequestLedger;

    #[test]
    fn each_request_is_answered_once() {
        let mut ledger = RequestLedger::new(|n| format!("ren-{n}"));
        assert_eq!(ledger.send('a'), "ren-0");
        assert_eq!(ledger.send('b'), "ren-1");
        assert_eq!(ledger.answer("ren-1"), Some('b'));
        assert_eq!(ledger.answer("ren-1"), None);
        assert_eq!(ledger.answer("ren-9"), None);
        assert_eq!(ledger.answer("ren-0"), Some('a'));
    }
}
