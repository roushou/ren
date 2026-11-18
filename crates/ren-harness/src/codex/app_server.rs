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
    codec::{ApprovalBook, EchoQueue, Envelope, JsonRpc, RequestLedger, RpcError, TurnSlot},
    commands::{ClientCommand, CommandKind, Sigil},
    drive::{
        ApprovalAsk, ApprovalId, CommandIntent, CommandPart, CommandRefused, Decision,
        DriveCommand, DriveEvent, DriveItem, ItemId, ItemKind, ItemPhase, ItemRead, ItemStatus,
        Opening, RawLine, Received, RequestKind, Retry, TurnEnd, Unreadable,
    },
    lines::{Framed, Line},
    models::{DefaultChoice, Listing, Model, ModelId, ModelListing},
    prompt::Prompt,
    settings::{ApprovalPolicy, Effort, RunSettings},
};
use ren_protocol::{
    label::{Labeled, Stored},
    native::{NativeThreadId, NativeTurnId},
};
use serde::Deserialize;
use serde_json::{Value, json};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::collections::BTreeMap;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use uuid::Uuid;

/// The client's side of one `codex app-server` process.
#[derive(Debug)]
pub struct CodexAppServer {
    phase: Phase,
    /// Requests Ren sent and waits on, by request id.
    requests: RequestLedger<u64, RequestKind>,
    turn: TurnSlot,
    /// Approvals waiting for an answer, with the request id to answer.
    approvals: ApprovalBook<Value>,
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
    /// What the images of each prompt and steer sent are, in order, until
    /// Codex echoes it as a user message.
    echoing: EchoQueue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Initializing(Opening),
    Opening,
    Ready(NativeThreadId),
}

/// Every method Ren sends Codex's app server or reads from it, named as in
/// Codex's protocol schema. A method read that Ren does not know stays
/// unknown.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CodexMethod {
    Initialize,
    Initialized,
    #[strum(to_string = "thread/start")]
    ThreadStart,
    #[strum(to_string = "thread/resume")]
    ThreadResume,
    #[strum(to_string = "model/list")]
    ModelList,
    #[strum(to_string = "turn/start")]
    TurnStart,
    #[strum(to_string = "turn/interrupt")]
    TurnInterrupt,
    #[strum(to_string = "turn/steer")]
    TurnSteer,
    #[strum(to_string = "skills/list")]
    SkillsList,
    #[strum(to_string = "skills/changed")]
    SkillsChanged,
    #[strum(to_string = "thread/started")]
    ThreadStarted,
    #[strum(to_string = "turn/started")]
    TurnStarted,
    #[strum(to_string = "turn/completed")]
    TurnCompleted,
    #[strum(to_string = "item/started")]
    ItemStarted,
    #[strum(to_string = "item/completed")]
    ItemCompleted,
    #[strum(to_string = "item/agentMessage/delta")]
    AgentMessageDelta,
    #[strum(to_string = "item/commandExecution/requestApproval")]
    CommandExecutionRequestApproval,
    #[strum(to_string = "item/fileChange/requestApproval")]
    FileChangeRequestApproval,
    #[strum(to_string = "serverRequest/resolved")]
    ServerRequestResolved,
    Error,
}

/// JSON-RPC's "method not found", for server requests Ren never serves.
const METHOD_NOT_FOUND: i64 = -32601;

impl CodexAppServer {
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
            echoing: EchoQueue::default(),
            phase: Phase::Initializing(opening),
            requests: RequestLedger::new(|n| n),
            turn: TurnSlot::Idle,
            approvals: ApprovalBook::new(),
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
            DriveCommand::Interrupt => {
                let turn = self.turn.running()?;
                Ok(vec![self.request(
                    RequestKind::TurnInterrupt,
                    json!({ "threadId": thread.to_string(), "turnId": turn.to_string() }),
                )])
            }
            DriveCommand::Answer { approval, decision } => {
                let id = self.approvals.answer(approval)?;
                Ok(vec![Line::json(
                    &json!({ "id": id, "result": { "decision": Self::decision(decision) } }),
                )])
            }
            DriveCommand::Steer(prompt) => {
                let turn = self.turn.running()?;
                let input = self.input(&prompt);
                Ok(vec![self.request(
                    RequestKind::TurnSteer,
                    json!({
                        "threadId": thread.to_string(),
                        "expectedTurnId": turn.to_string(),
                        "input": input,
                    }),
                )])
            }
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
        let Envelope { line, body } = match Envelope::open(frame) {
            Ok(envelope) => envelope,
            Err(received) => return received,
        };
        let mut received = Received::default();
        match JsonRpc::classify(body) {
            JsonRpc::Request { id, method, params } => {
                self.server_request(&method, id, params, line, &mut received);
            }
            JsonRpc::Notification { method, params } => {
                self.notification(&method, params, line, &mut received);
            }
            JsonRpc::Response { id, outcome } => self.response(&id, outcome, line, &mut received),
            JsonRpc::Invalid => received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::NotJsonRpc,
            }),
        }
        received
    }

    fn prompt(&mut self, thread: NativeThreadId, prompt: &Prompt) -> Result<Line, CommandRefused> {
        self.turn.request(())?;
        let input = self.input(prompt);
        Ok(self.request(RequestKind::TurnStart, {
            let mut params = json!({
                "threadId": thread.to_string(),
                "input": input,
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

    /// A prompt as Codex takes it: each image by the path of its file, its
    /// text, then each listed skill the text names (`$name`, standing on
    /// its own), once. The images are remembered for the prompt's echo.
    fn input(&mut self, prompt: &Prompt) -> Value {
        self.echoing.push(prompt);
        let mut input: Vec<Value> = prompt
            .images()
            .iter()
            .map(|image| json!({ "type": "localImage", "path": image.path() }))
            .collect();
        if !prompt.as_str().trim().is_empty() || input.is_empty() {
            input.push(json!({ "type": "text", "text": prompt.as_str() }));
        }
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
        for (skill, path) in Self::skills(result) {
            self.skill_paths.insert(skill.name.clone(), path);
            self.skills.push(skill);
        }
    }

    /// Each enabled skill in an answer to `skills/list`, once by name, with
    /// the path of its file.
    fn skills(result: &Value) -> Vec<(ClientCommand, String)> {
        let mut listed: Vec<(ClientCommand, String)> = Vec::new();
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
                || listed.iter().any(|(known, _)| known.name == name)
            {
                continue;
            }
            // The short description reads best in a list.
            let description = text(skill.pointer("/interface/shortDescription"))
                .or_else(|| text(skill.get("shortDescription")))
                .or_else(|| text(skill.get("description")))
                .unwrap_or_default();
            listed.push((
                ClientCommand {
                    name,
                    description,
                    hint: None,
                    kind: CommandKind::Skill,
                    sigil: Sigil::Dollar,
                },
                path,
            ));
        }
        listed
    }

    /// Asks for the skills of the thread's folder.
    fn ask_skills(&mut self) -> Line {
        let cwds = json!({ "cwds": [self.cwd] });
        self.request(RequestKind::ListCommands, cwds)
    }

    fn request(&mut self, kind: RequestKind, params: Value) -> Line {
        let id = self.requests.send(kind);
        let method = Self::method(kind).label();
        Line::json(&json!({ "id": id, "method": method, "params": params }))
    }

    fn method(kind: RequestKind) -> CodexMethod {
        match kind {
            RequestKind::Initialize => CodexMethod::Initialize,
            RequestKind::ThreadStart => CodexMethod::ThreadStart,
            RequestKind::ThreadResume => CodexMethod::ThreadResume,
            RequestKind::TurnStart => CodexMethod::TurnStart,
            RequestKind::TurnInterrupt => CodexMethod::TurnInterrupt,
            RequestKind::TurnSteer => CodexMethod::TurnSteer,
            RequestKind::ListCommands => CodexMethod::SkillsList,
            // Codex takes settings with each turn, never on their own.
            RequestKind::Configure => CodexMethod::TurnStart,
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
        outcome: Result<Value, RpcError>,
        line: RawLine,
        received: &mut Received,
    ) {
        // Codex's errors always say what went wrong.
        let outcome = match outcome {
            Ok(result) => Ok(result),
            Err(RpcError {
                message: Some(message),
            }) => Err(message),
            Err(RpcError { message: None }) => {
                received.events.push(DriveEvent::Unreadable {
                    line,
                    reason: Unreadable::NotJsonRpc,
                });
                return;
            }
        };
        let Some(kind) = id.as_u64().and_then(|id| self.requests.answer(&id)) else {
            received.events.push(DriveEvent::Unreadable {
                line,
                reason: Unreadable::UnexpectedResponse,
            });
            return;
        };
        let result = match outcome {
            Ok(result) => result,
            Err(message) => {
                // Without its skills a run goes on; the list stays empty.
                if kind == RequestKind::ListCommands {
                    return;
                }
                if kind == RequestKind::TurnStart {
                    self.turn.clear();
                }
                received.events.push(DriveEvent::Refused {
                    request: kind,
                    message,
                });
                return;
            }
        };
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
                received.replies.push(Line::json(
                    &json!({ "method": CodexMethod::Initialized.label() }),
                ));
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
        params: Value,
        line: RawLine,
        received: &mut Received,
    ) {
        let event = match Stored::<CodexMethod>::from_label(method) {
            Stored::Known(CodexMethod::TurnStarted) => match Self::parse::<TurnHolder>(params)
                .and_then(|holder| Self::native_turn(&holder.turn.id))
            {
                Some(turn) => {
                    self.turn_started(turn, received);
                    return;
                }
                None => None,
            },
            Stored::Known(CodexMethod::TurnCompleted) => Self::parse::<TurnHolder>(params)
                .and_then(|holder| {
                    let turn = Self::native_turn(&holder.turn.id)?;
                    self.turn.clear();
                    self.approvals.withdraw(received);
                    Some(DriveEvent::TurnEnded {
                        turn,
                        end: holder.turn.end(),
                    })
                }),
            Stored::Known(CodexMethod::ItemStarted) => self.item(ItemPhase::Started, params),
            Stored::Known(CodexMethod::ItemCompleted) => self.item(ItemPhase::Completed, params),
            Stored::Known(CodexMethod::AgentMessageDelta) => {
                Self::parse::<Delta>(params).map(|delta| DriveEvent::Delta {
                    item: ItemId::new(delta.item_id),
                    text: delta.delta,
                })
            }
            Stored::Known(CodexMethod::ServerRequestResolved) => Self::parse::<Resolved>(params)
                .map(|resolved| {
                    let approval = ApprovalId::of_request(&resolved.request_id);
                    self.approvals.resolve(&approval);
                    DriveEvent::ApprovalResolved { approval }
                }),
            Stored::Known(CodexMethod::Error) => {
                Self::parse::<ErrorParams>(params).map(|error| DriveEvent::Error {
                    message: error.error.message,
                    retry: if error.will_retry {
                        Retry::WillRetry
                    } else {
                        Retry::Final
                    },
                })
            }
            // Ren learns the thread from `thread/start`'s response.
            Stored::Known(CodexMethod::ThreadStarted) => return,
            // A skill was added or removed: listed again.
            Stored::Known(CodexMethod::SkillsChanged) => {
                if let Phase::Ready(_) = self.phase {
                    let ask = self.ask_skills();
                    received.replies.push(ask);
                }
                return;
            }
            Stored::Known(
                CodexMethod::Initialize
                | CodexMethod::Initialized
                | CodexMethod::ThreadStart
                | CodexMethod::ThreadResume
                | CodexMethod::ModelList
                | CodexMethod::TurnStart
                | CodexMethod::TurnInterrupt
                | CodexMethod::TurnSteer
                | CodexMethod::SkillsList
                | CodexMethod::CommandExecutionRequestApproval
                | CodexMethod::FileChangeRequestApproval,
            )
            | Stored::Unknown(_) => {
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

    /// An item started or completed.
    fn item(&mut self, phase: ItemPhase, params: Value) -> Option<DriveEvent> {
        let holder = Self::parse::<ItemHolder>(params)?;
        let id = holder.item.get("id")?.as_str()?.to_owned();
        let kind = Self::item_kind(holder.item.get("type")?.as_str()?);
        let mut read = Self::item_read(&kind, &holder.item, phase);
        // The echo of a prompt sent with images: its start shows them, its
        // completion takes them.
        let with_images = holder.item["content"].as_array().is_some_and(|parts| {
            parts.iter().any(|part| {
                matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("localImage" | "image")
                )
            })
        });
        if let ItemRead::Prompt { images, .. } = &mut read
            && with_images
        {
            *images = match phase {
                ItemPhase::Started => self.echoing.front(),
                ItemPhase::Completed => self.echoing.pop(),
            };
        }
        Some(DriveEvent::Item {
            phase,
            item: DriveItem {
                id: ItemId::new(id),
                kind,
                read,
                body: holder.item,
            },
        })
    }

    fn server_request(
        &mut self,
        method: &str,
        id: Value,
        params: Value,
        line: RawLine,
        received: &mut Received,
    ) {
        let ask = match Stored::<CodexMethod>::from_label(method) {
            Stored::Known(CodexMethod::CommandExecutionRequestApproval) => {
                Self::parse::<CommandApproval>(params).map(|ask| ApprovalAsk::Command {
                    command: ask.command,
                    cwd: ask.cwd,
                    reason: ask.reason,
                })
            }
            Stored::Known(CodexMethod::FileChangeRequestApproval) => {
                Self::parse::<FileChangeApproval>(params)
                    .map(|ask| ApprovalAsk::FileChange { reason: ask.reason })
            }
            Stored::Known(
                CodexMethod::Initialize
                | CodexMethod::Initialized
                | CodexMethod::ThreadStart
                | CodexMethod::ThreadResume
                | CodexMethod::ModelList
                | CodexMethod::TurnStart
                | CodexMethod::TurnInterrupt
                | CodexMethod::TurnSteer
                | CodexMethod::SkillsList
                | CodexMethod::SkillsChanged
                | CodexMethod::ThreadStarted
                | CodexMethod::TurnStarted
                | CodexMethod::TurnCompleted
                | CodexMethod::ItemStarted
                | CodexMethod::ItemCompleted
                | CodexMethod::AgentMessageDelta
                | CodexMethod::ServerRequestResolved
                | CodexMethod::Error,
            )
            | Stored::Unknown(_) => {
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
                let approval = ApprovalId::of_request(&id);
                self.approvals.ask(approval.clone(), id);
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
        if let Some(started) = self.turn.start(turn) {
            received.events.push(started);
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

    /// What Codex read of a command's parts (`commandActions`): unsaid when
    /// it gave none.
    fn command_intent(actions: Option<&Value>) -> CommandIntent {
        let parts: Vec<CommandPart> = actions
            .and_then(Value::as_array)
            .map(|actions| actions.iter().map(Self::command_part).collect())
            .unwrap_or_default();
        if parts.is_empty() {
            CommandIntent::Unsaid
        } else {
            CommandIntent::Parts { parts }
        }
    }

    fn command_part(action: &Value) -> CommandPart {
        let text = |key: &str| action.get(key).and_then(Value::as_str).map(str::to_owned);
        let unrecognized = || CommandPart::Unrecognized {
            raw: action.clone(),
        };
        let Some(command) = text("command") else {
            return unrecognized();
        };
        match action.get("type").and_then(Value::as_str) {
            Some("read") => match (text("path"), text("name")) {
                (Some(path), Some(name)) => CommandPart::Read {
                    command,
                    path,
                    name,
                },
                (None, _) | (_, None) => unrecognized(),
            },
            Some("listFiles") => CommandPart::ListFiles {
                command,
                path: text("path"),
            },
            Some("search") => CommandPart::Search {
                command,
                query: text("query"),
                path: text("path"),
            },
            Some("unknown") => CommandPart::Unknown { command },
            Some(_) | None => unrecognized(),
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
                images: Vec::new(),
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
                intent: Self::command_intent(body.get("commandActions")),
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
                effects: Vec::new(),
                status: status(),
            },
            ItemKind::WebSearch => ItemRead::Tool {
                name: "web search".to_owned(),
                effects: text("query")
                    .map(|target| crate::tools::ToolEffect {
                        kind: crate::tools::EffectKind::Search,
                        target,
                    })
                    .into_iter()
                    .collect(),
                status: status(),
            },
            ItemKind::Other(_) => ItemRead::Unread,
        }
    }
}

/// An error Codex reports in a notification.
#[derive(Deserialize)]
struct CodexError {
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
    error: Option<CodexError>,
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
    error: CodexError,
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

/// A listing process's side of `codex app-server` for a folder's skills:
/// `initialize`, then `skills/list` for that folder.
#[derive(Debug)]
pub struct CodexCommands {
    /// The folder whose skills are listed.
    folder: String,
    waiting: CodexListStep,
}

impl CodexCommands {
    const INITIALIZE: u64 = 0;
    const SKILLS: u64 = 1;

    /// The codec, and the first line to send. `version` is Ren's own version.
    pub fn open(version: &str, folder: &str) -> (Self, Line) {
        let (_, line) = CodexModels::open(version);
        (
            Self {
                folder: folder.to_owned(),
                waiting: CodexListStep::Initialize,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing<Vec<ClientCommand>> {
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return Listing::waiting();
        };
        if message.get("method").is_some() {
            // A request from the server: a listing answers none.
            return Listing::Waiting(vec![Line::json(&json!({
                "id": message["id"],
                "error": { "code": METHOD_NOT_FOUND, "message": "Ren is only listing skills" },
            }))]);
        }
        if let Some(error) = message.get("error") {
            return Listing::Failed(
                ModelListing::text(error, "message").unwrap_or_else(|| "refused".into()),
            );
        }
        match self.waiting {
            CodexListStep::Initialize if id == Self::INITIALIZE => {
                self.waiting = CodexListStep::List(Self::SKILLS);
                Listing::Waiting(vec![
                    Line::json(&json!({ "method": CodexMethod::Initialized.label() })),
                    Line::json(&json!({
                        "id": Self::SKILLS,
                        "method": CodexMethod::SkillsList.label(),
                        "params": { "cwds": [self.folder] },
                    })),
                ])
            }
            CodexListStep::List(request) if id == request => {
                self.waiting = CodexListStep::Done;
                Listing::Listed(
                    CodexAppServer::skills(&message["result"])
                        .into_iter()
                        .map(|(skill, _)| skill)
                        .collect(),
                )
            }
            CodexListStep::Initialize | CodexListStep::List(_) | CodexListStep::Done => {
                Listing::waiting()
            }
        }
    }
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
            "method": CodexMethod::Initialize.label(),
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
                Line::json(&json!({ "method": CodexMethod::Initialized.label() })),
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
            "method": CodexMethod::ModelList.label(),
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
            ApprovalAsk, ApprovalId, CommandIntent, CommandPart, CommandRefused, Decision,
            DriveCommand, DriveEvent, ItemKind, ItemPhase, ItemRead, ItemStatus, RequestKind,
            Retry, TurnEnd, Unreadable,
        },
        lines::{Framed, Line},
        prompt::Prompt,
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
                    intent: CommandIntent::Unsaid,
                    status: ItemStatus::Completed,
                    output: None,
                    exit_code: Some(0),
                }
        ));
        // Codex's reading of a command's parts is its intent; a part of a
        // kind Ren does not know, or missing what its kind needs, is kept.
        let parted = feed(
            &mut codec,
            json!({ "method": "item/completed", "params": {
                "threadId": THREAD, "turnId": TURN, "completedAtMs": 2,
                "item": { "type": "commandExecution", "id": "cmd_2",
                          "command": "sed -n 1,9p a.rs; rg -n todo src; ls; make",
                          "commandActions": [
                              { "type": "read", "command": "sed -n 1,9p a.rs",
                                "name": "a.rs", "path": "/work/acme/a.rs" },
                              { "type": "search", "command": "rg -n todo src",
                                "query": "todo", "path": "src" },
                              { "type": "listFiles", "command": "ls", "path": null },
                              { "type": "unknown", "command": "make" },
                              { "type": "read", "command": "cat" },
                              { "type": "teleport", "command": "beam" }
                          ],
                          "exitCode": 0 } } }),
        );
        let [DriveEvent::Item { item, .. }] = parted.events.as_slice() else {
            panic!("one item: {:?}", parted.events);
        };
        let ItemRead::Command { intent, .. } = &item.read else {
            panic!("a command: {:?}", item.read);
        };
        assert_eq!(
            intent,
            &CommandIntent::Parts {
                parts: vec![
                    CommandPart::Read {
                        command: "sed -n 1,9p a.rs".into(),
                        path: "/work/acme/a.rs".into(),
                        name: "a.rs".into(),
                    },
                    CommandPart::Search {
                        command: "rg -n todo src".into(),
                        query: Some("todo".into()),
                        path: Some("src".into()),
                    },
                    CommandPart::ListFiles {
                        command: "ls".into(),
                        path: None,
                    },
                    CommandPart::Unknown {
                        command: "make".into(),
                    },
                    CommandPart::Unrecognized {
                        raw: json!({ "type": "read", "command": "cat" }),
                    },
                    CommandPart::Unrecognized {
                        raw: json!({ "type": "teleport", "command": "beam" }),
                    },
                ],
            }
        );

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
        for method in <super::CodexMethod as strum::VariantArray>::VARIANTS {
            let method: &&str = &method.into();
            assert!(
                schema.contains(method),
                "{method} is not in the pinned schema"
            );
        }
    }

    /// A message added to the running turn goes to that turn; settings
    /// changed mid-run go with every turn from the next.
    #[test]
    fn sends_images_by_path_and_shows_them_on_the_echo() {
        use crate::prompt::PromptImage;
        let mut codec = ready();
        let image = PromptImage::sample();
        let start = codec
            .command(DriveCommand::Prompt(Prompt::sample("what is this")))
            .unwrap()
            .remove(0);
        assert_eq!(
            sent(&start)["params"]["input"],
            json!([
                { "type": "localImage", "path": "/data/attachments/s/a/shot.png" },
                { "type": "text", "text": "what is this" },
            ])
        );
        let echo = |method: &str| {
            json!({ "method": method, "params": { "threadId": THREAD, "turnId": TURN,
                "item": { "type": "userMessage", "id": "user_1", "content": [
                    { "type": "localImage", "path": "/data/attachments/s/a/shot.png" },
                    { "type": "text", "text": "what is this" } ] } } })
        };
        for method in ["item/started", "item/completed"] {
            let echoed = feed(&mut codec, echo(method));
            let [DriveEvent::Item { item, .. }] = echoed.events.as_slice() else {
                panic!("{:?}", echoed.events);
            };
            assert_eq!(
                item.read,
                ItemRead::Prompt {
                    text: "what is this".into(),
                    images: vec![image.read().clone()],
                },
                "{method}"
            );
        }
        // A message without images takes none.
        let plain = feed(
            &mut codec,
            json!({ "method": "item/completed", "params": { "threadId": THREAD, "turnId": TURN,
                "item": { "type": "userMessage", "id": "user_2",
                          "content": [{ "type": "text", "text": "and this" }] } } }),
        );
        assert!(matches!(
            plain.events.as_slice(),
            [DriveEvent::Item { item, .. }] if item.read == ItemRead::Prompt {
                text: "and this".into(), images: Vec::new() }
        ));
    }

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
