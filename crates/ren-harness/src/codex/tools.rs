//! Codex's tools: their names, what each does to the workspace, the context
//! Codex adds to user messages, and how it reports a command's exit code.

use crate::tools::{EffectKind, ToolEffect, UserContext};
use ren_protocol::activity::{ToolInput, ToolUse};
use ren_protocol::label::Labeled;
use serde_json::Value;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Codex's tools, by function name or by the item type that carries them.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CodexTool {
    Shell,
    #[strum(to_string = "container.exec")]
    ContainerExec,
    ExecCommand,
    LocalShellCall,
    ShellCall,
    ApplyPatch,
    ApplyPatchCall,
    ViewImage,
    ReadFile,
    WebSearchCall,
}

impl CodexTool {
    /// What Codex adds to user messages: its environment, the user's
    /// instructions, and AGENTS.md.
    pub(crate) const CONTEXT: UserContext = UserContext {
        wrappers: &[
            ("<environment_context>", "</environment_context>"),
            ("<user_instructions>", "</user_instructions>"),
            ("# AGENTS.md instructions", ""),
        ],
        notices: &[],
    };

    pub(crate) fn effects(tool: &ToolUse) -> Vec<ToolEffect> {
        let effect = |kind, name| ToolEffect::of(kind, tool, name);
        match Self::from_label(&tool.name).or_else(|| Self::from_label(&tool.kind)) {
            Some(Self::Shell | Self::ContainerExec | Self::LocalShellCall) => {
                effect(EffectKind::RunCommand, "command")
            }
            Some(Self::ExecCommand) => effect(EffectKind::RunCommand, "cmd"),
            Some(Self::ShellCall) => effect(EffectKind::RunCommand, "commands"),
            Some(Self::ApplyPatch) => match &tool.input {
                ToolInput::Text(patch) => ToolEffect::patched(patch),
                ToolInput::Json(input) => input
                    .get("input")
                    .and_then(Value::as_str)
                    .map(ToolEffect::patched)
                    .unwrap_or_default(),
            },
            Some(Self::ApplyPatchCall) => effect(EffectKind::WriteFile, "path"),
            Some(Self::ViewImage | Self::ReadFile) => effect(EffectKind::ReadFile, "path"),
            Some(Self::WebSearchCall) => effect(EffectKind::Search, "query"),
            None => Vec::new(),
        }
    }

    /// Codex reports a command's exit code as a first line `Exit code: N`,
    /// or as JSON `{"metadata": {"exit_code": N}}`.
    pub(crate) fn exit_code(output: &str) -> Option<i64> {
        if let Some(code) = output
            .lines()
            .next()
            .and_then(|line| line.trim().strip_prefix("Exit code:"))
        {
            return code.trim().parse().ok();
        }
        serde_json::from_str::<Value>(output)
            .ok()?
            .get("metadata")?
            .get("exit_code")?
            .as_i64()
    }
}
