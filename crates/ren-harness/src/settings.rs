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
    /// offers none.
    pub fn offered(harness: Harness) -> &'static [Self] {
        match harness {
            Harness::Codex | Harness::ClaudeCode => Self::VARIANTS,
            Harness::Pi | Harness::OpenCode => &[],
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
