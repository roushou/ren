//! How a run is set up beyond its folder and client: the model, how hard it
//! reasons, and what it may do without asking. Each is optional: none leaves
//! the choice to the client's own configuration.

use crate::Harness;
use crate::models::ModelId;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How much a model reasons before it answers, from none to the most. Each
/// model offers some of these; Pi names them thinking levels.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub enum Effort {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// What a driven client may do without asking the operator.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ApprovalPolicy {
    /// Edits and commands wait for the operator.
    Ask,
    /// Edits in the folder go ahead; commands wait.
    Edits,
    /// The client decides what is safe and asks only beyond that: Claude
    /// Code's auto mode, Codex inside its workspace sandbox.
    Auto,
}

impl ApprovalPolicy {
    /// The policies `harness` can be run under. Pi asks for nothing, so it
    /// offers none; OpenCode allows everything unless told to ask, so it
    /// offers asking first and letting edits go ahead, as permission rules.
    pub fn offered(harness: Harness) -> &'static [Self] {
        match harness {
            Harness::Codex | Harness::ClaudeCode => Self::VARIANTS,
            Harness::OpenCode => &[Self::Ask, Self::Edits],
            Harness::Pi => &[],
        }
    }
}

/// What a running conversation can take mid-run, by client.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Midway {
    /// A message added to the turn that is running.
    Steer,
    /// Another model from the next turn.
    Model,
    /// Another effort from the next turn.
    Effort,
    /// Another approval policy from the next turn.
    Approvals,
}

impl Midway {
    /// What `harness` takes mid-run: Codex sets each turn's model, effort,
    /// and approvals and takes steering; Claude Code switches model and
    /// permission mode; Pi and OpenCode switch model and thinking level and
    /// take steering.
    pub fn offered(harness: Harness) -> &'static [Self] {
        match harness {
            Harness::Codex => Self::VARIANTS,
            Harness::ClaudeCode => &[Self::Model, Self::Approvals],
            Harness::Pi => &[Self::Steer, Self::Model, Self::Effort],
            Harness::OpenCode => &[Self::Steer, Self::Model, Self::Effort],
        }
    }
}

/// A run's settings; each one left out is the client's own.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunSettings {
    pub model: Option<ModelId>,
    pub effort: Option<Effort>,
    pub approvals: Option<ApprovalPolicy>,
}
