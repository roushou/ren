//! What a gateway says about itself: whether it is a production instance or
//! a development one (such as `cargo xtask dev`'s slot). The gateway is the
//! authority; the UI and the CLI show what `/health` reports.

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
}

/// The `[instance]` configuration section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceOptions {
    pub environment: Environment,
}
