//! opencode's Agent Client Protocol mode (`opencode acp`): JSON-RPC 2.0, one
//! message per line on stdin and stdout. Read against opencode 2.2.7, whose
//! `acp` command runs a private server of its own (not the shared
//! background service), so the configuration Ren puts in its environment
//! applies to it.
//!
//! Ren initializes, then opens a session in the run's folder: `session/new`
//! for a new conversation, or, to continue one, `session/list` for the
//! folder's sessions (opencode names its own, `ses_…`, and Ren's thread is a
//! UUID derived from that name) and `session/resume`, which reopens it
//! without replaying its history. A prompt is a `session/prompt` request
//! that answers when the turn ends; `session/update` notifications stream its
//! reply, reasoning, tool calls, and plan meanwhile. `session/cancel`
//! interrupts it, and `_session/steering` (opencode's extension) adds to it.
//! opencode asks for approvals with `session/request_permission` requests.
//!
//! opencode does not echo a prompt back while a turn runs, so the prompt Ren
//! sent is reported as the turn's first item. opencode names no turns: a
//! turn's id is minted from the seed the process was opened with. Ren
//! advertises no file system or terminal of its own, so opencode uses its
//! own tools; any other request it sends is answered with an error.

use super::session::OpenCodeSessionId;
use crate::{
    commands::{ClientCommand, CommandKind, Sigil},
    drive::{
        ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, DriveItem,
        ItemId, ItemKind, ItemPhase, ItemRead, ItemStatus, Opening, Prompt, RawLine, Received,
        RequestKind, TurnEnd, Unreadable,
    },
    lines::{Framed, Line},
    models::{DefaultChoice, Listing, Model, ModelId, ModelListing},
    settings::{Midway, RunSettings},
};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The client's side of one `opencode acp` process.
#[derive(Debug)]
pub struct OpenCodeAcp {
    cwd: String,
    /// The conversation Ren asked for: a new one, or this thread again.
    resume: Option<NativeThreadId>,
    /// The settings the run starts with, applied once the session opens.
    wanted: RunSettings,
    /// opencode's session, once open, and the thread Ren reads it as.
    session: Option<String>,
    thread: Option<NativeThreadId>,
    ready: bool,
    /// Settings requests sent while opening, still unanswered.
    opening: usize,
    /// What the session runs on now, to tell what a change changes.
    settings: RunSettings,
    /// Turn ids are this, mixed with how many turns were minted.
    seed: Uuid,
    minted: u64,
    next_request: u64,
    /// Requests Ren sent and waits on, by request id.
    requests: BTreeMap<u64, Sent>,
    turn: TurnSlot,
    /// The reply or reasoning streaming now, until another item begins.
    streaming: Option<Streaming>,
    /// Tool calls of the running turn, by tool call id.
    tools: BTreeMap<String, ToolCall>,
    /// Approvals waiting on the operator: opencode's request id, and the
    /// options it offered, by kind.
    approvals: BTreeMap<ApprovalId, Waiting>,
    /// Steering messages added to the running turn.
    steered: u64,
    /// The commands opencode lists for a prompt.
    commands: Vec<ClientCommand>,
}

/// A request Ren sent, and what it was for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    Initialize,
    New,
    /// One page of the folder's sessions, looking for the thread to resume.
    List,
    Resume,
    /// A setting changed; `opening` while the session is still opening.
    Configure {
        opening: bool,
    },
    Prompt,
    Steer(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnSlot {
    Idle,
    /// A prompt was sent; opencode has reported nothing of its turn yet.
    Requested {
        prompt: String,
    },
    Running {
        turn: NativeTurnId,
    },
}

#[derive(Debug)]
struct Streaming {
    id: ItemId,
    kind: ItemKind,
    text: String,
}

#[derive(Debug, Clone)]
struct ToolCall {
    /// The tool's name, as opencode names it.
    name: String,
    title: Option<String>,
    /// opencode's kind for it (`execute`, `edit`, `read`, …).
    kind: String,
    status: Option<String>,
    input: Value,
    paths: Vec<String>,
    output: Vec<String>,
    exit: Option<i64>,
}

#[derive(Debug)]
struct Waiting {
    id: Value,
    /// The option offered for each kind (`allow_once`, `reject_once`, …).
    options: BTreeMap<String, String>,
}

impl OpenCodeAcp {
    /// The arguments that put opencode in this mode. The session is opened
    /// over the protocol, so they are the same for every opening.
    pub fn arguments() -> Vec<String> {
        vec!["acp".into()]
    }

    /// A codec for a freshly started process, and the first line to send it.
    /// `seed` is a fresh id the process's turn ids are minted from.
    pub fn open(opening: &Opening, version: &str, seed: Uuid) -> (Self, Line) {
        let (cwd, resume, settings) = match opening {
            Opening::New { cwd, settings, .. } => (cwd.clone(), None, settings.clone()),
            Opening::Resume {
                thread,
                cwd,
                settings,
            } => (cwd.clone(), Some(*thread), settings.clone()),
        };
        let mut codec = Self {
            cwd,
            resume,
            // The approval policy is opencode's configuration, set at launch.
            settings: RunSettings {
                model: None,
                effort: None,
                approvals: settings.approvals,
            },
            wanted: settings,
            session: None,
            thread: None,
            ready: false,
            opening: 0,
            seed,
            minted: 0,
            next_request: 0,
            requests: BTreeMap::new(),
            turn: TurnSlot::Idle,
            streaming: None,
            tools: BTreeMap::new(),
            approvals: BTreeMap::new(),
            steered: 0,
            commands: Vec::new(),
        };
        let line = codec.request(Sent::Initialize, "initialize", Self::initialize(version));
        (codec, line)
    }

    /// What Ren tells opencode of itself: the protocol version it speaks, and
    /// no file system or terminal for opencode to use instead of its own.
    fn initialize(version: &str) -> Value {
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false,
            },
            "clientInfo": { "name": "ren", "version": version },
        })
    }

    /// The thread, once opencode opened the session.
    pub fn thread(&self) -> Option<NativeThreadId> {
        if self.ready { self.thread } else { None }
    }

    /// The commands a prompt can run, as opencode listed them: each is
    /// `/name` in the prompt.
    pub fn commands(&self) -> &[ClientCommand] {
        &self.commands
    }

    /// The lines that carry `command`, or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Vec<Line>, CommandRefused> {
        let Some(session) = self.session.clone().filter(|_| self.ready) else {
            return Err(CommandRefused::NotReady);
        };
        match command {
            DriveCommand::Prompt(prompt) => {
                if self.turn != TurnSlot::Idle {
                    return Err(CommandRefused::TurnRunning);
                }
                self.turn = TurnSlot::Requested {
                    prompt: prompt.as_str().to_owned(),
                };
                Ok(vec![self.request(
                    Sent::Prompt,
                    "session/prompt",
                    json!({ "sessionId": session, "prompt": Self::content(&prompt) }),
                )])
            }
            // How the turn ended is what opencode reports: one asked to stop
            // that finished first is completed.
            DriveCommand::Interrupt => match self.turn {
                TurnSlot::Running { .. } | TurnSlot::Requested { .. } => Ok(self.cancel(&session)),
                TurnSlot::Idle => Err(CommandRefused::NoTurn),
            },
            DriveCommand::Answer { approval, decision } => {
                let waiting = self
                    .approvals
                    .remove(&approval)
                    .ok_or(CommandRefused::UnknownApproval(approval))?;
                let wanted = match decision {
                    Decision::Accept => ["allow_once", "allow_always"],
                    Decision::AcceptForSession => ["allow_always", "allow_once"],
                    Decision::Decline | Decision::Cancel => ["reject_once", "reject_always"],
                };
                let outcome = wanted
                    .iter()
                    .find_map(|kind| waiting.options.get(*kind))
                    .map_or_else(
                        || json!({ "outcome": "cancelled" }),
                        |option| json!({ "outcome": "selected", "optionId": option }),
                    );
                let mut lines = vec![Line::json(&json!({
                    "jsonrpc": "2.0",
                    "id": waiting.id,
                    "result": { "outcome": outcome },
                }))];
                if decision == Decision::Cancel {
                    lines.extend(self.cancel(&session));
                }
                Ok(lines)
            }
            DriveCommand::Steer(prompt) => match self.turn {
                TurnSlot::Running { .. } => Ok(vec![self.request(
                    Sent::Steer(prompt.as_str().to_owned()),
                    "_session/steering",
                    json!({ "sessionId": session, "prompt": Self::content(&prompt) }),
                )]),
                TurnSlot::Requested { .. } => Err(CommandRefused::TurnNotStarted),
                TurnSlot::Idle => Err(CommandRefused::NoTurn),
            },
            DriveCommand::Configure(settings) => self.configure(&session, settings, false),
        }
    }

    /// A prompt as ACP content: one text block.
    fn content(prompt: &Prompt) -> Value {
        json!([{ "type": "text", "text": prompt.as_str() }])
    }

    /// The notification that cancels the turn, and the answers that withdraw
    /// the approvals it waits on: the protocol has a cancelled turn's
    /// permission requests answered as cancelled.
    fn cancel(&mut self, session: &str) -> Vec<Line> {
        let mut lines: Vec<Line> = std::mem::take(&mut self.approvals)
            .into_values()
            .map(|waiting| {
                Line::json(&json!({
                    "jsonrpc": "2.0",
                    "id": waiting.id,
                    "result": { "outcome": { "outcome": "cancelled" } },
                }))
            })
            .collect();
        lines.push(Line::json(&json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": session },
        })));
        lines
    }

    /// The requests that move the session to `settings`. opencode switches
    /// model and effort from the next turn; its approvals are its
    /// configuration, set when the process starts. Effort goes back to the
    /// model's own; a model, once chosen, stays until another is.
    fn configure(
        &mut self,
        session: &str,
        settings: RunSettings,
        opening: bool,
    ) -> Result<Vec<Line>, CommandRefused> {
        if settings.approvals != self.settings.approvals {
            return Err(CommandRefused::NotOffered(Midway::Approvals));
        }
        let mut lines = Vec::new();
        if settings.model != self.settings.model {
            let model = settings
                .model
                .as_ref()
                .ok_or(CommandRefused::NotOffered(Midway::Model))?;
            lines.push(self.set(session, "model", model.as_str(), opening));
        }
        if settings.effort != self.settings.effort {
            let value = settings
                .effort
                .map_or_else(|| "default".to_owned(), |effort| effort.to_string());
            lines.push(self.set(session, "effort", &value, opening));
        }
        self.settings = settings;
        Ok(lines)
    }

    fn set(&mut self, session: &str, option: &str, value: &str, opening: bool) -> Line {
        self.request(
            Sent::Configure { opening },
            "session/set_config_option",
            json!({ "sessionId": session, "configId": option, "value": value }),
        )
    }

    fn request(&mut self, sent: Sent, method: &str, params: Value) -> Line {
        let id = self.next_request;
        self.next_request += 1;
        self.requests.insert(id, sent);
        Line::json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
    }

    /// Read one frame from opencode's stdout.
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
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").filter(|id| !id.is_null());
        match (method, id) {
            (Some(method), Some(id)) => {
                self.client_request(method, id.clone(), &message, line, &mut received);
            }
            (Some("session/update"), None) => self.update(&message, line, &mut received),
            (Some(_), None) => received.events.push(DriveEvent::Unknown { line }),
            (None, Some(id)) => self.response(id, &message, line, &mut received),
            (None, None) => received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::NotJsonRpc,
            }),
        }
        received
    }

    /// A request opencode sent Ren: an approval, or something Ren never
    /// provides, answered with an error so opencode never waits on it.
    fn client_request(
        &mut self,
        method: &str,
        id: Value,
        message: &Value,
        line: RawLine,
        received: &mut Received,
    ) {
        if method != "session/request_permission" {
            received
                .replies
                .push(Self::refusal(id, "not supported by Ren"));
            received.events.push(DriveEvent::Unsupported {
                method: method.to_owned(),
            });
            return;
        }
        let params = &message["params"];
        let options: BTreeMap<String, String> = params
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| {
                Some((Self::text(option, "kind")?, Self::text(option, "optionId")?))
            })
            .collect();
        let tool = &params["toolCall"];
        if options.is_empty() || !tool.is_object() {
            received
                .replies
                .push(Self::refusal(id, "unreadable request"));
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            });
            return;
        }
        let input = &tool["rawInput"];
        let title = Self::text(tool, "title");
        let ask = match tool.get("kind").and_then(Value::as_str) {
            Some("execute") => ApprovalAsk::Command {
                command: Self::text(input, "command").or(title),
                cwd: Self::text(input, "cwd"),
                reason: None,
            },
            Some("edit" | "delete" | "move") => ApprovalAsk::FileChange { reason: title },
            Some(_) | None => ApprovalAsk::Tool {
                tool: Self::text(tool, "name")
                    .or_else(|| title.clone())
                    .unwrap_or_else(|| "tool".into()),
                detail: title,
            },
        };
        let approval = Self::approval_id(&id);
        self.approvals
            .insert(approval.clone(), Waiting { id, options });
        received.events.push(DriveEvent::Approval { approval, ask });
    }

    fn refusal(id: Value, message: &str) -> Line {
        Line::json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": message },
        }))
    }

    /// A request id as text: strings as they are, numbers in decimal.
    fn approval_id(id: &Value) -> ApprovalId {
        ApprovalId(match id {
            Value::String(text) => text.clone(),
            other @ (Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_)) => other.to_string(),
        })
    }

    fn text(value: &Value, key: &str) -> Option<String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }

    fn response(&mut self, id: &Value, message: &Value, line: RawLine, received: &mut Received) {
        let Some(sent) = id.as_u64().and_then(|id| self.requests.remove(&id)) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::UnexpectedResponse,
            });
            return;
        };
        if let Some(error) = message.get("error") {
            let text = Self::text(error, "message").unwrap_or_else(|| "refused".into());
            self.failed(sent, text, received);
            return;
        }
        let result = &message["result"];
        match sent {
            Sent::Initialize => {
                let line = match self.resume {
                    None => self.request(
                        Sent::New,
                        "session/new",
                        json!({ "cwd": self.cwd, "mcpServers": [] }),
                    ),
                    Some(_) => self.list(None),
                };
                received.replies.push(line);
            }
            Sent::New => match Self::text(result, "sessionId")
                .as_deref()
                .and_then(OpenCodeSessionId::parse)
                .and_then(|id| NativeThreadId::new(id.uuid()).map(|thread| (id.as_str(), thread)))
            {
                Some((session, thread)) => {
                    let session = session.to_owned();
                    self.opened(session, thread, received);
                }
                None => received.events.push(DriveEvent::Refused {
                    request: RequestKind::ThreadStart,
                    message: "opencode opened a session Ren cannot read".into(),
                }),
            },
            Sent::List => self.listed(result, received),
            Sent::Resume => {
                if let (Some(session), Some(thread)) = (self.session.clone(), self.resume) {
                    self.opened(session, thread, received);
                }
            }
            Sent::Configure { opening } => {
                if opening {
                    self.settled(received);
                }
            }
            Sent::Prompt => self.ended(result, received),
            Sent::Steer(text) => {
                if result.get("outcome").and_then(Value::as_str) == Some("injected") {
                    self.steered += 1;
                    let id = self.item(format!("steer:{}", self.steered));
                    received
                        .events
                        .push(Self::prompt_item(id, text, Value::Null));
                } else {
                    received.events.push(DriveEvent::Refused {
                        request: RequestKind::TurnSteer,
                        message: "opencode had no turn running to add it to".into(),
                    });
                }
            }
        }
    }

    /// A request opencode refused.
    fn failed(&mut self, sent: Sent, message: String, received: &mut Received) {
        let request = match sent {
            Sent::Initialize => RequestKind::Initialize,
            Sent::New => RequestKind::ThreadStart,
            Sent::List | Sent::Resume => RequestKind::ThreadResume,
            Sent::Configure { opening } => {
                if opening {
                    // The session opens on opencode's own setting instead.
                    received.events.push(DriveEvent::Refused {
                        request: RequestKind::Configure,
                        message,
                    });
                    self.settled(received);
                    return;
                }
                RequestKind::Configure
            }
            Sent::Steer(_) => RequestKind::TurnSteer,
            Sent::Prompt => match std::mem::replace(&mut self.turn, TurnSlot::Idle) {
                // Nothing of the turn was reported: the prompt was refused.
                TurnSlot::Idle | TurnSlot::Requested { .. } => RequestKind::TurnStart,
                TurnSlot::Running { turn, .. } => {
                    self.close(received);
                    self.withdraw(received);
                    received.events.push(DriveEvent::TurnEnded {
                        turn,
                        end: TurnEnd::Failed {
                            message: Some(message),
                        },
                    });
                    return;
                }
            },
        };
        received
            .events
            .push(DriveEvent::Refused { request, message });
    }

    /// The next page of the folder's sessions.
    fn list(&mut self, cursor: Option<String>) -> Line {
        let mut params = json!({ "cwd": self.cwd });
        if let (Some(cursor), Value::Object(fields)) = (cursor, &mut params) {
            fields.insert("cursor".into(), cursor.into());
        }
        self.request(Sent::List, "session/list", params)
    }

    /// One page of the folder's sessions: the one whose UUID is the thread
    /// to resume is reopened; without it, the next page is asked for.
    fn listed(&mut self, result: &Value, received: &mut Received) {
        let Some(thread) = self.resume else {
            return;
        };
        let found = result
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("sessionId").and_then(Value::as_str))
            .filter_map(OpenCodeSessionId::parse)
            .find(|id| id.uuid() == thread.as_uuid())
            .map(|id| id.as_str().to_owned());
        if let Some(session) = found {
            self.session = Some(session.clone());
            let line = self.request(
                Sent::Resume,
                "session/resume",
                json!({ "sessionId": session, "cwd": self.cwd, "mcpServers": [] }),
            );
            received.replies.push(line);
            return;
        }
        match Self::text(result, "nextCursor") {
            Some(cursor) => {
                let line = self.list(Some(cursor));
                received.replies.push(line);
            }
            None => received.events.push(DriveEvent::Refused {
                request: RequestKind::ThreadResume,
                message: format!("opencode has no session {thread} in {}", self.cwd),
            }),
        }
    }

    /// The session is open: the run's settings are applied, then it is ready.
    fn opened(&mut self, session: String, thread: NativeThreadId, received: &mut Received) {
        self.session = Some(session.clone());
        self.thread = Some(thread);
        let wanted = self.wanted.clone();
        // A setting opencode does not take mid-run is left as opencode has it.
        let lines = self.configure(&session, wanted, true).unwrap_or_default();
        self.opening = lines.len();
        received.replies.extend(lines);
        if self.opening == 0 {
            self.ready(received);
        }
    }

    /// One opening setting answered.
    fn settled(&mut self, received: &mut Received) {
        self.opening = self.opening.saturating_sub(1);
        if self.opening == 0 && !self.ready {
            self.ready(received);
        }
    }

    fn ready(&mut self, received: &mut Received) {
        if let Some(thread) = self.thread {
            self.ready = true;
            received.events.push(DriveEvent::Ready { thread });
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

    /// The turn a prompt started, once opencode reports anything of it: it
    /// begins with the prompt Ren sent.
    fn start(&mut self, received: &mut Received) {
        if !matches!(self.turn, TurnSlot::Requested { .. }) {
            return;
        }
        let TurnSlot::Requested { prompt } = std::mem::replace(&mut self.turn, TurnSlot::Idle)
        else {
            return;
        };
        let turn = self.mint();
        self.turn = TurnSlot::Running { turn };
        received.events.push(DriveEvent::TurnStarted { turn });
        let id = self.item("prompt".into());
        received
            .events
            .push(Self::prompt_item(id, prompt, Value::Null));
    }

    fn prompt_item(id: ItemId, text: String, body: Value) -> DriveEvent {
        DriveEvent::Item {
            phase: ItemPhase::Completed,
            item: DriveItem {
                id,
                kind: ItemKind::UserMessage,
                read: ItemRead::Prompt { text },
                body,
            },
        }
    }

    /// An item id within the running turn.
    fn item(&self, name: String) -> ItemId {
        match self.turn {
            TurnSlot::Running { turn, .. } => ItemId::new(format!("{turn}:{name}")),
            TurnSlot::Idle | TurnSlot::Requested { .. } => ItemId::new(name),
        }
    }

    /// The turn's prompt answered: it ended.
    fn ended(&mut self, result: &Value, received: &mut Received) {
        self.start(received);
        let TurnSlot::Running { turn } = std::mem::replace(&mut self.turn, TurnSlot::Idle) else {
            return;
        };
        self.close(received);
        self.withdraw(received);
        // How opencode says it ended: a turn Ren asked to stop that ended on
        // its own first is completed, not interrupted.
        let end = match result.get("stopReason").and_then(Value::as_str) {
            Some("end_turn") => TurnEnd::Completed,
            Some("cancelled") => TurnEnd::Interrupted,
            Some("max_tokens") => TurnEnd::Failed {
                message: Some("the reply reached the model's output limit".into()),
            },
            Some("max_turn_requests") => TurnEnd::Failed {
                message: Some("the turn reached opencode's limit of model requests".into()),
            },
            Some("refusal") => TurnEnd::Failed {
                message: Some("the model refused".into()),
            },
            Some(status) => TurnEnd::Unknown {
                status: status.to_owned(),
            },
            None => TurnEnd::Unknown {
                status: String::new(),
            },
        };
        received.events.push(DriveEvent::TurnEnded { turn, end });
    }

    /// The approvals a turn's end leaves waiting no longer wait.
    fn withdraw(&mut self, received: &mut Received) {
        for approval in std::mem::take(&mut self.approvals).into_keys() {
            received
                .events
                .push(DriveEvent::ApprovalResolved { approval });
        }
        self.tools.clear();
    }

    /// The streaming reply or reasoning, finished.
    fn close(&mut self, received: &mut Received) {
        let Some(streaming) = self.streaming.take() else {
            return;
        };
        let read = match streaming.kind {
            ItemKind::AgentMessage => ItemRead::Reply {
                text: streaming.text,
            },
            ItemKind::UserMessage
            | ItemKind::Reasoning
            | ItemKind::Plan
            | ItemKind::CommandExecution
            | ItemKind::FileChange
            | ItemKind::McpToolCall
            | ItemKind::WebSearch
            | ItemKind::Other(_) => ItemRead::Reasoning,
        };
        received.events.push(DriveEvent::Item {
            phase: ItemPhase::Completed,
            item: DriveItem {
                id: streaming.id,
                kind: streaming.kind,
                read,
                body: Value::Null,
            },
        });
    }

    fn update(&mut self, message: &Value, line: RawLine, received: &mut Received) {
        let params = &message["params"];
        let update = &params["update"];
        let ours = params.get("sessionId").and_then(Value::as_str) == self.session.as_deref();
        let Some(kind) = update.get("sessionUpdate").and_then(Value::as_str) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            });
            return;
        };
        if !ours {
            received.events.push(DriveEvent::Unknown { line });
            return;
        }
        // Any news of the session after a prompt is its turn under way:
        // opencode lists its commands as a turn begins, before the model
        // answers, so the turn can be interrupted from then on.
        self.start(received);
        if kind == "available_commands_update" {
            self.commands = Self::listed_commands(update);
            return;
        }
        // Known, and nothing Ren shows: usage, mode, settings, and titles.
        if matches!(
            kind,
            "usage_update" | "current_mode_update" | "config_option_update" | "session_info_update"
        ) {
            return;
        }
        if !matches!(self.turn, TurnSlot::Running { .. }) {
            received.events.push(DriveEvent::Unknown { line });
            return;
        }
        match kind {
            "agent_message_chunk" => self.chunk(ItemKind::AgentMessage, update, received),
            "agent_thought_chunk" => self.chunk(ItemKind::Reasoning, update, received),
            "tool_call" | "tool_call_update" => self.tool(update, line, received),
            "plan" => {
                self.close(received);
                let text = update
                    .get("entries")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| {
                        let content = Self::text(entry, "content")?;
                        let done = entry.get("status").and_then(Value::as_str) == Some("completed");
                        Some(format!("- [{}] {content}", if done { "x" } else { " " }))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                received.events.push(DriveEvent::Item {
                    phase: ItemPhase::Completed,
                    item: DriveItem {
                        id: self.item("plan".into()),
                        kind: ItemKind::Plan,
                        read: ItemRead::Plan { text },
                        body: update.clone(),
                    },
                });
            }
            // A prompt opencode echoes; Ren reports the one it sent.
            "user_message_chunk" => {}
            _ => received.events.push(DriveEvent::Unknown { line }),
        }
    }

    /// A piece of the reply or the reasoning: a new item begins when the
    /// message it belongs to changes.
    fn chunk(&mut self, kind: ItemKind, update: &Value, received: &mut Received) {
        let text = update["content"]
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let named = Self::text(update, "messageId");
        let continues = self.streaming.as_ref().is_some_and(|streaming| {
            streaming.kind == kind
                && named
                    .as_ref()
                    .is_none_or(|name| streaming.id == self.item(name.clone()))
        });
        if !continues {
            self.close(received);
            let name =
                named.unwrap_or_else(|| format!("{}:{}", Self::kind_name(&kind), self.minted));
            let id = self.item(name);
            let read = match kind {
                ItemKind::AgentMessage => ItemRead::Reply {
                    text: String::new(),
                },
                ItemKind::UserMessage
                | ItemKind::Reasoning
                | ItemKind::Plan
                | ItemKind::CommandExecution
                | ItemKind::FileChange
                | ItemKind::McpToolCall
                | ItemKind::WebSearch
                | ItemKind::Other(_) => ItemRead::Reasoning,
            };
            received.events.push(DriveEvent::Item {
                phase: ItemPhase::Started,
                item: DriveItem {
                    id: id.clone(),
                    kind: kind.clone(),
                    read,
                    body: Value::Null,
                },
            });
            self.streaming = Some(Streaming {
                id,
                kind,
                text: String::new(),
            });
        }
        if let Some(streaming) = &mut self.streaming {
            streaming.text.push_str(&text);
            if streaming.kind == ItemKind::AgentMessage && !text.is_empty() {
                received.events.push(DriveEvent::Delta {
                    item: streaming.id.clone(),
                    text,
                });
            }
        }
    }

    /// A tool call, or news of one: what it is fills in as opencode reports
    /// it, and it ends when its status is final.
    fn tool(&mut self, update: &Value, line: RawLine, received: &mut Received) {
        let Some(id) = Self::text(update, "toolCallId") else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            });
            return;
        };
        self.close(received);
        let call = self.tools.entry(id.clone()).or_insert_with(|| ToolCall {
            name: String::new(),
            title: None,
            kind: String::new(),
            status: None,
            input: Value::Null,
            paths: Vec::new(),
            output: Vec::new(),
            exit: None,
        });
        if let Some(name) = Self::text(update, "name") {
            call.name = name;
        }
        if let Some(title) = Self::text(update, "title") {
            call.title = Some(title);
        }
        if let Some(kind) = Self::text(update, "kind") {
            call.kind = kind;
        }
        if let Some(status) = Self::text(update, "status") {
            call.status = Some(status);
        }
        if let Some(input) = update
            .get("rawInput")
            .filter(|input| input.as_object().is_some_and(|fields| !fields.is_empty()))
        {
            call.input = input.clone();
        }
        let paths: Vec<String> = update
            .get("locations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|location| Self::text(location, "path"))
            .collect();
        if !paths.is_empty() {
            call.paths = paths;
        }
        let output: Vec<String> = update
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|block| block["content"].get("text").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        if !output.is_empty() {
            call.output = output;
        }
        if let Some(exit) = update["rawOutput"]["metadata"]
            .get("exit")
            .and_then(Value::as_i64)
        {
            call.exit = Some(exit);
        }
        let call = call.clone();
        let done = matches!(call.status.as_deref(), Some("completed" | "failed"));
        if done {
            self.tools.remove(&id);
        }
        let phase = if done {
            ItemPhase::Completed
        } else {
            ItemPhase::Started
        };
        let (kind, read) = Self::tool_read(&call, phase);
        received.events.push(DriveEvent::Item {
            phase,
            item: DriveItem {
                id: self.item(id),
                kind,
                read,
                body: update.clone(),
            },
        });
    }

    /// What Ren reads of a tool call, by opencode's kind for it.
    fn tool_read(call: &ToolCall, phase: ItemPhase) -> (ItemKind, ItemRead) {
        let status = match call.status.as_deref() {
            Some("pending" | "in_progress") => ItemStatus::InProgress,
            Some("completed") => ItemStatus::Completed,
            Some("failed") => ItemStatus::Failed,
            Some(other) => ItemStatus::Other(other.to_owned()),
            None => ItemStatus::of(phase),
        };
        match call.kind.as_str() {
            "execute" => (
                ItemKind::CommandExecution,
                ItemRead::Command {
                    command: Self::text(&call.input, "command")
                        .or_else(|| call.title.clone())
                        .unwrap_or_default(),
                    status,
                    output: (!call.output.is_empty()).then(|| call.output.join("\n")),
                    exit_code: call.exit,
                },
            ),
            "edit" | "delete" | "move" => {
                let paths = if call.paths.is_empty() {
                    Self::text(&call.input, "path").into_iter().collect()
                } else {
                    call.paths.clone()
                };
                (ItemKind::FileChange, ItemRead::FileChange { paths, status })
            }
            other => (
                ItemKind::Other(if other.is_empty() { "tool" } else { other }.to_owned()),
                ItemRead::Tool {
                    name: if call.name.is_empty() {
                        call.title.clone().unwrap_or_else(|| "tool".into())
                    } else {
                        call.name.clone()
                    },
                    status,
                },
            ),
        }
    }

    /// A short name for an item kind, in ids opencode left unnamed.
    fn kind_name(kind: &ItemKind) -> &'static str {
        match kind {
            ItemKind::AgentMessage => "message",
            ItemKind::Reasoning => "reasoning",
            ItemKind::UserMessage
            | ItemKind::Plan
            | ItemKind::CommandExecution
            | ItemKind::FileChange
            | ItemKind::McpToolCall
            | ItemKind::WebSearch
            | ItemKind::Other(_) => "item",
        }
    }

    /// The commands in an `available_commands_update`.
    fn listed_commands(update: &Value) -> Vec<ClientCommand> {
        update
            .get("availableCommands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|command| {
                let name = Self::text(command, "name")?;
                Some(ClientCommand {
                    name: name.trim_start_matches('/').to_owned(),
                    description: Self::text(command, "description").unwrap_or_default(),
                    hint: command
                        .get("input")
                        .and_then(|input| Self::text(input, "hint")),
                    kind: CommandKind::Command,
                    sigil: Sigil::Slash,
                })
            })
            .collect()
    }
}

/// One `opencode acp` listing process: a session opened only to read the
/// model options opencode gives it, then deleted.
#[derive(Debug)]
pub struct OpenCodeModels {
    /// The folder the throwaway session opens in: absolute, as ACP requires.
    folder: String,
    step: ListStep,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ListStep {
    Initializing,
    Opening,
    /// The models are read; the session is being deleted.
    Deleting(Vec<Model>),
}

impl OpenCodeModels {
    const INITIALIZE: u64 = 0;
    const NEW: u64 = 1;
    const DELETE: u64 = 2;

    /// The arguments for a listing process: the same mode as a run.
    pub fn arguments() -> Vec<String> {
        OpenCodeAcp::arguments()
    }

    /// A codec for a freshly started listing process in `folder`, and its
    /// first line.
    pub fn open(version: &str, folder: &str) -> (Self, Line) {
        let line = Line::json(&json!({
            "jsonrpc": "2.0",
            "id": Self::INITIALIZE,
            "method": "initialize",
            "params": OpenCodeAcp::initialize(version),
        }));
        (
            Self {
                folder: folder.to_owned(),
                step: ListStep::Initializing,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        let id = message.get("id").and_then(Value::as_u64);
        if message.get("method").is_some() {
            return Listing::waiting();
        }
        let error = message.get("error").map(|error| {
            ModelListing::text(error, "message").unwrap_or_else(|| "opencode refused".into())
        });
        let result = &message["result"];
        match (
            std::mem::replace(&mut self.step, ListStep::Initializing),
            id,
        ) {
            (ListStep::Initializing, Some(Self::INITIALIZE)) => {
                if let Some(error) = error {
                    return Listing::Failed(error);
                }
                self.step = ListStep::Opening;
                Listing::Waiting(vec![Line::json(&json!({
                    "jsonrpc": "2.0",
                    "id": Self::NEW,
                    "method": "session/new",
                    "params": { "cwd": self.folder, "mcpServers": [] },
                }))])
            }
            (ListStep::Opening, Some(Self::NEW)) => {
                if let Some(error) = error {
                    return Listing::Failed(error);
                }
                let models = Self::models(result);
                // The session was only for its options: delete it, then list.
                match ModelListing::text(result, "sessionId") {
                    Some(session) => {
                        self.step = ListStep::Deleting(models);
                        Listing::Waiting(vec![Line::json(&json!({
                            "jsonrpc": "2.0",
                            "id": Self::DELETE,
                            "method": "session/delete",
                            "params": { "sessionId": session },
                        }))])
                    }
                    None => Listing::Listed(models),
                }
            }
            // Listed whether or not the session could be deleted.
            (ListStep::Deleting(models), Some(Self::DELETE)) => Listing::Listed(models),
            (step @ (ListStep::Initializing | ListStep::Opening | ListStep::Deleting(_)), _) => {
                self.step = step;
                Listing::waiting()
            }
        }
    }

    /// The models in a session's `model` option, its current one the default,
    /// and the efforts of that one's `effort` option.
    fn models(result: &Value) -> Vec<Model> {
        let options = result
            .get("configOptions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let option = |id: &str| {
            options
                .iter()
                .find(|option| option.get("id").and_then(Value::as_str) == Some(id))
                .cloned()
        };
        let Some(model) = option("model") else {
            return Vec::new();
        };
        let current = ModelListing::text(&model, "currentValue");
        let effort = option("effort");
        let efforts = ModelListing::efforts(
            effort.as_ref().map(|effort| &effort["options"]),
            Some("value"),
        );
        let default_effort = effort
            .as_ref()
            .and_then(|effort| ModelListing::text(effort, "currentValue"))
            .and_then(|value| value.parse().ok());
        model
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let value = ModelListing::text(entry, "value")?;
                let id: ModelId = value.parse().ok()?;
                let chosen = current.as_deref() == Some(value.as_str());
                let provider = value
                    .split_once('/')
                    .map(|(provider, _)| provider.to_owned());
                let name = ModelListing::text(entry, "name").unwrap_or_else(|| value.clone());
                // opencode names a model `provider/Name`; the provider is
                // shown on its own.
                let name = provider
                    .as_ref()
                    .and_then(|provider| name.strip_prefix(&format!("{provider}/")))
                    .map_or_else(|| name.clone(), str::to_owned);
                Some(Model {
                    id,
                    name,
                    description: ModelListing::text(entry, "description"),
                    provider,
                    choice: if chosen {
                        DefaultChoice::Default
                    } else {
                        DefaultChoice::Other
                    },
                    efforts: if chosen { efforts.clone() } else { Vec::new() },
                    effort: if chosen { default_effort } else { None },
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenCodeAcp, OpenCodeSessionId};
    use crate::{
        commands::{CommandKind, Sigil},
        drive::{
            ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, ItemKind,
            ItemPhase, ItemRead, ItemStatus, Opening, Prompt, Received, RequestKind, TurnEnd,
        },
        lines::{Framed, Line},
        settings::{ApprovalPolicy, Effort, Midway, RunSettings},
    };
    use ren_protocol::native::{NativeThreadId, NativeTurnId};
    use serde_json::{Value, json};
    use uuid::Uuid;

    const SESSION: &str = "ses_e2ccf969fffefSsV2wCxJkixaU";

    fn sent(line: &Line) -> Value {
        serde_json::from_slice(line.as_bytes()).unwrap()
    }

    fn feed(codec: &mut OpenCodeAcp, message: Value) -> Received {
        codec.receive(Framed::Line(message.to_string().into_bytes()))
    }

    fn thread() -> NativeThreadId {
        NativeThreadId::new(OpenCodeSessionId::parse(SESSION).unwrap().uuid()).unwrap()
    }

    fn seed() -> Uuid {
        Uuid::from_u128(0x5eed)
    }

    fn turn(minted: u128) -> NativeTurnId {
        NativeTurnId::new(Uuid::from_u128(0x5eed ^ minted)).unwrap()
    }

    fn opening(settings: RunSettings) -> Opening {
        Opening::New {
            cwd: "/work/acme".into(),
            id: NativeThreadId::new(Uuid::from_u128(1)).unwrap(),
            settings,
        }
    }

    fn update(update: Value) -> Value {
        json!({ "jsonrpc": "2.0", "method": "session/update",
            "params": { "sessionId": SESSION, "update": update } })
    }

    fn answer(id: &Value, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    /// A codec whose new session is open, on opencode's own settings.
    fn ready() -> OpenCodeAcp {
        let (mut codec, first) =
            OpenCodeAcp::open(&opening(RunSettings::default()), "0.1.0", seed());
        let first = sent(&first);
        assert_eq!(first["method"], "initialize");
        assert_eq!(first["params"]["clientCapabilities"]["terminal"], false);
        let replies = feed(
            &mut codec,
            answer(&first["id"], json!({ "protocolVersion": 1 })),
        )
        .replies;
        let new = sent(&replies[0]);
        assert_eq!(new["method"], "session/new");
        assert_eq!(
            new["params"],
            json!({ "cwd": "/work/acme", "mcpServers": [] })
        );
        let opened = feed(
            &mut codec,
            answer(&new["id"], json!({ "sessionId": SESSION })),
        );
        assert_eq!(opened.events, vec![DriveEvent::Ready { thread: thread() }]);
        assert_eq!(codec.thread(), Some(thread()));
        codec
    }

    fn prompt(text: &str) -> DriveCommand {
        DriveCommand::Prompt(Prompt::parse(text.into()).unwrap())
    }

    /// The prompt request sent for `text`.
    fn prompted(codec: &mut OpenCodeAcp, text: &str) -> Value {
        let lines = codec.command(prompt(text)).unwrap();
        let request = sent(&lines[0]);
        assert_eq!(request["method"], "session/prompt");
        assert_eq!(
            request["params"],
            json!({ "sessionId": SESSION, "prompt": [{ "type": "text", "text": text }] })
        );
        request
    }

    #[test]
    fn a_new_session_is_the_thread_its_name_derives_and_lists_its_commands() {
        let mut codec = ready();
        assert_eq!(
            codec.command(DriveCommand::Interrupt),
            Err(CommandRefused::NoTurn)
        );
        let listed = feed(
            &mut codec,
            update(
                json!({ "sessionUpdate": "available_commands_update", "availableCommands": [
                    { "name": "review", "description": "review changes", "input": { "hint": "[commit|branch]" } },
                    { "name": "compact", "description": "Compact the session" },
                ]}),
            ),
        );
        assert!(listed.events.is_empty());
        let commands = codec.commands();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "review");
        assert_eq!(commands[0].hint.as_deref(), Some("[commit|branch]"));
        assert_eq!(commands[0].kind, CommandKind::Command);
        assert_eq!(commands[0].sigil, Sigil::Slash);
    }

    #[test]
    fn opens_on_the_runs_settings_before_it_is_ready() {
        let settings = RunSettings {
            model: Some("opencode/big-pickle".parse().unwrap()),
            effort: Some(Effort::High),
            approvals: Some(ApprovalPolicy::Ask),
        };
        let (mut codec, first) = OpenCodeAcp::open(&opening(settings), "0.1.0", seed());
        let replies = feed(&mut codec, answer(&sent(&first)["id"], json!({}))).replies;
        let opened = feed(
            &mut codec,
            answer(&sent(&replies[0])["id"], json!({ "sessionId": SESSION })),
        );
        // The policy is opencode's configuration; model and effort are set.
        let set: Vec<Value> = opened.replies.iter().map(sent).collect();
        assert_eq!(
            set.iter()
                .map(|line| line["params"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!({ "sessionId": SESSION, "configId": "model", "value": "opencode/big-pickle" }),
                json!({ "sessionId": SESSION, "configId": "effort", "value": "high" }),
            ]
        );
        assert!(
            opened.events.is_empty(),
            "not ready until both are answered"
        );
        assert!(
            feed(&mut codec, answer(&set[0]["id"], json!({})))
                .events
                .is_empty()
        );
        // A refused setting leaves opencode's own, and the session opens.
        let last = feed(
            &mut codec,
            json!({ "jsonrpc": "2.0", "id": set[1]["id"], "error": { "code": -32602, "message": "bad effort" } }),
        );
        assert_eq!(
            last.events,
            vec![
                DriveEvent::Refused {
                    request: RequestKind::Configure,
                    message: "bad effort".into()
                },
                DriveEvent::Ready { thread: thread() },
            ]
        );
        // Effort back to the model's own; approvals cannot change mid-run.
        let lines = codec
            .command(DriveCommand::Configure(RunSettings {
                model: Some("opencode/big-pickle".parse().unwrap()),
                effort: None,
                approvals: Some(ApprovalPolicy::Ask),
            }))
            .unwrap();
        assert_eq!(sent(&lines[0])["params"]["value"], "default");
        assert_eq!(
            codec.command(DriveCommand::Configure(RunSettings::default())),
            Err(CommandRefused::NotOffered(Midway::Approvals))
        );
    }

    #[test]
    fn resumes_the_folders_session_whose_name_derives_the_thread() {
        let resume = Opening::Resume {
            thread: thread(),
            cwd: "/work/acme".into(),
            settings: RunSettings::default(),
        };
        let (mut codec, first) = OpenCodeAcp::open(&resume, "0.1.0", seed());
        let replies = feed(&mut codec, answer(&sent(&first)["id"], json!({}))).replies;
        let list = sent(&replies[0]);
        assert_eq!(list["method"], "session/list");
        assert_eq!(list["params"], json!({ "cwd": "/work/acme" }));
        // Not on the first page: the next one is asked for.
        let next = feed(
            &mut codec,
            answer(
                &list["id"],
                json!({ "sessions": [{ "sessionId": "ses_other1" }], "nextCursor": "c2" }),
            ),
        );
        let list = sent(&next.replies[0]);
        assert_eq!(list["params"]["cursor"], "c2");
        let found = feed(
            &mut codec,
            answer(
                &list["id"],
                json!({ "sessions": [{ "sessionId": SESSION }] }),
            ),
        );
        let resume = sent(&found.replies[0]);
        assert_eq!(resume["method"], "session/resume");
        assert_eq!(resume["params"]["sessionId"], SESSION);
        assert!(found.events.is_empty());
        assert_eq!(
            feed(&mut codec, answer(&resume["id"], json!({}))).events,
            vec![DriveEvent::Ready { thread: thread() }]
        );

        // A thread the folder has no session for is refused.
        let (mut lost, first) = OpenCodeAcp::open(
            &Opening::Resume {
                thread: NativeThreadId::new(Uuid::from_u128(9)).unwrap(),
                cwd: "/work/acme".into(),
                settings: RunSettings::default(),
            },
            "0.1.0",
            seed(),
        );
        let replies = feed(&mut lost, answer(&sent(&first)["id"], json!({}))).replies;
        let refused = feed(
            &mut lost,
            answer(&sent(&replies[0])["id"], json!({ "sessions": [] })),
        );
        assert!(matches!(
            refused.events.as_slice(),
            [DriveEvent::Refused {
                request: RequestKind::ThreadResume,
                ..
            }]
        ));
    }

    #[test]
    fn a_turn_starts_with_the_prompt_streams_its_reply_and_runs_tools() {
        let mut codec = ready();
        let request = prompted(&mut codec, "run ls");
        assert_eq!(
            codec.command(prompt("again")),
            Err(CommandRefused::TurnRunning)
        );
        let turn = turn(1);
        let thought = feed(
            &mut codec,
            update(
                json!({ "sessionUpdate": "agent_thought_chunk", "messageId": "msg_1:reasoning:0",
                "content": { "type": "text", "text": "List it." } }),
            ),
        );
        assert_eq!(thought.events.len(), 3);
        assert_eq!(thought.events[0], DriveEvent::TurnStarted { turn });
        let DriveEvent::Item {
            item,
            phase: ItemPhase::Completed,
        } = &thought.events[1]
        else {
            panic!("the prompt first: {:?}", thought.events);
        };
        assert_eq!(
            item.read,
            ItemRead::Prompt {
                text: "run ls".into()
            }
        );
        let DriveEvent::Item {
            item,
            phase: ItemPhase::Started,
        } = &thought.events[2]
        else {
            panic!("reasoning starts");
        };
        assert_eq!(item.kind, ItemKind::Reasoning);

        let call = feed(
            &mut codec,
            update(
                json!({ "sessionUpdate": "tool_call", "toolCallId": "call_1", "name": "shell",
                "title": "ls -1", "kind": "execute", "status": "pending", "rawInput": {} }),
            ),
        );
        // The reasoning closes; the command starts.
        let [
            DriveEvent::Item {
                phase: ItemPhase::Completed,
                item: reasoning,
            },
            DriveEvent::Item {
                phase: ItemPhase::Started,
                item: command,
            },
        ] = call.events.as_slice()
        else {
            panic!("{:?}", call.events);
        };
        assert_eq!(reasoning.kind, ItemKind::Reasoning);
        assert_eq!(command.id.as_str(), format!("{turn}:call_1"));
        let done = feed(
            &mut codec,
            update(
                json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call_1",
                "status": "completed", "rawInput": { "command": "ls -1", "cwd": "/work/acme" },
                "content": [{ "type": "content", "content": { "type": "text", "text": "a.txt\n" } }],
                "rawOutput": { "metadata": { "exit": 0 } } }),
            ),
        );
        let [
            DriveEvent::Item {
                phase: ItemPhase::Completed,
                item,
            },
        ] = done.events.as_slice()
        else {
            panic!("{:?}", done.events);
        };
        assert_eq!(
            item.read,
            ItemRead::Command {
                command: "ls -1".into(),
                status: ItemStatus::Completed,
                output: Some("a.txt\n".into()),
                exit_code: Some(0),
            }
        );
        let wrote = feed(
            &mut codec,
            update(
                json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call_2", "name": "write",
                "kind": "edit", "status": "completed",
                "locations": [{ "path": "/work/acme/x.txt" }] }),
            ),
        );
        let [DriveEvent::Item { item, .. }] = wrote.events.as_slice() else {
            panic!("{:?}", wrote.events);
        };
        assert_eq!(
            item.read,
            ItemRead::FileChange {
                paths: vec!["/work/acme/x.txt".into()],
                status: ItemStatus::Completed
            }
        );

        let reply = |text: &str| {
            update(
                json!({ "sessionUpdate": "agent_message_chunk", "messageId": "msg_2",
                "content": { "type": "text", "text": text } }),
            )
        };
        let first = feed(&mut codec, reply("Done"));
        assert!(matches!(
            first.events.as_slice(),
            [
                DriveEvent::Item {
                    phase: ItemPhase::Started,
                    ..
                },
                DriveEvent::Delta { .. }
            ]
        ));
        let more = feed(&mut codec, reply(", listed."));
        assert!(
            matches!(more.events.as_slice(), [DriveEvent::Delta { text, .. }] if text == ", listed.")
        );
        assert!(
            feed(
                &mut codec,
                update(json!({ "sessionUpdate": "usage_update", "used": 7 }))
            )
            .events
            .is_empty()
        );

        let ended = feed(
            &mut codec,
            answer(&request["id"], json!({ "stopReason": "end_turn" })),
        );
        let [
            DriveEvent::Item {
                phase: ItemPhase::Completed,
                item,
            },
            DriveEvent::TurnEnded { turn: ended, end },
        ] = ended.events.as_slice()
        else {
            panic!("{:?}", ended.events);
        };
        assert_eq!(
            item.read,
            ItemRead::Reply {
                text: "Done, listed.".into()
            }
        );
        assert_eq!((*ended, end), (turn, &TurnEnd::Completed));
        // Idle again: the next prompt is taken.
        prompted(&mut codec, "next");
    }

    /// opencode lists its commands as a turn begins, before the model
    /// answers: the turn has started, so it can be interrupted.
    #[test]
    fn the_command_list_a_prompt_brings_starts_its_turn() {
        let mut codec = ready();
        let request = prompted(&mut codec, "write an essay");
        let listed = feed(
            &mut codec,
            update(
                json!({ "sessionUpdate": "available_commands_update", "availableCommands": [] }),
            ),
        );
        assert_eq!(listed.events[0], DriveEvent::TurnStarted { turn: turn(1) });
        let lines = codec.command(DriveCommand::Interrupt).unwrap();
        assert_eq!(sent(&lines[0])["method"], "session/cancel");
        let ended = feed(
            &mut codec,
            answer(&request["id"], json!({ "stopReason": "end_turn" })),
        );
        // A stop asked for is not a stop observed: it finished first.
        assert_eq!(
            ended.events.last(),
            Some(&DriveEvent::TurnEnded {
                turn: turn(1),
                end: TurnEnd::Completed
            })
        );
    }

    #[test]
    fn asks_for_approval_and_answers_with_the_option_offered() {
        let mut codec = ready();
        let request = prompted(&mut codec, "run ls");
        let options = json!([
            { "optionId": "once", "kind": "allow_once", "name": "Allow once" },
            { "optionId": "always", "kind": "allow_always", "name": "Always allow" },
            { "optionId": "reject", "kind": "reject_once", "name": "Reject" },
        ]);
        let asked = feed(
            &mut codec,
            json!({ "jsonrpc": "2.0", "id": 0, "method": "session/request_permission", "params": {
                "sessionId": SESSION, "options": options,
                "toolCall": { "toolCallId": "call_1", "name": "shell", "title": "ls -1", "kind": "execute",
                    "rawInput": { "command": "ls -1", "cwd": "/work/acme" } } } }),
        );
        let approval: ApprovalId = "0".parse().unwrap();
        assert_eq!(
            asked.events.last(),
            Some(&DriveEvent::Approval {
                approval: approval.clone(),
                ask: ApprovalAsk::Command {
                    command: Some("ls -1".into()),
                    cwd: Some("/work/acme".into()),
                    reason: None
                },
            })
        );
        let lines = codec
            .command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::AcceptForSession,
            })
            .unwrap();
        assert_eq!(
            sent(&lines[0]),
            json!({ "jsonrpc": "2.0", "id": 0, "result": { "outcome": { "outcome": "selected", "optionId": "always" } } })
        );
        assert_eq!(
            codec.command(DriveCommand::Answer {
                approval,
                decision: Decision::Accept
            }),
            Err(CommandRefused::UnknownApproval("0".parse().unwrap()))
        );

        // An edit asked for, then the turn cancelled: the approval is
        // answered as cancelled and withdrawn when the turn ends.
        let edit = feed(
            &mut codec,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "session/request_permission", "params": {
                "sessionId": SESSION, "options": options,
                "toolCall": { "toolCallId": "call_2", "name": "write", "title": "x.txt", "kind": "edit" } } }),
        );
        assert!(matches!(
            edit.events.last(),
            Some(DriveEvent::Approval {
                ask: ApprovalAsk::FileChange { .. },
                ..
            })
        ));
        let lines: Vec<Value> = codec
            .command(DriveCommand::Interrupt)
            .unwrap()
            .iter()
            .map(sent)
            .collect();
        assert_eq!(
            lines[0],
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "outcome": { "outcome": "cancelled" } } })
        );
        assert_eq!(
            lines[1],
            json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": SESSION } })
        );
        let ended = feed(
            &mut codec,
            answer(&request["id"], json!({ "stopReason": "cancelled" })),
        );
        assert_eq!(
            ended.events.last(),
            Some(&DriveEvent::TurnEnded {
                turn: turn(1),
                end: TurnEnd::Interrupted
            })
        );
    }

    #[test]
    fn steering_adds_to_the_running_turn_and_other_requests_are_refused() {
        let mut codec = ready();
        assert_eq!(
            codec.command(DriveCommand::Steer(Prompt::parse("also".into()).unwrap())),
            Err(CommandRefused::NoTurn)
        );
        let request = prompted(&mut codec, "work");
        assert_eq!(
            codec.command(DriveCommand::Steer(Prompt::parse("also".into()).unwrap())),
            Err(CommandRefused::TurnNotStarted)
        );
        let _ = feed(
            &mut codec,
            update(json!({ "sessionUpdate": "agent_thought_chunk", "content": { "text": "…" } })),
        );
        let lines = codec
            .command(DriveCommand::Steer(Prompt::parse("also".into()).unwrap()))
            .unwrap();
        let steer = sent(&lines[0]);
        assert_eq!(steer["method"], "_session/steering");
        let injected = feed(
            &mut codec,
            answer(&steer["id"], json!({ "outcome": "injected" })),
        );
        assert!(matches!(
            injected.events.as_slice(),
            [DriveEvent::Item { item, .. }] if item.read == ItemRead::Prompt { text: "also".into() }
        ));

        // opencode asks for a file Ren never serves: refused with an error.
        let asked = feed(
            &mut codec,
            json!({ "jsonrpc": "2.0", "id": 7, "method": "fs/read_text_file", "params": {} }),
        );
        assert_eq!(
            asked.events,
            vec![DriveEvent::Unsupported {
                method: "fs/read_text_file".into()
            }]
        );
        assert_eq!(sent(&asked.replies[0])["error"]["code"], -32601);

        // A turn that fails ends failed.
        let failed = feed(
            &mut codec,
            json!({ "jsonrpc": "2.0", "id": request["id"], "error": { "code": -32603, "message": "provider down" } }),
        );
        assert_eq!(
            failed.events.last(),
            Some(&DriveEvent::TurnEnded {
                turn: turn(1),
                end: TurnEnd::Failed {
                    message: Some("provider down".into())
                }
            })
        );
        // A prompt refused before anything of its turn: no turn at all.
        let request = prompted(&mut codec, "again");
        let refused = feed(
            &mut codec,
            json!({ "jsonrpc": "2.0", "id": request["id"], "error": { "code": -32602, "message": "no model" } }),
        );
        assert_eq!(
            refused.events,
            vec![DriveEvent::Refused {
                request: RequestKind::TurnStart,
                message: "no model".into()
            }]
        );
    }
}
