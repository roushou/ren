//! Driving a client through its structured mode: the official client program,
//! started by Ren, exchanging messages on stdin and stdout instead of drawing
//! a terminal interface. The client keeps its own sign-in; nothing here reads,
//! refreshes, or answers for credentials.
//!
//! Each client's protocol is a pure codec (no I/O): commands in, lines for its
//! stdin out; lines from its stdout in, events and replies out. Framing
//! (`lines`) knows nothing about JSON.

use crate::Harness;
use crate::attachment::{Attachment, Offer};
use crate::claude_code::ClaudeStreamJson;
use crate::codex::CodexAppServer;
use crate::commands::ClientCommand;
use crate::lines::{Framed, Line};
use crate::opencode::OpenCodeAcp;
use crate::pi::PiRpc;
use crate::prompt::Prompt;
use crate::settings::{Midway, RunSettings};
use crate::tools::ToolEffect;
use ren_protocol::activity::{ToolInput, ToolUse};
use ren_protocol::inspect::ToolCall;
use ren_protocol::messages::BlockKind;
use ren_protocol::text::WireText;

use ren_protocol::native::{NativeThreadId, NativeTurnId};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How a driven process's one conversation is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// A new conversation in this working directory. `id` is the id Ren
    /// gives it where the client lets Ren choose (Claude Code, Pi); Codex
    /// chooses its own and reports it.
    New {
        cwd: String,
        id: NativeThreadId,
        settings: RunSettings,
    },
    /// An existing conversation, continued in this working directory.
    Resume {
        thread: NativeThreadId,
        cwd: String,
        settings: RunSettings,
    },
}

impl Opening {
    /// The settings the conversation opens with.
    pub fn settings(&self) -> &RunSettings {
        match self {
            Self::New { settings, .. } | Self::Resume { settings, .. } => settings,
        }
    }
}

/// What one line from a driven client produced.
#[derive(Debug, Default)]
#[must_use]
pub struct Received {
    pub events: Vec<DriveEvent>,
    /// Lines to write to the client's stdin, in order.
    pub replies: Vec<Line>,
}

impl Received {
    /// What a line produced: `event`, and no replies.
    pub(crate) fn event(event: DriveEvent) -> Self {
        Self {
            events: vec![event],
            replies: Vec::new(),
        }
    }
}

/// One driven client's side of its structured mode, chosen by its harness.
#[derive(Debug)]
pub enum Driver {
    Codex(CodexAppServer),
    ClaudeCode(ClaudeStreamJson),
    Pi(PiRpc),
    OpenCode(OpenCodeAcp),
}

impl Driver {
    /// The codec for a freshly started `harness` process, and the lines to
    /// send it first. `version` is Ren's own version; `seed` is a fresh id
    /// that a client which names no turns (Pi, OpenCode) has its turn ids
    /// minted from.
    pub fn open(
        harness: Harness,
        opening: Opening,
        version: &str,
        seed: uuid::Uuid,
    ) -> (Self, Vec<Line>) {
        match harness {
            Harness::Codex => {
                let (codec, first) = CodexAppServer::open(opening, version);
                (Self::Codex(codec), vec![first])
            }
            Harness::ClaudeCode => {
                let (codec, first) = ClaudeStreamJson::open(&opening);
                (Self::ClaudeCode(codec), vec![first])
            }
            Harness::Pi => {
                let (codec, first) = PiRpc::open(&opening, seed);
                (Self::Pi(codec), vec![first])
            }
            Harness::OpenCode => {
                let (codec, first) = OpenCodeAcp::open(&opening, version, seed);
                (Self::OpenCode(codec), vec![first])
            }
        }
    }

    /// The lines that carry `command` (none when it waits for the next
    /// turn), or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Vec<Line>, CommandRefused> {
        match self {
            Self::Codex(codec) => codec.command(command),
            Self::ClaudeCode(codec) => codec.command(command),
            Self::Pi(codec) => codec.command(command),
            Self::OpenCode(codec) => codec.command(command),
        }
    }

    /// The commands the client offers a prompt, as far as it has listed
    /// them: none before its conversation opens.
    pub fn commands(&self) -> &[ClientCommand] {
        match self {
            Self::Codex(codec) => codec.commands(),
            Self::ClaudeCode(codec) => codec.commands(),
            Self::Pi(codec) => codec.commands(),
            Self::OpenCode(codec) => codec.commands(),
        }
    }

    /// What the client said of images: only OpenCode says, in its answer
    /// to `initialize`; the others always take them (`Harness::acceptance`).
    pub fn image_offer(&self) -> Offer {
        match self {
            Self::OpenCode(codec) => codec.image_offer(),
            Self::Codex(_) | Self::ClaudeCode(_) | Self::Pi(_) => Offer::Unknown,
        }
    }

    /// Read one frame from the client's stdout.
    pub fn receive(&mut self, frame: Framed) -> Received {
        match self {
            Self::Codex(codec) => codec.receive(frame),
            Self::ClaudeCode(codec) => codec.receive(frame),
            Self::Pi(codec) => codec.receive(frame),
            Self::OpenCode(codec) => codec.receive(frame),
        }
    }
}

/// What Ren asks a driven client to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveCommand {
    /// Start a turn with this prompt.
    Prompt(Prompt),
    /// Answer an approval the client asked for.
    Answer {
        approval: ApprovalId,
        decision: Decision,
    },
    /// Interrupt the running turn.
    Interrupt,
    /// Add to the running turn: the client reads it before its next step.
    Steer(Prompt),
    /// Run on these settings from the next turn on.
    Configure(RunSettings),
}

/// An approval request, named as the client named it within one process.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct ApprovalId(pub(crate) String);

impl ApprovalId {
    /// A JSON-RPC request's id as an approval: strings as they are, numbers
    /// in decimal.
    pub(crate) fn of_request(id: &serde_json::Value) -> Self {
        use serde_json::Value;
        Self(match id {
            Value::String(text) => text.clone(),
            other @ (Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_)) => other.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an approval identifier cannot be empty")]
pub struct EmptyApprovalId;

impl std::str::FromStr for ApprovalId {
    type Err = EmptyApprovalId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Err(EmptyApprovalId)
        } else {
            Ok(Self(value.into()))
        }
    }
}

impl std::fmt::Display for ApprovalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// How the operator answered an approval.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Decision {
    /// Allow this one action.
    Accept,
    /// Allow it, and the same kind of action for the rest of the session.
    AcceptForSession,
    /// Refuse it; the turn goes on.
    Decline,
    /// Refuse it and interrupt the turn.
    Cancel,
}

/// A request Ren sent to a driven client.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum RequestKind {
    Initialize,
    ThreadStart,
    ThreadResume,
    TurnStart,
    TurnInterrupt,
    TurnSteer,
    /// A change of model, effort, or approvals mid-run.
    Configure,
    /// The commands, skills, and templates the client offers a prompt.
    ListCommands,
}

/// What a driven client reported, as Ren reads it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriveEvent {
    /// The conversation is open and takes prompts.
    Ready {
        thread: NativeThreadId,
    },
    TurnStarted {
        turn: NativeTurnId,
    },
    /// Streamed text of the reply item `item`.
    Delta {
        item: ItemId,
        text: String,
    },
    Item {
        phase: ItemPhase,
        item: DriveItem,
    },
    /// The client waits for the operator's decision.
    Approval {
        approval: ApprovalId,
        ask: ApprovalAsk,
    },
    /// An approval no longer waits: answered, or withdrawn by the client.
    ApprovalResolved {
        approval: ApprovalId,
    },
    TurnEnded {
        turn: NativeTurnId,
        end: TurnEnd,
    },
    /// The client reported an error; it may retry on its own.
    Error {
        message: String,
        retry: Retry,
    },
    /// The client refused a request Ren sent.
    Refused {
        request: RequestKind,
        message: String,
    },
    /// The client asked Ren for something Ren never provides (credentials,
    /// tool calls, elicitation). Ren answered with an error.
    Unsupported {
        method: String,
    },
    /// A well-formed message Ren does not read, kept as it arrived.
    Unknown {
        line: RawLine,
    },
    /// A line Ren could not read.
    Unreadable {
        line: RawLine,
        reason: Unreadable,
    },
}

/// Whether the client said it would retry after an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    WillRetry,
    Final,
}

/// Why a line could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Unreadable {
    NotJson,
    NotJsonRpc,
    /// A response to a request Ren never sent.
    UnexpectedResponse,
    /// A known message whose parameters did not match its shape.
    BadParams,
    /// JSON without the message type every line of the client's protocol has.
    Untyped,
    /// A line longer than the framer keeps.
    TooLong {
        bytes: usize,
    },
}

/// A line exactly as the client wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine(Vec<u8>);

/// Serialized as text, with invalid UTF-8 replaced; the bytes stay kept.
impl serde::Serialize for RawLine {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&String::from_utf8_lossy(&self.0))
    }
}

/// Read back as the text it was serialized as.
impl<'de> serde::Deserialize<'de> for RawLine {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|text| Self(text.into_bytes()))
    }
}

impl RawLine {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// An item of a turn, named as the client named it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ItemId(String);

impl ItemId {
    pub(crate) fn new(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemPhase {
    Started,
    Completed,
}

/// One item of a turn: its kind, what Ren read of it, and the client's own
/// body for it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DriveItem {
    pub id: ItemId,
    pub kind: ItemKind,
    pub read: ItemRead,
    pub body: serde_json::Value,
}

impl DriveItem {
    /// `body` without the bytes of any image inlined in it: the `data` of
    /// each part whose `type` is `image` or `base64` becomes a note of how
    /// long it was. An echoed prompt then never carries an image's bytes
    /// into what Ren records and publishes.
    pub fn without_image_bytes(body: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match body {
            Value::Object(mut fields) => {
                let inline = matches!(
                    fields.get("type").and_then(Value::as_str),
                    Some("image" | "base64")
                );
                if inline && let Some(Value::String(data)) = fields.get("data") {
                    let note = format!("{} characters of base64, not kept", data.len());
                    fields.insert("data".into(), Value::String(note));
                }
                Value::Object(
                    fields
                        .into_iter()
                        .map(|(key, value)| (key, Self::without_image_bytes(value)))
                        .collect(),
                )
            }
            Value::Array(parts) => {
                Value::Array(parts.into_iter().map(Self::without_image_bytes).collect())
            }
            other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
        }
    }
}

/// What Ren read from an item, the same for every client, so consumers never
/// parse a client's own body. Each client's codec fills it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum ItemRead {
    /// The operator's prompt, as the client echoed it, with what each
    /// image Ren sent with it is.
    Prompt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<Attachment>,
    },
    /// Reply text; empty while it is still streaming as deltas.
    Reply {
        text: String,
    },
    Reasoning,
    Plan {
        text: String,
    },
    Command {
        command: String,
        /// What the command is for, as its client said; absent from items
        /// stored before Ren read it.
        #[serde(default, skip_serializing_if = "CommandIntent::is_unsaid")]
        intent: CommandIntent,
        status: ItemStatus,
        output: Option<String>,
        exit_code: Option<i64>,
    },
    FileChange {
        paths: Vec<String>,
        status: ItemStatus,
    },
    /// Any other tool the client used: its name as the client gave it (with
    /// what its input names, when Ren reads no effect from it), and what it
    /// does as Ren reads any tool call (`Harness::effects`); none for items
    /// stored before Ren read them.
    Tool {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ToolEffect>,
        status: ItemStatus,
    },
    /// An item Ren reads nothing from; its kind and body say what it is.
    Unread,
}

/// What a command is for, in its client's terms: Ren reads nothing into
/// the command's own text.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "said", rename_all = "snake_case")]
pub enum CommandIntent {
    /// Its client said nothing of it.
    #[default]
    Unsaid,
    /// Its client's own words for it: the `description` Claude Code and
    /// OpenCode give their shell tool.
    Described { text: String },
    /// Its client's reading of its parts: Codex's `commandActions`.
    Parts { parts: Vec<CommandPart> },
}

impl CommandIntent {
    pub fn is_unsaid(&self) -> bool {
        matches!(self, Self::Unsaid)
    }

    /// Its client's words, when it gave some that are not blank.
    pub fn described(text: Option<&str>) -> Self {
        match text.map(str::trim) {
            Some(text) if !text.is_empty() => Self::Described {
                text: text.to_owned(),
            },
            Some(_) | None => Self::Unsaid,
        }
    }
}

/// One part of a command as its client read it, with the part's own text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "part", rename_all = "snake_case")]
pub enum CommandPart {
    /// It reads the file at `path`, named `name`.
    Read {
        command: String,
        path: String,
        name: String,
    },
    /// It lists the files under `path`, or where it runs.
    ListFiles {
        command: String,
        path: Option<String>,
    },
    /// It searches for `query` under `path`.
    Search {
        command: String,
        query: Option<String>,
        path: Option<String>,
    },
    /// A part its client could not read either.
    Unknown { command: String },
    /// A part of a kind Ren does not know, kept as its client sent it.
    Unrecognized { raw: serde_json::Value },
}

/// Where an item's action stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemStatus {
    InProgress,
    Completed,
    Failed,
    /// The operator or a policy refused it.
    Declined,
    /// A status Ren does not know, as the client named it.
    Other(String),
}

impl ItemRead {
    /// The fields of a tool's input that say what it acts on, in the order
    /// one is shown after a name Ren reads no effect from.
    const DETAILS: [&'static str; 6] = [
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ];

    /// The tool `name` that `harness` ran with `input`: what it does, read
    /// as Ren reads the same tool on the wire; with none read, its name
    /// followed by what its input names.
    pub(crate) fn tool(
        harness: Harness,
        name: &str,
        input: &serde_json::Value,
        status: ItemStatus,
    ) -> Self {
        let effects = harness.effects(&ToolUse {
            call: ToolCall::named(WireText::from_label(BlockKind::ToolUse), name),
            input: ToolInput::Json(input.clone()),
        });
        let name = match (
            effects.is_empty(),
            Self::DETAILS
                .into_iter()
                .find_map(|key| input.get(key).and_then(serde_json::Value::as_str)),
        ) {
            (true, Some(detail)) => format!("{name} {detail}"),
            (true, None) | (false, _) => name.to_owned(),
        };
        Self::Tool {
            name,
            effects,
            status,
        }
    }
}

impl ItemStatus {
    /// The status an item has when its client names none.
    pub fn of(phase: ItemPhase) -> Self {
        match phase {
            ItemPhase::Started => Self::InProgress,
            ItemPhase::Completed => Self::Completed,
        }
    }
}

/// Known statuses by their snake_case name; any other name is kept.
impl<'de> serde::Deserialize<'de> for ItemStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(match name.as_str() {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "declined" => Self::Declined,
            _ => Self::Other(name),
        })
    }
}

impl serde::Serialize for ItemStatus {
    /// Known statuses by their snake_case name; others by the client's own.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Declined => "declined",
            Self::Other(status) => status,
        })
    }
}

/// The kinds of items Ren reads. Others keep the client's own name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    UserMessage,
    AgentMessage,
    Reasoning,
    Plan,
    CommandExecution,
    FileChange,
    McpToolCall,
    WebSearch,
    Other(String),
}

impl ItemKind {
    /// The snake_case name a known kind serializes as.
    fn known(name: &str) -> Option<Self> {
        Some(match name {
            "user_message" => Self::UserMessage,
            "agent_message" => Self::AgentMessage,
            "reasoning" => Self::Reasoning,
            "plan" => Self::Plan,
            "command_execution" => Self::CommandExecution,
            "file_change" => Self::FileChange,
            "mcp_tool_call" => Self::McpToolCall,
            "web_search" => Self::WebSearch,
            _ => return None,
        })
    }
}

/// Known kinds by their snake_case name; any other name is kept.
impl<'de> serde::Deserialize<'de> for ItemKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Self::known(&name).unwrap_or(Self::Other(name)))
    }
}

impl serde::Serialize for ItemKind {
    /// Known kinds by their snake_case name; others by the client's own.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::UserMessage => "user_message",
            Self::AgentMessage => "agent_message",
            Self::Reasoning => "reasoning",
            Self::Plan => "plan",
            Self::CommandExecution => "command_execution",
            Self::FileChange => "file_change",
            Self::McpToolCall => "mcp_tool_call",
            Self::WebSearch => "web_search",
            Self::Other(kind) => kind,
        })
    }
}

/// What an approval asks for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalAsk {
    /// Run a command.
    Command {
        command: Option<String>,
        cwd: Option<String>,
        reason: Option<String>,
    },
    /// Apply file changes.
    FileChange { reason: Option<String> },
    /// Use another of the client's tools.
    Tool {
        tool: String,
        detail: Option<String>,
    },
}

/// How a turn ended, as the client reported it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnEnd {
    Completed,
    Interrupted,
    Failed {
        message: Option<String>,
    },
    /// A status Ren does not know, as the client named it.
    Unknown {
        status: String,
    },
}

/// Why a command could not be sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandRefused {
    #[error("the conversation is not open yet")]
    NotReady,
    #[error("a turn is already running")]
    TurnRunning,
    #[error("no turn is running")]
    NoTurn,
    #[error("the turn has not started yet")]
    TurnNotStarted,
    #[error("no approval {0} is waiting")]
    UnknownApproval(ApprovalId),
    /// The client has no way to do this mid-run.
    #[error("this client cannot change {0} in a running conversation")]
    NotOffered(Midway),
    /// The client did not say it takes images.
    #[error("this client does not take images")]
    NoImages,
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalAsk, ApprovalId, CommandIntent, CommandPart, DriveEvent, DriveItem, ItemId,
        ItemKind, ItemPhase, ItemRead, ItemStatus, RawLine, Retry, TurnEnd, Unreadable,
    };

    #[test]
    fn an_echoed_body_keeps_no_image_bytes() {
        let body = serde_json::json!({ "role": "user", "content": [
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "iVBORw0K" } },
            { "type": "image", "data": "iVBORw0K", "mimeType": "image/png" },
            { "type": "text", "text": "look", "data": "kept" },
        ] });
        let trimmed = DriveItem::without_image_bytes(body);
        let content = &trimmed["content"];
        assert_eq!(
            content[0]["source"]["data"],
            "8 characters of base64, not kept"
        );
        assert_eq!(content[1]["data"], "8 characters of base64, not kept");
        assert_eq!(content[1]["mimeType"], "image/png");
        assert_eq!(content[2]["data"], "kept");
    }

    /// A command stored before Ren read its intent reads as unsaid; one with
    /// parts reads back whole, a part of an unknown kind kept as it came.
    #[test]
    fn a_commands_intent_reads_back_and_older_ones_are_unsaid() {
        let older: ItemRead = serde_json::from_value(serde_json::json!({
            "as": "command", "command": "ls", "status": "completed",
            "output": null, "exit_code": 0
        }))
        .unwrap();
        assert!(matches!(
            older,
            ItemRead::Command {
                intent: CommandIntent::Unsaid,
                ..
            }
        ));
        let unsaid = serde_json::to_value(&older).unwrap();
        assert!(unsaid.get("intent").is_none(), "nothing written for unsaid");
        let read = ItemRead::Command {
            command: "sed -n 1,9p a.rs && rg x".into(),
            intent: CommandIntent::Parts {
                parts: vec![
                    CommandPart::Read {
                        command: "sed -n 1,9p a.rs".into(),
                        path: "/work/acme/a.rs".into(),
                        name: "a.rs".into(),
                    },
                    CommandPart::Unrecognized {
                        raw: serde_json::json!({ "type": "teleport", "command": "rg x" }),
                    },
                ],
            },
            status: ItemStatus::Completed,
            output: None,
            exit_code: Some(0),
        };
        let back: ItemRead = serde_json::from_value(serde_json::to_value(&read).unwrap()).unwrap();
        assert_eq!(back, read);
        assert_eq!(
            CommandIntent::described(Some("  ")),
            CommandIntent::Unsaid,
            "blank words say nothing"
        );
    }

    /// A tool reads as what it does, the way Ren reads it on the wire; one
    /// Ren reads nothing from keeps what its input names in its name, and
    /// a tool stored before Ren read effects reads with none.
    #[test]
    fn a_tool_reads_as_its_effects_or_keeps_its_detail() {
        use crate::Harness;
        use crate::tools::{EffectKind, ToolEffect};
        let read = ItemRead::tool(
            Harness::ClaudeCode,
            "Read",
            &serde_json::json!({ "file_path": "/work/acme/a.rs", "offset": 10 }),
            ItemStatus::InProgress,
        );
        assert_eq!(
            read,
            ItemRead::Tool {
                name: "Read".into(),
                effects: vec![ToolEffect {
                    kind: EffectKind::ReadFile,
                    target: "/work/acme/a.rs".into(),
                }],
                status: ItemStatus::InProgress,
            }
        );
        let unknown = ItemRead::tool(
            Harness::ClaudeCode,
            "TodoWrite",
            &serde_json::json!({ "description": "plan the fix" }),
            ItemStatus::Completed,
        );
        assert_eq!(
            unknown,
            ItemRead::Tool {
                name: "TodoWrite plan the fix".into(),
                effects: Vec::new(),
                status: ItemStatus::Completed,
            }
        );
        let older: ItemRead = serde_json::from_value(serde_json::json!({
            "as": "tool", "name": "Read /work/acme/a.rs", "status": "completed"
        }))
        .unwrap();
        assert!(matches!(older, ItemRead::Tool { effects, .. } if effects.is_empty()));
        let back: ItemRead = serde_json::from_value(serde_json::to_value(&read).unwrap()).unwrap();
        assert_eq!(back, read);
    }

    /// What the daemon sends on its run feed reads back as the same events,
    /// unknown kinds and statuses keeping the client's own names.
    #[test]
    fn events_read_back_as_they_were_sent() {
        let turn = ren_protocol::native::NativeTurnId::new(uuid::Uuid::from_u128(7)).unwrap();
        let events = [
            DriveEvent::TurnStarted { turn },
            DriveEvent::Item {
                phase: ItemPhase::Completed,
                item: DriveItem {
                    id: ItemId::new("cmd_1".into()),
                    kind: ItemKind::CommandExecution,
                    read: ItemRead::Command {
                        command: "cargo test".into(),
                        intent: CommandIntent::Described {
                            text: "Run the tests".into(),
                        },
                        status: ItemStatus::Other("queued".into()),
                        output: Some("ok".into()),
                        exit_code: Some(0),
                    },
                    body: serde_json::json!({ "anything": true }),
                },
            },
            DriveEvent::Item {
                phase: ItemPhase::Started,
                item: DriveItem {
                    id: ItemId::new("x".into()),
                    kind: ItemKind::Other("hologram".into()),
                    read: ItemRead::Unread,
                    body: serde_json::Value::Null,
                },
            },
            DriveEvent::Approval {
                approval: "perm-1".parse::<ApprovalId>().unwrap(),
                ask: ApprovalAsk::Tool {
                    tool: "WebFetch".into(),
                    detail: None,
                },
            },
            DriveEvent::TurnEnded {
                turn,
                end: TurnEnd::Failed {
                    message: Some("boom".into()),
                },
            },
            DriveEvent::Error {
                message: "overloaded".into(),
                retry: Retry::WillRetry,
            },
            DriveEvent::Unreadable {
                line: RawLine::new(b"not json".to_vec()),
                reason: Unreadable::TooLong { bytes: 9 },
            },
        ];
        for event in events {
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(
                serde_json::from_str::<DriveEvent>(&text).unwrap(),
                event,
                "{text}"
            );
        }
    }
}
