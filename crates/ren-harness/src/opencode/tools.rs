//! opencode's tools: their names, what each does to the workspace, the
//! notices opencode adds to user messages, and how it reports a command
//! that failed.

use crate::tools::{EffectKind, ToolEffect, UserContext};
use ren_protocol::activity::ToolUse;
use ren_protocol::label::Labeled;
use serde_json::Value;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// opencode's built-in tools (opencode 2.x).
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum OpenCodeTool {
    Shell,
    Read,
    Write,
    Edit,
    Patch,
    Glob,
    Grep,
    Subagent,
    Webfetch,
    Websearch,
}

impl OpenCodeTool {
    /// opencode's notices of a finished background command or subagent, and
    /// of a command the user ran themself.
    pub(crate) const CONTEXT: UserContext = UserContext {
        wrappers: &[
            ("<shell ", "</shell>"),
            ("<subagent ", "</subagent>"),
            (
                "The following shell command was executed by the user:",
                "</shell>",
            ),
        ],
        notices: &[],
    };

    pub(crate) fn effects(tool: &ToolUse) -> Vec<ToolEffect> {
        let effect = |kind, name| ToolEffect::of(kind, tool, name);
        match Self::from_label(&tool.name) {
            Some(Self::Shell) => effect(EffectKind::RunCommand, "command"),
            Some(Self::Read) => effect(EffectKind::ReadFile, "path"),
            Some(Self::Write | Self::Edit) => effect(EffectKind::WriteFile, "path"),
            Some(Self::Patch) => ToolEffect::field(tool, "patchText")
                .and_then(Value::as_str)
                .map(ToolEffect::patched)
                .unwrap_or_default(),
            Some(Self::Glob | Self::Grep) => effect(EffectKind::Search, "pattern"),
            Some(Self::Subagent) => effect(EffectKind::Delegate, "description"),
            Some(Self::Webfetch) => effect(EffectKind::Fetch, "url"),
            Some(Self::Websearch) => effect(EffectKind::Search, "query"),
            None => Vec::new(),
        }
    }

    /// opencode ends a command's output with `Exited with code N` (never 0),
    /// `Killed by SIGNAL`, or `Timed out before completion`; from 2.2.7, with
    /// `Exit code: N` whatever N is.
    pub(crate) fn failed(output: &str) -> bool {
        let Some(last) = output.lines().map(str::trim).rfind(|line| !line.is_empty()) else {
            return false;
        };
        if let Some(code) = last
            .strip_prefix("Exited with code ")
            .or_else(|| last.strip_prefix("Exit code: "))
        {
            return code.parse::<i64>().is_ok_and(|code| code != 0);
        }
        last == "Timed out before completion"
            || last
                .strip_prefix("Killed by ")
                .is_some_and(|signal| !signal.is_empty())
    }
}
