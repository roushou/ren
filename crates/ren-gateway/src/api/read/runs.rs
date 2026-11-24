//! Agent runs and their conversations, as the read API sends them. Each type
//! converts once from the type it reports (`From`), naming every field.

use super::drive::DriveEvent;
use ren_depot::package_set::Generation;
use ren_harness::Harness;
use ren_harness::settings::ApprovalPolicy;
use ren_harness::settings::Effort;
use ren_protocol::label::Stored;
use ren_recorder::capture::ExitKind;
use ren_recorder::capture::RunEnd;
use ren_recorder::id::RecordingId;
use ren_recorder::id::RunId;
use ren_recorder::id::SessionId;
use ren_recorder::passage::{PassageEnd, Purpose};
use ren_recorder::time::UnixMillis;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A run's conversation, whole: its entries, and those of the runs before
/// it that continued the same client conversation, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConversationView {
    pub entries: Vec<RunEntryView>,
    /// Older entries were left out: the conversation holds more than one
    /// read returns.
    pub earlier: bool,
}

impl From<ren_store::view::ConversationView> for ConversationView {
    fn from(value: ren_store::view::ConversationView) -> Self {
        let ren_store::view::ConversationView { entries, earlier } = value;
        Self {
            entries: entries.into_iter().map(Into::into).collect(),
            earlier,
        }
    }
}

/// One entry of a run's conversation: an event as its client reported it,
/// in the order it came.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunEntryView {
    #[ts(as = "String")]
    pub run: RunId,
    pub seq: u64,
    /// When it was last reported: an item reported again (finished,
    /// completed) is the same entry, as it last stood.
    #[ts(as = "u64")]
    pub at: UnixMillis,
    /// When it was first reported: an item's start.
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    /// The client's turn it belongs to, once one started.
    pub turn: Option<String>,
    pub event: DriveEvent,
}

impl From<ren_store::view::RunEntryView> for RunEntryView {
    fn from(value: ren_store::view::RunEntryView) -> Self {
        let ren_store::view::RunEntryView {
            run,
            seq,
            at,
            started_at,
            turn,
            event,
        } = value;
        Self {
            run,
            seq,
            at,
            started_at,
            turn,
            event: event.into(),
        }
    }
}

/// A relayed run's passages through the relay, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Passages {
    pub passages: Vec<PassageView>,
}

/// Where a run reached through the relay: what it asked for, how those
/// requests ended, and how many.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PassageView {
    /// A tunnel's `host:port`, or the method of a request that was not one.
    pub asked: String,
    /// A `passage_end` label: `closed`, `broken`, a refusal (`unlisted`,
    /// `private_address`, `not_a_tunnel`), or why it was not reached
    /// (`unresolved`, `connect_refused`, `timed_out`).
    #[ts(as = "String")]
    pub end: Stored<PassageEnd>,
    /// The listed destination that let it through, last time; `None` when
    /// it was refused.
    pub through: Option<String>,
    /// A `purpose` label, why that destination is listed: `configuration`,
    /// `client`, `project`, or `grant`; `None` when refused or not known.
    #[ts(as = "Option<String>")]
    pub listed_by: Option<Stored<Purpose>>,
    pub count: u64,
    /// Bytes each way over the ones that closed.
    pub sent: u64,
    pub received: u64,
    #[ts(as = "u64")]
    pub first_at: UnixMillis,
    #[ts(as = "u64")]
    pub last_at: UnixMillis,
}

impl From<ren_store::view::PassageView> for PassageView {
    fn from(value: ren_store::view::PassageView) -> Self {
        let ren_store::view::PassageView {
            asked,
            end,
            through,
            listed_by,
            count,
            sent,
            received,
            first_at,
            last_at,
        } = value;
        Self {
            asked,
            end,
            through,
            listed_by,
            count,
            sent,
            received,
            first_at,
            last_at,
        }
    }
}

/// One agent run, as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunView {
    #[ts(as = "String")]
    pub id: RunId,
    #[ts(as = "String")]
    pub harness: Stored<Harness>,
    pub workdir: String,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    /// The client's own conversation id, once it opened one.
    pub thread: Option<String>,
    /// The recording whose calls name the run's thread as their session:
    /// the run's model traffic, once a call was recorded.
    #[ts(as = "Option<String>")]
    pub recording: Option<RecordingId>,
    /// The operator session the run served.
    #[ts(as = "String")]
    pub operator_session: SessionId,
    #[ts(as = "Option<u64>")]
    pub ended_at: Option<UnixMillis>,
    /// How the run ended; `None` while it runs.
    #[ts(as = "Option<String>")]
    pub end: Option<Stored<RunEnd>>,
    /// How its process exited, when one did.
    pub exit: Option<ProcessExit>,
    /// When the operator archived it; it is listed as archived once ended.
    #[ts(as = "Option<u64>")]
    pub archived_at: Option<UnixMillis>,
    /// The model it started on, as its client names it; `None` is the
    /// client's own.
    pub model: Option<String>,
    #[ts(as = "Option<String>")]
    pub effort: Option<Stored<Effort>>,
    #[ts(as = "Option<String>")]
    pub approvals: Option<Stored<ApprovalPolicy>>,
    /// The package set generation on its search path: 0 when Ren had installed
    /// none; `None` when unknown: the package store was unreadable, or the store
    /// holds none for the run.
    #[ts(as = "Option<u64>")]
    pub toolset: Option<Generation>,
}

impl From<ren_store::view::RunView> for RunView {
    fn from(value: ren_store::view::RunView) -> Self {
        let ren_store::view::RunView {
            id,
            harness,
            workdir,
            started_at,
            thread,
            recording,
            operator_session,
            ended_at,
            end,
            exit,
            archived_at,
            model,
            effort,
            approvals,
            toolset,
        } = value;
        Self {
            id,
            harness,
            workdir,
            started_at,
            thread,
            recording,
            operator_session,
            ended_at,
            end,
            exit: exit.map(Into::into),
            archived_at,
            model,
            effort,
            approvals,
            toolset,
        }
    }
}

/// How a run's process exited: its kind, and the value a code or a signal
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProcessExit {
    /// `code`, `signal`, or `unknown`.
    #[ts(as = "String")]
    pub kind: ExitKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<i32>,
}

impl From<ren_recorder::capture::ProcessExit> for ProcessExit {
    fn from(exit: ren_recorder::capture::ProcessExit) -> Self {
        use ren_recorder::capture::ProcessExit as Source;
        let (kind, value) = match exit {
            Source::Code(code) => (ExitKind::Code, Some(code)),
            Source::Signal(signal) => (ExitKind::Signal, Some(signal)),
            Source::Unknown => (ExitKind::Unknown, None),
        };
        Self { kind, value }
    }
}
