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
//! answered with the operator's decision; `AskUserQuestion` asks its
//! questions the same way, answered with the operator's replies as the
//! tool's input (`question`). Every other control request is refused: Ren
//! provides no hooks, tools, or MCP servers.
//!
//! Claude Code reports no turn identifier that matches the prompt id its
//! model requests carry, so a driven turn's id is its prompt's message id.

use super::question::AskUserQuestion;
use super::tools::ClaudeCodeTool;
use crate::{
    codec::{ApprovalBook, EchoQueue, Envelope, RequestLedger, Stop, TurnSlot},
    commands::{ClientCommand, CommandKind, Sigil},
    drive::{
        ApprovalAsk, ApprovalId, CommandIntent, CommandRefused, Decision, DriveCommand, DriveEvent,
        DriveItem, ItemId, ItemKind, ItemPhase, ItemRead, ItemStatus, Opening, RawLine, Received,
        RequestKind, Retry, TurnEnd, Unreadable,
    },
    lines::{Framed, Line},
    models::ModelId,
    prompt::Prompt,
    settings::{ApprovalPolicy, Midway, RunSettings, Setting, SettingsBook},
};
use ren_protocol::{
    label::{Labeled, Stored},
    native::{NativeThreadId, NativeTurnId},
};
use serde_json::{Value, json};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::collections::BTreeMap;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use uuid::Uuid;

/// The client's side of one `claude -p` stream-json process.
#[derive(Debug)]
pub struct ClaudeStreamJson {
    phase: Phase,
    /// What the conversation runs on, and what Ren asked it to change to.
    settings: SettingsBook,
    /// Control requests Ren sent and waits on, by request id.
    requests: RequestLedger<String, Pending>,
    turn: TurnSlot,
    /// Permission prompts waiting for an answer.
    approvals: ApprovalBook<Asked>,
    /// Tool calls started in this turn, by tool use id, until their result.
    tools: BTreeMap<String, ToolCall>,
    /// Text and thinking blocks counted per message, so a streamed block and
    /// its whole copy get the same item id.
    blocks: BTreeMap<String, BlockCount>,
    streaming: Option<Streaming>,
    /// The commands and skills Claude Code listed as it opened.
    commands: Vec<ClientCommand>,
    /// What the images sent with the last prompt are, until it is echoed.
    echoing: EchoQueue,
}

/// Whether the conversation Ren named is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `initialize` was sent; Claude Code has not answered.
    Opening(NativeThreadId),
    Ready(NativeThreadId),
}

/// Every message type Claude Code writes in stream-json; a type Ren does not
/// know stays unknown.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ClaudeMessageType {
    ControlResponse,
    ControlRequest,
    ControlCancelRequest,
    User,
    Assistant,
    StreamEvent,
    Result,
    System,
    /// The allowance status, which Ren reads from the recorded calls.
    RateLimitEvent,
}

/// The `system` messages Claude Code writes, by subtype.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ClaudeSystemSubtype {
    ApiRetry,
    Init,
    Status,
    ThinkingTokens,
    TaskSummary,
    PostTurnSummary,
    HookStarted,
    HookProgress,
    HookResponse,
}

/// A control request Ren sent, and what it was for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Initialize,
    Interrupt,
    /// A setting changed mid-run, committed once Claude Code takes it.
    Configure(Setting),
}

impl Pending {
    fn kind(&self) -> RequestKind {
        match self {
            Self::Initialize => RequestKind::Initialize,
            Self::Interrupt => RequestKind::TurnInterrupt,
            Self::Configure(_) => RequestKind::Configure,
        }
    }
}

/// What a permission prompt asked, kept to answer it.
#[derive(Debug)]
enum Asked {
    /// Whether a tool may run on `input`.
    Permission {
        input: Value,
        suggestions: Vec<Value>,
    },
    /// `AskUserQuestion`'s questions, in its `input`.
    Questions { input: Value },
}

#[derive(Debug)]
struct ToolCall {
    kind: Stored<ItemKind>,
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
/// What Ren tells Claude Code when the operator declines its questions.
const UNANSWERED: &str = "The operator declined to answer these questions.";

impl ClaudeStreamJson {
    /// The arguments every driven Claude Code process starts with.
    pub(super) const MODE: [&'static str; 10] = [
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
            arguments.extend([
                "--permission-mode".into(),
                Self::permission_mode(policy).into(),
            ]);
        }
        arguments
    }

    /// A codec for a freshly started process, and the first line to send it.
    pub fn open(opening: &Opening) -> (Self, Line) {
        let (thread, settings) = match opening {
            Opening::New { id, settings, .. } => (*id, settings.clone()),
            Opening::Resume {
                thread, settings, ..
            } => (*thread, settings.clone()),
        };
        let mut codec = Self {
            phase: Phase::Opening(thread),
            settings: SettingsBook::new(settings),
            requests: RequestLedger::new(|n| format!("ren-{n}")),
            turn: TurnSlot::Idle,
            approvals: ApprovalBook::new(),
            tools: BTreeMap::new(),
            blocks: BTreeMap::new(),
            streaming: None,
            commands: Vec::new(),
            echoing: EchoQueue::default(),
        };
        let line = codec.request(Pending::Initialize, json!({ "subtype": "initialize" }));
        (codec, line)
    }

    /// The conversation, once Claude Code accepted the opening.
    pub fn thread(&self) -> Option<NativeThreadId> {
        match self.phase {
            Phase::Ready(thread) => Some(thread),
            Phase::Opening(_) => None,
        }
    }

    /// The commands and skills a prompt can run, as Claude Code listed them
    /// in its answer to `initialize`: each is `/name` in the prompt.
    pub fn commands(&self) -> &[ClientCommand] {
        &self.commands
    }

    /// The commands in Claude Code's answer to `initialize`.
    pub(crate) fn listed(answer: &Value) -> Vec<ClientCommand> {
        answer
            .get("commands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|command| {
                let name = command.get("name").and_then(Value::as_str)?;
                let text = |field| {
                    command
                        .get(field)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                };
                Some(ClientCommand {
                    name: name.to_owned(),
                    description: text("description").unwrap_or_default().to_owned(),
                    hint: text("argumentHint").map(str::to_owned),
                    kind: CommandKind::Command,
                    sigil: Sigil::Slash,
                })
            })
            .collect()
    }

    /// The line that carries `command`, or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Vec<Line>, CommandRefused> {
        let Phase::Ready(thread) = self.phase else {
            return Err(CommandRefused::NotReady);
        };
        match command {
            DriveCommand::Prompt(prompt) => self.prompt(thread, &prompt).map(|line| vec![line]),
            // Claude Code holds a message sent mid-turn for after it; it has
            // no way to add one to the turn.
            DriveCommand::Steer(_) => Err(CommandRefused::NotOffered(Midway::Steer)),
            DriveCommand::Configure(settings) => self.configure(settings),
            DriveCommand::Interrupt => {
                self.turn.stop()?;
                Ok(vec![self.request(
                    Pending::Interrupt,
                    json!({ "subtype": "interrupt" }),
                )])
            }
            DriveCommand::Answer { approval, decision } => {
                if let (Asked::Questions { .. }, Decision::Accept | Decision::AcceptForSession) =
                    (self.approvals.get(&approval)?, decision)
                {
                    return Err(CommandRefused::Unanswered(approval));
                }
                let asked = self.approvals.answer(approval.clone())?;
                let refusal = match asked {
                    Asked::Questions { .. } => UNANSWERED,
                    Asked::Permission { .. } => DECLINED,
                };
                let answer = match (decision, asked) {
                    (Decision::Accept, Asked::Permission { input, .. }) => {
                        json!({ "behavior": "allow", "updatedInput": input })
                    }
                    (Decision::AcceptForSession, Asked::Permission { input, suggestions }) => {
                        json!({
                            "behavior": "allow",
                            "updatedInput": input,
                            "updatedPermissions": Self::for_session(suggestions),
                        })
                    }
                    // Refused above, before the approval was taken.
                    (Decision::Accept | Decision::AcceptForSession, Asked::Questions { .. }) => {
                        return Err(CommandRefused::Unanswered(approval));
                    }
                    (Decision::Decline, Asked::Permission { .. } | Asked::Questions { .. }) => {
                        json!({ "behavior": "deny", "message": refusal })
                    }
                    (Decision::Cancel, Asked::Permission { .. } | Asked::Questions { .. }) => {
                        // The turn ends interrupted, as far as one runs.
                        if self.turn.under_way() {
                            self.turn.stop()?;
                        }
                        json!({ "behavior": "deny", "message": refusal, "interrupt": true })
                    }
                };
                Ok(vec![Self::answer(&approval, &answer)])
            }
            DriveCommand::Reply { approval, replies } => {
                let Asked::Questions { .. } = self.approvals.get(&approval)? else {
                    return Err(CommandRefused::NotQuestions(approval));
                };
                let (Asked::Questions { input } | Asked::Permission { input, .. }) =
                    self.approvals.answer(approval.clone())?;
                let answer = json!({
                    "behavior": "allow",
                    "updatedInput": AskUserQuestion::answered(&input, &replies),
                });
                Ok(vec![Self::answer(&approval, &answer)])
            }
        }
    }

    /// The line that answers the permission prompt `approval`.
    fn answer(approval: &ApprovalId, answer: &Value) -> Line {
        Line::json(&json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": approval.to_string(),
                "response": answer,
            },
        }))
    }

    /// The control requests that move the conversation to `settings`:
    /// Claude Code switches model and permission mode mid-run, not effort.
    /// Each is committed once Claude Code answers it.
    fn configure(&mut self, settings: RunSettings) -> Result<Vec<Line>, CommandRefused> {
        let change = self
            .settings
            .change_to(&settings, crate::Harness::ClaudeCode.profile().midway)?;
        let requests = change
            .into_iter()
            .map(|setting| {
                let request = match &setting {
                    // `default` is Claude Code's own choice.
                    Setting::Model(model) => json!({
                        "subtype": "set_model",
                        "model": model.as_ref().map_or("default", ModelId::as_str),
                    }),
                    Setting::Approvals(policy) => json!({
                        "subtype": "set_permission_mode",
                        "mode": policy.map_or("default", Self::permission_mode),
                    }),
                    Setting::Effort(_) => return Err(CommandRefused::NotOffered(Midway::Effort)),
                };
                Ok((setting, request))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(requests
            .into_iter()
            .map(|(setting, request)| {
                self.settings.ask(setting.clone());
                self.request(Pending::Configure(setting), request)
            })
            .collect())
    }

    /// The permission mode a policy runs Claude Code in.
    fn permission_mode(policy: ApprovalPolicy) -> &'static str {
        match policy {
            ApprovalPolicy::Ask => "default",
            ApprovalPolicy::Edits => "acceptEdits",
            ApprovalPolicy::Auto => "auto",
        }
    }

    /// Read one frame from Claude Code's stdout.
    pub fn receive(&mut self, frame: Framed) -> Received {
        let envelope = match Envelope::open(frame) {
            Ok(envelope) => envelope,
            Err(received) => return received,
        };
        let Some(kind) = envelope.typed::<ClaudeMessageType>() else {
            return Received::event(envelope.untyped());
        };
        let Envelope {
            line,
            body: message,
        } = envelope;
        let mut received = Received::default();
        // Subagents' own messages carry the tool call that started them; the
        // call itself is the item Ren shows.
        let main = message.get("parent_tool_use_id").is_none_or(Value::is_null);
        let Stored::Known(kind) = kind else {
            received.events.push(DriveEvent::Unknown { line });
            return received;
        };
        match kind {
            ClaudeMessageType::ControlResponse => {
                self.control_response(&message, line, &mut received);
            }
            ClaudeMessageType::ControlRequest => {
                self.control_request(&message, line, &mut received);
            }
            ClaudeMessageType::ControlCancelRequest => {
                if let Some(id) = message.get("request_id").and_then(Value::as_str) {
                    let approval = ApprovalId(id.to_owned());
                    if self.approvals.resolve(&approval).is_some() {
                        received
                            .events
                            .push(DriveEvent::ApprovalResolved { approval });
                    }
                }
            }
            ClaudeMessageType::User if main => self.user(&message, line, &mut received),
            ClaudeMessageType::Assistant if main => self.assistant(&message, &mut received),
            ClaudeMessageType::StreamEvent if main => self.stream_event(&message, &mut received),
            ClaudeMessageType::Result => self.result(&message, line, &mut received),
            ClaudeMessageType::System => self.system(&message, line, &mut received),
            // Known, and nothing Ren shows: subagents' messages, and the
            // allowance status Ren reads from the recorded calls instead.
            ClaudeMessageType::User
            | ClaudeMessageType::Assistant
            | ClaudeMessageType::StreamEvent
            | ClaudeMessageType::RateLimitEvent => {}
        }
        received
    }

    fn prompt(&mut self, thread: NativeThreadId, prompt: &Prompt) -> Result<Line, CommandRefused> {
        self.turn.request(())?;
        self.echoing.only(prompt);
        // Text alone goes as text; with images, as the Messages API's
        // content blocks: each image in base64, then the text.
        let content = if prompt.images().is_empty() {
            json!(prompt.as_str())
        } else {
            let mut blocks: Vec<Value> = prompt
                .images()
                .iter()
                .map(|image| {
                    json!({ "type": "image", "source": {
                        "type": "base64", "media_type": image.media_type(), "data": image.base64() } })
                })
                .collect();
            if !prompt.as_str().trim().is_empty() {
                blocks.push(json!({ "type": "text", "text": prompt.as_str() }));
            }
            Value::Array(blocks)
        };
        Ok(Line::json(&json!({
            "type": "user",
            "message": { "role": "user", "content": content },
            "parent_tool_use_id": null,
            "session_id": thread.to_string(),
        })))
    }

    fn request(&mut self, pending: Pending, request: Value) -> Line {
        let id = self.requests.send(pending);
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
        let Some(pending) = self.requests.answer(id) else {
            return;
        };
        match response.get("subtype").and_then(Value::as_str) {
            Some("success") => match pending {
                Pending::Initialize => {
                    let (Phase::Opening(thread) | Phase::Ready(thread)) = self.phase;
                    self.phase = Phase::Ready(thread);
                    self.commands = Self::listed(&response["response"]);
                    received.events.push(DriveEvent::Ready { thread });
                }
                Pending::Configure(setting) => self.settings.acknowledge(setting),
                Pending::Interrupt => {}
            },
            Some(_) | None => {
                if let Pending::Configure(setting) = &pending {
                    self.settings.refused(setting);
                }
                received.events.push(DriveEvent::Refused {
                    request: pending.kind(),
                    message: response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("refused")
                        .to_owned(),
                });
            }
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
                let approval = ApprovalId(id.to_owned());
                let questions = (ClaudeCodeTool::from_label(tool)
                    == Some(ClaudeCodeTool::AskUserQuestion))
                .then(|| AskUserQuestion::read(&input))
                .flatten();
                let (ask, asked) = match questions {
                    Some((title, questions)) => (
                        ApprovalAsk::Questions { title, questions },
                        Asked::Questions { input },
                    ),
                    None => (
                        Self::ask(tool, request, &input),
                        Asked::Permission {
                            input,
                            suggestions: request
                                .get("permission_suggestions")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default(),
                        },
                    ),
                };
                self.approvals.ask(approval.clone(), asked);
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
                    kind: ItemKind::UserMessage.into(),
                    read: ItemRead::Prompt {
                        text,
                        images: self.echoing.pop(),
                    },
                    body: DriveItem::without_image_bytes(message["message"].clone()),
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
                ItemRead::Command {
                    command, intent, ..
                } => ItemRead::Command {
                    exit_code: failed.then(|| Self::exit_code(output.as_deref())).flatten(),
                    command,
                    intent,
                    status: Stored::Known(status),
                    output,
                },
                ItemRead::FileChange { paths, .. } => ItemRead::FileChange {
                    paths,
                    status: Stored::Known(status),
                },
                ItemRead::Tool { name, effects, .. } => ItemRead::Tool {
                    name,
                    effects,
                    status: Stored::Known(status),
                },
                ItemRead::Questions { questions, .. } => ItemRead::Questions {
                    questions,
                    answers: AskUserQuestion::answers(&message["tool_use_result"]),
                    status: Stored::Known(status),
                },
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
        if let Some(started) = self.turn.start(turn) {
            received.events.push(started);
        }
    }

    /// A turn Claude Code started on its own (after background work), which
    /// no prompt's echo announced.
    fn unprompted(&mut self, message: &Value, received: &mut Received) {
        if self.turn == TurnSlot::Idle
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
                            kind: ItemKind::AgentMessage.into(),
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
                            kind: ItemKind::Reasoning.into(),
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
    fn tool(name: &str, input: &Value) -> (Stored<ItemKind>, ItemRead) {
        let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
        let other = |kind| {
            (
                kind,
                ItemRead::tool(
                    crate::Harness::ClaudeCode,
                    name,
                    input,
                    ItemStatus::InProgress.into(),
                ),
            )
        };
        match ClaudeCodeTool::from_label(name) {
            Some(ClaudeCodeTool::Bash) => (
                ItemKind::CommandExecution.into(),
                ItemRead::Command {
                    command: text("command").unwrap_or_default(),
                    intent: CommandIntent::described(
                        input.get("description").and_then(Value::as_str),
                    ),
                    status: ItemStatus::InProgress.into(),
                    output: None,
                    exit_code: None,
                },
            ),
            Some(
                ClaudeCodeTool::Edit
                | ClaudeCodeTool::Write
                | ClaudeCodeTool::MultiEdit
                | ClaudeCodeTool::NotebookEdit,
            ) => (
                ItemKind::FileChange.into(),
                ItemRead::FileChange {
                    paths: text("file_path")
                        .or_else(|| text("notebook_path"))
                        .into_iter()
                        .collect(),
                    status: ItemStatus::InProgress.into(),
                },
            ),
            Some(ClaudeCodeTool::WebSearch) => other(ItemKind::WebSearch.into()),
            Some(ClaudeCodeTool::AskUserQuestion) => match AskUserQuestion::read(input) {
                Some((_, questions)) => (
                    Stored::from_label(name),
                    ItemRead::Questions {
                        questions,
                        answers: BTreeMap::new(),
                        status: ItemStatus::InProgress.into(),
                    },
                ),
                None => other(Stored::from_label(name)),
            },
            Some(
                ClaudeCodeTool::Read
                | ClaudeCodeTool::Glob
                | ClaudeCodeTool::Grep
                | ClaudeCodeTool::Task
                | ClaudeCodeTool::Agent
                | ClaudeCodeTool::WebFetch,
            ) => other(Stored::from_label(name)),
            None if name.starts_with("mcp__") => other(ItemKind::McpToolCall.into()),
            None => other(Stored::from_label(name)),
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
        let Some((turn, stop)) = self.turn.end() else {
            // A result for no turn Ren knows of: one that failed is the
            // client's error before any turn (a conversation it could not
            // resume); anything else stays unknown.
            let failed = message.get("is_error").and_then(Value::as_bool) == Some(true);
            match Self::failure(message) {
                Some(message) if failed => received.events.push(DriveEvent::Error {
                    message,
                    retry: Retry::Final,
                }),
                Some(_) | None => received.events.push(DriveEvent::Unknown { line }),
            }
            return;
        };
        self.tools.clear();
        self.blocks.clear();
        self.streaming = None;
        self.approvals.withdraw(received);
        let text = |key: &str| message.get(key).and_then(Value::as_str);
        let aborted = text("terminal_reason").is_some_and(|reason| reason.starts_with("aborted"));
        let failed = message.get("is_error").and_then(Value::as_bool) == Some(true);
        let end = if stop == Stop::Asked || aborted {
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
        let subtype = field("subtype")
            .and_then(Value::as_str)
            .map(Stored::<ClaudeSystemSubtype>::from_label);
        match subtype {
            Some(Stored::Known(ClaudeSystemSubtype::ApiRetry)) => {
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
            Some(Stored::Known(
                ClaudeSystemSubtype::Init
                | ClaudeSystemSubtype::Status
                | ClaudeSystemSubtype::ThinkingTokens
                | ClaudeSystemSubtype::TaskSummary
                | ClaudeSystemSubtype::PostTurnSummary
                | ClaudeSystemSubtype::HookStarted
                | ClaudeSystemSubtype::HookProgress
                | ClaudeSystemSubtype::HookResponse,
            )) => {}
            Some(Stored::Unknown(_)) | None => received.events.push(DriveEvent::Unknown { line }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClaudeStreamJson, UNANSWERED};
    use crate::{
        drive::{
            ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, ItemKind,
            ItemPhase, ItemRead, ItemStatus, Opening, Received, RequestKind, Retry, TurnEnd,
            Unreadable,
        },
        lines::{Framed, Line},
        prompt::Prompt,
    };
    use ren_protocol::label::Stored;
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
        DriveCommand::Prompt(Prompt::try_from(text.to_owned()).unwrap())
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
                    "request_id": "ren-0", "response": { "commands": [
                        { "name": "review", "description": "Review the diff",
                          "argumentHint": "<PR or path>" },
                        { "name": "release-notes", "description": "", "argumentHint": "",
                          "builtin": true },
                        { "description": "no name" }
                    ] } } }),
        );
        assert_eq!(
            received.events,
            vec![DriveEvent::Ready { thread: session() }]
        );
        codec
    }

    #[test]
    fn lists_the_commands_claude_code_opened_with() {
        use crate::commands::{ClientCommand, CommandKind, Sigil};
        assert_eq!(
            ready().commands(),
            [
                ClientCommand {
                    name: "review".into(),
                    description: "Review the diff".into(),
                    hint: Some("<PR or path>".into()),
                    kind: CommandKind::Command,
                    sigil: Sigil::Slash,
                },
                ClientCommand {
                    name: "release-notes".into(),
                    description: String::new(),
                    hint: None,
                    kind: CommandKind::Command,
                    sigil: Sigil::Slash,
                },
            ]
        );
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
    fn sends_images_as_content_blocks_and_shows_them_on_the_echo() {
        use crate::prompt::PromptImage;
        let mut codec = ready();
        let image = PromptImage::sample();
        let line = codec
            .command(DriveCommand::Prompt(Prompt::sample("what is this")))
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&line)["message"]["content"],
            json!([
                { "type": "image", "source": { "type": "base64", "media_type": "image/png",
                  "data": image.base64() } },
                { "type": "text", "text": "what is this" },
            ])
        );
        let echoed = feed(
            &mut codec,
            json!({ "type": "user", "message": { "role": "user", "content": [
                        { "type": "image", "source": { "type": "base64", "media_type": "image/png",
                          "data": image.base64() } },
                        { "type": "text", "text": "what is this" } ] },
                    "session_id": SESSION, "parent_tool_use_id": null, "uuid": PROMPT,
                    "isReplay": true }),
        );
        let [_, DriveEvent::Item { item, .. }] = echoed.events.as_slice() else {
            panic!("{:?}", echoed.events);
        };
        assert_eq!(
            item.read,
            ItemRead::Prompt {
                text: "what is this".into(),
                images: vec![image.read().clone()],
            }
        );
        assert!(!item.body.to_string().contains(&image.base64()));
        // Images alone go without a text block.
        let _ = feed(
            &mut codec,
            json!({ "type": "result", "subtype": "success", "is_error": false }),
        );
        let alone = codec
            .command(DriveCommand::Prompt(Prompt::sample("")))
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&alone)["message"]["content"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn runs_a_turn_with_streamed_text_and_a_command() {
        let mut codec = ready();
        let line = codec.command(prompt("run the tests")).unwrap().remove(0);
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
                && item.read == ItemRead::Prompt { text: "run the tests".into(), images: Vec::new() }
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
                if item.kind == ItemKind::CommandExecution.into()
                    && item.read == ItemRead::Command {
                        command: "cargo test".into(),
                        intent: crate::drive::CommandIntent::Described { text: "Run tests".into() },
                        status: ItemStatus::InProgress.into(),
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
                    intent: crate::drive::CommandIntent::Described { text: "Run tests".into() },
                    status: ItemStatus::Failed.into(),
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
    fn asks_ask_user_question_as_questions_and_answers_with_replies() {
        use crate::question::{QuestionKey, Replies, Reply};
        let mut codec = ready();
        codec.command(prompt("plan it")).unwrap().remove(0);
        let _ = echo(&mut codec, "plan it");
        let input = json!({ "questions": [
            { "question": "Which store?", "header": "Store", "multiSelect": false,
              "options": [{ "label": "SQLite", "description": "One file" },
                          { "label": "Postgres", "description": "A server" }] }] });
        let call = feed(
            &mut codec,
            json!({ "type": "assistant", "message": { "id": "msg_9", "role": "assistant",
                    "content": [{ "type": "tool_use", "id": "toolu_9",
                                  "name": "AskUserQuestion", "input": input }] },
                    "parent_tool_use_id": null, "session_id": SESSION }),
        );
        assert!(matches!(
            call.events.as_slice(),
            [DriveEvent::Item { item, .. }]
                if matches!(&item.read, ItemRead::Questions { questions, answers, .. }
                    if questions.len() == 1 && answers.is_empty())
        ));
        let asked = feed(
            &mut codec,
            json!({ "type": "control_request", "request_id": "perm-9", "request": {
                "subtype": "can_use_tool", "tool_name": "AskUserQuestion",
                "input": input, "tool_use_id": "toolu_9" } }),
        );
        let approval = ApprovalId("perm-9".into());
        let [
            DriveEvent::Approval {
                ask: ApprovalAsk::Questions { questions, .. },
                ..
            },
        ] = asked.events.as_slice()
        else {
            panic!("not questions: {:?}", asked.events);
        };
        assert_eq!(questions[0].text, "Which store?");
        // Questions are answered with replies, not allowed.
        assert_eq!(
            codec.command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::Accept,
            }),
            Err(CommandRefused::Unanswered(approval.clone()))
        );
        let replies = Replies::new(
            [(
                QuestionKey::new("Which store?".into()),
                Reply::Chose {
                    picked: vec!["SQLite".into()],
                    written: None,
                },
            )]
            .into(),
        )
        .fit(questions)
        .unwrap();
        let answer = codec
            .command(DriveCommand::Reply {
                approval: approval.clone(),
                replies,
            })
            .unwrap()
            .remove(0);
        let mut answered = input.clone();
        answered["answers"] = json!({ "Which store?": "SQLite" });
        assert_eq!(
            sent(&answer),
            json!({ "type": "control_response", "response": { "subtype": "success",
                "request_id": "perm-9", "response": {
                    "behavior": "allow", "updatedInput": answered } } })
        );
        // The call's result shows what was answered.
        let result = feed(
            &mut codec,
            json!({ "type": "user", "message": { "role": "user", "content": [
                      { "type": "tool_result", "tool_use_id": "toolu_9",
                        "content": "The user answered: Which store? = SQLite" }] },
                    "parent_tool_use_id": null, "session_id": SESSION,
                    "tool_use_result": { "questions": input["questions"],
                                         "answers": { "Which store?": "SQLite" } } }),
        );
        assert!(matches!(
            result.events.as_slice(),
            [DriveEvent::Item { item, .. }]
                if matches!(&item.read, ItemRead::Questions { answers, status, .. }
                    if answers.get(&QuestionKey::new("Which store?".into()))
                        == Some(&"SQLite".to_owned())
                        && *status == Stored::Known(ItemStatus::Completed))
        ));

        // Declined, Claude Code hears the operator chose not to answer.
        let _ = feed(
            &mut codec,
            json!({ "type": "control_request", "request_id": "perm-10", "request": {
                "subtype": "can_use_tool", "tool_name": "AskUserQuestion",
                "input": input, "tool_use_id": "toolu_10" } }),
        );
        let declined = codec
            .command(DriveCommand::Answer {
                approval: ApprovalId("perm-10".into()),
                decision: Decision::Decline,
            })
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&declined)["response"]["response"],
            json!({ "behavior": "deny", "message": UNANSWERED })
        );
        // Replies answer only questions.
        let _ = feed(
            &mut codec,
            json!({ "type": "control_request", "request_id": "perm-11", "request": {
                "subtype": "can_use_tool", "tool_name": "Bash",
                "input": { "command": "ls" } } }),
        );
        assert_eq!(
            codec.command(DriveCommand::Reply {
                approval: ApprovalId("perm-11".into()),
                replies: Replies::default().fit(&[]).unwrap(),
            }),
            Err(CommandRefused::NotQuestions(ApprovalId("perm-11".into())))
        );
    }

    #[test]
    fn answers_permission_prompts_with_the_operators_decision() {
        let mut codec = ready();
        codec.command(prompt("write it")).unwrap().remove(0);
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
            .unwrap()
            .remove(0);
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
            .unwrap()
            .remove(0);
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
    fn a_failed_result_before_any_turn_is_an_error() {
        let mut codec = ready();
        let failed = feed(
            &mut codec,
            json!({ "type": "result", "subtype": "error_during_execution", "is_error": true,
                    "errors": ["No conversation found with session ID: x"] }),
        );
        assert!(matches!(
            failed.events.as_slice(),
            [DriveEvent::Error { message, retry: Retry::Final }]
                if message == "No conversation found with session ID: x"
        ));
        let other = feed(
            &mut codec,
            json!({ "type": "result", "subtype": "success", "is_error": false }),
        );
        assert!(matches!(
            other.events.as_slice(),
            [DriveEvent::Unknown { .. }]
        ));
    }

    #[test]
    fn interrupts_and_resolves_withdrawn_or_unanswered_prompts() {
        let mut codec = ready();
        codec.command(prompt("go")).unwrap().remove(0);
        let _ = echo(&mut codec, "go");
        let interrupt = codec.command(DriveCommand::Interrupt).unwrap().remove(0);
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

    /// A setting is Claude Code's once it answers; one it refuses is sent
    /// again when asked for again.
    #[test]
    fn a_refused_setting_is_not_kept() {
        use crate::settings::RunSettings;
        let mut codec = ready();
        let small = || {
            DriveCommand::Configure(RunSettings {
                model: Some("small".parse().unwrap()),
                ..RunSettings::default()
            })
        };
        let first = sent(&codec.command(small()).unwrap()[0]);
        let refused = feed(
            &mut codec,
            json!({ "type": "control_response", "response": { "subtype": "error",
                    "request_id": first["request_id"], "error": "no such model" } }),
        );
        assert_eq!(
            refused.events,
            vec![DriveEvent::Refused {
                request: RequestKind::Configure,
                message: "no such model".into()
            }]
        );
        let again = codec.command(small()).unwrap();
        assert_eq!(
            sent(&again[0])["request"],
            json!({ "subtype": "set_model", "model": "small" })
        );
        // Taken this time: asking again sends nothing.
        let _ = feed(
            &mut codec,
            json!({ "type": "control_response", "response": { "subtype": "success",
                    "request_id": sent(&again[0])["request_id"], "response": {} } }),
        );
        assert_eq!(codec.command(small()), Ok(Vec::new()));
    }

    /// Claude Code switches model and permission mode mid-run; it has no
    /// way to change effort or add to a running turn.
    #[test]
    fn switches_model_and_permission_mode_but_not_effort_or_steering() {
        use crate::settings::{ApprovalPolicy, Effort, Midway, RunSettings};
        let mut codec = ready();
        assert_eq!(
            codec.command(DriveCommand::Steer(
                Prompt::try_from("x".to_owned()).unwrap()
            )),
            Err(CommandRefused::NotOffered(Midway::Steer))
        );
        let lines = codec
            .command(DriveCommand::Configure(RunSettings {
                model: Some("small".parse().unwrap()),
                effort: None,
                approvals: Some(ApprovalPolicy::Edits),
            }))
            .unwrap();
        let requests: Vec<Value> = lines
            .iter()
            .map(|line| sent(line)["request"].clone())
            .collect();
        assert_eq!(
            requests,
            vec![
                json!({ "subtype": "set_model", "model": "small" }),
                json!({ "subtype": "set_permission_mode", "mode": "acceptEdits" }),
            ]
        );
        // Unchanged settings send nothing; back to the client's own model.
        let back = codec
            .command(DriveCommand::Configure(RunSettings {
                model: None,
                effort: None,
                approvals: Some(ApprovalPolicy::Edits),
            }))
            .unwrap();
        assert_eq!(
            sent(&back[0])["request"],
            json!({ "subtype": "set_model", "model": "default" })
        );
        assert_eq!(
            codec.command(DriveCommand::Configure(RunSettings {
                model: None,
                effort: Some(Effort::High),
                approvals: Some(ApprovalPolicy::Edits),
            })),
            Err(CommandRefused::NotOffered(Midway::Effort))
        );
    }
}
