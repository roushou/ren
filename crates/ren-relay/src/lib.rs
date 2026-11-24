//! A forward proxy that lets a sandboxed run's connections out, and only
//! to where its policy lists (docs/plans/self-build.md, phase 12).
//!
//! A holder (a run, to Ren) is issued a pass: a [`Ticket`] it presents as
//! its proxy URL's password, and the [`RelayPolicy`] of [`Destination`]s
//! it may reach. It asks the [`Relay`] for a tunnel (`CONNECT host:port`);
//! the relay decides it ([`Verdict`]), reaches the target (a name only at
//! its public addresses, so a run never reaches this host or a private
//! network by a name), and passes the bytes through unread: TLS stays end
//! to end. Once it ends, the relay reports a [`Passage`]: the holder, the
//! target, and whether it was refused, unreachable, or closed with the
//! bytes each way.
//!
//! Reading a request's head ([`Head`]), deciding, and choosing addresses
//! are pure; [`Relay::serve`] is the shell, and spawns no task. The crate
//! depends on none of Ren's.

mod head;
mod passage;
mod policy;
mod relay;
mod ticket;

pub use head::{Answer, Ask, Head, HeadError, Reading};
pub use passage::{Course, Passage, TunnelEnd, Unreachable};
pub use policy::{NotPublic, PublicAddress, Reachable, Refusal, RelayPolicy, Verdict};
pub use relay::{IssueError, PassHandle, Relay};
pub use ren_destination::{
    Destination, Host, HostName, HostPattern, NotADestination, NotAHostName, NotATarget, Target,
};
pub use ticket::{InvalidTicket, Ticket};
