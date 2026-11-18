//! What a run's client reported, as the read API sends it: `DriveEvent` and the
//! items, approvals, and attachments it carries. Each type converts once from
//! the type it reports (`From`), naming every field.

use ren_depot::source::Sha256;
use ren_harness::attachment::{ByteSize, ImageFormat};
use ren_harness::drive::{ApprovalId, ItemId, ItemKind, ItemStatus, RawLine, RequestKind};
use ren_harness::tools::EffectKind;
use ren_protocol::label::Stored;
use ren_protocol::native::NativeThreadId;
use ren_protocol::native::NativeTurnId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a driven client reported, as Ren reads it, tagged by its `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum DriveEvent {
    /// The conversation is open and takes prompts.
    Ready {
        #[ts(as = "String")]
        thread: NativeThreadId,
    },
    TurnStarted {
        #[ts(as = "String")]
        turn: NativeTurnId,
    },
    /// Streamed text of the reply item `item`.
    Delta {
        #[ts(as = "String")]
        item: ItemId,
        text: String,
    },
    Item {
        phase: ItemPhase,
        item: DriveItem,
    },
    /// The client waits for the operator's decision.
    Approval {
        #[ts(as = "String")]
        approval: ApprovalId,
        ask: ApprovalAsk,
    },
    /// An approval no longer waits: answered, or withdrawn by the client.
    ApprovalResolved {
        #[ts(as = "String")]
        approval: ApprovalId,
    },
    TurnEnded {
        #[ts(as = "String")]
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
        #[ts(as = "String")]
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
        #[ts(as = "String")]
        line: RawLine,
    },
    /// A line Ren could not read.
    Unreadable {
        #[ts(as = "String")]
        line: RawLine,
        reason: Unreadable,
    },
}

impl From<ren_harness::drive::DriveEvent> for DriveEvent {
    fn from(value: ren_harness::drive::DriveEvent) -> Self {
        match value {
            ren_harness::drive::DriveEvent::Ready { thread } => Self::Ready { thread },
            ren_harness::drive::DriveEvent::TurnStarted { turn } => Self::TurnStarted { turn },
            ren_harness::drive::DriveEvent::Delta { item, text } => Self::Delta { item, text },
            ren_harness::drive::DriveEvent::Item { phase, item } => Self::Item {
                phase: phase.into(),
                item: item.into(),
            },
            ren_harness::drive::DriveEvent::Approval { approval, ask } => Self::Approval {
                approval,
                ask: ask.into(),
            },
            ren_harness::drive::DriveEvent::ApprovalResolved { approval } => {
                Self::ApprovalResolved { approval }
            }
            ren_harness::drive::DriveEvent::TurnEnded { turn, end } => Self::TurnEnded {
                turn,
                end: end.into(),
            },
            ren_harness::drive::DriveEvent::Error { message, retry } => Self::Error {
                message,
                retry: retry.into(),
            },
            ren_harness::drive::DriveEvent::Refused { request, message } => {
                Self::Refused { request, message }
            }
            ren_harness::drive::DriveEvent::Unsupported { method } => Self::Unsupported { method },
            ren_harness::drive::DriveEvent::Unknown { line } => Self::Unknown { line },
            ren_harness::drive::DriveEvent::Unreadable { line, reason } => Self::Unreadable {
                line,
                reason: reason.into(),
            },
        }
    }
}

/// Whether the client said it would retry after an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Retry {
    WillRetry,
    Final,
}

impl From<ren_harness::drive::Retry> for Retry {
    fn from(value: ren_harness::drive::Retry) -> Self {
        match value {
            ren_harness::drive::Retry::WillRetry => Self::WillRetry,
            ren_harness::drive::Retry::Final => Self::Final,
        }
    }
}

/// Why a line could not be read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
#[ts(export)]
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

impl From<ren_harness::drive::Unreadable> for Unreadable {
    fn from(value: ren_harness::drive::Unreadable) -> Self {
        match value {
            ren_harness::drive::Unreadable::NotJson => Self::NotJson,
            ren_harness::drive::Unreadable::NotJsonRpc => Self::NotJsonRpc,
            ren_harness::drive::Unreadable::UnexpectedResponse => Self::UnexpectedResponse,
            ren_harness::drive::Unreadable::BadParams => Self::BadParams,
            ren_harness::drive::Unreadable::Untyped => Self::Untyped,
            ren_harness::drive::Unreadable::TooLong { bytes } => Self::TooLong { bytes },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ItemPhase {
    Started,
    Completed,
}

impl From<ren_harness::drive::ItemPhase> for ItemPhase {
    fn from(value: ren_harness::drive::ItemPhase) -> Self {
        match value {
            ren_harness::drive::ItemPhase::Started => Self::Started,
            ren_harness::drive::ItemPhase::Completed => Self::Completed,
        }
    }
}

/// One item of a turn: its kind, what Ren read of it, and the client's own
/// body for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DriveItem {
    #[ts(as = "String")]
    pub id: ItemId,
    #[ts(as = "String")]
    pub kind: Stored<ItemKind>,
    pub read: ItemRead,
    pub body: serde_json::Value,
}

impl From<ren_harness::drive::DriveItem> for DriveItem {
    fn from(value: ren_harness::drive::DriveItem) -> Self {
        let ren_harness::drive::DriveItem {
            id,
            kind,
            read,
            body,
        } = value;
        Self {
            id,
            kind,
            read: read.into(),
            body,
        }
    }
}

/// What Ren read from an item, the same for every client, so consumers never
/// parse a client's own body. Each client's codec fills it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "as", rename_all = "snake_case")]
#[ts(export)]
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
        #[ts(as = "String")]
        status: Stored<ItemStatus>,
        output: Option<String>,
        exit_code: Option<i64>,
    },
    FileChange {
        paths: Vec<String>,
        #[ts(as = "String")]
        status: Stored<ItemStatus>,
    },
    /// Any other tool the client used: its name as the client gave it (with
    /// what its input names, when Ren reads no effect from it), and what it
    /// does as Ren reads any tool call (`Harness::effects`); none for items
    /// stored before Ren read them.
    Tool {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<ToolEffect>,
        #[ts(as = "String")]
        status: Stored<ItemStatus>,
    },
    /// An item Ren reads nothing from; its kind and body say what it is.
    Unread,
}

impl From<ren_harness::drive::ItemRead> for ItemRead {
    fn from(value: ren_harness::drive::ItemRead) -> Self {
        match value {
            ren_harness::drive::ItemRead::Prompt { text, images } => Self::Prompt {
                text,
                images: images.into_iter().map(Into::into).collect(),
            },
            ren_harness::drive::ItemRead::Reply { text } => Self::Reply { text },
            ren_harness::drive::ItemRead::Reasoning => Self::Reasoning,
            ren_harness::drive::ItemRead::Plan { text } => Self::Plan { text },
            ren_harness::drive::ItemRead::Command {
                command,
                intent,
                status,
                output,
                exit_code,
            } => Self::Command {
                command,
                intent: intent.into(),
                status,
                output,
                exit_code,
            },
            ren_harness::drive::ItemRead::FileChange { paths, status } => {
                Self::FileChange { paths, status }
            }
            ren_harness::drive::ItemRead::Tool {
                name,
                effects,
                status,
            } => Self::Tool {
                name,
                effects: effects.into_iter().map(Into::into).collect(),
                status,
            },
            ren_harness::drive::ItemRead::Unread => Self::Unread,
        }
    }
}

/// What a command is for, in its client's terms: Ren reads nothing into
/// the command's own text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "said", rename_all = "snake_case")]
#[ts(export)]
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
    fn is_unsaid(&self) -> bool {
        matches!(self, Self::Unsaid)
    }
}

impl From<ren_harness::drive::CommandIntent> for CommandIntent {
    fn from(value: ren_harness::drive::CommandIntent) -> Self {
        match value {
            ren_harness::drive::CommandIntent::Unsaid => Self::Unsaid,
            ren_harness::drive::CommandIntent::Described { text } => Self::Described { text },
            ren_harness::drive::CommandIntent::Parts { parts } => Self::Parts {
                parts: parts.into_iter().map(Into::into).collect(),
            },
        }
    }
}

/// One part of a command as its client read it, with the part's own text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "part", rename_all = "snake_case")]
#[ts(export)]
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

impl From<ren_harness::drive::CommandPart> for CommandPart {
    fn from(value: ren_harness::drive::CommandPart) -> Self {
        match value {
            ren_harness::drive::CommandPart::Read {
                command,
                path,
                name,
            } => Self::Read {
                command,
                path,
                name,
            },
            ren_harness::drive::CommandPart::ListFiles { command, path } => {
                Self::ListFiles { command, path }
            }
            ren_harness::drive::CommandPart::Search {
                command,
                query,
                path,
            } => Self::Search {
                command,
                query,
                path,
            },
            ren_harness::drive::CommandPart::Unknown { command } => Self::Unknown { command },
            ren_harness::drive::CommandPart::Unrecognized { raw } => Self::Unrecognized { raw },
        }
    }
}

/// What an approval asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
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

impl From<ren_harness::drive::ApprovalAsk> for ApprovalAsk {
    fn from(value: ren_harness::drive::ApprovalAsk) -> Self {
        match value {
            ren_harness::drive::ApprovalAsk::Command {
                command,
                cwd,
                reason,
            } => Self::Command {
                command,
                cwd,
                reason,
            },
            ren_harness::drive::ApprovalAsk::FileChange { reason } => Self::FileChange { reason },
            ren_harness::drive::ApprovalAsk::Tool { tool, detail } => Self::Tool { tool, detail },
        }
    }
}

/// How a turn ended, as the client reported it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
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

impl From<ren_harness::drive::TurnEnd> for TurnEnd {
    fn from(value: ren_harness::drive::TurnEnd) -> Self {
        match value {
            ren_harness::drive::TurnEnd::Completed => Self::Completed,
            ren_harness::drive::TurnEnd::Interrupted => Self::Interrupted,
            ren_harness::drive::TurnEnd::Failed { message } => Self::Failed { message },
            ren_harness::drive::TurnEnd::Unknown { status } => Self::Unknown { status },
        }
    }
}

/// An image's size in pixels: never zero on either side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl From<ren_harness::attachment::Dimensions> for Dimensions {
    fn from(dimensions: ren_harness::attachment::Dimensions) -> Self {
        Self {
            width: dimensions.width(),
            height: dimensions.height(),
        }
    }
}

/// An image Ren read: its format and its size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Image {
    #[ts(as = "String")]
    pub format: ImageFormat,
    pub dimensions: Dimensions,
}

impl From<ren_harness::attachment::Image> for Image {
    fn from(image: ren_harness::attachment::Image) -> Self {
        Self {
            format: image.format(),
            dimensions: image.dimensions().into(),
        }
    }
}

/// What an attachment is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AttachmentKind {
    Image(Image),
}

impl From<ren_harness::attachment::AttachmentKind> for AttachmentKind {
    fn from(value: ren_harness::attachment::AttachmentKind) -> Self {
        match value {
            ren_harness::attachment::AttachmentKind::Image(v0) => Self::Image(v0.into()),
        }
    }
}

/// One file uploaded to go with a prompt, read and known to be what it
/// says. Ren's identity for it is the store's; this is what it is. Read
/// back from JSON by the harness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
    #[ts(as = "u64")]
    pub size: ByteSize,
    #[ts(as = "String")]
    pub sha256: Sha256,
}

impl From<ren_harness::attachment::Attachment> for Attachment {
    fn from(attachment: ren_harness::attachment::Attachment) -> Self {
        Self {
            name: attachment.name().as_str().to_owned(),
            kind: attachment.kind().into(),
            size: attachment.size(),
            sha256: attachment.sha256(),
        }
    }
}

/// One effect with its target: a path, command, pattern, task, or URL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolEffect {
    #[ts(as = "String")]
    pub kind: EffectKind,
    pub target: String,
}

impl From<ren_harness::tools::ToolEffect> for ToolEffect {
    fn from(value: ren_harness::tools::ToolEffect) -> Self {
        let ren_harness::tools::ToolEffect { kind, target } = value;
        Self { kind, target }
    }
}
