//! Driving a client through its structured mode: the official client program,
//! started by Ren, exchanging messages on stdin and stdout instead of drawing
//! a terminal interface. The client keeps its own sign-in; nothing here reads,
//! refreshes, or answers for credentials.
//!
//! Each client's protocol is a pure codec (no I/O): commands in, lines for its
//! stdin out; lines from its stdout in, events and replies out. Framing
//! (`lines`) knows nothing about JSON.

pub mod codex;
mod lines;

pub use lines::{Framed, Line, LineLimit, Lines};

use ren_protocol::native::{NativeThreadId, NativeTurnId};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

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
pub struct ApprovalId(String);

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
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    WillRetry,
    Final,
}

/// Why a line could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Unreadable {
    NotJson,
    NotJsonRpc,
    /// A response to a request Ren never sent.
    UnexpectedResponse,
    /// A known message whose parameters did not match its shape.
    BadParams,
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

impl RawLine {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// An item of a turn, named as the client named it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemPhase {
    Started,
    Completed,
}

/// One item of a turn: its kind, and the client's own body for it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DriveItem {
    pub id: ItemId,
    pub kind: ItemKind,
    pub body: serde_json::Value,
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
}

/// How a turn ended, as the client reported it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
