//! The clients a run can be started with, as this host has them: ready,
//! not installed, or not one Ren can drive yet.

use ren_harness::{
    Harness,
    settings::{ApprovalPolicy, Midway},
};
use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A client, whether a run can start with it here, and the approval
/// policies it can run under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Client {
    pub harness: Harness,
    pub status: ClientStatus,
    pub approvals: Vec<ApprovalPolicy>,
    /// What a running conversation takes mid-run: steering, and which
    /// settings change from the next turn.
    pub midway: Vec<Midway>,
}

/// Whether a run can start with a client on this host.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, SerializeDisplay, DeserializeFromStr)]
pub enum ClientStatus {
    /// Ren drives it, and its program is on this host's path.
    Ready,
    /// Ren drives it, but its program is not on this host's path.
    NotInstalled,
    /// Ren cannot drive it yet.
    NotDrivable,
}
