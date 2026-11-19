//! DRAFT

use super::runs::RunSettings;
use crate::api::read::drive::Attachment;
use crate::api::read::runs::RunView;
use ren_harness::Harness;
use ren_harness::settings::ApprovalPolicy;
use ren_harness::settings::Effort;
use ren_protocol::label::Stored;
use ren_recorder::id::AttachmentId;
use ren_recorder::id::ProjectId;
use ren_recorder::id::RunId;
use ren_recorder::id::SessionId;
use ren_recorder::time::UnixMillis;
use ren_supervisor::ImageTaking;
use ren_supervisor::PathKind;
use ren_supervisor::SessionPhase;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// A session as the supervisor serves it: what it is, where it stands,
/// and its newest process, whose state the run feed carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionSnapshot {
    #[ts(as = "String")]
    pub session: SessionId,
    #[ts(as = "String")]
    pub harness: Harness,
    #[ts(as = "String")]
    pub workdir: PathBuf,
    /// What its next turn runs on; each left out is the client's own.
    pub settings: RunSettings,
    pub state: SessionState,
    #[ts(as = "Option<String>")]
    pub process: Option<RunId>,
    /// Whether its client takes images with a prompt.
    #[ts(as = "String")]
    pub images: ImageTaking,
}

impl From<ren_supervisor::SessionSnapshot> for SessionSnapshot {
    fn from(value: ren_supervisor::SessionSnapshot) -> Self {
        let ren_supervisor::SessionSnapshot {
            session,
            harness,
            workdir,
            settings,
            state,
            process,
            images,
        } = value;
        Self {
            session,
            harness,
            workdir,
            settings: settings.into(),
            state: state.into(),
            process,
            images,
        }
    }
}

/// Where a session stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "shelf", content = "phase", rename_all = "snake_case")]
#[ts(export)]
pub enum SessionState {
    /// Listed with the current sessions.
    Open(#[ts(as = "String")] SessionPhase),
    /// Shelved by the operator, in the phase it had.
    Archived(#[ts(as = "String")] SessionPhase),
}

impl From<ren_supervisor::SessionState> for SessionState {
    fn from(value: ren_supervisor::SessionState) -> Self {
        match value {
            ren_supervisor::SessionState::Open(v0) => Self::Open(v0),
            ren_supervisor::SessionState::Archived(v0) => Self::Archived(v0),
        }
    }
}

/// One session an operator works in, as recorded, with the runs (client
/// processes) that served it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionView {
    #[ts(as = "String")]
    pub id: SessionId,
    #[ts(as = "String")]
    pub harness: Stored<Harness>,
    pub workdir: String,
    /// The project it works for; `None` outside every project, or from
    /// before projects.
    #[ts(as = "Option<String>")]
    pub project: Option<ProjectId>,
    #[ts(as = "u64")]
    pub created_at: UnixMillis,
    /// The settings its next turn runs on; `None` is the client's own.
    pub model: Option<String>,
    #[ts(as = "Option<String>")]
    pub effort: Option<Stored<Effort>>,
    #[ts(as = "Option<String>")]
    pub approvals: Option<Stored<ApprovalPolicy>>,
    #[ts(as = "Option<u64>")]
    pub archived_at: Option<UnixMillis>,
    /// When its first turn started; `None` while it is a draft (no prompt
    /// has gone out).
    #[ts(as = "Option<u64>")]
    pub first_turn_at: Option<UnixMillis>,
    /// The first line of its first prompt, as its client echoed it, cut to
    /// `SessionView::FIRST_PROMPT_CHARS`; `None` until one is recorded.
    pub first_prompt: Option<String>,
    /// Its runs, newest first.
    pub runs: Vec<RunView>,
}

impl From<ren_store::view::SessionView> for SessionView {
    fn from(value: ren_store::view::SessionView) -> Self {
        let ren_store::view::SessionView {
            id,
            harness,
            workdir,
            project,
            created_at,
            model,
            effort,
            approvals,
            archived_at,
            first_turn_at,
            first_prompt,
            runs,
        } = value;
        Self {
            id,
            harness,
            workdir,
            project,
            created_at,
            model,
            effort,
            approvals,
            archived_at,
            first_turn_at,
            first_prompt,
            runs: runs.into_iter().map(Into::into).collect(),
        }
    }
}

/// A path that matched, with where: the positions (in chars) of the
/// path's characters the query matched, for drawing them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileMatch {
    pub path: String,
    #[ts(as = "String")]
    pub kind: PathKind,
    pub positions: Vec<usize>,
}

impl From<ren_supervisor::FileMatch> for FileMatch {
    fn from(value: ren_supervisor::FileMatch) -> Self {
        let ren_supervisor::FileMatch {
            path,
            kind,
            positions,
        } = value;
        Self {
            path,
            kind,
            positions,
        }
    }
}

/// One session: as the store holds it (with its runs), and as the
/// supervisor serves it now. Either may be missing: the store records a
/// new session a moment after it is made, and the supervisor serves a
/// stored one only once asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionBody {
    #[ts(as = "String")]
    pub session: SessionId,
    pub stored: Option<SessionView>,
    pub served: Option<SessionSnapshot>,
}

/// The sessions on one shelf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionsBody {
    pub sessions: Vec<SessionBody>,
}

/// One session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OneSession {
    pub session: SessionBody,
}

/// A session made: its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Created {
    #[ts(as = "String")]
    pub session: SessionId,
}

/// A command accepted, with nothing more to say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Accepted {}

/// Files in a session's folder that match a query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FilesBody {
    pub files: Vec<FileMatch>,
}

/// An attachment kept: its id, and what it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Attached {
    pub attachment: AttachmentEntry,
}

/// An attachment: its id, and what it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttachmentEntry {
    #[ts(as = "String")]
    pub id: AttachmentId,
    pub read: Attachment,
}

/// A session's attachments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttachmentsBody {
    pub attachments: Vec<KeptAttachment>,
}

/// One of a session's attachments: its id, what it is, and whether it went
/// out with a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeptAttachment {
    #[ts(as = "String")]
    pub id: AttachmentId,
    pub read: Attachment,
    pub sent: bool,
}

impl From<ren_supervisor::KeptAttachment> for KeptAttachment {
    fn from(kept: ren_supervisor::KeptAttachment) -> Self {
        let ren_supervisor::KeptAttachment { id, read, used } = kept;
        Self {
            id,
            read: read.into(),
            sent: match used {
                ren_supervisor::AttachmentUse::Sent => true,
                ren_supervisor::AttachmentUse::Unsent => false,
            },
        }
    }
}
