//! The pure lifecycles forwarding drives: an HTTP attempt ([`attempt`]), a
//! call's way through its candidates ([`call`]), a WebSocket channel and its
//! calls ([`channel`]), and whether a failed attempt may be retried
//! ([`retry`]). None of them performs I/O; the forwarding shell feeds them
//! events and carries out what they return.

pub mod attempt;
pub mod call;
pub mod channel;
pub mod retry;
