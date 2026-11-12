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

use super::{
    CommandRefused, DriveCommand, DriveEvent, DriveItem, Framed, ItemId, ItemKind, ItemPhase,
    ItemRead, ItemStatus, Line, Opening, Prompt, RawLine, Received, RequestKind, Retry, TurnEnd,
    Unreadable,
};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The client's side of one `pi --mode rpc` process.
#[derive(Debug)]
pub struct PiRpc {
    thread: NativeThreadId,
    ready: bool,
    /// Turn ids are this, mixed with how many turns were minted.
    seed: Uuid,
    minted: u64,
    next_request: u64,
    /// Commands Ren sent and waits on, by request id.
    requests: BTreeMap<String, RequestKind>,
    turn: TurnSlot,
    /// Assistant messages started in this process, naming their items.
    messages: u64,
    /// The assistant message whose blocks are streaming.
    streaming: Option<u64>,
    /// How the newest assistant message of the turn stopped.
    stopped: Option<Stopped>,
    /// Tool calls started in this turn, by tool call id, until their end.
    tools: BTreeMap<String, ToolCall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnSlot {
    Idle,
    /// A prompt was sent; Pi's run has not started.
    Requested,
    Running {
        turn: NativeTurnId,
        /// Ren asked for the turn to stop.
        interrupted: bool,
    },
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
        let thread = match opening {
            Opening::New { id, .. } => id,
            Opening::Resume { thread, .. } => thread,
        };
        // `--session-id` keeps the session in the run's folder; `--session`
        // would move to the folder the session was started in.
        vec![
            "--mode".into(),
            "rpc".into(),
            "--session-id".into(),
            thread.to_string(),
        ]
    }

    /// A codec for a freshly started process, and the first line to send it.
    /// `seed` is a fresh id the process's turn ids are minted from.
    pub fn open(opening: &Opening, seed: Uuid) -> (Self, Line) {
        let (thread, kind) = match opening {
            Opening::New { id, .. } => (*id, RequestKind::ThreadStart),
            Opening::Resume { thread, .. } => (*thread, RequestKind::ThreadResume),
        };
        let mut codec = Self {
            thread,
            ready: false,
            seed,
            minted: 0,
            next_request: 0,
            requests: BTreeMap::new(),
            turn: TurnSlot::Idle,
            messages: 0,
            streaming: None,
            stopped: None,
            tools: BTreeMap::new(),
        };
        let line = codec.request(kind, json!({ "type": "get_state" }));
        (codec, line)
    }

    /// The session, once Pi reported it open.
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
                    Ok(self.request(RequestKind::TurnInterrupt, json!({ "type": "abort" })))
                }
                TurnSlot::Requested => Err(CommandRefused::TurnNotStarted),
                TurnSlot::Idle => Err(CommandRefused::NoTurn),
            },
            // Pi asks for no approvals.
            DriveCommand::Answer { approval, .. } => Err(CommandRefused::UnknownApproval(approval)),
        }
    }

    /// Read one frame from Pi's stdout.
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
        match kind {
            "response" => self.response(&message, line, &mut received),
            "agent_start" => self.agent_start(&mut received),
            "agent_settled" => self.settled(&message, line, &mut received),
            "message_start" => self.message_start(&message, &mut received),
            "message_update" => self.message_update(&message, &mut received),
            "message_end" => self.message_end(&message, &mut received),
            "tool_execution_start" => self.tool_start(&message, line, &mut received),
            "tool_execution_end" => self.tool_end(&message, &mut received),
            "auto_retry_start" => received.events.push(DriveEvent::Error {
                message: Self::retrying(&message),
                retry: Retry::WillRetry,
            }),
            "auto_retry_end" => {
                if message.get("success").and_then(Value::as_bool) == Some(false) {
                    received.events.push(DriveEvent::Error {
                        message: Self::text(&message, "finalError")
                            .unwrap_or_else(|| "the request failed after retrying".into()),
                        retry: Retry::Final,
                    });
                }
            }
            "extension_error" => received.events.push(DriveEvent::Error {
                message: format!(
                    "extension {}: {}",
                    Self::text(&message, "extensionPath").unwrap_or_default(),
                    Self::text(&message, "error").unwrap_or_default(),
                ),
                retry: Retry::Final,
            }),
            "extension_ui_request" => Self::dialog(&message, line, &mut received),
            // Known, and nothing Ren shows: Pi's inner model turns, the end
            // of one low-level run (more may follow until it settles),
            // streamed tool output, queues, compaction, and session metadata.
            "turn_start"
            | "turn_end"
            | "agent_end"
            | "tool_execution_update"
            | "bash_execution_update"
            | "queue_update"
            | "compaction_start"
            | "compaction_end"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start"
            | "summarization_retry_finished"
            | "entry_appended"
            | "session_info_changed"
            | "thinking_level_changed" => {}
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
        Ok(self.request(
            RequestKind::TurnStart,
            json!({ "type": "prompt", "message": prompt.as_str() }),
        ))
    }

    fn request(&mut self, kind: RequestKind, mut command: Value) -> Line {
        let id = format!("ren-{}", self.next_request);
        self.next_request += 1;
        self.requests.insert(id.clone(), kind);
        if let Value::Object(fields) = &mut command {
            fields.insert("id".into(), id.into());
        }
        Line::json(&command)
    }

    fn text(value: &Value, key: &str) -> Option<String> {
        value.get(key).and_then(Value::as_str).map(str::to_owned)
    }

    fn response(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let Some(kind) = message
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| self.requests.remove(id))
        else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::UnexpectedResponse,
            });
            return;
        };
        if message.get("success").and_then(Value::as_bool) != Some(true) {
            if let (RequestKind::TurnStart, TurnSlot::Requested) = (kind, self.turn) {
                self.turn = TurnSlot::Idle;
            }
            received.events.push(DriveEvent::Refused {
                request: kind,
                message: Self::text(message, "error").unwrap_or_else(|| "refused".into()),
            });
            return;
        }
        let data = &message["data"];
        match kind {
            RequestKind::ThreadStart | RequestKind::ThreadResume => {
                // Pi opened the session Ren named; should it report another,
                // that is the one its model traffic is recorded under.
                if let Some(reported) = data
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .and_then(|id| Uuid::try_parse(id).ok())
                    .and_then(NativeThreadId::new)
                {
                    self.thread = reported;
                }
                self.ready = true;
                received.events.push(DriveEvent::Ready {
                    thread: self.thread,
                });
            }
            RequestKind::TurnStart => {
                // An extension consumed the prompt: no run starts for it.
                if data.get("disposition").and_then(Value::as_str) == Some("handled")
                    && let TurnSlot::Requested = self.turn
                {
                    self.turn = TurnSlot::Idle;
                    received.events.push(DriveEvent::Refused {
                        request: kind,
                        message: "an extension handled the prompt; no turn ran".into(),
                    });
                }
            }
            RequestKind::TurnInterrupt | RequestKind::Initialize => {}
        }
    }

    /// The next turn id: never nil, never one this process minted before.
    fn mint(&mut self) -> NativeTurnId {
        loop {
            self.minted += 1;
            let mixed = Uuid::from_u128(self.seed.as_u128() ^ u128::from(self.minted));
            if let Some(turn) = NativeTurnId::new(mixed) {
                return turn;
            }
        }
    }

    fn agent_start(&mut self, received: &mut Received) {
        // A retry or follow-up run within the turn starts no new one.
        match self.turn {
            TurnSlot::Running { .. } => {}
            TurnSlot::Idle | TurnSlot::Requested => {
                let turn = self.mint();
                self.turn = TurnSlot::Running {
                    turn,
                    interrupted: false,
                };
                received.events.push(DriveEvent::TurnStarted { turn });
            }
        }
    }

    fn settled(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let TurnSlot::Running { turn, interrupted } = self.turn else {
            // Settled with no turn Ren knows of.
            received.events.push(DriveEvent::Unknown { line });
            return;
        };
        self.turn = TurnSlot::Idle;
        self.streaming = None;
        self.tools.clear();
        let aborted = message.get("aborted").and_then(Value::as_bool) == Some(true);
        let end = match (interrupted || aborted, self.stopped.take()) {
            (true, _) | (false, Some(Stopped::Aborted)) => TurnEnd::Interrupted,
            (false, Some(Stopped::Error(message))) => TurnEnd::Failed { message },
            (false, None) => TurnEnd::Completed,
        };
        received.events.push(DriveEvent::TurnEnded { turn, end });
    }

    /// An item id within the running turn.
    fn item(&self, name: String) -> ItemId {
        match self.turn {
            TurnSlot::Running { turn, .. } => ItemId::new(format!("{turn}:{name}")),
            TurnSlot::Idle | TurnSlot::Requested => ItemId::new(name),
        }
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
                received.events.push(DriveEvent::Item {
                    phase: ItemPhase::Completed,
                    item: DriveItem {
                        id: self.item(format!("user:{stamp}")),
                        kind: ItemKind::UserMessage,
                        read: ItemRead::Prompt { text },
                        body: body.clone(),
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
        match name {
            "bash" => (
                ItemKind::CommandExecution,
                ItemRead::Command {
                    command: text("command").unwrap_or_default(),
                    status: ItemStatus::InProgress,
                    output: None,
                    exit_code: None,
                },
            ),
            "edit" | "write" => (
                ItemKind::FileChange,
                ItemRead::FileChange {
                    paths: text("path").into_iter().collect(),
                    status: ItemStatus::InProgress,
                },
            ),
            other => (
                ItemKind::Other(other.to_owned()),
                ItemRead::Tool {
                    name: match ["path", "pattern", "url", "query"]
                        .into_iter()
                        .find_map(text)
                    {
                        Some(detail) => format!("{other} {detail}"),
                        None => other.to_owned(),
                    },
                    status: ItemStatus::InProgress,
                },
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
            ItemRead::Command { command, .. } => ItemRead::Command {
                command,
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
    use crate::drive::{
        CommandRefused, DriveCommand, DriveEvent, Framed, ItemKind, ItemPhase, ItemRead,
        ItemStatus, Line, Opening, Prompt, Received, RequestKind, Retry, TurnEnd, Unreadable,
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
        codec
    }

    /// A prompt sent and Pi's run started.
    fn started(codec: &mut PiRpc, text: &str) -> NativeTurnId {
        let line = codec.command(prompt(text)).unwrap();
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
                        text: "fix the test".into()
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
                    ItemRead::Tool {
                        name: "grep TODO".into(),
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
        let abort = sent(&codec.command(DriveCommand::Interrupt).unwrap());
        assert_eq!(abort, json!({ "type": "abort", "id": "ren-2" }));
        assert!(
            feed(
                &mut codec,
                json!({ "id": "ren-2", "type": "response", "command": "abort", "success": true }),
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
        let line = codec.command(prompt("hi")).unwrap();
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
        let line = codec.command(prompt("/reload")).unwrap();
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
}
