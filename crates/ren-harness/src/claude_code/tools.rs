//! Claude Code's tools: their names, what each does to the workspace, the
//! context Claude Code adds to user messages, and its quota probe.

use crate::tools::{EffectKind, ToolEffect, UserContext};
use ren_protocol::activity::{RequestActivity, ToolUse};
use ren_protocol::label::Labeled;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Claude Code's built-in tools.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ClaudeCodeTool {
    #[strum(to_string = "Read")]
    Read,
    #[strum(to_string = "Write")]
    Write,
    #[strum(to_string = "Edit")]
    Edit,
    #[strum(to_string = "MultiEdit")]
    MultiEdit,
    #[strum(to_string = "NotebookEdit")]
    NotebookEdit,
    #[strum(to_string = "Bash")]
    Bash,
    #[strum(to_string = "Glob")]
    Glob,
    #[strum(to_string = "Grep")]
    Grep,
    #[strum(to_string = "Task")]
    Task,
    #[strum(to_string = "Agent")]
    Agent,
    #[strum(to_string = "WebFetch")]
    WebFetch,
    #[strum(to_string = "WebSearch")]
    WebSearch,
}

impl ClaudeCodeTool {
    /// What Claude Code adds to user messages: `<system-reminder>`s, and
    /// "Tool loaded." after the result of a deferred tool it loaded.
    pub(crate) const CONTEXT: UserContext = UserContext {
        wrappers: &[("<system-reminder>", "</system-reminder>")],
        notices: &["Tool loaded."],
    };
    /// What Claude Code's quota probe sends, alone.
    const QUOTA_PROBE: &str = "quota";

    pub(crate) fn effects(tool: &ToolUse) -> Vec<ToolEffect> {
        let effect = |kind, name| ToolEffect::of(kind, tool, name);
        match Self::from_label(&tool.name) {
            Some(Self::Read) => effect(EffectKind::ReadFile, "file_path"),
            Some(Self::Write | Self::Edit | Self::MultiEdit) => {
                effect(EffectKind::WriteFile, "file_path")
            }
            Some(Self::NotebookEdit) => effect(EffectKind::WriteFile, "notebook_path"),
            Some(Self::Bash) => effect(EffectKind::RunCommand, "command"),
            Some(Self::Glob | Self::Grep) => effect(EffectKind::Search, "pattern"),
            Some(Self::Task | Self::Agent) => effect(EffectKind::Delegate, "description"),
            Some(Self::WebFetch) => effect(EffectKind::Fetch, "url"),
            Some(Self::WebSearch) => effect(EffectKind::Search, "query"),
            None => Vec::new(),
        }
    }

    /// Whether `request` is Claude Code's check of the account's quota as it
    /// starts: one token asked for, with the probe's message alone.
    pub(crate) fn quota_probe(request: &RequestActivity) -> bool {
        request.max_output_tokens == Some(1)
            && request.tool_results.is_empty()
            && matches!(request.user_text.as_slice(), [text] if text == Self::QUOTA_PROBE)
    }
}
