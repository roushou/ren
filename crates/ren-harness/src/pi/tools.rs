//! Pi's tools: their names and what each does to the workspace.

use crate::tools::{EffectKind, ToolEffect};
use ren_protocol::activity::ToolUse;
use ren_protocol::label::Labeled;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Pi's built-in tools.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PiTool {
    Read,
    Write,
    Edit,
    Bash,
    Grep,
    Find,
    Ls,
}

impl PiTool {
    pub(crate) fn effects(tool: &ToolUse) -> Vec<ToolEffect> {
        let effect = |kind, name| ToolEffect::of(kind, tool, name);
        match Self::from_label(&tool.name) {
            Some(Self::Read) => effect(EffectKind::ReadFile, "path"),
            Some(Self::Write | Self::Edit) => effect(EffectKind::WriteFile, "path"),
            Some(Self::Bash) => effect(EffectKind::RunCommand, "command"),
            Some(Self::Grep | Self::Find) => effect(EffectKind::Search, "pattern"),
            Some(Self::Ls) => effect(EffectKind::ReadFile, "path"),
            None => Vec::new(),
        }
    }
}
