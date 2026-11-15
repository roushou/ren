//! The parts every client's codec is built from, composed into each one
//! (there is no codec trait): a line read as JSON (`Envelope`), a JSON-RPC
//! message told apart (`JsonRpc`), the requests Ren sent and waits on
//! (`RequestLedger`), the approvals that wait on the operator
//! (`ApprovalBook`), the turn under way and the ids a client that names no
//! turns gets (`TurnSlot`, `TurnMint`), and what each prompt's images are
//! until the client echoes it (`EchoQueue`). Each codec keeps only its
//! client's wire mapping. Sans-IO: values in, values out.

mod approvals;
mod envelope;
mod ledger;
mod rpc;
mod turn;

pub(crate) use approvals::ApprovalBook;
pub(crate) use envelope::Envelope;
pub(crate) use ledger::RequestLedger;
pub(crate) use rpc::{JsonRpc, RpcError};
pub(crate) use turn::{EchoQueue, Stop, TurnMint, TurnSlot};
