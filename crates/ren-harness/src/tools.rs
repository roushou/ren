//! What a harness's tool calls and messages mean: which tools read or write
//! files, run commands, or start subagents; which user text is the client's
//! own context rather than the user's prompt; and how a client reports a
//! command's exit status.
//!
//! Tool names and input fields follow each client's current tools. A tool
//! this does not know has no effect rather than a guessed one, and its call
//! is still reported with its name.

use crate::Harness;
use crate::codex::CodexTool;
use ren_protocol::activity::{
    ActivityLimitation, Reading, RequestActivity, ResultStatus, ToolInput, ToolResult, ToolUse,
};
use ren_protocol::label::Labeled;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffect {
    pub kind: EffectKind,
    pub target: String,
}

/// Why a client sent a call: to have the model work, or to check something
/// for itself. A client's own checks are bookkeeping, like a token count:
/// their answers never reach the user.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CallPurpose {
    /// The model's output is used.
    Work,
    /// Claude Code's check of the account's quota as it starts: one token
    /// asked for, and only the response's status and headers read.
    QuotaProbe,
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

impl ToolEffect {
    /// The effect of `kind` on what the field `name` of `tool`'s input names;
    /// none when the input has no such field.
    pub(crate) fn of(kind: EffectKind, tool: &ToolUse, name: &str) -> Vec<Self> {
        Self::field(tool, name)
            .and_then(Self::command_text)
            .map(|target| Self { kind, target })
            .into_iter()
            .collect()
    }

    /// The field `name` of `tool`'s JSON input.
    pub(crate) fn field<'a>(tool: &'a ToolUse, name: &str) -> Option<&'a Value> {
        match &tool.input {
            ToolInput::Json(input) => input.get(name),
            ToolInput::Text(_) => None,
        }
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
    pub(crate) fn patched(patch: &str) -> Vec<Self> {
        const MARKERS: [&str; 4] = [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ];
        patch
            .lines()
            .filter_map(|line| MARKERS.iter().find_map(|marker| line.strip_prefix(marker)))
            .map(|path| Self {
                kind: EffectKind::WriteFile,
                target: path.trim().to_owned(),
            })
            .collect()
    }
}

/// What a client adds to a user message on its own: wrappers around the
/// context it adds, and whole notices.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UserContext {
    pub(crate) wrappers: &'static [(&'static str, &'static str)],
    pub(crate) notices: &'static [&'static str],
}

impl UserContext {
    pub(crate) const NONE: Self = Self {
        wrappers: &[],
        notices: &[],
    };

    /// The user's own words in `user_text`: the client's additions removed.
    /// `None` when nothing else is left.
    fn prompt(self, user_text: &[String]) -> Option<String> {
        let own: Vec<&str> = user_text
            .iter()
            .map(|text| text.trim())
            .filter(|text| {
                !text.is_empty()
                    && !self.notices.contains(text)
                    && !self
                        .wrappers
                        .iter()
                        .any(|(open, close)| text.starts_with(open) && text.ends_with(close))
            })
            .collect();
        (!own.is_empty()).then(|| own.join("\n\n"))
    }
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
    /// `Killed by SIGNAL`, or `Timed out before completion`.
    pub(crate) fn failed(output: &str) -> bool {
        let Some(last) = output.lines().map(str::trim).rfind(|line| !line.is_empty()) else {
            return false;
        };
        if let Some(code) = last.strip_prefix("Exited with code ") {
            return code.parse::<i64>().is_ok_and(|code| code != 0);
        }
        last == "Timed out before completion"
            || last
                .strip_prefix("Killed by ")
                .is_some_and(|signal| !signal.is_empty())
    }
}

impl Harness {
    /// What `tool` does, from its name and input. Unknown tools, and inputs
    /// without the expected fields, have no effects.
    pub fn effects(self, tool: &ToolUse) -> Vec<ToolEffect> {
        match self {
            Self::ClaudeCode => ClaudeCodeTool::effects(tool),
            Self::Codex => CodexTool::effects(tool),
            Self::Pi => PiTool::effects(tool),
            Self::OpenCode => OpenCodeTool::effects(tool),
        }
    }

    /// Why the client sent a request.
    pub fn purpose(self, request: &RequestActivity) -> CallPurpose {
        match self {
            Self::ClaudeCode if ClaudeCodeTool::quota_probe(request) => CallPurpose::QuotaProbe,
            Self::ClaudeCode | Self::Codex | Self::Pi | Self::OpenCode => CallPurpose::Work,
        }
    }

    /// The user's own words in a request's newest user text: the client's
    /// added context removed. `None` when nothing else is left.
    pub fn prompt(self, user_text: &[String]) -> Option<String> {
        match self {
            Self::ClaudeCode => ClaudeCodeTool::CONTEXT,
            Self::Codex => CodexTool::CONTEXT,
            Self::Pi => UserContext::NONE,
            Self::OpenCode => OpenCodeTool::CONTEXT,
        }
        .prompt(user_text)
    }

    /// A tool result's status: the protocol's flag, or, for Codex commands,
    /// the exit code its output reports, or, for opencode commands, the
    /// failure its output ends with. opencode says nothing when a command
    /// exits 0, so its silence stays unreported.
    pub fn result_status(self, result: &ToolResult) -> ResultStatus {
        match (self, result.status) {
            (Self::OpenCode, ResultStatus::Unreported) => {
                if OpenCodeTool::failed(&result.output) {
                    ResultStatus::Failed
                } else {
                    ResultStatus::Unreported
                }
            }
            (Self::Codex, ResultStatus::Unreported) => {
                CodexTool::exit_code(&result.output).map_or(ResultStatus::Unreported, |code| {
                    if code == 0 {
                        ResultStatus::Succeeded
                    } else {
                        ResultStatus::Failed
                    }
                })
            }
            (Self::ClaudeCode | Self::Codex | Self::Pi | Self::OpenCode, status) => status,
        }
    }
}

/// A call's reading as its client uses it: the user's own words, what each
/// tool call does, and each result's status. Without a known client the text
/// is taken as it is and tool calls have no effects.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpretedCall {
    pub purpose: CallPurpose,
    /// All of the newest user input, client context included. It is sent to
    /// the provider too, so it is what secrets are looked for in.
    pub user_text: Vec<String>,
    /// The user's own words, when the request starts with them.
    pub prompt: Option<String>,
    pub tool_uses: Vec<InterpretedTool>,
    /// The results the request carried, each with its status resolved.
    pub tool_results: Vec<ToolResult>,
    pub reply: String,
    /// The request's limitations, then the response's, each once.
    pub limitations: Vec<ActivityLimitation>,
}

/// A tool call and what it does.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpretedTool {
    pub tool: ToolUse,
    pub effects: Vec<ToolEffect>,
}

impl InterpretedCall {
    pub fn new(harness: Option<Harness>, reading: Reading) -> Self {
        let Reading { request, response } = reading;
        let purpose = harness.map_or(CallPurpose::Work, |harness| harness.purpose(&request));
        let prompt = match (harness, purpose) {
            // A client's own check holds no words of the user's.
            (_, CallPurpose::QuotaProbe) => None,
            (Some(harness), CallPurpose::Work) => harness.prompt(&request.user_text),
            (None, CallPurpose::Work) => {
                let text = request.user_text.join("\n\n");
                (!text.trim().is_empty()).then_some(text)
            }
        };
        let tool_uses = response
            .tool_uses
            .into_iter()
            .map(|tool| InterpretedTool {
                effects: harness
                    .map(|harness| harness.effects(&tool))
                    .unwrap_or_default(),
                tool,
            })
            .collect();
        let tool_results = request
            .tool_results
            .into_iter()
            .map(|result| ToolResult {
                status: harness.map_or(result.status, |harness| harness.result_status(&result)),
                ..result
            })
            .collect();
        let mut limitations = request.limitations;
        for limitation in response.limitations {
            if !limitations.contains(&limitation) {
                limitations.push(limitation);
            }
        }
        Self {
            purpose,
            user_text: request.user_text,
            prompt,
            tool_uses,
            tool_results,
            reply: response.reply,
            limitations,
        }
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
        // Claude Code's own notice after a deferred tool loads is not the user's words.
        assert_eq!(Harness::ClaudeCode.prompt(&text(&["Tool loaded."])), None);
        assert_eq!(
            Harness::ClaudeCode.prompt(&text(&["Tool loaded.", "Now run the tests"])),
            Some("Now run the tests".to_owned())
        );
        // Another client's user may type it.
        assert_eq!(
            Harness::Pi.prompt(&text(&["Tool loaded."])),
            Some("Tool loaded.".to_owned())
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

    #[test]
    fn a_call_is_read_as_its_client_uses_it() {
        use ren_protocol::activity::{RequestActivity, ResponseActivity};
        let reading = Reading {
            request: RequestActivity {
                user_text: vec![
                    "<system-reminder>context</system-reminder>".to_owned(),
                    "Make the tests pass".to_owned(),
                ],
                tool_results: vec![ToolResult {
                    id: "a".to_owned(),
                    status: ResultStatus::Unreported,
                    output: "Exit code: 2\nfailed".to_owned(),
                }],
                max_output_tokens: Some(32_000),
                limitations: vec![ActivityLimitation::Truncated],
            },
            response: ResponseActivity {
                tool_uses: vec![tool(
                    "Bash",
                    "tool_use",
                    ToolInput::Json(json!({ "command": "cargo test" })),
                )],
                reply: "Running.".to_owned(),
                limitations: vec![
                    ActivityLimitation::Truncated,
                    ActivityLimitation::UnfinishedToolCall,
                ],
            },
        };
        let claude = InterpretedCall::new(Some(Harness::ClaudeCode), reading.clone());
        assert_eq!(claude.purpose, CallPurpose::Work);
        assert_eq!(claude.prompt.as_deref(), Some("Make the tests pass"));
        assert_eq!(claude.user_text.len(), 2);
        assert_eq!(claude.tool_uses[0].effects[0].kind, EffectKind::RunCommand);
        // Claude Code reports errors with a flag, never an exit code.
        assert_eq!(claude.tool_results[0].status, ResultStatus::Unreported);
        assert_eq!(
            claude.limitations,
            [
                ActivityLimitation::Truncated,
                ActivityLimitation::UnfinishedToolCall
            ]
        );
        assert_eq!(claude.reply, "Running.");

        let codex = InterpretedCall::new(Some(Harness::Codex), reading.clone());
        assert_eq!(codex.tool_results[0].status, ResultStatus::Failed);

        // Without a known client, nothing is filtered or guessed.
        let unknown = InterpretedCall::new(None, reading);
        assert_eq!(
            unknown.prompt.as_deref(),
            Some("<system-reminder>context</system-reminder>\n\nMake the tests pass")
        );
        assert!(unknown.tool_uses[0].effects.is_empty());
        assert_eq!(unknown.tool_results[0].status, ResultStatus::Unreported);
    }

    #[test]
    fn opencode_tools_prompts_and_failures() {
        let json = |value| ToolInput::Json(value);
        let patch = "*** Begin Patch\n*** Update File: src/app.py\n*** Move to: src/main.py\n@@\n*** Delete File: old.txt\n*** End Patch";
        let cases = [
            (
                "shell",
                json!({ "command": "cargo test", "workdir": "/x" }),
                vec![(EffectKind::RunCommand, "cargo test")],
            ),
            (
                "read",
                json!({ "path": "README.md" }),
                vec![(EffectKind::ReadFile, "README.md")],
            ),
            (
                "edit",
                json!({ "path": "src/a.rs", "oldString": "x", "newString": "y" }),
                vec![(EffectKind::WriteFile, "src/a.rs")],
            ),
            (
                "patch",
                json!({ "patchText": patch }),
                vec![
                    (EffectKind::WriteFile, "src/app.py"),
                    (EffectKind::WriteFile, "src/main.py"),
                    (EffectKind::WriteFile, "old.txt"),
                ],
            ),
            (
                "grep",
                json!({ "pattern": "Store", "include": "*.rs" }),
                vec![(EffectKind::Search, "Store")],
            ),
            (
                "subagent",
                json!({ "agent": "general", "description": "Find callers", "prompt": "..." }),
                vec![(EffectKind::Delegate, "Find callers")],
            ),
            (
                "webfetch",
                json!({ "url": "https://example.com", "format": "markdown" }),
                vec![(EffectKind::Fetch, "https://example.com")],
            ),
            // opencode 1.x's names, and Claude Code's, are not opencode 2.x's.
            ("bash", json!({ "command": "ls" }), vec![]),
            ("Read", json!({ "path": "a" }), vec![]),
            ("todowrite", json!({ "todos": [] }), vec![]),
        ];
        for (name, input, expected) in cases {
            let expected: Vec<(EffectKind, String)> = expected
                .into_iter()
                .map(|(kind, target)| (kind, target.to_owned()))
                .collect();
            assert_eq!(
                targets(Harness::OpenCode, &tool(name, "tool_use", json(input))),
                expected,
                "{name}"
            );
        }

        let text = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            Harness::OpenCode.prompt(&text(&[
                "<shell id=\"sh_1\" state=\"completed\" command=\"make\">\nok\n</shell>",
                "<subagent sessionID=\"ses_1\" state=\"completed\">\ndone\n</subagent>",
                "The following shell command was executed by the user:\n<shell id=\"sh_2\" state=\"completed\" command=\"ls\">\na\n</shell>",
                "Now ship it",
            ])),
            Some("Now ship it".to_owned())
        );

        let result = |output: &str, status| ToolResult {
            id: "c".to_owned(),
            status,
            output: output.to_owned(),
        };
        for (output, expected) in [
            ("error: x\n\nExited with code 101", ResultStatus::Failed),
            ("partial\nKilled by SIGTERM\n", ResultStatus::Failed),
            ("Timed out before completion", ResultStatus::Failed),
            // Success is silent, so it stays unreported.
            ("test result: ok", ResultStatus::Unreported),
            ("Exited with code 0", ResultStatus::Unreported),
            (
                "Exited with code 1\nthen more output",
                ResultStatus::Unreported,
            ),
            ("", ResultStatus::Unreported),
        ] {
            assert_eq!(
                Harness::OpenCode.result_status(&result(output, ResultStatus::Unreported)),
                expected,
                "{output}"
            );
        }
        assert_eq!(
            Harness::OpenCode.result_status(&result("Exited with code 1", ResultStatus::Succeeded)),
            ResultStatus::Succeeded
        );
        assert_eq!(
            Harness::Codex.result_status(&result("Exited with code 1", ResultStatus::Unreported)),
            ResultStatus::Unreported
        );
    }

    #[test]
    fn claude_codes_quota_probe_is_bookkeeping() {
        use ren_protocol::activity::RequestActivity;
        let probe = RequestActivity {
            user_text: vec!["quota".to_owned()],
            max_output_tokens: Some(1),
            ..RequestActivity::default()
        };
        assert_eq!(Harness::ClaudeCode.purpose(&probe), CallPurpose::QuotaProbe);
        let read = InterpretedCall::new(
            Some(Harness::ClaudeCode),
            Reading {
                request: probe.clone(),
                response: Default::default(),
            },
        );
        assert_eq!(read.purpose, CallPurpose::QuotaProbe);
        // Its message is the client's, not the user's.
        assert_eq!(read.prompt, None);
        assert_eq!(read.user_text, ["quota"]);
        // A user who asks about their quota is working, whatever the
        // output limit; so is any other client, or an unknown one.
        let asked = RequestActivity {
            max_output_tokens: Some(32_000),
            ..probe.clone()
        };
        assert_eq!(Harness::ClaudeCode.purpose(&asked), CallPurpose::Work);
        let longer = RequestActivity {
            user_text: vec!["quota left?".to_owned()],
            ..probe.clone()
        };
        assert_eq!(Harness::ClaudeCode.purpose(&longer), CallPurpose::Work);
        for harness in [Harness::Codex, Harness::Pi, Harness::OpenCode] {
            assert_eq!(harness.purpose(&probe), CallPurpose::Work);
        }
        let unknown = InterpretedCall::new(
            None,
            Reading {
                request: probe,
                response: Default::default(),
            },
        );
        assert_eq!(unknown.purpose, CallPurpose::Work);
    }
}
