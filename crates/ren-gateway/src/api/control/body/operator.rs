//! DRAFT

use crate::api::health::Pause;
use ren_protocol::native::NativeSessionId;
use ren_protocol::route::Transport;
use ren_recorder::caller::UserId;
use ren_recorder::id::CallId;
use ren_recorder::routing::OperatorRevision;
use ren_recorder::time::UnixMillis;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// The operator state: the pause, and the session and user blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OperatorState {
    #[ts(as = "u64")]
    pub revision: OperatorRevision,
    pub pause: Pause,
    #[ts(as = "BTreeMap<String, BlockedUntil>")]
    pub blocked_sessions: BTreeMap<NativeSessionId, BlockedUntil>,
    #[ts(as = "BTreeMap<String, BlockedUntil>")]
    pub blocked_users: BTreeMap<UserId, BlockedUntil>,
}

impl From<&crate::operator::OperatorState> for OperatorState {
    fn from(state: &crate::operator::OperatorState) -> Self {
        Self {
            revision: state.revision(),
            pause: state.pause().into(),
            blocked_sessions: state
                .session_blocks()
                .iter()
                .map(|(session, until)| (*session, (*until).into()))
                .collect(),
            blocked_users: state
                .user_blocks()
                .iter()
                .map(|(user, until)| (user.clone(), (*until).into()))
                .collect(),
        }
    }
}

/// A running call, as the control API lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunningCall {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "String")]
    pub transport: Transport,
    /// The named user who made it, if any.
    #[ts(as = "Option<String>")]
    pub user: Option<UserId>,
}

impl From<crate::control::RunningCall> for RunningCall {
    fn from(value: crate::control::RunningCall) -> Self {
        let crate::control::RunningCall {
            call,
            transport,
            user,
        } = value;
        Self {
            call,
            transport,
            user,
        }
    }
}

/// How long a session or user block lasts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "kind", content = "at")]
#[ts(export)]
pub enum BlockedUntil {
    Unblocked,
    Time(#[ts(as = "u64")] UnixMillis),
}

impl From<crate::operator::BlockedUntil> for BlockedUntil {
    fn from(value: crate::operator::BlockedUntil) -> Self {
        match value {
            crate::operator::BlockedUntil::Unblocked => Self::Unblocked,
            crate::operator::BlockedUntil::Time(v0) => Self::Time(v0),
        }
    }
}

/// The operator state, and the calls running now: what every operator
/// command answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StateBody {
    pub state: OperatorState,
    pub running: Vec<RunningCall>,
}
