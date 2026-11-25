//! What kind of instance a gateway is ([`Environment`]), as `/health`
//! reports it beside its build ([`crate::release::Build`]): production, the
//! canary, or a preview. The gateway is the authority; the UI and the CLI
//! show what `/health` reports.

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
    /// The tip of `main` checked on a copy of production's history before
    /// production takes it (`ren deployer canary`).
    Canary,
    /// A pull request's head or a chosen commit, on a copy of production's
    /// history or an invented one, for judging its change (`ren deploy`).
    Preview,
}

/// The `[instance]` configuration section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceOptions {
    pub environment: Environment,
}
