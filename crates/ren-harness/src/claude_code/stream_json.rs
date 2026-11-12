//! Claude Code's print mode with streaming JSON on both sides (`claude -p
//! --input-format stream-json --output-format stream-json`): one JSON
//! message per line, each with a `type`. Read against the output of Claude
//! Code 2.1.321 and the control protocol its Agent SDK speaks.
//!
//! Ren names the conversation itself (`--session-id`, or `--resume` for an
//! existing one), so the conversation is open once Claude Code answers the
//! `initialize` control request. A prompt is a `user` message; Claude Code
//! echoes it back (`--replay-user-messages`) with its own id, which names the
//! turn, and ends the turn with a `result`. Permission prompts arrive as
//! `can_use_tool` control requests (`--permission-prompt-tool stdio`) and are
//! answered with the operator's decision. Every other control request is
//! refused: Ren provides no hooks, tools, or MCP servers.
//!
//! Claude Code reports no turn identifier that matches the prompt id its
//! model requests carry, so a driven turn's id is its prompt's message id.

use crate::{
    drive::{
        ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, DriveItem,
        ItemId, ItemKind, ItemPhase, ItemRead, ItemStatus, Opening, Prompt, RawLine, Received,
        RequestKind, Retry, TurnEnd, Unreadable,
    },
    lines::{Framed, Line},
    models::{DefaultChoice, Listing, Model, ModelId, ModelListing},
    settings::{ApprovalPolicy, RunSettings},
};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The client's side of one `claude -p` stream-json process.
#[derive(Debug)]
pub struct ClaudeStreamJson {
    thread: NativeThreadId,
    ready: bool,
    next_request: u64,
    /// Control requests Ren sent and waits on, by request id.
    requests: BTreeMap<String, RequestKind>,
    turn: TurnSlot,
    /// Permission prompts waiting for an answer.
    approvals: BTreeMap<ApprovalId, Asked>,
    /// Tool calls started in this turn, by tool use id, until their result.
    tools: BTreeMap<String, ToolCall>,
    /// Text and thinking blocks counted per message, so a streamed block and
    /// its whole copy get the same item id.
    blocks: BTreeMap<String, BlockCount>,
    streaming: Option<Streaming>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnSlot {
    Idle,
    /// A prompt was sent; its echo has not arrived.
    Requested,
    Running {
        turn: NativeTurnId,
        /// Ren asked for the turn to stop.
        interrupted: bool,
    },
}

/// What a permission prompt asked, kept to answer it.
#[derive(Debug)]
struct Asked {
    input: Value,
    suggestions: Vec<Value>,
}

#[derive(Debug)]
struct ToolCall {
    kind: ItemKind,
    read: ItemRead,
    block: Value,
}

#[derive(Debug, Default)]
struct BlockCount {
    texts: usize,
    thinking: usize,
}

/// The message whose blocks are streaming, and the item each text block
/// index belongs to.
#[derive(Debug)]
struct Streaming {
    message: String,
    texts: usize,
    items: BTreeMap<u64, ItemId>,
}

/// What Ren tells Claude Code when the operator refuses a tool call.
const DECLINED: &str = "The operator declined this action.";

impl ClaudeStreamJson {
    /// The arguments every driven Claude Code process starts with.
    const MODE: [&'static str; 10] = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--replay-user-messages",
        "--permission-prompt-tool",
        "stdio",
    ];

    /// The arguments that start Claude Code in this mode, after `claude`.
    pub fn arguments(opening: &Opening) -> Vec<String> {
        let mut arguments: Vec<String> = Self::MODE.map(str::to_owned).into();
        let settings = match opening {
            Opening::New { id, settings, .. } => {
                arguments.extend(["--session-id".into(), id.to_string()]);
                settings
            }
            Opening::Resume {
                thread, settings, ..
            } => {
                arguments.extend(["--resume".into(), thread.to_string()]);
                settings
            }
        };
        arguments.extend(Self::settings(settings));
        arguments
    }

    /// A run's settings as Claude Code's flags; each left out is its own.
    fn settings(settings: &RunSettings) -> Vec<String> {
        let mut arguments = Vec::new();
        if let Some(model) = &settings.model {
            arguments.extend(["--model".into(), model.to_string()]);
        }
        if let Some(effort) = settings.effort {
            arguments.extend(["--effort".into(), effort.to_string()]);
        }
        if let Some(policy) = settings.approvals {
            let mode = match policy {
                ApprovalPolicy::Ask => "default",
                ApprovalPolicy::Edits => "acceptEdits",
                ApprovalPolicy::Auto => "auto",
            };
            arguments.extend(["--permission-mode".into(), mode.into()]);
        }
        arguments
    }

    /// A codec for a freshly started process, and the first line to send it.
    pub fn open(opening: &Opening) -> (Self, Line) {
        let thread = match opening {
            Opening::New { id, .. } => *id,
            Opening::Resume { thread, .. } => *thread,
        };
        let mut codec = Self {
            thread,
            ready: false,
            next_request: 0,
            requests: BTreeMap::new(),
            turn: TurnSlot::Idle,
            approvals: BTreeMap::new(),
            tools: BTreeMap::new(),
            blocks: BTreeMap::new(),
            streaming: None,
        };
        let line = codec.request(RequestKind::Initialize, json!({ "subtype": "initialize" }));
        (codec, line)
    }

    /// The conversation, once Claude Code accepted the opening.
    pub fn thread(&self) -> Option<NativeThreadId> {
        self.ready.then_some(self.thread)
    }

    /// The line that carries `command`, or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Line, CommandRefused> {
        if !self.ready {
            return Err(CommandRefused::NotReady);
        }
        match command {
            DriveCommand::Prompt(prompt) => self.prompt(&prompt),
            DriveCommand::Interrupt => match self.turn {
                TurnSlot::Running { turn, .. } => {
                    self.turn = TurnSlot::Running {
                        turn,
                        interrupted: true,
                    };
                    Ok(self.request(
                        RequestKind::TurnInterrupt,
                        json!({ "subtype": "interrupt" }),
                    ))
                }
                TurnSlot::Requested => Err(CommandRefused::TurnNotStarted),
                TurnSlot::Idle => Err(CommandRefused::NoTurn),
            },
            DriveCommand::Answer { approval, decision } => {
                let Some(asked) = self.approvals.remove(&approval) else {
                    return Err(CommandRefused::UnknownApproval(approval));
                };
                let answer = match decision {
                    Decision::Accept => json!({ "behavior": "allow", "updatedInput": asked.input }),
                    Decision::AcceptForSession => json!({
                        "behavior": "allow",
                        "updatedInput": asked.input,
                        "updatedPermissions": Self::for_session(asked.suggestions),
                    }),
                    Decision::Decline => json!({ "behavior": "deny", "message": DECLINED }),
                    Decision::Cancel => {
                        if let TurnSlot::Running { turn, .. } = self.turn {
                            self.turn = TurnSlot::Running {
                                turn,
                                interrupted: true,
                            };
                        }
                        json!({ "behavior": "deny", "message": DECLINED, "interrupt": true })
                    }
                };
                Ok(Line::json(&json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": approval.to_string(),
                        "response": answer,
                    },
                })))
            }
        }
    }

    /// Read one frame from Claude Code's stdout.
    pub fn receive(&mut self, frame: Framed) -> Received {
        let mut received = Received::default();
        let bytes = match frame {
            Framed::Line(bytes) => bytes,
            Framed::TooLong { bytes } => {
                received.events.push(DriveEvent::Unreadable {
                    line: RawLine::new(Vec::new()),
                    reason: Unreadable::TooLong { bytes },
                });
                return received;
            }
        };
        if bytes.is_empty() {
            return received;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
            received.events.push(DriveEvent::Unreadable {
                line: RawLine::new(bytes),
                reason: Unreadable::NotJson,
            });
            return received;
        };
        let line = RawLine::new(bytes);
        let Some(kind) = message.get("type").and_then(Value::as_str) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::Untyped,
            });
            return received;
        };
        // Subagents' own messages carry the tool call that started them; the
        // call itself is the item Ren shows.
        let main = message.get("parent_tool_use_id").is_none_or(Value::is_null);
        match kind {
            "control_response" => self.control_response(&message, line, &mut received),
            "control_request" => self.control_request(&message, line, &mut received),
            "control_cancel_request" => {
                if let Some(id) = message.get("request_id").and_then(Value::as_str) {
                    let approval = ApprovalId(id.to_owned());
                    if self.approvals.remove(&approval).is_some() {
                        received
                            .events
                            .push(DriveEvent::ApprovalResolved { approval });
                    }
                }
            }
            "user" if main => self.user(&message, line, &mut received),
            "assistant" if main => self.assistant(&message, &mut received),
            "stream_event" if main => self.stream_event(&message, &mut received),
            "result" => self.result(&message, line, &mut received),
            "system" => self.system(&message, line, &mut received),
            // Known, and nothing Ren shows: subagents' messages, and the
            // allowance status Ren reads from the recorded calls instead.
            "user" | "assistant" | "stream_event" | "rate_limit_event" => {}
            _ => received.events.push(DriveEvent::Unknown { line }),
        }
        received
    }

    fn prompt(&mut self, prompt: &Prompt) -> Result<Line, CommandRefused> {
        match self.turn {
            TurnSlot::Idle => {}
            TurnSlot::Requested | TurnSlot::Running { .. } => {
                return Err(CommandRefused::TurnRunning);
            }
        }
        self.turn = TurnSlot::Requested;
        Ok(Line::json(&json!({
            "type": "user",
            "message": { "role": "user", "content": prompt.as_str() },
            "parent_tool_use_id": null,
            "session_id": self.thread.to_string(),
        })))
    }

    fn request(&mut self, kind: RequestKind, request: Value) -> Line {
        let id = format!("ren-{}", self.next_request);
        self.next_request += 1;
        self.requests.insert(id.clone(), kind);
        Line::json(&json!({ "type": "control_request", "request_id": id, "request": request }))
    }

    /// Claude Code's suggested rules for the rest of the session. Kept to
    /// this session: Ren never writes the operator's settings files.
    fn for_session(suggestions: Vec<Value>) -> Value {
        Value::Array(
            suggestions
                .into_iter()
                .filter_map(|suggestion| match suggestion {
                    Value::Object(mut fields) => {
                        fields.insert("destination".into(), "session".into());
                        Some(Value::Object(fields))
                    }
                    Value::Null
                    | Value::Bool(_)
                    | Value::Number(_)
                    | Value::String(_)
                    | Value::Array(_) => None,
                })
                .collect(),
        )
    }

    fn control_response(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let response = &message["response"];
        let Some(id) = response.get("request_id").and_then(Value::as_str) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            });
            return;
        };
        // Claude Code also echoes Ren's own answers; only Ren's requests
        // wait on a response.
        let Some(kind) = self.requests.remove(id) else {
            return;
        };
        match response.get("subtype").and_then(Value::as_str) {
            Some("success") => match kind {
                RequestKind::Initialize => {
                    self.ready = true;
                    received.events.push(DriveEvent::Ready {
                        thread: self.thread,
                    });
                }
                RequestKind::TurnInterrupt
                | RequestKind::ThreadStart
                | RequestKind::ThreadResume
                | RequestKind::TurnStart => {}
            },
            Some(_) | None => received.events.push(DriveEvent::Refused {
                request: kind,
                message: response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("refused")
                    .to_owned(),
            }),
        }
    }

    fn control_request(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let id = message.get("request_id").and_then(Value::as_str);
        let request = &message["request"];
        let subtype = request.get("subtype").and_then(Value::as_str);
        match (id, subtype) {
            (Some(id), Some("can_use_tool")) => {
                let Some(tool) = request.get("tool_name").and_then(Value::as_str) else {
                    received
                        .replies
                        .push(Self::refuse(id, "unreadable request"));
                    received.events.push(DriveEvent::Unreadable {
                        line,
                        reason: Unreadable::BadParams,
                    });
                    return;
                };
                let input = request.get("input").cloned().unwrap_or(Value::Null);
                let ask = Self::ask(tool, request, &input);
                let approval = ApprovalId(id.to_owned());
                self.approvals.insert(
                    approval.clone(),
                    Asked {
                        input,
                        suggestions: request
                            .get("permission_suggestions")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default(),
                    },
                );
                received.events.push(DriveEvent::Approval { approval, ask });
            }
            (Some(id), subtype) => {
                // Never leave Claude Code waiting on a request Ren does not serve.
                received
                    .replies
                    .push(Self::refuse(id, "not supported by Ren"));
                received.events.push(DriveEvent::Unsupported {
                    method: subtype.unwrap_or_default().to_owned(),
                });
            }
            (None, _) => received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            }),
        }
    }

    fn refuse(id: &str, error: &str) -> Line {
        Line::json(&json!({
            "type": "control_response",
            "response": { "subtype": "error", "request_id": id, "error": error },
        }))
    }

    /// What a permission prompt asks, from its tool and input.
    fn ask(tool: &str, request: &Value, input: &Value) -> ApprovalAsk {
        let text =
            |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let reason = text(request, "description")
            .or_else(|| text(request, "decision_reason"))
            .or_else(|| text(input, "description"));
        match tool {
            "Bash" => ApprovalAsk::Command {
                command: text(input, "command"),
                cwd: None,
                reason,
            },
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => ApprovalAsk::FileChange {
                reason: text(input, "file_path")
                    .or_else(|| text(input, "notebook_path"))
                    .or(reason),
            },
            other => ApprovalAsk::Tool {
                tool: text(request, "display_name").unwrap_or_else(|| other.to_owned()),
                detail: reason.or_else(|| Self::detail(input)),
            },
        }
    }

    /// The one input field that says what a tool call is about.
    fn detail(input: &Value) -> Option<String> {
        [
            "file_path",
            "path",
            "pattern",
            "url",
            "query",
            "description",
        ]
        .into_iter()
        .find_map(|key| input.get(key).and_then(Value::as_str))
        .map(str::to_owned)
    }

    fn user(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let content = &message["message"]["content"];
        if message.get("isReplay").and_then(Value::as_bool) == Some(true) {
            let Some(uuid) = message.get("uuid").and_then(Value::as_str) else {
                received.events.push(DriveEvent::Unreadable {
                    line,
                    reason: Unreadable::BadParams,
                });
                return;
            };
            let text = match content {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect(),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => String::new(),
            };
            self.start_turn(uuid, received);
            received.events.push(DriveEvent::Item {
                phase: ItemPhase::Completed,
                item: DriveItem {
                    id: ItemId::new(uuid.to_owned()),
                    kind: ItemKind::UserMessage,
                    read: ItemRead::Prompt { text },
                    body: message["message"].clone(),
                },
            });
            return;
        }
        // Otherwise: tool results, and Claude Code's own notes (such as an
        // interruption), which the turn's end reports.
        let Some(parts) = content.as_array() else {
            return;
        };
        for part in parts {
            if part.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(id) = part.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(call) = self.tools.remove(id) else {
                continue;
            };
            let declined = message
                .get("tool_result_meta")
                .and_then(Value::as_array)
                .is_some_and(|metas| {
                    metas.iter().any(|meta| {
                        meta.get("id").and_then(Value::as_str) == Some(id)
                            && meta.get("non_execution_kind").is_some()
                    })
                });
            let failed = part.get("is_error").and_then(Value::as_bool) == Some(true);
            let status = if declined {
                ItemStatus::Declined
            } else if failed {
                ItemStatus::Failed
            } else {
                ItemStatus::Completed
            };
            let output = Self::output(part, &message["tool_use_result"]);
            let read = match call.read {
                ItemRead::Command { command, .. } => ItemRead::Command {
                    exit_code: failed.then(|| Self::exit_code(output.as_deref())).flatten(),
                    command,
                    status,
                    output,
                },
                ItemRead::FileChange { paths, .. } => ItemRead::FileChange { paths, status },
                ItemRead::Tool { name, .. } => ItemRead::Tool { name, status },
                other @ (ItemRead::Prompt { .. }
                | ItemRead::Reply { .. }
                | ItemRead::Reasoning
                | ItemRead::Plan { .. }
                | ItemRead::Unread) => other,
            };
            received.events.push(DriveEvent::Item {
                phase: ItemPhase::Completed,
                item: DriveItem {
                    id: ItemId::new(id.to_owned()),
                    kind: call.kind,
                    read,
                    body: json!({ "tool_use": call.block, "tool_result": part }),
                },
            });
        }
    }

    /// A tool result's text: a command's own output where Claude Code gives
    /// it, else the result's content.
    fn output(part: &Value, result: &Value) -> Option<String> {
        if let Some(stdout) = result.get("stdout").and_then(Value::as_str) {
            let stderr = result
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Some(match (stdout.is_empty(), stderr.is_empty()) {
                (_, true) => stdout.to_owned(),
                (true, false) => stderr.to_owned(),
                (false, false) => format!("{stdout}\n{stderr}"),
            });
        }
        match part.get("content")? {
            Value::String(text) => Some(text.clone()),
            Value::Array(parts) => Some(
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
        }
    }

    /// A failed command's exit code, which Claude Code puts first in its
    /// result as `Exit code N`.
    fn exit_code(output: Option<&str>) -> Option<i64> {
        output?
            .strip_prefix("Exit code ")?
            .split(|c: char| !c.is_ascii_digit() && c != '-')
            .next()?
            .parse()
            .ok()
    }

    fn start_turn(&mut self, id: &str, received: &mut Received) {
        let Some(turn) = Uuid::try_parse(id).ok().and_then(NativeTurnId::new) else {
            return;
        };
        match self.turn {
            TurnSlot::Running { turn: running, .. } if running == turn => {}
            TurnSlot::Idle | TurnSlot::Requested | TurnSlot::Running { .. } => {
                self.turn = TurnSlot::Running {
                    turn,
                    interrupted: false,
                };
                received.events.push(DriveEvent::TurnStarted { turn });
            }
        }
    }

    /// A turn Claude Code started on its own (after background work), which
    /// no prompt's echo announced.
    fn unprompted(&mut self, message: &Value, received: &mut Received) {
        if let TurnSlot::Idle = self.turn
            && let Some(uuid) = message.get("uuid").and_then(Value::as_str)
        {
            self.start_turn(uuid, received);
        }
    }

    fn assistant(&mut self, message: &Value, received: &mut Received) {
        self.unprompted(message, received);
        let body = &message["message"];
        let message_id = body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(blocks) = body.get("content").and_then(Value::as_array) else {
            return;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let count = self.blocks.entry(message_id.clone()).or_default();
                    let id = ItemId::new(format!("{message_id}:text:{}", count.texts));
                    count.texts += 1;
                    received.events.push(DriveEvent::Item {
                        phase: ItemPhase::Completed,
                        item: DriveItem {
                            id,
                            kind: ItemKind::AgentMessage,
                            read: ItemRead::Reply {
                                text: block
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                            },
                            body: block.clone(),
                        },
                    });
                }
                Some("thinking" | "redacted_thinking") => {
                    let count = self.blocks.entry(message_id.clone()).or_default();
                    let id = ItemId::new(format!("{message_id}:thinking:{}", count.thinking));
                    count.thinking += 1;
                    received.events.push(DriveEvent::Item {
                        phase: ItemPhase::Completed,
                        item: DriveItem {
                            id,
                            kind: ItemKind::Reasoning,
                            read: ItemRead::Reasoning,
                            // The signature is opaque to Ren and large.
                            body: json!({ "type": block["type"] }),
                        },
                    });
                }
                Some("tool_use") => {
                    let (Some(id), Some(name)) = (
                        block.get("id").and_then(Value::as_str),
                        block.get("name").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    let (kind, read) = Self::tool(name, &block["input"]);
                    self.tools.insert(
                        id.to_owned(),
                        ToolCall {
                            kind: kind.clone(),
                            read: read.clone(),
                            block: block.clone(),
                        },
                    );
                    received.events.push(DriveEvent::Item {
                        phase: ItemPhase::Started,
                        item: DriveItem {
                            id: ItemId::new(id.to_owned()),
                            kind,
                            read,
                            body: block.clone(),
                        },
                    });
                }
                Some(_) | None => {}
            }
        }
    }

    /// A tool call's kind and reading, from Claude Code's tool name.
    fn tool(name: &str, input: &Value) -> (ItemKind, ItemRead) {
        let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
        match name {
            "Bash" => (
                ItemKind::CommandExecution,
                ItemRead::Command {
                    command: text("command").unwrap_or_default(),
                    status: ItemStatus::InProgress,
                    output: None,
                    exit_code: None,
                },
            ),
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => (
                ItemKind::FileChange,
                ItemRead::FileChange {
                    paths: text("file_path")
                        .or_else(|| text("notebook_path"))
                        .into_iter()
                        .collect(),
                    status: ItemStatus::InProgress,
                },
            ),
            "WebSearch" => (
                ItemKind::WebSearch,
                ItemRead::Tool {
                    name: text("query").map_or_else(
                        || "web search".to_owned(),
                        |query| format!("search: {query}"),
                    ),
                    status: ItemStatus::InProgress,
                },
            ),
            other => (
                if other.starts_with("mcp__") {
                    ItemKind::McpToolCall
                } else {
                    ItemKind::Other(other.to_owned())
                },
                ItemRead::Tool {
                    name: match Self::detail(input) {
                        Some(detail) => format!("{other} {detail}"),
                        None => other.to_owned(),
                    },
                    status: ItemStatus::InProgress,
                },
            ),
        }
    }

    fn stream_event(&mut self, message: &Value, received: &mut Received) {
        self.unprompted(message, received);
        let event = &message["event"];
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.streaming = event["message"]
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| Streaming {
                        message: id.to_owned(),
                        texts: 0,
                        items: BTreeMap::new(),
                    });
            }
            Some("content_block_start") => {
                if let (Some(streaming), Some(index), Some("text")) = (
                    self.streaming.as_mut(),
                    event.get("index").and_then(Value::as_u64),
                    event["content_block"].get("type").and_then(Value::as_str),
                ) {
                    let id = ItemId::new(format!("{}:text:{}", streaming.message, streaming.texts));
                    streaming.texts += 1;
                    streaming.items.insert(index, id);
                }
            }
            Some("content_block_delta") => {
                let delta = &event["delta"];
                if let (Some(streaming), Some(index), Some("text_delta"), Some(text)) = (
                    self.streaming.as_ref(),
                    event.get("index").and_then(Value::as_u64),
                    delta.get("type").and_then(Value::as_str),
                    delta.get("text").and_then(Value::as_str),
                ) && let Some(item) = streaming.items.get(&index)
                {
                    received.events.push(DriveEvent::Delta {
                        item: item.clone(),
                        text: text.to_owned(),
                    });
                }
            }
            Some(_) | None => {}
        }
    }

    fn result(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let TurnSlot::Running { turn, interrupted } = self.turn else {
            // A result for no turn Ren knows of.
            received.events.push(DriveEvent::Unknown { line });
            return;
        };
        self.turn = TurnSlot::Idle;
        self.tools.clear();
        self.blocks.clear();
        self.streaming = None;
        let pending = std::mem::take(&mut self.approvals);
        received.events.extend(
            pending
                .into_keys()
                .map(|approval| DriveEvent::ApprovalResolved { approval }),
        );
        let text = |key: &str| message.get(key).and_then(Value::as_str);
        let aborted = text("terminal_reason").is_some_and(|reason| reason.starts_with("aborted"));
        let failed = message.get("is_error").and_then(Value::as_bool) == Some(true);
        let end = if interrupted || aborted {
            TurnEnd::Interrupted
        } else {
            match text("subtype") {
                Some("success") if !failed => TurnEnd::Completed,
                Some(subtype) => TurnEnd::Failed {
                    message: Some(Self::failure(message).unwrap_or_else(|| subtype.to_owned())),
                },
                None => TurnEnd::Unknown {
                    status: String::new(),
                },
            }
        };
        received.events.push(DriveEvent::TurnEnded { turn, end });
    }

    /// What a failed result says went wrong.
    fn failure(message: &Value) -> Option<String> {
        if let Some(errors) = message.get("errors").and_then(Value::as_array) {
            let errors: Vec<&str> = errors.iter().filter_map(Value::as_str).collect();
            if !errors.is_empty() {
                return Some(errors.join("; "));
            }
        }
        message
            .get("result")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    fn system(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let field = |key: &str| message.get(key);
        match field("subtype").and_then(Value::as_str) {
            Some("api_retry") => {
                let mut text = String::from("API request failed");
                if let Some(error) = field("error").and_then(Value::as_str) {
                    text = format!("API request failed: {error}");
                }
                if let (Some(attempt), Some(max)) = (
                    field("attempt").and_then(Value::as_u64),
                    field("max_retries").and_then(Value::as_u64),
                ) {
                    text = format!("{text} (attempt {attempt} of {max})");
                }
                received.events.push(DriveEvent::Error {
                    message: text,
                    retry: Retry::WillRetry,
                });
            }
            // Known, and nothing Ren shows: the session's metadata, progress
            // notes, and summaries its own interface draws.
            Some(
                "init" | "status" | "thinking_tokens" | "task_summary" | "post_turn_summary"
                | "hook_started" | "hook_progress" | "hook_response",
            ) => {}
            Some(_) | None => received.events.push(DriveEvent::Unknown { line }),
        }
    }
}

/// A listing process's side of `claude -p` in stream-json: the models are in
/// Claude Code's answer to `initialize`. The listing keeps no session.
#[derive(Debug)]
pub struct ClaudeModels;

impl ClaudeModels {
    const REQUEST: &'static str = "ren-models";
    /// The entry Claude Code lists for "no model chosen".
    const DEFAULT: &'static str = "default";

    /// The arguments that start a listing process, after `claude`.
    pub fn arguments() -> Vec<String> {
        let mut arguments: Vec<String> = ClaudeStreamJson::MODE.map(str::to_owned).into();
        arguments.push("--no-session-persistence".into());
        arguments
    }

    /// The codec, and the first line to send.
    pub fn open() -> (Self, Line) {
        let line = Line::json(&json!({
            "type": "control_request",
            "request_id": Self::REQUEST,
            "request": { "subtype": "initialize" },
        }));
        (Self, line)
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        let response = &message["response"];
        let answers = message.get("type").and_then(Value::as_str) == Some("control_response")
            && response.get("request_id").and_then(Value::as_str) == Some(Self::REQUEST);
        if !answers {
            return Listing::waiting();
        }
        if response.get("subtype").and_then(Value::as_str) != Some("success") {
            return Listing::Failed(
                ModelListing::text(response, "error").unwrap_or_else(|| "refused".into()),
            );
        }
        let Some(entries) = response["response"].get("models").and_then(Value::as_array) else {
            return Listing::Failed("Claude Code listed no models".into());
        };
        // The "default" entry stands for no choice; the first entry it
        // resolves to is marked as the default instead.
        let resolved = entries
            .iter()
            .find(|entry| ModelListing::text(entry, "value").as_deref() == Some(Self::DEFAULT))
            .and_then(|entry| ModelListing::text(entry, "resolvedModel"));
        let mut marked = false;
        let models = entries
            .iter()
            .filter_map(|entry| {
                let value = ModelListing::text(entry, "value")?;
                if value == Self::DEFAULT {
                    return None;
                }
                let id: ModelId = value.parse().ok()?;
                let default = !marked
                    && resolved.is_some()
                    && ModelListing::text(entry, "resolvedModel") == resolved;
                marked |= default;
                Some(Model {
                    name: ModelListing::text(entry, "displayName")
                        .unwrap_or_else(|| id.to_string()),
                    description: ModelListing::text(entry, "description"),
                    provider: None,
                    choice: if default {
                        DefaultChoice::Default
                    } else {
                        DefaultChoice::Other
                    },
                    // Claude Code lists the efforts a model takes, not which
                    // one it starts at.
                    efforts: match entry.get("supportsEffort").and_then(Value::as_bool) {
                        Some(true) => {
                            ModelListing::efforts(entry.get("supportedEffortLevels"), None)
                        }
                        Some(false) | None => Vec::new(),
                    },
                    effort: None,
                    id,
                })
            })
            .collect();
        Listing::Listed(models)
    }
}

#[cfg(test)]
mod tests {
    use super::ClaudeStreamJson;
    use crate::{
        drive::{
            ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, ItemKind,
            ItemPhase, ItemRead, ItemStatus, Opening, Prompt, Received, RequestKind, Retry,
            TurnEnd, Unreadable,
        },
        lines::{Framed, Line},
    };
    use ren_protocol::native::NativeThreadId;
    use serde_json::{Value, json};

    const SESSION: &str = "0190f3a0-0000-7000-8000-0000000000c1";
    const PROMPT: &str = "0190f3a0-0000-7000-8000-0000000000c2";

    fn sent(line: &Line) -> Value {
        let bytes = line.as_bytes();
        assert_eq!(bytes.last(), Some(&b'\n'));
        serde_json::from_slice(bytes).unwrap()
    }

    fn feed(codec: &mut ClaudeStreamJson, message: Value) -> Received {
        codec.receive(Framed::Line(message.to_string().into_bytes()))
    }

    fn session() -> NativeThreadId {
        NativeThreadId::new(uuid::Uuid::parse_str(SESSION).unwrap()).unwrap()
    }

    fn prompt(text: &str) -> DriveCommand {
        DriveCommand::Prompt(Prompt::parse(text.into()).unwrap())
    }

    /// A codec whose conversation is open, as after `initialize`.
    fn ready() -> ClaudeStreamJson {
        let (mut codec, first) = ClaudeStreamJson::open(&Opening::New {
            cwd: "/work/acme".into(),
            id: session(),
            settings: crate::settings::RunSettings::default(),
        });
        assert_eq!(
            sent(&first),
            json!({ "type": "control_request", "request_id": "ren-0",
                    "request": { "subtype": "initialize" } })
        );
        assert_eq!(codec.command(prompt("hi")), Err(CommandRefused::NotReady));
        let received = feed(
            &mut codec,
            json!({ "type": "control_response", "response": { "subtype": "success",
                    "request_id": "ren-0", "response": { "commands": [] } } }),
        );
        assert_eq!(
            received.events,
            vec![DriveEvent::Ready { thread: session() }]
        );
        codec
    }

    /// The prompt's echo, which starts the turn.
    fn echo(codec: &mut ClaudeStreamJson, text: &str) -> Received {
        feed(
            codec,
            json!({ "type": "user", "message": { "role": "user", "content": text },
                    "session_id": SESSION, "parent_tool_use_id": null, "uuid": PROMPT,
                    "isReplay": true }),
        )
    }

    #[test]
    fn runs_a_turn_with_streamed_text_and_a_command() {
        let mut codec = ready();
        let line = codec.command(prompt("run the tests")).unwrap();
        assert_eq!(
            sent(&line),
            json!({ "type": "user", "message": { "role": "user", "content": "run the tests" },
                    "parent_tool_use_id": null, "session_id": SESSION })
        );
        assert_eq!(
            codec.command(prompt("again")),
            Err(CommandRefused::TurnRunning)
        );
        assert_eq!(
            codec.command(DriveCommand::Interrupt),
            Err(CommandRefused::TurnNotStarted)
        );

        let started = echo(&mut codec, "run the tests");
        assert!(matches!(
            started.events.as_slice(),
            [
                DriveEvent::TurnStarted { turn },
                DriveEvent::Item { phase: ItemPhase::Completed, item },
            ] if turn.to_string() == PROMPT
                && item.read == ItemRead::Prompt { text: "run the tests".into() }
        ));

        let tool_use = json!({ "type": "tool_use", "id": "toolu_1", "name": "Bash",
                               "input": { "command": "cargo test", "description": "Run tests" } });
        let call = feed(
            &mut codec,
            json!({ "type": "assistant", "message": { "id": "msg_1", "role": "assistant",
                    "content": [tool_use] }, "parent_tool_use_id": null, "session_id": SESSION,
                    "uuid": "0190f3a0-0000-7000-8000-0000000000c3" }),
        );
        assert!(matches!(
            call.events.as_slice(),
            [DriveEvent::Item { phase: ItemPhase::Started, item }]
                if item.kind == ItemKind::CommandExecution
                    && item.read == ItemRead::Command {
                        command: "cargo test".into(),
                        status: ItemStatus::InProgress,
                        output: None,
                        exit_code: None,
                    }
        ));
        let result = feed(
            &mut codec,
            json!({ "type": "user", "message": { "role": "user", "content": [
                      { "type": "tool_result", "tool_use_id": "toolu_1",
                        "content": "Exit code 101\ntest failed", "is_error": true }] },
                    "parent_tool_use_id": null, "session_id": SESSION,
                    "tool_use_result": "Error: Exit code 101\ntest failed" }),
        );
        assert!(matches!(
            result.events.as_slice(),
            [DriveEvent::Item { phase: ItemPhase::Completed, item }]
                if item.id.as_str() == "toolu_1" && item.read == ItemRead::Command {
                    command: "cargo test".into(),
                    status: ItemStatus::Failed,
                    output: Some("Exit code 101\ntest failed".into()),
                    exit_code: Some(101),
                }
        ));

        // A streamed text block and its whole copy are one item.
        let mut deltas = Vec::new();
        for event in [
            json!({ "type": "message_start", "message": { "id": "msg_2" } }),
            json!({ "type": "content_block_start", "index": 0,
                    "content_block": { "type": "thinking", "thinking": "" } }),
            json!({ "type": "content_block_start", "index": 1,
                    "content_block": { "type": "text", "text": "" } }),
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "text_delta", "text": "One test " } }),
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "text_delta", "text": "fails." } }),
        ] {
            deltas.extend(
                feed(
                    &mut codec,
                    json!({ "type": "stream_event", "event": event,
                            "parent_tool_use_id": null, "session_id": SESSION }),
                )
                .events,
            );
        }
        assert!(matches!(
            deltas.as_slice(),
            [DriveEvent::Delta { item: first, .. }, DriveEvent::Delta { item: second, text }]
                if first.as_str() == "msg_2:text:0" && second == first && text == "fails."
        ));
        let whole = feed(
            &mut codec,
            json!({ "type": "assistant", "message": { "id": "msg_2", "role": "assistant",
                    "content": [{ "type": "text", "text": "One test fails." }] },
                    "parent_tool_use_id": null, "session_id": SESSION }),
        );
        assert!(matches!(
            whole.events.as_slice(),
            [DriveEvent::Item { item, .. }] if item.id.as_str() == "msg_2:text:0"
                && item.read == ItemRead::Reply { text: "One test fails.".into() }
        ));

        let ended = feed(
            &mut codec,
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "result": "One test fails.", "terminal_reason": "completed",
                    "session_id": SESSION }),
        );
        assert!(matches!(
            ended.events.as_slice(),
            [DriveEvent::TurnEnded { end: TurnEnd::Completed, turn }] if turn.to_string() == PROMPT
        ));
        assert_eq!(
            codec.command(DriveCommand::Interrupt),
            Err(CommandRefused::NoTurn)
        );
        assert!(codec.command(prompt("next")).is_ok());
    }

    #[test]
    fn answers_permission_prompts_with_the_operators_decision() {
        let mut codec = ready();
        codec.command(prompt("write it")).unwrap();
        let _ = echo(&mut codec, "write it");
        let input = json!({ "file_path": "/work/acme/notes.txt", "content": "hi" });
        let asked = feed(
            &mut codec,
            json!({ "type": "control_request", "request_id": "perm-1", "request": {
                "subtype": "can_use_tool", "tool_name": "Write", "display_name": "Write",
                "input": input, "description": "notes.txt", "tool_use_id": "toolu_2",
                "permission_suggestions": [
                    { "type": "setMode", "mode": "acceptEdits", "destination": "localSettings" }] } }),
        );
        let approval = ApprovalId("perm-1".into());
        assert_eq!(
            asked.events,
            vec![DriveEvent::Approval {
                approval: approval.clone(),
                ask: ApprovalAsk::FileChange {
                    reason: Some("/work/acme/notes.txt".into())
                },
            }]
        );
        // For the session: Claude Code's suggestion, kept to this session.
        let answer = codec
            .command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::AcceptForSession,
            })
            .unwrap();
        assert_eq!(
            sent(&answer),
            json!({ "type": "control_response", "response": { "subtype": "success",
                "request_id": "perm-1", "response": {
                    "behavior": "allow", "updatedInput": input,
                    "updatedPermissions": [
                        { "type": "setMode", "mode": "acceptEdits", "destination": "session" }] } } })
        );
        assert_eq!(
            codec.command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::Accept,
            }),
            Err(CommandRefused::UnknownApproval(approval))
        );
        // Claude Code echoes the answer back; it is not a new response.
        assert!(feed(&mut codec, sent(&answer)).events.is_empty());

        // Cancelling refuses the call and ends the turn as interrupted.
        let asked = feed(
            &mut codec,
            json!({ "type": "control_request", "request_id": "perm-2", "request": {
                "subtype": "can_use_tool", "tool_name": "WebFetch",
                "input": { "url": "https://example.com" } } }),
        );
        assert!(matches!(
            asked.events.as_slice(),
            [DriveEvent::Approval { ask: ApprovalAsk::Tool { tool, detail }, .. }]
                if tool == "WebFetch" && detail.as_deref() == Some("https://example.com")
        ));
        let cancel = codec
            .command(DriveCommand::Answer {
                approval: ApprovalId("perm-2".into()),
                decision: Decision::Cancel,
            })
            .unwrap();
        assert_eq!(sent(&cancel)["response"]["response"]["interrupt"], true);
        let ended = feed(
            &mut codec,
            json!({ "type": "result", "subtype": "error_during_execution", "is_error": true,
                    "terminal_reason": "aborted_tools", "errors": ["diagnostic"] }),
        );
        assert!(matches!(
            ended.events.as_slice(),
            [DriveEvent::TurnEnded {
                end: TurnEnd::Interrupted,
                ..
            }]
        ));
    }

    #[test]
    fn interrupts_and_resolves_withdrawn_or_unanswered_prompts() {
        let mut codec = ready();
        codec.command(prompt("go")).unwrap();
        let _ = echo(&mut codec, "go");
        let interrupt = codec.command(DriveCommand::Interrupt).unwrap();
        assert_eq!(
            sent(&interrupt),
            json!({ "type": "control_request", "request_id": "ren-1",
                    "request": { "subtype": "interrupt" } })
        );
        let asked = json!({ "type": "control_request", "request_id": "perm-3", "request": {
            "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "ls" } } });
        let _ = feed(&mut codec, asked.clone());
        let withdrawn = feed(
            &mut codec,
            json!({ "type": "control_cancel_request", "request_id": "perm-3" }),
        );
        assert_eq!(
            withdrawn.events,
            vec![DriveEvent::ApprovalResolved {
                approval: ApprovalId("perm-3".into())
            }]
        );
        let _ = feed(&mut codec, asked);
        // A turn that ends with a prompt unanswered resolves it, and an
        // interrupt Ren asked for ends the turn as interrupted whatever the
        // result says.
        let ended = feed(
            &mut codec,
            json!({ "type": "result", "subtype": "success", "is_error": false }),
        );
        assert!(matches!(
            ended.events.as_slice(),
            [
                DriveEvent::ApprovalResolved { .. },
                DriveEvent::TurnEnded {
                    end: TurnEnd::Interrupted,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn refuses_what_ren_does_not_serve_and_keeps_the_unknown() {
        let mut codec = ready();
        let hook = feed(
            &mut codec,
            json!({ "type": "control_request", "request_id": "h-1",
                    "request": { "subtype": "hook_callback", "callback_id": "x" } }),
        );
        assert_eq!(
            sent(&hook.replies[0]),
            json!({ "type": "control_response", "response": { "subtype": "error",
                    "request_id": "h-1", "error": "not supported by Ren" } })
        );
        assert_eq!(
            hook.events,
            vec![DriveEvent::Unsupported {
                method: "hook_callback".into()
            }]
        );
        let retry = feed(
            &mut codec,
            json!({ "type": "system", "subtype": "api_retry", "attempt": 1, "max_retries": 10,
                    "error": "overloaded", "error_status": 529 }),
        );
        assert_eq!(
            retry.events,
            vec![DriveEvent::Error {
                message: "API request failed: overloaded (attempt 1 of 10)".into(),
                retry: Retry::WillRetry,
            }]
        );
        assert!(matches!(
            feed(&mut codec, json!({ "type": "brand_new" }))
                .events
                .as_slice(),
            [DriveEvent::Unknown { .. }]
        ));
        assert!(matches!(
            feed(&mut codec, json!({ "no": "type" })).events.as_slice(),
            [DriveEvent::Unreadable {
                reason: Unreadable::Untyped,
                ..
            }]
        ));
        assert!(matches!(
            codec
                .receive(Framed::Line(b"not json".to_vec()))
                .events
                .as_slice(),
            [DriveEvent::Unreadable {
                reason: Unreadable::NotJson,
                ..
            }]
        ));
        // Noise Claude Code's own interface draws says nothing to Ren.
        assert!(
            feed(
                &mut codec,
                json!({ "type": "system", "subtype": "thinking_tokens", "estimated_tokens": 5 })
            )
            .events
            .is_empty()
        );
    }

    #[test]
    fn a_refused_opening_is_reported() {
        let (mut codec, _) = ClaudeStreamJson::open(&Opening::Resume {
            thread: session(),
            cwd: "/w".into(),
            settings: crate::settings::RunSettings::default(),
        });
        let refused = feed(
            &mut codec,
            json!({ "type": "control_response", "response": { "subtype": "error",
                    "request_id": "ren-0", "error": "no such session" } }),
        );
        assert_eq!(
            refused.events,
            vec![DriveEvent::Refused {
                request: RequestKind::Initialize,
                message: "no such session".into(),
            }]
        );
        assert_eq!(codec.thread(), None);
    }

    #[test]
    fn a_turn_claude_code_starts_on_its_own_is_followed() {
        let mut codec = ready();
        let unprompted = feed(
            &mut codec,
            json!({ "type": "assistant", "message": { "id": "msg_9", "content": [
                { "type": "text", "text": "The build finished." }] },
                "parent_tool_use_id": null, "uuid": PROMPT }),
        );
        assert!(matches!(
            unprompted.events.as_slice(),
            [DriveEvent::TurnStarted { .. }, DriveEvent::Item { .. }]
        ));
        // A subagent's own messages are not the conversation's.
        assert!(
            feed(
                &mut codec,
                json!({ "type": "assistant", "message": { "id": "msg_s", "content": [
                    { "type": "text", "text": "inner" }] }, "parent_tool_use_id": "toolu_9" }),
            )
            .events
            .is_empty()
        );
    }
}
