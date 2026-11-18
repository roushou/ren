//! Pi's RPC mode (`pi --mode rpc`): one JSON object per line on both sides,
//! each with a `type`. Read against the RPC and JSON event documentation of
//! `@earendil-works/pi-coding-agent` 1.5.2.
//!
//! Ren names the session itself (`--session-id`, which opens the run's
//! folder's session of that id or creates it), so the session is open once
//! Pi answers `get_state`. A prompt is a `prompt` command; Pi's run starts
//! with `agent_start` and, after any retries, compaction, or follow-up work,
//! ends with `agent_settled`, which is the end of Ren's turn. `abort`
//! interrupts it.
//!
//! Pi names no turns and asks for no approvals: it runs its tools without
//! asking. A driven turn's id is minted from the seed the process was opened
//! with. Dialogs an extension opens (`extension_ui_request`) are cancelled:
//! Ren provides no interface for them.

use super::tools::PiTool;
use crate::{
    codec::{EchoQueue, Envelope, RequestLedger, Stop, TurnMint, TurnSlot},
    commands::{ClientCommand, CommandKind, Sigil},
    drive::{
        CommandIntent, CommandRefused, DriveCommand, DriveEvent, DriveItem, ItemId, ItemKind,
        ItemPhase, ItemRead, ItemStatus, Opening, RawLine, Received, RequestKind, Retry, TurnEnd,
        Unreadable,
    },
    lines::{Framed, Line},
    prompt::Prompt,
    settings::{Midway, RunSettings, Setting, SettingsBook},
};
use ren_protocol::{
    label::{Labeled, Stored},
    native::NativeThreadId,
};
use serde_json::{Value, json};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::collections::BTreeMap;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use uuid::Uuid;

/// The client's side of one `pi --mode rpc` process.
#[derive(Debug)]
pub struct PiRpc {
    phase: Phase,
    /// What the session runs on, and what Ren asked it to change to.
    settings: SettingsBook,
    /// Pi names no turns: each is minted from the process's seed.
    turns: TurnMint,
    /// Commands Ren sent and waits on, by request id.
    requests: RequestLedger<String, Pending>,
    turn: TurnSlot,
    /// Assistant messages started in this process, naming their items.
    messages: u64,
    /// The assistant message whose blocks are streaming.
    streaming: Option<u64>,
    /// How the newest assistant message of the turn stopped.
    stopped: Option<Stopped>,
    /// Tool calls started in this turn, by tool call id, until their end.
    tools: BTreeMap<String, ToolCall>,
    /// The extension commands, prompt templates, and skills Pi listed.
    commands: Vec<ClientCommand>,
    /// What the images of each prompt and steer sent are, in order, until
    /// Pi echoes it as a user message.
    echoing: EchoQueue,
}

/// Whether the session Ren named is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// `get_state` was sent; Pi has not answered.
    Opening(NativeThreadId),
    /// Open, under the session Pi reported.
    Ready(NativeThreadId),
}

/// Every message type Pi writes in RPC mode; a type Ren does not know stays
/// unknown.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PiEvent {
    Response,
    AgentStart,
    AgentSettled,
    MessageStart,
    MessageUpdate,
    MessageEnd,
    ToolExecutionStart,
    ToolExecutionEnd,
    AutoRetryStart,
    AutoRetryEnd,
    ExtensionError,
    ExtensionUiRequest,
    TurnStart,
    TurnEnd,
    AgentEnd,
    ToolExecutionUpdate,
    BashExecutionUpdate,
    QueueUpdate,
    CompactionStart,
    CompactionEnd,
    SummarizationRetryScheduled,
    SummarizationRetryAttemptStart,
    SummarizationRetryFinished,
    EntryAppended,
    SessionInfoChanged,
    ThinkingLevelChanged,
}

/// A command Ren sent, and what it was for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Request(RequestKind),
    /// A setting changed mid-run, committed once Pi takes it.
    Configure(Setting),
}

impl Pending {
    fn kind(&self) -> RequestKind {
        match self {
            Self::Request(kind) => *kind,
            Self::Configure(_) => RequestKind::Configure,
        }
    }
}

/// How an assistant message stopped, where it says the turn did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stopped {
    Error(Option<String>),
    Aborted,
}

#[derive(Debug)]
struct ToolCall {
    kind: ItemKind,
    read: ItemRead,
    start: Value,
}

impl PiRpc {
    /// The arguments that put Pi in this mode on `opening`'s session, after
    /// the extension arguments of a launched Pi.
    pub fn arguments(opening: &Opening) -> Vec<String> {
        let (thread, settings) = match opening {
            Opening::New { id, settings, .. } => (id, settings),
            Opening::Resume {
                thread, settings, ..
            } => (thread, settings),
        };
        // `--session-id` keeps the session in the run's folder; `--session`
        // would move to the folder the session was started in.
        let mut arguments = vec![
            "--mode".into(),
            "rpc".into(),
            "--session-id".into(),
            thread.to_string(),
        ];
        if let Some(model) = &settings.model {
            arguments.extend(["--model".into(), model.to_string()]);
        }
        // Pi asks for no approvals, so a policy changes nothing.
        if let Some(effort) = settings.effort {
            arguments.extend(["--thinking".into(), effort.to_string()]);
        }
        arguments
    }

    /// A codec for a freshly started process, and the first line to send it.
    /// `seed` is a fresh id the process's turn ids are minted from.
    pub fn open(opening: &Opening, seed: Uuid) -> (Self, Line) {
        let (thread, kind, settings) = match opening {
            Opening::New { id, settings, .. } => (*id, RequestKind::ThreadStart, settings.clone()),
            Opening::Resume {
                thread, settings, ..
            } => (*thread, RequestKind::ThreadResume, settings.clone()),
        };
        let mut codec = Self {
            phase: Phase::Opening(thread),
            settings: SettingsBook::new(settings),
            turns: TurnMint::new(seed),
            requests: RequestLedger::new(|n| format!("ren-{n}")),
            turn: TurnSlot::Idle,
            messages: 0,
            streaming: None,
            stopped: None,
            tools: BTreeMap::new(),
            commands: Vec::new(),
            echoing: EchoQueue::default(),
        };
        let line = codec.request(kind, json!({ "type": "get_state" }));
        (codec, line)
    }

    /// The session, once Pi reported it open.
    pub fn thread(&self) -> Option<NativeThreadId> {
        match self.phase {
            Phase::Ready(thread) => Some(thread),
            Phase::Opening(_) => None,
        }
    }

    /// The commands a prompt can run, as Pi listed them once the session
    /// opened: each is `/name` in the prompt (a skill's name starts
    /// `skill:`).
    pub fn commands(&self) -> &[ClientCommand] {
        &self.commands
    }

    /// The commands in Pi's answer to `get_commands`.
    pub(crate) fn listed(data: &Value) -> Vec<ClientCommand> {
        data.get("commands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|command| {
                let name = command.get("name").and_then(Value::as_str)?.trim();
                if name.is_empty() {
                    return None;
                }
                let kind = match command.get("source").and_then(Value::as_str) {
                    Some("skill") => CommandKind::Skill,
                    Some("prompt") => CommandKind::Template,
                    // Extension commands, and any source Pi adds later.
                    Some(_) | None => CommandKind::Command,
                };
                Some(ClientCommand {
                    name: name.to_owned(),
                    description: Self::text(command, "description")
                        .map(|text| text.trim().to_owned())
                        .unwrap_or_default(),
                    hint: None,
                    kind,
                    sigil: Sigil::Slash,
                })
            })
            .collect()
    }

    /// The line that carries `command`, or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Vec<Line>, CommandRefused> {
        if let Phase::Opening(_) = self.phase {
            return Err(CommandRefused::NotReady);
        }
        match command {
            DriveCommand::Prompt(prompt) => self.prompt(&prompt).map(|line| vec![line]),
            DriveCommand::Interrupt => {
                self.turn.stop()?;
                Ok(vec![self.request(
                    RequestKind::TurnInterrupt,
                    json!({ "type": "abort" }),
                )])
            }
            // Pi asks for no approvals.
            DriveCommand::Answer { approval, .. } => Err(CommandRefused::UnknownApproval(approval)),
            // Read after the running assistant message's tool calls, before
            // its next model call.
            DriveCommand::Steer(prompt) => {
                self.turn.running()?;
                let command = self.message("steer", &prompt);
                Ok(vec![self.request(RequestKind::TurnSteer, command)])
            }
            DriveCommand::Configure(settings) => self.configure(settings),
        }
    }

    /// Read one frame from Pi's stdout.
    pub fn receive(&mut self, frame: Framed) -> Received {
        let envelope = match Envelope::open(frame) {
            Ok(envelope) => envelope,
            Err(received) => return received,
        };
        let Some(kind) = envelope.typed::<PiEvent>() else {
            return Received::event(envelope.untyped());
        };
        let Envelope {
            line,
            body: message,
        } = envelope;
        let mut received = Received::default();
        let Stored::Known(kind) = kind else {
            received.events.push(DriveEvent::Unknown { line });
            return received;
        };
        match kind {
            PiEvent::Response => self.response(&message, line, &mut received),
            PiEvent::AgentStart => self.agent_start(&mut received),
            PiEvent::AgentSettled => self.settled(&message, line, &mut received),
            PiEvent::MessageStart => self.message_start(&message, &mut received),
            PiEvent::MessageUpdate => self.message_update(&message, &mut received),
            PiEvent::MessageEnd => self.message_end(&message, &mut received),
            PiEvent::ToolExecutionStart => self.tool_start(&message, line, &mut received),
            PiEvent::ToolExecutionEnd => self.tool_end(&message, &mut received),
            PiEvent::AutoRetryStart => received.events.push(DriveEvent::Error {
                message: Self::retrying(&message),
                retry: Retry::WillRetry,
            }),
            PiEvent::AutoRetryEnd => {
                if message.get("success").and_then(Value::as_bool) == Some(false) {
                    received.events.push(DriveEvent::Error {
                        message: Self::text(&message, "finalError")
                            .unwrap_or_else(|| "the request failed after retrying".into()),
                        retry: Retry::Final,
                    });
                }
            }
            PiEvent::ExtensionError => received.events.push(DriveEvent::Error {
                message: format!(
                    "extension {}: {}",
                    Self::text(&message, "extensionPath").unwrap_or_default(),
                    Self::text(&message, "error").unwrap_or_default(),
                ),
                retry: Retry::Final,
            }),
            PiEvent::ExtensionUiRequest => Self::dialog(&message, line, &mut received),
            // Known, and nothing Ren shows: Pi's inner model turns, the end
            // of one low-level run (more may follow until it settles),
            // streamed tool output, queues, compaction, and session metadata.
            PiEvent::TurnStart
            | PiEvent::TurnEnd
            | PiEvent::AgentEnd
            | PiEvent::ToolExecutionUpdate
            | PiEvent::BashExecutionUpdate
            | PiEvent::QueueUpdate
            | PiEvent::CompactionStart
            | PiEvent::CompactionEnd
            | PiEvent::SummarizationRetryScheduled
            | PiEvent::SummarizationRetryAttemptStart
            | PiEvent::SummarizationRetryFinished
            | PiEvent::EntryAppended
            | PiEvent::SessionInfoChanged
            | PiEvent::ThinkingLevelChanged => {}
        }
        received
    }

    fn prompt(&mut self, prompt: &Prompt) -> Result<Line, CommandRefused> {
        self.turn.request(())?;
        let command = self.message("prompt", prompt);
        Ok(self.request(RequestKind::TurnStart, command))
    }

    /// A `prompt` or `steer` command: its text, and each image in base64,
    /// remembered for the message's echo.
    fn message(&mut self, kind: &str, prompt: &Prompt) -> Value {
        let mut command = json!({ "type": kind, "message": prompt.as_str() });
        self.echoing.push(prompt);
        if !prompt.images().is_empty() {
            command["images"] = prompt
                .images()
                .iter()
                .map(|image| {
                    json!({ "type": "image", "data": image.base64(), "mimeType": image.media_type() })
                })
                .collect();
        }
        command
    }

    fn request(&mut self, kind: RequestKind, command: Value) -> Line {
        self.send(Pending::Request(kind), command)
    }

    fn send(&mut self, pending: Pending, mut command: Value) -> Line {
        let id = self.requests.send(pending);
        if let Value::Object(fields) = &mut command {
            fields.insert("id".into(), id.into());
        }
        Line::json(&command)
    }

    fn text(value: &Value, key: &str) -> Option<String> {
        value.get(key).and_then(Value::as_str).map(str::to_owned)
    }

    fn response(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let Some(pending) = message
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| self.requests.answer(id))
        else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::UnexpectedResponse,
            });
            return;
        };
        let kind = pending.kind();
        if message.get("success").and_then(Value::as_bool) != Some(true) {
            if let Pending::Configure(setting) = &pending {
                self.settings.refused(setting);
            }
            // Without its commands a run goes on; the list stays empty.
            if kind == RequestKind::ListCommands {
                return;
            }
            if kind == RequestKind::TurnStart {
                self.turn.withdraw();
            }
            received.events.push(DriveEvent::Refused {
                request: kind,
                message: Self::text(message, "error").unwrap_or_else(|| "refused".into()),
            });
            return;
        }
        let data = &message["data"];
        if let Pending::Configure(setting) = pending {
            self.settings.acknowledge(setting);
            return;
        }
        match kind {
            RequestKind::ThreadStart | RequestKind::ThreadResume => {
                // Pi opened the session Ren named; should it report another,
                // that is the one its model traffic is recorded under.
                let (Phase::Opening(named) | Phase::Ready(named)) = self.phase;
                let thread = data
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .and_then(|id| Uuid::try_parse(id).ok())
                    .and_then(NativeThreadId::new)
                    .unwrap_or(named);
                self.phase = Phase::Ready(thread);
                received.events.push(DriveEvent::Ready { thread });
                let ask =
                    self.request(RequestKind::ListCommands, json!({ "type": "get_commands" }));
                received.replies.push(ask);
            }
            RequestKind::ListCommands => self.commands = Self::listed(data),
            RequestKind::TurnStart => {
                // An extension consumed the prompt: no run starts for it.
                if data.get("disposition").and_then(Value::as_str) == Some("handled")
                    && self.turn.withdraw().is_some()
                {
                    received.events.push(DriveEvent::Refused {
                        request: kind,
                        message: "an extension handled the prompt; no turn ran".into(),
                    });
                }
            }
            RequestKind::TurnInterrupt
            | RequestKind::Initialize
            | RequestKind::TurnSteer
            | RequestKind::Configure => {}
        }
    }

    /// The commands that move the session to `settings`: Pi switches model
    /// and thinking level mid-run, and asks for no approvals. Neither goes
    /// back to "Pi's own" once chosen: it keeps the last one set.
    /// Each is committed once Pi answers it.
    fn configure(&mut self, settings: RunSettings) -> Result<Vec<Line>, CommandRefused> {
        let change = self
            .settings
            .change_to(&settings, crate::Harness::Pi.profile().midway)?;
        let commands = change
            .into_iter()
            .map(|setting| {
                let command = match &setting {
                    Setting::Model(model) => {
                        let (provider, id) = model
                            .as_ref()
                            .and_then(|model| model.as_str().split_once('/'))
                            .ok_or(CommandRefused::NotOffered(Midway::Model))?;
                        json!({ "type": "set_model", "provider": provider, "modelId": id })
                    }
                    Setting::Effort(effort) => {
                        let level = effort.ok_or(CommandRefused::NotOffered(Midway::Effort))?;
                        json!({ "type": "set_thinking_level", "level": level.to_string() })
                    }
                    Setting::Approvals(_) => {
                        return Err(CommandRefused::NotOffered(Midway::Approvals));
                    }
                };
                Ok((setting, command))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(commands
            .into_iter()
            .map(|(setting, command)| {
                self.settings.ask(setting.clone());
                self.send(Pending::Configure(setting), command)
            })
            .collect())
    }

    fn agent_start(&mut self, received: &mut Received) {
        // A retry or follow-up run within the turn starts no new one.
        if !self.turn.under_way() {
            let turn = self.turns.mint();
            received.events.extend(self.turn.start(turn));
        }
    }

    fn settled(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let Some((turn, stop)) = self.turn.end() else {
            // Settled with no turn Ren knows of.
            received.events.push(DriveEvent::Unknown { line });
            return;
        };
        self.streaming = None;
        self.tools.clear();
        let aborted = message.get("aborted").and_then(Value::as_bool) == Some(true);
        let end = match (stop == Stop::Asked || aborted, self.stopped.take()) {
            (true, _) | (false, Some(Stopped::Aborted)) => TurnEnd::Interrupted,
            (false, Some(Stopped::Error(message))) => TurnEnd::Failed { message },
            (false, None) => TurnEnd::Completed,
        };
        received.events.push(DriveEvent::TurnEnded { turn, end });
    }

    /// An item id within the running turn.
    fn item(&self, name: String) -> ItemId {
        self.turn.scope(name)
    }

    fn message_start(&mut self, message: &Value, received: &mut Received) {
        let body = &message["message"];
        match body.get("role").and_then(Value::as_str) {
            Some("user") => {
                let text = match &body["content"] {
                    Value::String(text) => text.clone(),
                    Value::Array(parts) => parts
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect(),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
                        String::new()
                    }
                };
                let stamp = body.get("timestamp").and_then(Value::as_u64).unwrap_or(0);
                // A message echoing images is the oldest one sent with them.
                let with_images = body["content"].as_array().is_some_and(|parts| {
                    parts
                        .iter()
                        .any(|part| part.get("type").and_then(Value::as_str) == Some("image"))
                });
                let images = match with_images {
                    true => self.echoing.pop(),
                    false => Vec::new(),
                };
                received.events.push(DriveEvent::Item {
                    phase: ItemPhase::Completed,
                    item: DriveItem {
                        id: self.item(format!("user:{stamp}")),
                        kind: ItemKind::UserMessage,
                        read: ItemRead::Prompt { text, images },
                        body: DriveItem::without_image_bytes(body.clone()),
                    },
                });
            }
            Some("assistant") => {
                self.streaming = Some(self.messages);
                self.messages += 1;
            }
            Some(_) | None => {}
        }
    }

    fn block(&self, message: u64, kind: &str, index: u64) -> ItemId {
        self.item(format!("assistant:{message}:{kind}:{index}"))
    }

    fn message_update(&self, message: &Value, received: &mut Received) {
        let event = &message["assistantMessageEvent"];
        if let (Some(streaming), Some("text_delta"), Some(index), Some(delta)) = (
            self.streaming,
            event.get("type").and_then(Value::as_str),
            event.get("contentIndex").and_then(Value::as_u64),
            event.get("delta").and_then(Value::as_str),
        ) {
            received.events.push(DriveEvent::Delta {
                item: self.block(streaming, "text", index),
                text: delta.to_owned(),
            });
        }
    }

    fn message_end(&mut self, message: &Value, received: &mut Received) {
        let body = &message["message"];
        if body.get("role").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(streaming) = self.streaming.take() else {
            return;
        };
        self.stopped = match body.get("stopReason").and_then(Value::as_str) {
            Some("error") => Some(Stopped::Error(Self::text(body, "errorMessage"))),
            Some("aborted") => Some(Stopped::Aborted),
            Some(_) | None => None,
        };
        let Some(blocks) = body.get("content").and_then(Value::as_array) else {
            return;
        };
        for (index, block) in (0u64..).zip(blocks) {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => received.events.push(DriveEvent::Item {
                    phase: ItemPhase::Completed,
                    item: DriveItem {
                        id: self.block(streaming, "text", index),
                        kind: ItemKind::AgentMessage,
                        read: ItemRead::Reply {
                            text: Self::text(block, "text").unwrap_or_default(),
                        },
                        body: block.clone(),
                    },
                }),
                Some("thinking") => received.events.push(DriveEvent::Item {
                    phase: ItemPhase::Completed,
                    item: DriveItem {
                        id: self.block(streaming, "thinking", index),
                        kind: ItemKind::Reasoning,
                        read: ItemRead::Reasoning,
                        // The signature is opaque to Ren and large.
                        body: json!({ "type": "thinking" }),
                    },
                }),
                // Tool calls are shown from their execution events.
                Some(_) | None => {}
            }
        }
    }

    fn tool_start(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let (Some(id), Some(name)) = (
            message.get("toolCallId").and_then(Value::as_str),
            message.get("toolName").and_then(Value::as_str),
        ) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            });
            return;
        };
        let (kind, read) = Self::tool(name, &message["args"]);
        self.tools.insert(
            id.to_owned(),
            ToolCall {
                kind: kind.clone(),
                read: read.clone(),
                start: message.clone(),
            },
        );
        received.events.push(DriveEvent::Item {
            phase: ItemPhase::Started,
            item: DriveItem {
                id: ItemId::new(id.to_owned()),
                kind,
                read,
                body: message.clone(),
            },
        });
    }

    /// A tool call's kind and reading, from Pi's tool name.
    fn tool(name: &str, args: &Value) -> (ItemKind, ItemRead) {
        let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_owned);
        match PiTool::from_label(name) {
            Some(PiTool::Bash) => (
                ItemKind::CommandExecution,
                ItemRead::Command {
                    command: text("command").unwrap_or_default(),
                    intent: CommandIntent::Unsaid,
                    status: ItemStatus::InProgress,
                    output: None,
                    exit_code: None,
                },
            ),
            Some(PiTool::Edit | PiTool::Write) => (
                ItemKind::FileChange,
                ItemRead::FileChange {
                    paths: text("path").into_iter().collect(),
                    status: ItemStatus::InProgress,
                },
            ),
            Some(PiTool::Read | PiTool::Grep | PiTool::Find | PiTool::Ls) | None => (
                ItemKind::Other(name.to_owned()),
                ItemRead::tool(crate::Harness::Pi, name, args, ItemStatus::InProgress),
            ),
        }
    }

    fn tool_end(&mut self, message: &Value, received: &mut Received) {
        let Some(id) = message.get("toolCallId").and_then(Value::as_str) else {
            return;
        };
        let Some(call) = self.tools.remove(id) else {
            return;
        };
        let status = if message.get("isError").and_then(Value::as_bool) == Some(true) {
            ItemStatus::Failed
        } else {
            ItemStatus::Completed
        };
        let result = &message["result"];
        let read = match call.read {
            ItemRead::Command {
                command, intent, ..
            } => ItemRead::Command {
                command,
                intent,
                status,
                output: result
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                // Pi's bash tool reports no exit code apart from its text.
                exit_code: None,
            },
            ItemRead::FileChange { paths, .. } => ItemRead::FileChange { paths, status },
            ItemRead::Tool { name, effects, .. } => ItemRead::Tool {
                name,
                effects,
                status,
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
                body: json!({ "start": call.start, "end": message }),
            },
        });
    }

    fn retrying(message: &Value) -> String {
        let mut text = match Self::text(message, "errorMessage") {
            Some(error) => format!("request failed: {error}"),
            None => "request failed".to_owned(),
        };
        if let (Some(attempt), Some(max)) = (
            message.get("attempt").and_then(Value::as_u64),
            message.get("maxAttempts").and_then(Value::as_u64),
        ) {
            text = format!("{text} (retry {attempt} of {max})");
        }
        text
    }

    /// An extension's request. Dialogs wait for an answer, so Ren cancels
    /// them; notifications and status updates need none.
    fn dialog(message: &Value, line: RawLine, received: &mut Received) {
        let method = Self::text(message, "method").unwrap_or_default();
        match method.as_str() {
            "select" | "confirm" | "input" | "editor" => {
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    received.events.push(DriveEvent::Unreadable {
                        line,
                        reason: Unreadable::BadParams,
                    });
                    return;
                };
                received.replies.push(Line::json(&json!({
                    "type": "extension_ui_response",
                    "id": id,
                    "cancelled": true,
                })));
                received.events.push(DriveEvent::Unsupported { method });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PiRpc;
    use crate::{
        drive::{
            CommandRefused, DriveCommand, DriveEvent, ItemKind, ItemPhase, ItemRead, ItemStatus,
            Opening, Received, RequestKind, Retry, TurnEnd, Unreadable,
        },
        lines::{Framed, Line},
        prompt::Prompt,
    };
    use ren_protocol::native::{NativeThreadId, NativeTurnId};
    use serde_json::{Value, json};
    use uuid::Uuid;

    const SESSION: &str = "0190f3a0-0000-7000-8000-0000000000d1";

    fn sent(line: &Line) -> Value {
        let bytes = line.as_bytes();
        assert_eq!(bytes.last(), Some(&b'\n'));
        serde_json::from_slice(bytes).unwrap()
    }

    fn feed(codec: &mut PiRpc, message: Value) -> Received {
        codec.receive(Framed::Line(message.to_string().into_bytes()))
    }

    fn session() -> NativeThreadId {
        NativeThreadId::new(Uuid::parse_str(SESSION).unwrap()).unwrap()
    }

    fn seed() -> Uuid {
        Uuid::from_u128(0x0190_f3a0_0000_7000_8000_0000_0000_0e00)
    }

    fn turn(minted: u128) -> NativeTurnId {
        NativeTurnId::new(Uuid::from_u128(seed().as_u128() ^ minted)).unwrap()
    }

    fn prompt(text: &str) -> DriveCommand {
        DriveCommand::Prompt(Prompt::parse(text.into()).unwrap())
    }

    fn opening() -> Opening {
        Opening::New {
            cwd: "/work/acme".into(),
            id: session(),
            settings: crate::settings::RunSettings::default(),
        }
    }

    /// A codec whose session is open, as after `get_state`.
    fn ready() -> PiRpc {
        let (mut codec, first) = PiRpc::open(&opening(), seed());
        assert_eq!(sent(&first), json!({ "type": "get_state", "id": "ren-0" }));
        assert_eq!(codec.command(prompt("hi")), Err(CommandRefused::NotReady));
        let received = feed(
            &mut codec,
            json!({ "id": "ren-0", "type": "response", "command": "get_state", "success": true,
                    "data": { "sessionId": SESSION, "isStreaming": false, "messageCount": 0 } }),
        );
        assert_eq!(
            received.events,
            vec![DriveEvent::Ready { thread: session() }]
        );
        assert_eq!(codec.thread(), Some(session()));
        // Once open, Ren asks for Pi's commands.
        assert_eq!(
            sent(&received.replies[0]),
            json!({ "type": "get_commands", "id": "ren-1" })
        );
        let _ = feed(
            &mut codec,
            json!({ "id": "ren-1", "type": "response", "command": "get_commands", "success": true,
                    "data": { "commands": [
                        { "name": "fix-tests", "description": "Fix failing tests", "source": "prompt" },
                        { "name": "skill:release", "description": " Cut a release ", "source": "skill" },
                        { "name": "todos", "source": "extension" },
                        { "name": "", "source": "prompt" }
                    ] } }),
        );
        codec
    }

    #[test]
    fn lists_pis_commands_once_open() {
        use crate::commands::{CommandKind, Sigil};
        let codec = ready();
        let listed: Vec<_> = codec
            .commands()
            .iter()
            .map(|command| {
                (
                    command.invocation(),
                    command.description.as_str(),
                    command.kind,
                    command.sigil,
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "/fix-tests".to_owned(),
                    "Fix failing tests",
                    CommandKind::Template,
                    Sigil::Slash
                ),
                (
                    "/skill:release".to_owned(),
                    "Cut a release",
                    CommandKind::Skill,
                    Sigil::Slash
                ),
                ("/todos".to_owned(), "", CommandKind::Command, Sigil::Slash),
            ]
        );
    }

    /// A prompt sent and Pi's run started.
    fn started(codec: &mut PiRpc, text: &str) -> NativeTurnId {
        let line = codec.command(prompt(text)).unwrap().remove(0);
        let request = sent(&line);
        assert_eq!(request["type"], "prompt");
        assert_eq!(request["message"], text);
        let id = request["id"].as_str().unwrap().to_owned();
        assert_eq!(
            codec.command(prompt("again")),
            Err(CommandRefused::TurnRunning)
        );
        let accepted = feed(
            codec,
            json!({ "id": id, "type": "response", "command": "prompt", "success": true,
                    "data": { "disposition": "started" } }),
        );
        assert!(accepted.events.is_empty());
        let events = feed(codec, json!({ "type": "agent_start" })).events;
        let [DriveEvent::TurnStarted { turn }] = events.as_slice() else {
            panic!("{events:?}");
        };
        *turn
    }

    #[test]
    fn sends_images_inline_and_shows_them_on_the_echo() {
        use crate::prompt::PromptImage;
        let mut codec = ready();
        let image = PromptImage::sample();
        let line = codec
            .command(DriveCommand::Prompt(Prompt::sample("what is this")))
            .unwrap()
            .remove(0);
        let command = sent(&line);
        assert_eq!(command["type"], "prompt");
        assert_eq!(command["message"], "what is this");
        assert_eq!(
            command["images"],
            json!([{ "type": "image", "data": image.base64(), "mimeType": "image/png" }])
        );
        let echoed = feed(
            &mut codec,
            json!({ "type": "message_start", "message": { "role": "user", "timestamp": 7,
                "content": [
                    { "type": "text", "text": "what is this" },
                    { "type": "image", "data": image.base64(), "mimeType": "image/png" } ] } }),
        );
        let [DriveEvent::Item { item, .. }] = echoed.events.as_slice() else {
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
    }

    #[test]
    fn pi_is_started_in_rpc_mode_on_the_session_ren_names() {
        assert_eq!(
            PiRpc::arguments(&opening()),
            ["--mode", "rpc", "--session-id", SESSION]
        );
        // Resuming keeps the run's folder: the session of that id there.
        assert_eq!(
            PiRpc::arguments(&Opening::Resume {
                thread: session(),
                cwd: "/work/acme".into(),
                settings: crate::settings::RunSettings::default(),
            }),
            ["--mode", "rpc", "--session-id", SESSION]
        );
    }

    #[test]
    fn a_turn_streams_its_reply_runs_tools_and_ends_when_pi_settles() {
        let mut codec = ready();
        let turn = started(&mut codec, "fix the test");
        assert_eq!(turn, self::turn(1));
        let mut events = Vec::new();
        for message in [
            json!({ "type": "turn_start" }),
            json!({ "type": "message_start", "message": { "role": "user",
                    "content": [{ "type": "text", "text": "fix the test" }], "timestamp": 5 } }),
            json!({ "type": "message_end", "message": { "role": "user", "content": "fix the test" } }),
            json!({ "type": "message_start", "message": { "role": "assistant", "content": [],
                    "stopReason": "pending" } }),
            json!({ "type": "message_update", "assistantMessageEvent": { "type": "text_delta",
                    "contentIndex": 1, "delta": "Running " } }),
            json!({ "type": "message_update", "assistantMessageEvent": { "type": "thinking_delta",
                    "contentIndex": 0, "delta": "hmm" } }),
            json!({ "type": "message_end", "message": { "role": "assistant", "stopReason": "toolUse",
                    "content": [{ "type": "thinking", "thinking": "hmm", "thinkingSignature": "sig" },
                                { "type": "text", "text": "Running the tests." },
                                { "type": "toolCall", "id": "call_1", "name": "bash",
                                  "arguments": { "command": "cargo test" } }] } }),
            json!({ "type": "tool_execution_start", "toolCallId": "call_1", "toolName": "bash",
                    "args": { "command": "cargo test" } }),
            json!({ "type": "tool_execution_update", "toolCallId": "call_1", "toolName": "bash",
                    "args": {}, "partialResult": {} }),
            json!({ "type": "tool_execution_end", "toolCallId": "call_1", "toolName": "bash",
                    "result": { "content": [{ "type": "text", "text": "ok" }] }, "isError": false }),
            json!({ "type": "tool_execution_start", "toolCallId": "call_2", "toolName": "edit",
                    "args": { "path": "src/lib.rs" } }),
            json!({ "type": "tool_execution_end", "toolCallId": "call_2", "toolName": "edit",
                    "result": { "content": [] }, "isError": true }),
            json!({ "type": "tool_execution_start", "toolCallId": "call_3", "toolName": "grep",
                    "args": { "pattern": "TODO" } }),
            json!({ "type": "turn_end", "message": {}, "toolResults": [] }),
            json!({ "type": "agent_end", "messages": [], "willRetry": false }),
        ] {
            events.extend(feed(&mut codec, message).events);
        }
        let reads: Vec<(ItemPhase, ItemKind, ItemRead, String)> = events
            .iter()
            .filter_map(|event| {
                if let DriveEvent::Item { phase, item } = event {
                    Some((
                        *phase,
                        item.kind.clone(),
                        item.read.clone(),
                        item.id.as_str().to_owned(),
                    ))
                } else {
                    None
                }
            })
            .collect();
        let at = |name: &str| format!("{turn}:{name}");
        assert_eq!(
            reads,
            vec![
                (
                    ItemPhase::Completed,
                    ItemKind::UserMessage,
                    ItemRead::Prompt {
                        text: "fix the test".into(),
                        images: Vec::new(),
                    },
                    at("user:5"),
                ),
                (
                    ItemPhase::Completed,
                    ItemKind::Reasoning,
                    ItemRead::Reasoning,
                    at("assistant:0:thinking:0"),
                ),
                (
                    ItemPhase::Completed,
                    ItemKind::AgentMessage,
                    ItemRead::Reply {
                        text: "Running the tests.".into()
                    },
                    at("assistant:0:text:1"),
                ),
                (
                    ItemPhase::Started,
                    ItemKind::CommandExecution,
                    ItemRead::Command {
                        command: "cargo test".into(),
                        intent: crate::drive::CommandIntent::Unsaid,
                        status: ItemStatus::InProgress,
                        output: None,
                        exit_code: None,
                    },
                    "call_1".into(),
                ),
                (
                    ItemPhase::Completed,
                    ItemKind::CommandExecution,
                    ItemRead::Command {
                        command: "cargo test".into(),
                        intent: crate::drive::CommandIntent::Unsaid,
                        status: ItemStatus::Completed,
                        output: Some("ok".into()),
                        exit_code: None,
                    },
                    "call_1".into(),
                ),
                (
                    ItemPhase::Started,
                    ItemKind::FileChange,
                    ItemRead::FileChange {
                        paths: vec!["src/lib.rs".into()],
                        status: ItemStatus::InProgress,
                    },
                    "call_2".into(),
                ),
                (
                    ItemPhase::Completed,
                    ItemKind::FileChange,
                    ItemRead::FileChange {
                        paths: vec!["src/lib.rs".into()],
                        status: ItemStatus::Failed,
                    },
                    "call_2".into(),
                ),
                (
                    ItemPhase::Started,
                    ItemKind::Other("grep".into()),
                    // What it does is read, so its name is Pi's alone.
                    ItemRead::Tool {
                        name: "grep".into(),
                        effects: vec![crate::tools::ToolEffect {
                            kind: crate::tools::EffectKind::Search,
                            target: "TODO".into(),
                        }],
                        status: ItemStatus::InProgress,
                    },
                    "call_3".into(),
                ),
            ]
        );
        // The streamed text belongs to the reply item its whole copy names.
        assert!(events.contains(&DriveEvent::Delta {
            item: crate::drive::ItemId::new(at("assistant:0:text:1")),
            text: "Running ".into(),
        }));
        // `agent_end` is not the end: retries and follow-ups may come.
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, DriveEvent::TurnEnded { .. }))
        );
        assert!(
            feed(&mut codec, json!({ "type": "agent_start" }))
                .events
                .is_empty()
        );
        let settled = feed(
            &mut codec,
            json!({ "type": "agent_settled", "aborted": false }),
        );
        assert_eq!(
            settled.events,
            vec![DriveEvent::TurnEnded {
                turn,
                end: TurnEnd::Completed
            }]
        );
        // The next turn gets the next id.
        assert_eq!(started(&mut codec, "and the docs"), self::turn(2));
    }

    #[test]
    fn an_interrupted_or_failed_turn_says_so() {
        let mut codec = ready();
        let turn = started(&mut codec, "go");
        let abort = sent(&codec.command(DriveCommand::Interrupt).unwrap().remove(0));
        assert_eq!(abort, json!({ "type": "abort", "id": "ren-3" }));
        assert!(
            feed(
                &mut codec,
                json!({ "id": "ren-3", "type": "response", "command": "abort", "success": true }),
            )
            .events
            .is_empty()
        );
        let settled = feed(
            &mut codec,
            json!({ "type": "agent_settled", "aborted": true }),
        );
        assert_eq!(
            settled.events,
            vec![DriveEvent::TurnEnded {
                turn,
                end: TurnEnd::Interrupted
            }]
        );
        assert_eq!(
            codec.command(DriveCommand::Interrupt),
            Err(CommandRefused::NoTurn)
        );

        let turn = started(&mut codec, "again");
        let mut events = Vec::new();
        for message in [
            json!({ "type": "auto_retry_start", "attempt": 1, "maxAttempts": 3, "delayMs": 2000,
                    "errorMessage": "overloaded" }),
            json!({ "type": "message_start", "message": { "role": "assistant", "content": [] } }),
            json!({ "type": "message_end", "message": { "role": "assistant", "content": [],
                    "stopReason": "error", "errorMessage": "overloaded" } }),
            json!({ "type": "auto_retry_end", "success": false, "attempt": 3,
                    "finalError": "overloaded" }),
            json!({ "type": "agent_settled", "aborted": false }),
        ] {
            events.extend(feed(&mut codec, message).events);
        }
        assert_eq!(
            events,
            vec![
                DriveEvent::Error {
                    message: "request failed: overloaded (retry 1 of 3)".into(),
                    retry: Retry::WillRetry,
                },
                DriveEvent::Error {
                    message: "overloaded".into(),
                    retry: Retry::Final,
                },
                DriveEvent::TurnEnded {
                    turn,
                    end: TurnEnd::Failed {
                        message: Some("overloaded".into())
                    },
                },
            ]
        );
    }

    #[test]
    fn a_refused_or_handled_prompt_leaves_no_turn() {
        let mut codec = ready();
        let line = codec.command(prompt("hi")).unwrap().remove(0);
        let id = sent(&line)["id"].clone();
        let refused = feed(
            &mut codec,
            json!({ "id": id, "type": "response", "command": "prompt", "success": false,
                    "error": "No model selected" }),
        );
        assert_eq!(
            refused.events,
            vec![DriveEvent::Refused {
                request: RequestKind::TurnStart,
                message: "No model selected".into(),
            }]
        );
        let line = codec.command(prompt("/reload")).unwrap().remove(0);
        let id = sent(&line)["id"].clone();
        let handled = feed(
            &mut codec,
            json!({ "id": id, "type": "response", "command": "prompt", "success": true,
                    "data": { "disposition": "handled" } }),
        );
        assert!(matches!(
            handled.events.as_slice(),
            [DriveEvent::Refused {
                request: RequestKind::TurnStart,
                ..
            }]
        ));
        assert!(codec.command(prompt("next")).is_ok());
    }

    #[test]
    fn dialogs_are_cancelled_and_unknown_lines_kept() {
        let mut codec = ready();
        let dialog = feed(
            &mut codec,
            json!({ "type": "extension_ui_request", "id": "ui-1", "method": "confirm",
                    "title": "Allow?" }),
        );
        assert_eq!(
            dialog.events,
            vec![DriveEvent::Unsupported {
                method: "confirm".into()
            }]
        );
        assert_eq!(
            dialog.replies.iter().map(sent).collect::<Vec<_>>(),
            vec![json!({ "type": "extension_ui_response", "id": "ui-1", "cancelled": true })]
        );
        let notice = feed(
            &mut codec,
            json!({ "type": "extension_ui_request", "id": "ui-2", "method": "notify",
                    "message": "hello" }),
        );
        assert!(notice.events.is_empty() && notice.replies.is_empty());
        assert!(matches!(
            feed(&mut codec, json!({ "type": "hologram" }))
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
            feed(
                &mut codec,
                json!({ "id": "ren-99", "type": "response", "success": true })
            )
            .events
            .as_slice(),
            [DriveEvent::Unreadable {
                reason: Unreadable::UnexpectedResponse,
                ..
            }]
        ));
        // Pi asks for no approvals, so none can be answered.
        let approval = "x".parse().unwrap();
        assert_eq!(
            codec.command(DriveCommand::Answer {
                approval,
                decision: crate::drive::Decision::Accept,
            }),
            Err(CommandRefused::UnknownApproval("x".parse().unwrap()))
        );
    }

    /// Pi takes a message into the running turn, and switches model and
    /// thinking level mid-run; it asks for no approvals.
    #[test]
    fn steers_and_switches_model_and_thinking_level() {
        use crate::settings::{ApprovalPolicy, Effort, Midway, RunSettings};
        let mut codec = ready();
        let _ = started(&mut codec, "go");
        let steer = codec
            .command(DriveCommand::Steer(
                Prompt::parse("stop and test".into()).unwrap(),
            ))
            .unwrap()
            .remove(0);
        let steer = sent(&steer);
        assert_eq!(
            (steer["type"].as_str(), steer["message"].as_str()),
            (Some("steer"), Some("stop and test"))
        );
        let lines = codec
            .command(DriveCommand::Configure(RunSettings {
                model: Some("openai/gpt-fast".parse().unwrap()),
                effort: Some(Effort::Low),
                approvals: None,
            }))
            .unwrap();
        let sent_lines: Vec<Value> = lines.iter().map(sent).collect();
        assert_eq!(sent_lines[0]["type"], "set_model");
        assert_eq!(
            (
                sent_lines[0]["provider"].as_str(),
                sent_lines[0]["modelId"].as_str()
            ),
            (Some("openai"), Some("gpt-fast"))
        );
        assert_eq!(
            (
                sent_lines[1]["type"].as_str(),
                sent_lines[1]["level"].as_str()
            ),
            (Some("set_thinking_level"), Some("low"))
        );
        assert_eq!(
            codec.command(DriveCommand::Configure(RunSettings {
                model: Some("openai/gpt-fast".parse().unwrap()),
                effort: Some(Effort::Low),
                approvals: Some(ApprovalPolicy::Ask),
            })),
            Err(CommandRefused::NotOffered(Midway::Approvals))
        );
    }
}
