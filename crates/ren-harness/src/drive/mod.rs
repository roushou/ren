//! Driving a client through its structured mode: the official client program,
//! started by Ren, exchanging messages on stdin and stdout instead of drawing
//! a terminal interface. The client keeps its own sign-in; nothing here reads,
//! refreshes, or answers for credentials.
//!
//! Each client's protocol is a pure codec (no I/O): commands in, lines for its
//! stdin out; lines from its stdout in, events and replies out. Framing
//! (`lines`) knows nothing about JSON.

mod lines;
mod model;
mod settings;

pub use lines::{Framed, Line, LineLimit, Lines};
pub use model::{BlankModelId, DefaultChoice, Listing, Model, ModelId, ModelListing};
pub use settings::{ApprovalPolicy, Effort, RunSettings};

use crate::claude_code::ClaudeStreamJson;
use crate::codex::CodexAppServer;
use crate::pi::PiRpc;
use crate::{Harness, launch::NotDrivable};

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

/// What one line from a driven client produced.
#[derive(Debug, Default)]
#[must_use]
pub struct Received {
    pub events: Vec<DriveEvent>,
    /// Lines to write to the client's stdin, in order.
    pub replies: Vec<Line>,
}

/// One driven client's side of its structured mode, chosen by its harness.
#[derive(Debug)]
pub enum Driver {
    Codex(CodexAppServer),
    ClaudeCode(ClaudeStreamJson),
    Pi(PiRpc),
}

impl Driver {
    /// The codec for a freshly started `harness` process, and the lines to
    /// send it first. `version` is Ren's own version; `seed` is a fresh id
    /// that a client which names no turns (Pi) has its turn ids minted from.
    pub fn open(
        harness: Harness,
        opening: Opening,
        version: &str,
        seed: uuid::Uuid,
    ) -> Result<(Self, Vec<Line>), NotDrivable> {
        match harness {
            Harness::Codex => {
                let (codec, first) = CodexAppServer::open(opening, version);
                Ok((Self::Codex(codec), vec![first]))
            }
            Harness::ClaudeCode => {
                let (codec, first) = ClaudeStreamJson::open(&opening);
                Ok((Self::ClaudeCode(codec), vec![first]))
            }
            Harness::Pi => {
                let (codec, first) = PiRpc::open(&opening, seed);
                Ok((Self::Pi(codec), vec![first]))
            }
            Harness::OpenCode => Err(NotDrivable(harness)),
        }
    }

    /// The line that carries `command`, or why it cannot be sent now.
    pub fn command(&mut self, command: DriveCommand) -> Result<Line, CommandRefused> {
        match self {
            Self::Codex(codec) => codec.command(command),
            Self::ClaudeCode(codec) => codec.command(command),
            Self::Pi(codec) => codec.command(command),
        }
    }

    /// Read one frame from the client's stdout.
    pub fn receive(&mut self, frame: Framed) -> Received {
        match self {
            Self::Codex(codec) => codec.receive(frame),
            Self::ClaudeCode(codec) => codec.receive(frame),
            Self::Pi(codec) => codec.receive(frame),
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
}

/// The text of a prompt: never blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a prompt cannot be blank")]
pub struct BlankPrompt;

impl Prompt {
    pub fn parse(text: String) -> Result<Self, BlankPrompt> {
        if text.trim().is_empty() {
            Err(BlankPrompt)
        } else {
            Ok(Self(text))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An approval request, named as the client named it within one process.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct ApprovalId(pub(crate) String);

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

/// What Ren read from an item, the same for every client, so consumers never
/// parse a client's own body. Each client's codec fills it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum ItemRead {
    /// The operator's prompt, as the client echoed it.
    Prompt {
        text: String,
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
        status: ItemStatus,
        output: Option<String>,
        exit_code: Option<i64>,
    },
    FileChange {
        paths: Vec<String>,
        status: ItemStatus,
    },
    /// Any other tool the client used, by the name it gave.
    Tool {
        name: String,
        status: ItemStatus,
    },
    /// An item Ren reads nothing from; its kind and body say what it is.
    Unread,
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
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalAsk, ApprovalId, DriveEvent, DriveItem, ItemId, ItemKind, ItemPhase, ItemRead,
        ItemStatus, RawLine, Retry, TurnEnd, Unreadable,
    };

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
