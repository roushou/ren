//! The control API's requests as values: each knows its method, its path
//! under `/control/v1/`, and its body, and the bodies are the types the
//! routes parse. The CLI builds requests from these, so a route and its
//! clients cannot drift apart; `crates/ren-daemon/tests/contract.rs` sends
//! each one to the router.

use crate::api::control::projects::RegisterProjectRequest;
use crate::api::read::work::{WorkOrderBody, WorkPreviewBody};
use ren_git::Commit;
use ren_harness::{
    Harness,
    drive::Decision,
    question::{Confirmation, QuestionKey, Replies, Reply, SecretText},
    settings::{ApprovalPolicy, Effort},
};
use ren_protocol::native::NativeSessionId;
use ren_recorder::{
    batch::{HistoryChange, HistorySubject},
    caller::UserId,
    id::{AttachmentId, CallId, SessionId, WorkItemId},
    project::ProjectName,
};
use ren_release::Revision;
use ren_supervisor::Delivery;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// `POST work/{item}/send`: `session` takes the item up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SendWorkRequest {
    #[ts(as = "String")]
    pub session: SessionId,
}

/// `POST work/{item}/land`: the item's change is on `main` as `commit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct LandWorkRequest {
    #[ts(as = "String")]
    pub commit: Commit,
}

/// `POST work/{item}/ship`: production runs `revision`, which holds the
/// item's change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ShipWorkRequest {
    #[ts(as = "String")]
    pub revision: Revision,
}

/// `POST work/{item}/preview`: a preview of the item's change, or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PreviewWorkRequest {
    pub preview: Option<WorkPreviewBody>,
}

/// `POST release/promote`: production is to take the canary's build of
/// `commit`, on the factory's next pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PromoteRequest {
    #[ts(as = "String")]
    pub commit: Commit,
}

/// `POST previews/ask` and `previews/withdraw`: the operator asks for a
/// preview of `commit`, or withdraws the ask, for the factory's next pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PreviewAskRequest {
    #[ts(as = "String")]
    pub commit: Commit,
}

/// What happens to a work item, as a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkChange {
    Send(SendWorkRequest),
    Land(LandWorkRequest),
    Ship(ShipWorkRequest),
    Dismiss,
    Preview(PreviewWorkRequest),
}

impl WorkChange {
    /// Its path segment under `work/{item}/`.
    pub fn path(&self) -> &'static str {
        match self {
            Self::Send(_) => "send",
            Self::Land(_) => "land",
            Self::Ship(_) => "ship",
            Self::Dismiss => "dismiss",
            Self::Preview(_) => "preview",
        }
    }
}

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
    pub checkout: Option<Checkout>,
    /// A registered project to work for, by name, in place of `dir`: the
    /// session works in its repository, in a worktree of its own unless
    /// `checkout` says `folder`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// Where a session with a folder works: in the folder itself, or in a
/// worktree of its own, made from the repository in the folder.
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
pub enum Checkout {
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
            checkout: None,
            project: None,
        }
    }

    /// Where it starts: its folder, a worktree of the repository in it,
    /// or, with no folder, a new scratch folder. A worktree needs a folder.
    /// A session asked for by `project` starts where its project is.
    pub fn place(&self) -> Option<ren_supervisor::StartPlace> {
        match (&self.dir, self.checkout.unwrap_or_default()) {
            (Some(dir), Checkout::Folder) => Some(ren_supervisor::StartPlace::Folder(dir.clone())),
            (Some(dir), Checkout::Worktree) => {
                Some(ren_supervisor::StartPlace::Worktree(dir.clone()))
            }
            (None, Checkout::Folder) => Some(ren_supervisor::StartPlace::Scratch),
            (None, Checkout::Worktree) => None,
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

/// `POST sessions/{session}/approvals/{approval}/replies`: the operator's
/// replies to the questions an approval asks, by question key. A question
/// left out is skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ReplyRequest {
    pub replies: BTreeMap<String, ReplyBody>,
}

/// The operator's reply to one question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "as", rename_all = "snake_case")]
#[ts(export)]
pub enum ReplyBody {
    /// Options picked by label, with an answer in the operator's own words
    /// where the choice takes one.
    Chose {
        picked: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        written: Option<String>,
    },
    Wrote {
        text: String,
    },
    /// The answer to a question whose text is hidden.
    Secret {
        #[ts(as = "String")]
        text: SecretBody,
    },
    Number {
        #[ts(as = "f64")]
        value: serde_json::Number,
    },
    Confirmed {
        #[ts(as = "String")]
        answer: Confirmation,
    },
    Skipped,
}

/// A secret reply's text as a request carries it: never shown by `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretBody(String);

impl SecretBody {
    pub fn new(text: String) -> Self {
        Self(text)
    }
}

impl std::fmt::Debug for SecretBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBody(hidden)")
    }
}

impl From<ReplyRequest> for Replies {
    fn from(value: ReplyRequest) -> Self {
        Replies::new(
            value
                .replies
                .into_iter()
                .map(|(key, reply)| (QuestionKey::new(key), reply.into()))
                .collect(),
        )
    }
}

impl From<ReplyBody> for Reply {
    fn from(value: ReplyBody) -> Self {
        match value {
            ReplyBody::Chose { picked, written } => Self::Chose { picked, written },
            ReplyBody::Wrote { text } => Self::Wrote { text },
            ReplyBody::Secret { text } => Self::Secret {
                text: SecretText::new(text.0),
            },
            ReplyBody::Number { value } => Self::Number { value },
            ReplyBody::Confirmed { answer } => Self::Confirmed { answer },
            ReplyBody::Skipped => Self::Skipped,
        }
    }
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
    Places,
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
    Reply {
        session: SessionId,
        approval: String,
        reply: ReplyRequest,
    },
    Interrupt(SessionId),
    Stop(SessionId),
    /// Queue a work item, for the project named, or else Ren itself.
    QueueWork {
        order: Box<WorkOrderBody>,
        project: Option<ProjectName>,
    },
    /// Ask that production take the canary's build.
    Promote(PromoteRequest),
    /// Ask for a preview of a commit.
    AskPreview(PreviewAskRequest),
    /// Withdraw the ask for a preview of a commit.
    WithdrawPreview(PreviewAskRequest),
    /// Move a work item on.
    Work {
        item: WorkItemId,
        change: WorkChange,
    },
    /// Register a project.
    RegisterProject(RegisterProjectRequest),
    ArchiveProject(ProjectName),
    UnarchiveProject(ProjectName),
    /// Assign a project the unowned sessions in its repository or its
    /// worktrees.
    AdoptSessions(ProjectName),
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
            Self::State | Self::Runs | Self::Sessions | Self::Places => ControlMethod::Get,
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
            | Self::Reply { .. }
            | Self::Interrupt(_)
            | Self::Stop(_)
            | Self::QueueWork { .. }
            | Self::Promote(_)
            | Self::AskPreview(_)
            | Self::WithdrawPreview(_)
            | Self::RegisterProject(_)
            | Self::ArchiveProject(_)
            | Self::UnarchiveProject(_)
            | Self::AdoptSessions(_)
            | Self::Work { .. } => ControlMethod::Post,
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
            Self::Places => "runs/places".to_owned(),
            Self::Create(_) => "sessions".to_owned(),
            Self::Prompt { session, .. } => format!("sessions/{session}/prompt"),
            Self::Answer {
                session, approval, ..
            } => format!("sessions/{session}/approvals/{approval}"),
            Self::Reply {
                session, approval, ..
            } => format!("sessions/{session}/approvals/{approval}/replies"),
            Self::Interrupt(session) => format!("sessions/{session}/interrupt"),
            Self::Stop(session) => format!("sessions/{session}/stop"),
            Self::QueueWork { project, .. } => match project {
                Some(project) => format!("work?project={project}"),
                None => "work".to_owned(),
            },
            Self::Promote(_) => "release/promote".to_owned(),
            Self::AskPreview(_) => "previews/ask".to_owned(),
            Self::WithdrawPreview(_) => "previews/withdraw".to_owned(),
            Self::RegisterProject(_) => "projects".to_owned(),
            Self::ArchiveProject(name) => format!("projects/{name}/archive"),
            Self::UnarchiveProject(name) => format!("projects/{name}/unarchive"),
            Self::AdoptSessions(name) => format!("projects/{name}/adopt"),
            Self::Work { item, change } => format!("work/{item}/{}", change.path()),
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
            Self::Reply { reply, .. } => serde_json::to_vec(reply).map(Some),
            Self::QueueWork { order, .. } => serde_json::to_vec(order).map(Some),
            Self::Promote(promote) => serde_json::to_vec(promote).map(Some),
            Self::AskPreview(ask) | Self::WithdrawPreview(ask) => serde_json::to_vec(ask).map(Some),
            Self::RegisterProject(project) => serde_json::to_vec(project).map(Some),
            Self::Work { change, .. } => match change {
                WorkChange::Send(send) => serde_json::to_vec(send).map(Some),
                WorkChange::Land(land) => serde_json::to_vec(land).map(Some),
                WorkChange::Ship(ship) => serde_json::to_vec(ship).map(Some),
                WorkChange::Dismiss => Ok(None),
                WorkChange::Preview(preview) => serde_json::to_vec(preview).map(Some),
            },
            Self::State
            | Self::Resume
            | Self::UnblockSession(_)
            | Self::UnblockUser(_)
            | Self::Backup
            | Self::Edit { .. }
            | Self::Runs
            | Self::Sessions
            | Self::Places
            | Self::Interrupt(_)
            | Self::Stop(_)
            | Self::ArchiveProject(_)
            | Self::UnarchiveProject(_)
            | Self::AdoptSessions(_) => Ok(None),
        }
    }
}
