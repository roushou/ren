//! What became of one request to the relay: a [`Passage`], reported once
//! it ends.

use crate::{destination::Destination, destination::Target, policy::Refusal};

/// One request a holder made of the relay, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Passage<H> {
    holder: H,
    course: Course,
}

/// What a request was, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Course {
    /// A tunnel to `target`.
    Tunnel { target: Target, end: TunnelEnd },
    /// A request that was not a tunnel, refused as such.
    NotATunnel { method: Box<str> },
}

/// How a tunnel ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelEnd {
    /// Refused before anything was connected.
    Refused(Refusal),
    /// Admitted `through` a destination, but its target could not be
    /// reached.
    Unreachable {
        through: Destination,
        why: Unreachable,
    },
    /// Open, then closed by either side, with the bytes each way.
    Closed {
        through: Destination,
        sent: u64,
        received: u64,
    },
    /// Open, then failed mid-way; how many bytes passed is not known.
    Broken { through: Destination },
}

/// Why an admitted tunnel's target could not be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Unreachable {
    /// Its name did not resolve to any address.
    Unresolved,
    /// No address it resolved to took the connection.
    Refused,
    /// Resolving or connecting took longer than the relay waits.
    TimedOut,
}

impl Unreachable {
    pub fn label(self) -> &'static str {
        self.into()
    }
}

impl<H> Passage<H> {
    pub(crate) fn new(holder: H, course: Course) -> Self {
        Self { holder, course }
    }

    pub fn holder(&self) -> &H {
        &self.holder
    }

    pub fn course(&self) -> &Course {
        &self.course
    }
}
