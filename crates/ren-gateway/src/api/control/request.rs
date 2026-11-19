//! The control API's requests as values: each knows its method, its path
//! under `/control/v1/`, and its body, and the bodies are the types the
//! routes parse. The CLI builds requests from these, so a route and its
//! clients cannot drift apart; `crates/ren-daemon/tests/contract.rs` sends
//! each one to the router.

use ren_harness::{
    Harness,
    drive::Decision,
    settings::{ApprovalPolicy, Effort},
};
use ren_protocol::native::NativeSessionId;
use ren_recorder::{
    batch::{HistoryChange, HistorySubject},
    caller::UserId,
    id::{AttachmentId, CallId, SessionId},
};
use ren_supervisor::Delivery;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `POST pause`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PauseRequest {
    pub retry_after_seconds: u32,
}

/// `POST client-sessions/{native}/block` and `users/{user}/block`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, optional_fields = nullable)]
#[serde(default, deny_unknown_fields)]
pub struct BlockRequest {
    /// How long the block lasts; absent means until unblocked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
}

/// `POST calls/{call}/cancel`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, optional_fields = nullable)]
#[serde(default, deny_unknown_fields)]
pub struct CancelRequest {
    /// Also block the call's session this long, so its client does not retry
    /// at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_session_seconds: Option<u64>,
}

/// `POST sessions`: a new session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    #[ts(as = "String")]
    pub harness: Harness,
    /// The folder, inside a root or another place; left out, the session
    /// gets a new folder of its own in scratch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(as = "Option<String>")]
    pub effort: Option<Effort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(as = "Option<String>")]
    pub approvals: Option<ApprovalPolicy>,
    /// A client conversation to go on with, begun elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(as = "Option<String>")]
    pub resume: Option<uuid::Uuid>,
    /// Where in `dir` it works; left out, in the folder itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(as = "Option<String>")]
    pub workspace: Option<WorkspaceKind>,
}

/// Where a session with a folder works: in the folder itself, or in a
/// workspace of its own, a new worktree of the repository in the folder.
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub enum WorkspaceKind {
    #[default]
    Folder,
    Worktree,
}

impl CreateRequest {
    /// A session of `harness` in `dir`, on the client's own settings.
    pub fn new(harness: Harness, dir: Option<String>) -> Self {
        Self {
            harness,
            dir,
            model: None,
            effort: None,
            approvals: None,
            resume: None,
            workspace: None,
        }
    }

    /// Where it starts: its folder, a workspace of the repository in it,
    /// or, with no folder, a new scratch folder. A workspace needs a folder.
    pub fn place(&self) -> Option<ren_supervisor::StartPlace> {
        match (&self.dir, self.workspace.unwrap_or_default()) {
            (Some(dir), WorkspaceKind::Folder) => {
                Some(ren_supervisor::StartPlace::Folder(dir.clone()))
            }
            (Some(dir), WorkspaceKind::Worktree) => {
                Some(ren_supervisor::StartPlace::Worktree(dir.clone()))
            }
            (None, WorkspaceKind::Folder) => Some(ren_supervisor::StartPlace::Scratch),
            (None, WorkspaceKind::Worktree) => None,
        }
    }
}

/// `POST sessions/{session}/prompt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PromptRequest {
    /// May be blank when attachments go.
    #[serde(default)]
    pub text: String,
    /// The session's attachments to send with it, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Vec<String>")]
    pub attachments: Vec<AttachmentId>,
    /// How it goes: start a turn (the default), queue it for after the
    /// running turn, or steer the running turn with it.
    #[serde(
        default = "PromptRequest::start",
        skip_serializing_if = "PromptRequest::starts"
    )]
    #[ts(as = "String")]
    pub delivery: Delivery,
}

impl PromptRequest {
    /// `text` alone, starting a turn.
    pub fn text(text: String) -> Self {
        Self {
            text,
            attachments: Vec::new(),
            delivery: Delivery::Start,
        }
    }

    fn start() -> Delivery {
        Delivery::Start
    }

    /// Whether `delivery` is the default, left off the wire.
    fn starts(delivery: &Delivery) -> bool {
        *delivery == Delivery::Start
    }
}

/// `POST sessions/{session}/approvals/{approval}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    #[ts(as = "String")]
    pub decision: Decision,
}

/// One control API request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    /// The operator state and the running calls.
    State,
    Pause(PauseRequest),
    Resume,
    BlockSession {
        session: NativeSessionId,
        block: BlockRequest,
    },
    UnblockSession(NativeSessionId),
    BlockUser {
        user: UserId,
        block: BlockRequest,
    },
    UnblockUser(UserId),
    Cancel {
        call: CallId,
        cancel: CancelRequest,
    },
    Backup,
    /// Archive, unarchive, or delete what was recorded.
    Edit {
        subject: HistorySubject,
        change: HistoryChange,
    },
    /// The live runs.
    Runs,
    /// The current sessions.
    Sessions,
    /// The places a session can start in.
    Projects,
    Create(CreateRequest),
    Prompt {
        session: SessionId,
        prompt: PromptRequest,
    },
    Answer {
        session: SessionId,
        approval: String,
        answer: AnswerRequest,
    },
    Interrupt(SessionId),
    Stop(SessionId),
}

/// Whether a request reads or changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMethod {
    Get,
    Post,
}

/// A control API event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFeed {
    /// `GET runs/events`: every run's updates.
    Runs,
    /// `GET sessions/events`: every session's changes.
    Sessions,
}

impl ControlFeed {
    /// The feed's path under `/control/v1/`.
    pub fn path(self) -> &'static str {
        match self {
            Self::Runs => "runs/events",
            Self::Sessions => "sessions/events",
        }
    }
}

impl ControlRequest {
    pub fn method(&self) -> ControlMethod {
        match self {
            Self::State | Self::Runs | Self::Sessions | Self::Projects => ControlMethod::Get,
            Self::Pause(_)
            | Self::Resume
            | Self::BlockSession { .. }
            | Self::UnblockSession(_)
            | Self::BlockUser { .. }
            | Self::UnblockUser(_)
            | Self::Cancel { .. }
            | Self::Backup
            | Self::Edit { .. }
            | Self::Create(_)
            | Self::Prompt { .. }
            | Self::Answer { .. }
            | Self::Interrupt(_)
            | Self::Stop(_) => ControlMethod::Post,
        }
    }

    /// The path under `/control/v1/`.
    pub fn path(&self) -> String {
        match self {
            Self::State => "state".to_owned(),
            Self::Pause(_) => "pause".to_owned(),
            Self::Resume => "resume".to_owned(),
            Self::BlockSession { session, .. } => {
                format!("client-sessions/{}/block", session.as_uuid())
            }
            Self::UnblockSession(session) => {
                format!("client-sessions/{}/unblock", session.as_uuid())
            }
            Self::BlockUser { user, .. } => format!("users/{user}/block"),
            Self::UnblockUser(user) => format!("users/{user}/unblock"),
            Self::Cancel { call, .. } => format!("calls/{call}/cancel"),
            Self::Backup => "backup".to_owned(),
            Self::Edit { subject, change } => match subject {
                HistorySubject::Recording(recording) => {
                    format!("history/recordings/{recording}/{change}")
                }
                HistorySubject::Run(run) => format!("history/runs/{run}/{change}"),
                HistorySubject::OperatorSession(session) => format!("sessions/{session}/{change}"),
            },
            Self::Runs => "runs".to_owned(),
            Self::Sessions => "sessions".to_owned(),
            Self::Projects => "runs/projects".to_owned(),
            Self::Create(_) => "sessions".to_owned(),
            Self::Prompt { session, .. } => format!("sessions/{session}/prompt"),
            Self::Answer {
                session, approval, ..
            } => format!("sessions/{session}/approvals/{approval}"),
            Self::Interrupt(session) => format!("sessions/{session}/interrupt"),
            Self::Stop(session) => format!("sessions/{session}/stop"),
        }
    }

    /// The JSON body, for a request that carries one.
    pub fn body(&self) -> Result<Option<Vec<u8>>, serde_json::Error> {
        match self {
            Self::Pause(pause) => serde_json::to_vec(pause).map(Some),
            Self::BlockSession { block, .. } | Self::BlockUser { block, .. } => {
                serde_json::to_vec(block).map(Some)
            }
            Self::Cancel { cancel, .. } => serde_json::to_vec(cancel).map(Some),
            Self::Create(create) => serde_json::to_vec(create).map(Some),
            Self::Prompt { prompt, .. } => serde_json::to_vec(prompt).map(Some),
            Self::Answer { answer, .. } => serde_json::to_vec(answer).map(Some),
            Self::State
            | Self::Resume
            | Self::UnblockSession(_)
            | Self::UnblockUser(_)
            | Self::Backup
            | Self::Edit { .. }
            | Self::Runs
            | Self::Sessions
            | Self::Projects
            | Self::Interrupt(_)
            | Self::Stop(_) => Ok(None),
        }
    }
}
