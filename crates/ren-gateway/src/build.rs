//! What kind of instance a gateway is ([`Environment`]), as `/health`
//! reports it beside its build ([`ren_release::Build`]): a production
//! instance, a development one (such as `cargo xtask dev`'s slot), the
//! canary, or a worktree's preview. The gateway is the authority; the UI and the CLI show what
//! `/health` reports.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// The kind of instance a gateway is.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr,
)]
pub enum Environment {
    /// Real traffic. The default.
    #[default]
    Production,
    /// Fake or test traffic, for development.
    Development,
    /// The tip of `main` checked on a copy of production's history before
    /// production takes it (`cargo xtask canary`).
    Canary,
    /// One worktree's branch on a copy of production's history, for
    /// judging its change (`cargo xtask preview`).
    Preview,
}

/// The `[instance]` configuration section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceOptions {
    pub environment: Environment,
}
