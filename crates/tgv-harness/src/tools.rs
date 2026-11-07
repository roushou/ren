//! What a harness's tool calls and messages mean: which tools read or write
//! files, run commands, or start subagents; which user text is the client's
//! own context rather than the user's prompt; and how a client reports a
//! command's exit status.
//!
//! Tool names and input fields follow each client's current tools. A tool
//! this does not know has no effect rather than a guessed one, and its call
//! is still reported with its name.

use crate::Harness;
use serde::Serialize;
use serde_json::Value;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_protocol::activity::{ResultStatus, ToolInput, ToolResult, ToolUse};
use tgv_protocol::label::Labeled;

/// What a tool call does to the workspace, as its input states it.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum EffectKind {
    ReadFile,
    WriteFile,
    RunCommand,
    Search,
    /// Starts a subagent.
    Delegate,
    Fetch,
}

/// One effect with its target: a path, command, pattern, task, or URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolEffect {
    pub kind: EffectKind,
    pub target: String,
}

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

impl Harness {
    /// Wrappers a client puts around context it adds to user messages.
    const CLAUDE_CODE_CONTEXT: &[(&str, &str)] = &[("<system-reminder>", "</system-reminder>")];
    const CODEX_CONTEXT: &[(&str, &str)] = &[
        ("<environment_context>", "</environment_context>"),
        ("<user_instructions>", "</user_instructions>"),
        ("# AGENTS.md instructions", ""),
    ];

    /// What `tool` does, from its name and input. Unknown tools, and inputs
    /// without the expected fields, have no effects.
    pub fn effects(self, tool: &ToolUse) -> Vec<ToolEffect> {
        let field = |name: &str| match &tool.input {
            ToolInput::Json(input) => input.get(name),
            ToolInput::Text(_) => None,
        };
        let effect = |kind: EffectKind, name: &str| {
            field(name)
                .and_then(Self::command_text)
                .map(|target| ToolEffect { kind, target })
                .into_iter()
                .collect()
        };
        match self {
            Self::ClaudeCode => match ClaudeCodeTool::from_label(&tool.name) {
                Some(ClaudeCodeTool::Read) => effect(EffectKind::ReadFile, "file_path"),
                Some(ClaudeCodeTool::Write | ClaudeCodeTool::Edit | ClaudeCodeTool::MultiEdit) => {
                    effect(EffectKind::WriteFile, "file_path")
                }
                Some(ClaudeCodeTool::NotebookEdit) => {
                    effect(EffectKind::WriteFile, "notebook_path")
                }
                Some(ClaudeCodeTool::Bash) => effect(EffectKind::RunCommand, "command"),
                Some(ClaudeCodeTool::Glob | ClaudeCodeTool::Grep) => {
                    effect(EffectKind::Search, "pattern")
                }
                Some(ClaudeCodeTool::Task | ClaudeCodeTool::Agent) => {
                    effect(EffectKind::Delegate, "description")
                }
                Some(ClaudeCodeTool::WebFetch) => effect(EffectKind::Fetch, "url"),
                Some(ClaudeCodeTool::WebSearch) => effect(EffectKind::Search, "query"),
                None => Vec::new(),
            },
            Self::Codex => {
                let tool_kind =
                    CodexTool::from_label(&tool.name).or_else(|| CodexTool::from_label(&tool.kind));
                match tool_kind {
                    Some(
                        CodexTool::Shell | CodexTool::ContainerExec | CodexTool::LocalShellCall,
                    ) => effect(EffectKind::RunCommand, "command"),
                    Some(CodexTool::ExecCommand) => effect(EffectKind::RunCommand, "cmd"),
                    Some(CodexTool::ShellCall) => effect(EffectKind::RunCommand, "commands"),
                    Some(CodexTool::ApplyPatch) => match &tool.input {
                        ToolInput::Text(patch) => Self::patch_files(patch),
                        ToolInput::Json(input) => input
                            .get("input")
                            .and_then(Value::as_str)
                            .map(Self::patch_files)
                            .unwrap_or_default(),
                    },
                    Some(CodexTool::ApplyPatchCall) => effect(EffectKind::WriteFile, "path"),
                    Some(CodexTool::ViewImage | CodexTool::ReadFile) => {
                        effect(EffectKind::ReadFile, "path")
                    }
                    Some(CodexTool::WebSearchCall) => effect(EffectKind::Search, "query"),
                    None => Vec::new(),
                }
            }
            Self::Pi => match PiTool::from_label(&tool.name) {
                Some(PiTool::Read) => effect(EffectKind::ReadFile, "path"),
                Some(PiTool::Write | PiTool::Edit) => effect(EffectKind::WriteFile, "path"),
                Some(PiTool::Bash) => effect(EffectKind::RunCommand, "command"),
                Some(PiTool::Grep | PiTool::Find) => effect(EffectKind::Search, "pattern"),
                Some(PiTool::Ls) => effect(EffectKind::ReadFile, "path"),
                None => Vec::new(),
            },
        }
    }

    /// The user's own words in a request's newest user text: the client's
    /// added context removed. `None` when nothing else is left.
    pub fn prompt(self, user_text: &[String]) -> Option<String> {
        let context = match self {
            Self::ClaudeCode => Self::CLAUDE_CODE_CONTEXT,
            Self::Codex => Self::CODEX_CONTEXT,
            Self::Pi => &[],
        };
        let own: Vec<&str> = user_text
            .iter()
            .map(|text| text.trim())
            .filter(|text| {
                !text.is_empty()
                    && !context
                        .iter()
                        .any(|(open, close)| text.starts_with(open) && text.ends_with(close))
            })
            .collect();
        (!own.is_empty()).then(|| own.join("\n\n"))
    }

    /// A tool result's status: the protocol's flag, or, for Codex commands,
    /// the exit code its output reports.
    pub fn result_status(self, result: &ToolResult) -> ResultStatus {
        match (self, result.status) {
            (Self::Codex, ResultStatus::Unreported) => {
                Self::exit_code(&result.output).map_or(ResultStatus::Unreported, |code| {
                    if code == 0 {
                        ResultStatus::Succeeded
                    } else {
                        ResultStatus::Failed
                    }
                })
            }
            (Self::ClaudeCode | Self::Codex | Self::Pi, status) => status,
        }
    }

    /// Codex reports a command's exit code as a first line `Exit code: N`,
    /// or as JSON `{"metadata": {"exit_code": N}}`.
    fn exit_code(output: &str) -> Option<i64> {
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

    /// A command as text: a string, or an argument list. A shell invocation
    /// (`bash -lc "..."`) shows the script it runs.
    fn command_text(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Array(parts) => {
                let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                match parts.as_slice() {
                    [] => None,
                    [shell, flag, script] if shell.ends_with("sh") && flag.starts_with('-') => {
                        Some((*script).to_owned())
                    }
                    parts => Some(parts.join(" ")),
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
        }
    }

    /// The files an `apply_patch` envelope adds, updates, moves, or deletes.
    fn patch_files(patch: &str) -> Vec<ToolEffect> {
        const MARKERS: [&str; 4] = [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ];
        patch
            .lines()
            .filter_map(|line| MARKERS.iter().find_map(|marker| line.strip_prefix(marker)))
            .map(|path| ToolEffect {
                kind: EffectKind::WriteFile,
                target: path.trim().to_owned(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, kind: &str, input: ToolInput) -> ToolUse {
        ToolUse {
            id: None,
            kind: kind.to_owned(),
            name: name.to_owned(),
            input,
        }
    }

    fn targets(harness: Harness, tool: &ToolUse) -> Vec<(EffectKind, String)> {
        harness
            .effects(tool)
            .into_iter()
            .map(|effect| (effect.kind, effect.target))
            .collect()
    }

    #[test]
    fn tools_map_to_effects_per_harness() {
        let json = |value| ToolInput::Json(value);
        assert_eq!(
            targets(
                Harness::ClaudeCode,
                &tool(
                    "Edit",
                    "tool_use",
                    json(json!({ "file_path": "src/a.rs", "old_string": "x" }))
                )
            ),
            [(EffectKind::WriteFile, "src/a.rs".to_owned())]
        );
        assert_eq!(
            targets(
                Harness::ClaudeCode,
                &tool("Bash", "tool_use", json(json!({ "command": "cargo test" })))
            ),
            [(EffectKind::RunCommand, "cargo test".to_owned())]
        );
        assert_eq!(
            targets(
                Harness::Codex,
                &tool(
                    "shell",
                    "function_call",
                    json(json!({ "command": ["bash", "-lc", "cargo test -p x"] }))
                )
            ),
            [(EffectKind::RunCommand, "cargo test -p x".to_owned())]
        );
        assert_eq!(
            targets(
                Harness::Codex,
                &tool(
                    "shell",
                    "function_call",
                    json(json!({ "command": ["rg", "Store"] }))
                )
            ),
            [(EffectKind::RunCommand, "rg Store".to_owned())]
        );
        let patch = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n*** Add File: tests/new.rs\n+x\n*** End Patch";
        assert_eq!(
            targets(
                Harness::Codex,
                &tool(
                    "apply_patch",
                    "custom_tool_call",
                    ToolInput::Text(patch.to_owned())
                )
            ),
            [
                (EffectKind::WriteFile, "src/lib.rs".to_owned()),
                (EffectKind::WriteFile, "tests/new.rs".to_owned())
            ]
        );
        assert_eq!(
            targets(
                Harness::Pi,
                &tool("read", "tool_use", json(json!({ "path": "README.md" })))
            ),
            [(EffectKind::ReadFile, "README.md".to_owned())]
        );
        // Unknown tools and missing fields have no effects.
        assert!(
            targets(
                Harness::ClaudeCode,
                &tool("mcp__x__y", "tool_use", json(json!({ "file_path": "a" })))
            )
            .is_empty()
        );
        assert!(
            targets(
                Harness::ClaudeCode,
                &tool("Read", "tool_use", json(json!({})))
            )
            .is_empty()
        );
        assert!(
            targets(
                Harness::Pi,
                &tool("Read", "tool_use", json(json!({ "path": "a" })))
            )
            .is_empty()
        );
    }

    #[test]
    fn client_context_is_not_the_users_prompt() {
        let text = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            Harness::ClaudeCode.prompt(&text(&[
                "<system-reminder>\nnotes\n</system-reminder>",
                "Fix the test"
            ])),
            Some("Fix the test".to_owned())
        );
        assert_eq!(
            Harness::ClaudeCode.prompt(&text(&["<system-reminder>only</system-reminder>"])),
            None
        );
        assert_eq!(
            Harness::Codex.prompt(&text(&[
                "<environment_context>\n<cwd>/x</cwd>\n</environment_context>",
                "# AGENTS.md instructions for /x\n...",
                "Add a test"
            ])),
            Some("Add a test".to_owned())
        );
        assert_eq!(
            Harness::Pi.prompt(&text(&["<system-reminder>kept</system-reminder>"])),
            Some("<system-reminder>kept</system-reminder>".to_owned())
        );
    }

    #[test]
    fn codex_exit_codes_decide_unreported_statuses() {
        let result = |output: &str, status| ToolResult {
            id: "c".to_owned(),
            status,
            output: output.to_owned(),
        };
        assert_eq!(
            Harness::Codex.result_status(&result(
                "Exit code: 0\nWall time: 1s\nOutput:\nok",
                ResultStatus::Unreported
            )),
            ResultStatus::Succeeded
        );
        assert_eq!(
            Harness::Codex.result_status(&result(
                r#"{"output":"boom","metadata":{"exit_code":101}}"#,
                ResultStatus::Unreported
            )),
            ResultStatus::Failed
        );
        assert_eq!(
            Harness::Codex.result_status(&result("Done!", ResultStatus::Unreported)),
            ResultStatus::Unreported
        );
        assert_eq!(
            Harness::ClaudeCode.result_status(&result("Exit code: 1", ResultStatus::Succeeded)),
            ResultStatus::Succeeded
        );
    }
}
