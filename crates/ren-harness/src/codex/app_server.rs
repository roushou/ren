//! Codex's app server (`codex app-server`): JSON-RPC 2.0 over stdin and stdout,
//! one message per line, without the `"jsonrpc"` member. Read against the
//! protocol schema of Codex 0.162.1 (`codex app-server generate-json-schema`);
//! `tests/fixtures/codex-app-server-methods.txt` lists the methods that schema
//! has, and a test checks every method used here is among them.
//!
//! Ren opens one thread per process: `initialize`, then `initialized`, then
//! `thread/start` (or `thread/resume`). Approvals arrive as server requests and
//! are answered with the operator's decision. Codex's requests for ChatGPT
//! tokens, tool calls, and elicitation are refused: the client keeps its own
//! sign-in, and Ren provides no tools.

use crate::{
    commands::{ClientCommand, CommandKind, Sigil},
    drive::{
        ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, DriveItem,
        ItemId, ItemKind, ItemPhase, ItemRead, ItemStatus, Opening, Prompt, RawLine, Received,
        RequestKind, Retry, TurnEnd, Unreadable,
    },
    lines::{Framed, Line},
    models::{DefaultChoice, Listing, Model, ModelId, ModelListing},
    settings::{ApprovalPolicy, Effort, RunSettings},
};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The client's side of one `codex app-server` process.
#[derive(Debug)]
pub struct CodexAppServer {
    phase: Phase,
    next_request: u64,
    requests: BTreeMap<u64, RequestKind>,
    turn: TurnSlot,
    /// Approvals waiting for an answer, with the request id to answer.
    approvals: BTreeMap<ApprovalId, Value>,
    /// The effort every turn asks for; none is Codex's own.
    effort: Option<Effort>,
    /// Settings changed mid-run: from then on every turn asks for them.
    changed: Option<RunSettings>,
    /// The folder the thread works in, whose skills are listed.
    cwd: String,
    /// The skills Codex listed for the folder.
    skills: Vec<ClientCommand>,
    /// Each listed skill's path, by name: a prompt naming a skill (`$name`)
    /// sends it along, as Codex's own interface does.
    skill_paths: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Initializing(Opening),
    Opening,
    Ready(NativeThreadId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnSlot {
    Idle,
    /// `turn/start` was sent; its id is not known yet.
    Requested,
    Running(NativeTurnId),
}

/// JSON-RPC's "method not found", for server requests Ren never serves.
const METHOD_NOT_FOUND: i64 = -32601;

impl CodexAppServer {
    /// Every method this codec sends or reads, for the schema check.
    #[cfg(test)]
    const METHODS: &'static [&'static str] = &[
        "initialize",
        "initialized",
        "thread/start",
        "thread/resume",
        "model/list",
        "turn/start",
        "turn/interrupt",
        "turn/steer",
        "skills/list",
        "skills/changed",
        "thread/started",
        "turn/started",
        "turn/completed",
        "item/started",
        "item/completed",
        "item/agentMessage/delta",
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
        "serverRequest/resolved",
        "error",
    ];

    /// A codec for a freshly started server, and the first line to send it.
    /// `version` is Ren's own version, reported as the client's.
    pub fn open(opening: Opening, version: &str) -> (Self, Line) {
        let (effort, cwd) = match &opening {
            Opening::New { settings, cwd, .. } | Opening::Resume { settings, cwd, .. } => {
                (settings.effort, cwd.clone())
            }
        };
        let mut codec = Self {
            effort,
            changed: None,
            cwd,
            skills: Vec::new(),
            skill_paths: BTreeMap::new(),
            phase: Phase::Initializing(opening),
            next_request: 0,
            requests: BTreeMap::new(),
            turn: TurnSlot::Idle,
            approvals: BTreeMap::new(),
        };
        let line = codec.request(
            RequestKind::Initialize,
            json!({ "clientInfo": { "name": "ren", "title": "Ren", "version": version } }),
        );
        (codec, line)
    }

    /// The thread, once the server opened it.
    pub fn thread(&self) -> Option<NativeThreadId> {
        match self.phase {
            Phase::Ready(thread) => Some(thread),
            Phase::Initializing(_) | Phase::Opening => None,
        }
    }

    /// The skills a prompt can name, as Codex listed them for the thread's
    /// folder: each is `$name` in the prompt.
    pub fn commands(&self) -> &[ClientCommand] {
        &self.skills
    }

    /// The lines that carry `command`, or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Vec<Line>, CommandRefused> {
        let Phase::Ready(thread) = self.phase else {
            return Err(CommandRefused::NotReady);
        };
        match command {
            DriveCommand::Prompt(prompt) => self.prompt(thread, &prompt).map(|line| vec![line]),
            DriveCommand::Interrupt => match self.turn {
                TurnSlot::Running(turn) => Ok(vec![self.request(
                    RequestKind::TurnInterrupt,
                    json!({ "threadId": thread.to_string(), "turnId": turn.to_string() }),
                )]),
                TurnSlot::Requested => Err(CommandRefused::TurnNotStarted),
                TurnSlot::Idle => Err(CommandRefused::NoTurn),
            },
            DriveCommand::Answer { approval, decision } => {
                let Some(id) = self.approvals.remove(&approval) else {
                    return Err(CommandRefused::UnknownApproval(approval));
                };
                Ok(vec![Line::json(
                    &json!({ "id": id, "result": { "decision": Self::decision(decision) } }),
                )])
            }
            DriveCommand::Steer(prompt) => match self.turn {
                TurnSlot::Running(turn) => Ok(vec![self.request(
                    RequestKind::TurnSteer,
                    json!({
                        "threadId": thread.to_string(),
                        "expectedTurnId": turn.to_string(),
                        "input": self.input(&prompt),
                    }),
                )]),
                TurnSlot::Requested => Err(CommandRefused::TurnNotStarted),
                TurnSlot::Idle => Err(CommandRefused::NoTurn),
            },
            // Codex takes a turn's settings with the turn: kept for the
            // next one, and every one after.
            DriveCommand::Configure(settings) => {
                self.changed = Some(settings);
                Ok(Vec::new())
            }
        }
    }

    /// Read one frame from the server's stdout.
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
        let Ok(message) = serde_json::from_slice::<Message>(&bytes) else {
            let reason = match serde_json::from_slice::<Value>(&bytes) {
                Ok(_) => Unreadable::NotJsonRpc,
                Err(_) => Unreadable::NotJson,
            };
            received.events.push(DriveEvent::Unreadable {
                line: RawLine::new(bytes),
                reason,
            });
            return received;
        };
        let line = RawLine::new(bytes);
        match message {
            Message {
                method: Some(method),
                id: Some(id),
                params,
                ..
            } => self.server_request(&method, id, params, line, &mut received),
            Message {
                method: Some(method),
                id: None,
                params,
                ..
            } => self.notification(&method, params, line, &mut received),
            Message {
                method: None,
                id: Some(id),
                result,
                error,
                ..
            } => self.response(&id, result, error, line, &mut received),
            Message {
                method: None,
                id: None,
                ..
            } => received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::NotJsonRpc,
            }),
        }
        received
    }

    fn prompt(&mut self, thread: NativeThreadId, prompt: &Prompt) -> Result<Line, CommandRefused> {
        match self.turn {
            TurnSlot::Idle => {
                self.turn = TurnSlot::Requested;
                Ok(self.request(RequestKind::TurnStart, {
                    let mut params = json!({
                        "threadId": thread.to_string(),
                        "input": self.input(prompt),
                    });
                    let effort = match &self.changed {
                        Some(changed) => changed.effort,
                        None => self.effort,
                    };
                    if let (Some(effort), Value::Object(fields)) = (effort, &mut params) {
                        fields.insert("effort".into(), effort.to_string().into());
                    }
                    if let Some(changed) = &self.changed {
                        Self::turn_settings(&mut params, changed);
                    }
                    params
                }))
            }
            TurnSlot::Requested | TurnSlot::Running(_) => Err(CommandRefused::TurnRunning),
        }
    }

    /// A prompt as Codex takes it: its text, then each listed skill the
    /// text names (`$name`, standing on its own), once.
    fn input(&self, prompt: &Prompt) -> Value {
        let mut input = vec![json!({ "type": "text", "text": prompt.as_str() })];
        let mut named = Vec::new();
        for word in prompt.as_str().split_whitespace() {
            let Some(name) = word
                .strip_prefix('$')
                .map(|name| name.trim_end_matches(['.', ',', ';', '!', '?', ')', '"', '\'']))
            else {
                continue;
            };
            if let Some(path) = self.skill_paths.get(name)
                && !named.contains(&name)
            {
                named.push(name);
                input.push(json!({ "type": "skill", "name": name, "path": path }));
            }
        }
        Value::Array(input)
    }

    /// The skills in Codex's answer to `skills/list`: the enabled ones of
    /// every folder it answered for, by name.
    fn list_skills(&mut self, result: &Value) {
        self.skills.clear();
        self.skill_paths.clear();
        let skills = result
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("skills").and_then(Value::as_array))
            .flatten();
        for skill in skills {
            let text = |value: Option<&Value>| {
                value
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            };
            let (Some(name), Some(path)) = (text(skill.get("name")), text(skill.get("path")))
            else {
                continue;
            };
            if skill.get("enabled").and_then(Value::as_bool) == Some(false)
                || self.skill_paths.contains_key(&name)
            {
                continue;
            }
            // The short description reads best in a list.
            let description = text(skill.pointer("/interface/shortDescription"))
                .or_else(|| text(skill.get("shortDescription")))
                .or_else(|| text(skill.get("description")))
                .unwrap_or_default();
            self.skill_paths.insert(name.clone(), path);
            self.skills.push(ClientCommand {
                name,
                description,
                hint: None,
                kind: CommandKind::Skill,
                sigil: Sigil::Dollar,
            });
        }
    }

    /// Asks for the skills of the thread's folder.
    fn ask_skills(&mut self) -> Line {
        let cwds = json!({ "cwds": [self.cwd] });
        self.request(RequestKind::ListCommands, cwds)
    }

    fn request(&mut self, kind: RequestKind, params: Value) -> Line {
        let id = self.next_request;
        self.next_request += 1;
        self.requests.insert(id, kind);
        Line::json(&json!({ "id": id, "method": Self::method(kind), "params": params }))
    }

    fn method(kind: RequestKind) -> &'static str {
        match kind {
            RequestKind::Initialize => "initialize",
            RequestKind::ThreadStart => "thread/start",
            RequestKind::ThreadResume => "thread/resume",
            RequestKind::TurnStart => "turn/start",
            RequestKind::TurnInterrupt => "turn/interrupt",
            RequestKind::TurnSteer => "turn/steer",
            RequestKind::ListCommands => "skills/list",
            // Codex takes settings with each turn, never on their own.
            RequestKind::Configure => "turn/start",
        }
    }

    /// A turn on the model and approval policy chosen mid-run; each left
    /// out is Codex's own. The policy goes with its sandbox, as at the
    /// thread's start.
    fn turn_settings(params: &mut Value, settings: &RunSettings) {
        let Value::Object(fields) = params else {
            return;
        };
        if let Some(model) = &settings.model {
            fields.insert("model".into(), model.as_str().into());
        }
        if let Some(policy) = settings.approvals {
            let (approval, sandbox) = match policy {
                ApprovalPolicy::Ask => ("untrusted", "readOnly"),
                ApprovalPolicy::Edits => ("untrusted", "workspaceWrite"),
                ApprovalPolicy::Auto => ("on-request", "workspaceWrite"),
            };
            fields.insert("approvalPolicy".into(), approval.into());
            fields.insert("sandboxPolicy".into(), json!({ "type": sandbox }));
        }
    }

    /// Open the thread on the run's model and approval policy; each left
    /// out is Codex's own. A policy is Codex's approval policy with the
    /// sandbox it runs in: asking for everything keeps the folder read-only.
    fn choose(params: &mut Value, settings: &RunSettings) {
        let Value::Object(fields) = params else {
            return;
        };
        if let Some(model) = &settings.model {
            fields.insert("model".into(), model.as_str().into());
        }
        if let Some(policy) = settings.approvals {
            let (approval, sandbox) = match policy {
                ApprovalPolicy::Ask => ("untrusted", "read-only"),
                ApprovalPolicy::Edits => ("untrusted", "workspace-write"),
                ApprovalPolicy::Auto => ("on-request", "workspace-write"),
            };
            fields.insert("approvalPolicy".into(), approval.into());
            fields.insert("sandbox".into(), sandbox.into());
        }
    }

    fn decision(decision: Decision) -> &'static str {
        match decision {
            Decision::Accept => "accept",
            Decision::AcceptForSession => "acceptForSession",
            Decision::Decline => "decline",
            Decision::Cancel => "cancel",
        }
    }

    fn response(
        &mut self,
        id: &Value,
        result: Option<Value>,
        error: Option<RpcError>,
        line: RawLine,
        received: &mut Received,
    ) {
        let Some(kind) = id.as_u64().and_then(|id| self.requests.remove(&id)) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::UnexpectedResponse,
            });
            return;
        };
        if let Some(error) = error {
            // Without its skills a run goes on; the list stays empty.
            if kind == RequestKind::ListCommands {
                return;
            }
            if kind == RequestKind::TurnStart {
                self.turn = TurnSlot::Idle;
            }
            received.events.push(DriveEvent::Refused {
                request: kind,
                message: error.message,
            });
            return;
        }
        let result = result.unwrap_or(Value::Null);
        match kind {
            RequestKind::Initialize => {
                let opening = std::mem::replace(&mut self.phase, Phase::Opening);
                let Phase::Initializing(opening) = opening else {
                    self.phase = opening;
                    received.events.push(DriveEvent::Unreadable {
                        line,
                        reason: Unreadable::UnexpectedResponse,
                    });
                    return;
                };
                received
                    .replies
                    .push(Line::json(&json!({ "method": "initialized" })));
                received.replies.push(match opening {
                    Opening::New { cwd, settings, .. } => {
                        let mut params = json!({ "cwd": cwd });
                        Self::choose(&mut params, &settings);
                        self.request(RequestKind::ThreadStart, params)
                    }
                    Opening::Resume {
                        thread,
                        cwd,
                        settings,
                    } => {
                        let mut params = json!({ "threadId": thread.to_string(), "cwd": cwd });
                        Self::choose(&mut params, &settings);
                        self.request(RequestKind::ThreadResume, params)
                    }
                });
            }
            RequestKind::ThreadStart | RequestKind::ThreadResume => {
                match serde_json::from_value::<ThreadOpened>(result)
                    .ok()
                    .and_then(|opened| Self::native_thread(&opened.thread.id))
                {
                    Some(thread) => {
                        self.phase = Phase::Ready(thread);
                        received.events.push(DriveEvent::Ready { thread });
                        let ask = self.ask_skills();
                        received.replies.push(ask);
                    }
                    None => received.events.push(DriveEvent::Unreadable {
                        line,
                        reason: Unreadable::BadParams,
                    }),
                }
            }
            RequestKind::TurnStart => {
                match serde_json::from_value::<TurnHolder>(result)
                    .ok()
                    .and_then(|holder| Self::native_turn(&holder.turn.id))
                {
                    Some(turn) => self.turn_started(turn, received),
                    None => received.events.push(DriveEvent::Unreadable {
                        line,
                        reason: Unreadable::BadParams,
                    }),
                }
            }
            RequestKind::ListCommands => self.list_skills(&result),
            RequestKind::TurnInterrupt | RequestKind::TurnSteer | RequestKind::Configure => {}
        }
    }

    fn notification(
        &mut self,
        method: &str,
        params: Option<Value>,
        line: RawLine,
        received: &mut Received,
    ) {
        let params = params.unwrap_or(Value::Null);
        let event = match method {
            "turn/started" => match Self::parse::<TurnHolder>(params)
                .and_then(|holder| Self::native_turn(&holder.turn.id))
            {
                Some(turn) => {
                    self.turn_started(turn, received);
                    return;
                }
                None => None,
            },
            "turn/completed" => Self::parse::<TurnHolder>(params).and_then(|holder| {
                let turn = Self::native_turn(&holder.turn.id)?;
                self.turn = TurnSlot::Idle;
                let pending = std::mem::take(&mut self.approvals);
                received.events.extend(
                    pending
                        .into_keys()
                        .map(|approval| DriveEvent::ApprovalResolved { approval }),
                );
                Some(DriveEvent::TurnEnded {
                    turn,
                    end: holder.turn.end(),
                })
            }),
            "item/started" | "item/completed" => {
                let phase = match method {
                    "item/started" => ItemPhase::Started,
                    _ => ItemPhase::Completed,
                };
                Self::parse::<ItemHolder>(params).and_then(|holder| {
                    let id = holder.item.get("id")?.as_str()?.to_owned();
                    let kind = Self::item_kind(holder.item.get("type")?.as_str()?);
                    let read = Self::item_read(&kind, &holder.item, phase);
                    Some(DriveEvent::Item {
                        phase,
                        item: DriveItem {
                            id: ItemId::new(id),
                            kind,
                            read,
                            body: holder.item,
                        },
                    })
                })
            }
            "item/agentMessage/delta" => {
                Self::parse::<Delta>(params).map(|delta| DriveEvent::Delta {
                    item: ItemId::new(delta.item_id),
                    text: delta.delta,
                })
            }
            "serverRequest/resolved" => Self::parse::<Resolved>(params).map(|resolved| {
                let approval = Self::approval_id(&resolved.request_id);
                self.approvals.remove(&approval);
                DriveEvent::ApprovalResolved { approval }
            }),
            "error" => Self::parse::<ErrorParams>(params).map(|error| DriveEvent::Error {
                message: error.error.message,
                retry: if error.will_retry {
                    Retry::WillRetry
                } else {
                    Retry::Final
                },
            }),
            // Ren learns the thread from `thread/start`'s response.
            "thread/started" => return,
            // A skill was added or removed: listed again.
            "skills/changed" => {
                if let Phase::Ready(_) = self.phase {
                    let ask = self.ask_skills();
                    received.replies.push(ask);
                }
                return;
            }
            _ => {
                received.events.push(DriveEvent::Unknown { line });
                return;
            }
        };
        received
            .events
            .push(event.unwrap_or(DriveEvent::Unreadable {
                line,
                reason: Unreadable::BadParams,
            }));
    }

    fn server_request(
        &mut self,
        method: &str,
        id: Value,
        params: Option<Value>,
        line: RawLine,
        received: &mut Received,
    ) {
        let params = params.unwrap_or(Value::Null);
        let ask = match method {
            "item/commandExecution/requestApproval" => {
                Self::parse::<CommandApproval>(params).map(|ask| ApprovalAsk::Command {
                    command: ask.command,
                    cwd: ask.cwd,
                    reason: ask.reason,
                })
            }
            "item/fileChange/requestApproval" => Self::parse::<FileChangeApproval>(params)
                .map(|ask| ApprovalAsk::FileChange { reason: ask.reason }),
            _ => {
                received.replies.push(Line::json(&json!({
                    "id": id,
                    "error": { "code": METHOD_NOT_FOUND, "message": "not supported by Ren" },
                })));
                received.events.push(DriveEvent::Unsupported {
                    method: method.to_owned(),
                });
                return;
            }
        };
        match ask {
            Some(ask) => {
                let approval = Self::approval_id(&id);
                self.approvals.insert(approval.clone(), id);
                received.events.push(DriveEvent::Approval { approval, ask });
            }
            None => {
                // Never leave the server waiting on a request Ren cannot read.
                received.replies.push(Line::json(&json!({
                    "id": id,
                    "error": { "code": METHOD_NOT_FOUND, "message": "unreadable request" },
                })));
                received.events.push(DriveEvent::Unreadable {
                    line,
                    reason: Unreadable::BadParams,
                });
            }
        }
    }

    fn turn_started(&mut self, turn: NativeTurnId, received: &mut Received) {
        match self.turn {
            TurnSlot::Running(running) if running == turn => {}
            TurnSlot::Idle | TurnSlot::Requested | TurnSlot::Running(_) => {
                self.turn = TurnSlot::Running(turn);
                received.events.push(DriveEvent::TurnStarted { turn });
            }
        }
    }

    fn parse<T: for<'de> Deserialize<'de>>(params: Value) -> Option<T> {
        serde_json::from_value(params).ok()
    }

    fn native_thread(id: &str) -> Option<NativeThreadId> {
        Uuid::try_parse(id).ok().and_then(NativeThreadId::new)
    }

    fn native_turn(id: &str) -> Option<NativeTurnId> {
        Uuid::try_parse(id).ok().and_then(NativeTurnId::new)
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

    fn item_kind(kind: &str) -> ItemKind {
        match kind {
            "userMessage" => ItemKind::UserMessage,
            "agentMessage" => ItemKind::AgentMessage,
            "reasoning" => ItemKind::Reasoning,
            "plan" => ItemKind::Plan,
            "commandExecution" => ItemKind::CommandExecution,
            "fileChange" => ItemKind::FileChange,
            "mcpToolCall" => ItemKind::McpToolCall,
            "webSearch" => ItemKind::WebSearch,
            other => ItemKind::Other(other.to_owned()),
        }
    }

    /// What Ren reads of an item, from the fields Codex's schema gives each
    /// kind. A field that is missing or of another type reads as absent.
    fn item_read(kind: &ItemKind, body: &Value, phase: ItemPhase) -> ItemRead {
        let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_owned);
        let status = || match body.get("status").and_then(Value::as_str) {
            Some("inProgress") => ItemStatus::InProgress,
            Some("completed") => ItemStatus::Completed,
            Some("failed") => ItemStatus::Failed,
            Some("declined") => ItemStatus::Declined,
            Some(other) => ItemStatus::Other(other.to_owned()),
            None => ItemStatus::of(phase),
        };
        match kind {
            ItemKind::UserMessage => ItemRead::Prompt {
                text: body
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            ItemKind::AgentMessage => ItemRead::Reply {
                text: text("text").unwrap_or_default(),
            },
            ItemKind::Reasoning => ItemRead::Reasoning,
            ItemKind::Plan => ItemRead::Plan {
                text: text("text").unwrap_or_default(),
            },
            ItemKind::CommandExecution => ItemRead::Command {
                command: text("command").unwrap_or_default(),
                status: status(),
                output: text("aggregatedOutput"),
                exit_code: body.get("exitCode").and_then(Value::as_i64),
            },
            ItemKind::FileChange => ItemRead::FileChange {
                paths: body
                    .get("changes")
                    .and_then(Value::as_array)
                    .map(|changes| {
                        changes
                            .iter()
                            .filter_map(|change| change.get("path").and_then(Value::as_str))
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                status: status(),
            },
            ItemKind::McpToolCall => ItemRead::Tool {
                name: text("tool").unwrap_or_else(|| "MCP tool".to_owned()),
                status: status(),
            },
            ItemKind::WebSearch => ItemRead::Tool {
                name: text("query").map_or_else(
                    || "web search".to_owned(),
                    |query| format!("search: {query}"),
                ),
                status: status(),
            },
            ItemKind::Other(_) => ItemRead::Unread,
        }
    }
}

/// Any JSON-RPC message, before it is told apart.
#[derive(Deserialize)]
struct Message {
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    message: String,
}

#[derive(Deserialize)]
struct ThreadOpened {
    thread: IdOnly,
}

#[derive(Deserialize)]
struct IdOnly {
    id: String,
}

#[derive(Deserialize)]
struct TurnHolder {
    turn: TurnBody,
}

#[derive(Deserialize)]
struct TurnBody {
    id: String,
    status: Option<String>,
    error: Option<RpcError>,
}

impl TurnBody {
    fn end(self) -> TurnEnd {
        match self.status.as_deref() {
            Some("completed") => TurnEnd::Completed,
            Some("interrupted") => TurnEnd::Interrupted,
            Some("failed") => TurnEnd::Failed {
                message: self.error.map(|error| error.message),
            },
            Some(other) => TurnEnd::Unknown {
                status: other.to_owned(),
            },
            None => TurnEnd::Unknown {
                status: String::new(),
            },
        }
    }
}

#[derive(Deserialize)]
struct ItemHolder {
    item: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Delta {
    item_id: String,
    delta: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resolved {
    request_id: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorParams {
    error: RpcError,
    will_retry: bool,
}

#[derive(Deserialize)]
struct CommandApproval {
    command: Option<String>,
    cwd: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct FileChangeApproval {
    reason: Option<String>,
}

/// A listing process's side of `codex app-server`: `initialize`, then
/// `model/list`, page by page, without the models Codex hides from its own
/// picker.
#[derive(Debug)]
pub struct CodexModels {
    models: Vec<Model>,
    /// The request Ren waits on.
    waiting: CodexListStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexListStep {
    Initialize,
    List(u64),
    Done,
}

impl CodexModels {
    const INITIALIZE: u64 = 0;

    /// The codec, and the first line to send. `version` is Ren's own version.
    pub fn open(version: &str) -> (Self, Line) {
        let line = Line::json(&json!({
            "id": Self::INITIALIZE,
            "method": "initialize",
            "params": { "clientInfo": { "name": "ren", "title": "Ren", "version": version } },
        }));
        (
            Self {
                models: Vec::new(),
                waiting: CodexListStep::Initialize,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return Listing::waiting();
        };
        if message.get("method").is_some() {
            // A request from the server: a listing answers none.
            return Listing::Waiting(vec![Line::json(&json!({
                "id": message["id"],
                "error": { "code": METHOD_NOT_FOUND, "message": "Ren is only listing models" },
            }))]);
        }
        if let Some(error) = message.get("error") {
            return Listing::Failed(
                ModelListing::text(error, "message").unwrap_or_else(|| "refused".into()),
            );
        }
        match self.waiting {
            CodexListStep::Initialize if id == Self::INITIALIZE => Listing::Waiting(vec![
                Line::json(&json!({ "method": "initialized" })),
                self.page(1, None),
            ]),
            CodexListStep::List(request) if id == request => {
                let result = &message["result"];
                let Some(data) = result.get("data").and_then(Value::as_array) else {
                    return Listing::Failed("Codex listed no models".into());
                };
                self.models.extend(data.iter().filter_map(Self::model));
                match ModelListing::text(result, "nextCursor") {
                    Some(cursor) => Listing::Waiting(vec![self.page(request + 1, Some(cursor))]),
                    None => {
                        self.waiting = CodexListStep::Done;
                        Listing::Listed(std::mem::take(&mut self.models))
                    }
                }
            }
            CodexListStep::Initialize | CodexListStep::List(_) | CodexListStep::Done => {
                Listing::waiting()
            }
        }
    }

    fn page(&mut self, request: u64, cursor: Option<String>) -> Line {
        self.waiting = CodexListStep::List(request);
        Line::json(&json!({
            "id": request,
            "method": "model/list",
            "params": { "includeHidden": false, "cursor": cursor },
        }))
    }

    /// One listed model; its `model` slug is what `thread/start` takes.
    fn model(entry: &Value) -> Option<Model> {
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let id: ModelId = ModelListing::text(entry, "model")
            .or_else(|| ModelListing::text(entry, "id"))?
            .parse()
            .ok()?;
        Some(Model {
            name: ModelListing::text(entry, "displayName").unwrap_or_else(|| id.to_string()),
            description: ModelListing::text(entry, "description"),
            provider: None,
            choice: match entry.get("isDefault").and_then(Value::as_bool) {
                Some(true) => DefaultChoice::Default,
                Some(false) | None => DefaultChoice::Other,
            },
            efforts: ModelListing::efforts(
                entry.get("supportedReasoningEfforts"),
                Some("reasoningEffort"),
            ),
            effort: ModelListing::text(entry, "defaultReasoningEffort")
                .and_then(|effort| effort.parse().ok()),
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CodexAppServer;
    use crate::commands::{ClientCommand, CommandKind, Sigil};
    use crate::drive::{Opening, Received};
    use crate::{
        drive::{
            ApprovalAsk, ApprovalId, CommandRefused, Decision, DriveCommand, DriveEvent, ItemKind,
            ItemPhase, ItemRead, ItemStatus, Prompt, RequestKind, Retry, TurnEnd, Unreadable,
        },
        lines::{Framed, Line},
    };
    use serde_json::{Value, json};

    const THREAD: &str = "0190f3a0-0000-7000-8000-000000000001";
    const TURN: &str = "0190f3a0-0000-7000-8000-000000000002";

    /// The id Ren would give a new conversation, which Codex ignores.
    fn named() -> ren_protocol::native::NativeThreadId {
        ren_protocol::native::NativeThreadId::new(uuid::Uuid::from_u128(9)).unwrap()
    }

    fn sent(line: &Line) -> Value {
        let bytes = line.as_bytes();
        assert_eq!(bytes.last(), Some(&b'\n'));
        serde_json::from_slice(bytes).unwrap()
    }

    fn feed(codec: &mut CodexAppServer, message: Value) -> Received {
        codec.receive(Framed::Line(message.to_string().into_bytes()))
    }

    /// A codec whose thread is open, as after the opening handshake.
    fn ready() -> CodexAppServer {
        let (mut codec, first) = CodexAppServer::open(
            Opening::New {
                cwd: "/work/acme".into(),
                id: named(),
                settings: crate::settings::RunSettings::default(),
            },
            "0.1.0",
        );
        assert_eq!(
            sent(&first),
            json!({ "id": 0, "method": "initialize", "params": {
                "clientInfo": { "name": "ren", "title": "Ren", "version": "0.1.0" } } })
        );
        let received = feed(&mut codec, json!({ "id": 0, "result": {} }));
        let replies: Vec<Value> = received.replies.iter().map(sent).collect();
        assert_eq!(
            replies,
            vec![
                json!({ "method": "initialized" }),
                json!({ "id": 1, "method": "thread/start", "params": { "cwd": "/work/acme" } }),
            ]
        );
        assert_eq!(codec.thread(), None);
        let received = feed(
            &mut codec,
            json!({ "id": 1, "result": { "thread": { "id": THREAD }, "model": "gpt" } }),
        );
        assert_eq!(
            received.events,
            vec![DriveEvent::Ready {
                thread: codec.thread().unwrap()
            }]
        );
        // Once open, Ren asks for the folder's skills.
        assert_eq!(
            received.replies.iter().map(sent).collect::<Vec<_>>(),
            [json!({ "id": 2, "method": "skills/list", "params": { "cwds": ["/work/acme"] } })]
        );
        let _ = feed(
            &mut codec,
            json!({ "id": 2, "result": { "data": [{ "cwd": "/work/acme", "errors": [], "skills": [
                { "name": "release-notes", "description": "Write release notes from the log",
                  "enabled": true, "path": "/work/acme/.codex/skills/release-notes/SKILL.md",
                  "scope": "repo" },
                { "name": "off", "description": "", "enabled": false,
                  "path": "/s/off/SKILL.md", "scope": "user" }
            ] }] } }),
        );
        codec
    }

    fn prompt(text: &str) -> DriveCommand {
        DriveCommand::Prompt(Prompt::parse(text.into()).unwrap())
    }

    #[test]
    fn refuses_commands_before_the_thread_opens() {
        let (mut codec, _) = CodexAppServer::open(
            Opening::New {
                cwd: "/w".into(),
                id: named(),
                settings: crate::settings::RunSettings::default(),
            },
            "0.1.0",
        );
        assert_eq!(codec.command(prompt("hi")), Err(CommandRefused::NotReady));
    }

    #[test]
    fn resumes_an_existing_thread() {
        let thread = uuid::Uuid::parse_str(THREAD).unwrap();
        let thread = ren_protocol::native::NativeThreadId::new(thread).unwrap();
        let (mut codec, _) = CodexAppServer::open(
            Opening::Resume {
                thread,
                cwd: "/w".into(),
                settings: crate::settings::RunSettings::default(),
            },
            "0.1.0",
        );
        let received = feed(&mut codec, json!({ "id": 0, "result": {} }));
        assert_eq!(
            sent(&received.replies[1]),
            json!({ "id": 1, "method": "thread/resume", "params": { "threadId": THREAD, "cwd": "/w" } })
        );
    }

    #[test]
    fn runs_a_turn_with_streamed_text_and_items() {
        let mut codec = ready();
        let line = codec.command(prompt("fix the build")).unwrap().remove(0);
        assert_eq!(
            sent(&line),
            json!({ "id": 3, "method": "turn/start", "params": {
                "threadId": THREAD, "input": [{ "type": "text", "text": "fix the build" }] } })
        );
        assert_eq!(
            codec.command(prompt("again")),
            Err(CommandRefused::TurnRunning)
        );
        assert_eq!(
            codec.command(DriveCommand::Interrupt),
            Err(CommandRefused::TurnNotStarted)
        );

        let turn = json!({ "id": TURN, "status": "inProgress", "items": [] });
        let started = feed(&mut codec, json!({ "id": 3, "result": { "turn": turn } }));
        let notified = feed(
            &mut codec,
            json!({ "method": "turn/started", "params": { "threadId": THREAD, "turn": turn } }),
        );
        assert!(matches!(
            started.events.as_slice(),
            [DriveEvent::TurnStarted { .. }]
        ));
        assert!(notified.events.is_empty(), "the turn starts once");

        let delta = feed(
            &mut codec,
            json!({ "method": "item/agentMessage/delta", "params": {
                "threadId": THREAD, "turnId": TURN, "itemId": "msg_1", "delta": "Done" } }),
        );
        assert!(matches!(
            delta.events.as_slice(),
            [DriveEvent::Delta { item, text }] if item.as_str() == "msg_1" && text == "Done"
        ));
        let item = feed(
            &mut codec,
            json!({ "method": "item/completed", "params": {
                "threadId": THREAD, "turnId": TURN, "completedAtMs": 1,
                "item": { "type": "commandExecution", "id": "cmd_1", "command": "cargo build",
                          "exitCode": 0 } } }),
        );
        assert!(matches!(
            item.events.as_slice(),
            [DriveEvent::Item { phase: ItemPhase::Completed, item }]
                if item.kind == ItemKind::CommandExecution && item.read == ItemRead::Command {
                    command: "cargo build".into(),
                    status: ItemStatus::Completed,
                    output: None,
                    exit_code: Some(0),
                }
        ));

        let interrupt = codec.command(DriveCommand::Interrupt).unwrap().remove(0);
        assert_eq!(
            sent(&interrupt),
            json!({ "id": 4, "method": "turn/interrupt", "params": { "threadId": THREAD, "turnId": TURN } })
        );
        let ended = feed(
            &mut codec,
            json!({ "method": "turn/completed", "params": { "threadId": THREAD,
                "turn": { "id": TURN, "status": "interrupted", "items": [] } } }),
        );
        assert!(matches!(
            ended.events.as_slice(),
            [DriveEvent::TurnEnded {
                end: TurnEnd::Interrupted,
                ..
            }]
        ));
        assert_eq!(
            codec.command(DriveCommand::Interrupt),
            Err(CommandRefused::NoTurn)
        );
        assert!(codec.command(prompt("next")).is_ok());
    }

    #[test]
    fn reports_failed_and_unknown_turn_ends() {
        let mut codec = ready();
        let _ = codec.command(prompt("go")).unwrap().remove(0);
        let failed = feed(
            &mut codec,
            json!({ "method": "turn/completed", "params": { "threadId": THREAD, "turn": {
                "id": TURN, "status": "failed", "error": { "message": "usage limit" } } } }),
        );
        assert!(matches!(
            failed.events.as_slice(),
            [DriveEvent::TurnEnded { end: TurnEnd::Failed { message: Some(m) }, .. }] if m == "usage limit"
        ));
        let _ = codec.command(prompt("go")).unwrap().remove(0);
        let odd = feed(
            &mut codec,
            json!({ "method": "turn/completed", "params": { "threadId": THREAD, "turn": {
                "id": TURN, "status": "paused" } } }),
        );
        assert!(matches!(
            odd.events.as_slice(),
            [DriveEvent::TurnEnded { end: TurnEnd::Unknown { status }, .. }] if status == "paused"
        ));
    }

    #[test]
    fn asks_for_approval_and_answers_with_the_same_request_id() {
        let mut codec = ready();
        let _ = codec.command(prompt("go")).unwrap().remove(0);
        let asked = feed(
            &mut codec,
            json!({ "id": "req-7", "method": "item/commandExecution/requestApproval", "params": {
                "threadId": THREAD, "turnId": TURN, "itemId": "cmd_1", "startedAtMs": 1,
                "command": "rm -rf target", "cwd": "/work/acme", "reason": "clean" } }),
        );
        let [DriveEvent::Approval { approval, ask }] = asked.events.as_slice() else {
            panic!("expected an approval, got {:?}", asked.events);
        };
        assert_eq!(
            ask,
            &ApprovalAsk::Command {
                command: Some("rm -rf target".into()),
                cwd: Some("/work/acme".into()),
                reason: Some("clean".into()),
            }
        );
        let answer = codec
            .command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::AcceptForSession,
            })
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&answer),
            json!({ "id": "req-7", "result": { "decision": "acceptForSession" } })
        );
        assert_eq!(
            codec.command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::Accept,
            }),
            Err(CommandRefused::UnknownApproval(approval.clone()))
        );

        let file = feed(
            &mut codec,
            json!({ "id": 9, "method": "item/fileChange/requestApproval", "params": {
                "threadId": THREAD, "turnId": TURN, "itemId": "fc_1", "startedAtMs": 1 } }),
        );
        assert!(matches!(
            file.events.as_slice(),
            [DriveEvent::Approval { approval, ask: ApprovalAsk::FileChange { reason: None } }]
                if approval == &"9".parse::<ApprovalId>().unwrap()
        ));
        let ended = feed(
            &mut codec,
            json!({ "method": "turn/completed", "params": { "threadId": THREAD,
                "turn": { "id": TURN, "status": "completed" } } }),
        );
        assert!(
            matches!(
                ended.events.as_slice(),
                [
                    DriveEvent::ApprovalResolved { .. },
                    DriveEvent::TurnEnded {
                        end: TurnEnd::Completed,
                        ..
                    }
                ]
            ),
            "a turn's end withdraws its waiting approvals: {:?}",
            ended.events
        );
    }

    #[test]
    fn a_resolved_request_no_longer_waits() {
        let mut codec = ready();
        let _ = codec.command(prompt("go")).unwrap().remove(0);
        let _ = feed(
            &mut codec,
            json!({ "id": 4, "method": "item/fileChange/requestApproval", "params": {
                "threadId": THREAD, "turnId": TURN, "itemId": "fc_1", "startedAtMs": 1 } }),
        );
        let resolved = feed(
            &mut codec,
            json!({ "method": "serverRequest/resolved", "params": { "threadId": THREAD, "requestId": 4 } }),
        );
        let [DriveEvent::ApprovalResolved { approval }] = resolved.events.as_slice() else {
            panic!("{:?}", resolved.events);
        };
        assert!(matches!(
            codec.command(DriveCommand::Answer {
                approval: approval.clone(),
                decision: Decision::Decline
            }),
            Err(CommandRefused::UnknownApproval(_))
        ));
    }

    #[test]
    fn never_answers_for_credentials_or_unknown_requests() {
        let mut codec = ready();
        for method in [
            "account/chatgptAuthTokens/refresh",
            "item/tool/call",
            "mcpServer/elicitation/request",
        ] {
            let received = feed(
                &mut codec,
                json!({ "id": 11, "method": method, "params": { "reason": "unauthorized" } }),
            );
            assert_eq!(
                received.replies.iter().map(sent).collect::<Vec<_>>(),
                vec![
                    json!({ "id": 11, "error": { "code": -32601, "message": "not supported by Ren" } })
                ]
            );
            assert_eq!(
                received.events,
                vec![DriveEvent::Unsupported {
                    method: method.into()
                }]
            );
        }
    }

    #[test]
    fn answers_an_unreadable_approval_with_an_error() {
        let mut codec = ready();
        let received = feed(
            &mut codec,
            json!({ "id": 5, "method": "item/commandExecution/requestApproval", "params": { "command": 3 } }),
        );
        assert_eq!(received.replies.len(), 1);
        assert!(matches!(
            received.events.as_slice(),
            [DriveEvent::Unreadable {
                reason: Unreadable::BadParams,
                ..
            }]
        ));
    }

    #[test]
    fn keeps_unknown_and_unreadable_lines() {
        let mut codec = ready();
        let unknown = json!({ "method": "thread/tokenUsage/updated", "params": { "x": 1 } });
        let received = feed(&mut codec, unknown.clone());
        let [DriveEvent::Unknown { line: raw }] = received.events.as_slice() else {
            panic!("{:?}", received.events);
        };
        assert_eq!(
            serde_json::from_slice::<Value>(raw.as_bytes()).unwrap(),
            unknown
        );

        for (bytes, reason) in [
            (&b"not json"[..], Unreadable::NotJson),
            (&b"[1,2]"[..], Unreadable::NotJsonRpc),
            (
                &b"{\"id\":99,\"result\":{}}"[..],
                Unreadable::UnexpectedResponse,
            ),
        ] {
            let received = codec.receive(Framed::Line(bytes.to_vec()));
            assert!(
                matches!(received.events.as_slice(), [DriveEvent::Unreadable { reason: r, line }]
                    if *r == reason && line.as_bytes() == bytes),
                "{:?}",
                received.events
            );
        }
        let long = codec.receive(Framed::TooLong { bytes: 40 });
        assert!(matches!(
            long.events.as_slice(),
            [DriveEvent::Unreadable {
                reason: Unreadable::TooLong { bytes: 40 },
                ..
            }]
        ));
        assert!(codec.receive(Framed::Line(Vec::new())).events.is_empty());
    }

    #[test]
    fn lists_the_folders_skills_and_sends_the_ones_a_prompt_names() {
        let mut codec = ready();
        assert_eq!(
            codec.commands(),
            [ClientCommand {
                name: "release-notes".into(),
                description: "Write release notes from the log".into(),
                hint: None,
                kind: CommandKind::Skill,
                sigil: Sigil::Dollar,
            }]
        );
        let line = codec
            .command(prompt(
                "use $release-notes, then $release-notes. $unknown too",
            ))
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&line)["params"]["input"],
            json!([
                { "type": "text", "text": "use $release-notes, then $release-notes. $unknown too" },
                { "type": "skill", "name": "release-notes",
                  "path": "/work/acme/.codex/skills/release-notes/SKILL.md" }
            ])
        );
        // A changed set of skills is listed again.
        let changed = feed(
            &mut codec,
            json!({ "method": "skills/changed", "params": {} }),
        );
        assert_eq!(sent(&changed.replies[0])["method"], "skills/list");
        assert!(changed.events.is_empty());
    }

    #[test]
    fn reports_refusals_and_errors() {
        let mut codec = ready();
        let _ = codec.command(prompt("go")).unwrap().remove(0);
        let refused = feed(
            &mut codec,
            json!({ "id": 3, "error": { "code": -32000, "message": "thread busy" } }),
        );
        assert_eq!(
            refused.events,
            vec![DriveEvent::Refused {
                request: RequestKind::TurnStart,
                message: "thread busy".into()
            }]
        );
        assert!(
            codec.command(prompt("go")).is_ok(),
            "a refused turn frees the slot"
        );
        let error = feed(
            &mut codec,
            json!({ "method": "error", "params": { "threadId": THREAD, "turnId": TURN,
                "willRetry": true, "error": { "message": "stream disconnected" } } }),
        );
        assert_eq!(
            error.events,
            vec![DriveEvent::Error {
                message: "stream disconnected".into(),
                retry: Retry::WillRetry
            }]
        );
    }

    #[test]
    fn every_method_used_is_in_the_pinned_schema() {
        let schema: Vec<&str> = include_str!("../../tests/fixtures/codex-app-server-methods.txt")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .collect();
        for method in CodexAppServer::METHODS {
            assert!(
                schema.contains(method),
                "{method} is not in the pinned schema"
            );
        }
    }

    /// A message added to the running turn goes to that turn; settings
    /// changed mid-run go with every turn from the next.
    #[test]
    fn steers_the_running_turn_and_takes_new_settings_with_the_next() {
        use crate::settings::{ApprovalPolicy, Effort, RunSettings};
        let mut codec = ready();
        assert_eq!(
            codec.command(DriveCommand::Steer(Prompt::parse("x".into()).unwrap())),
            Err(CommandRefused::NoTurn)
        );
        let _ = codec.command(prompt("fix the build")).unwrap();
        let _ = feed(
            &mut codec,
            json!({ "id": 3, "result": { "turn": { "id": TURN } } }),
        );
        let steer = codec
            .command(DriveCommand::Steer(
                Prompt::parse("use the old API".into()).unwrap(),
            ))
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&steer),
            json!({ "id": 4, "method": "turn/steer", "params": { "threadId": THREAD,
                "expectedTurnId": TURN,
                "input": [{ "type": "text", "text": "use the old API" }] } })
        );
        let settings = RunSettings {
            model: Some("gpt-deep".parse().unwrap()),
            effort: Some(Effort::High),
            approvals: Some(ApprovalPolicy::Ask),
        };
        assert_eq!(
            codec.command(DriveCommand::Configure(settings)),
            Ok(Vec::new())
        );
        let _ = feed(
            &mut codec,
            json!({ "method": "turn/completed", "params": { "turn": { "id": TURN, "status": "completed" } } }),
        );
        let next = codec.command(prompt("again")).unwrap().remove(0);
        let params = &sent(&next)["params"];
        assert_eq!(params["model"], "gpt-deep");
        assert_eq!(params["effort"], "high");
        assert_eq!(params["approvalPolicy"], "untrusted");
        assert_eq!(params["sandboxPolicy"], json!({ "type": "readOnly" }));
    }
}
