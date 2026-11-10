//! Operator state: a global pause, blocked sessions, and blocked users, changed only
//! by operator commands. Each change makes a new revision, recorded with every
//! call planned against it. Applying a command is pure; the shell swaps whole
//! snapshots, so a call keeps the state it was planned with.

use ren_protocol::native::NativeSessionId;
use ren_recorder::routing::OperatorRevision;
use ren_recorder::{caller::UserId, id::CallId, time::UnixMillis};
use std::{collections::BTreeMap, time::Duration};

/// Why a stored operator state could not be restored. Either stops startup:
/// starting unpaused and unblocked must not happen silently.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error(
        "stored operator state is unreadable ({0}); remove the operator_state row to start unpaused and unblocked"
    )]
    Unreadable(#[source] serde_json::Error),
    #[error("stored data is corrupt: operator state revision mismatch")]
    RevisionMismatch,
}

/// How long a paused client should wait before retrying, in whole seconds
/// (the `retry-after` header).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct RetryAfter(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("retry-after must be between 1 and 3600 seconds")]
pub struct InvalidRetryAfter;

impl RetryAfter {
    pub fn seconds(seconds: u32) -> Result<Self, InvalidRetryAfter> {
        if (1..=3600).contains(&seconds) {
            Ok(Self(seconds))
        } else {
            Err(InvalidRetryAfter)
        }
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl From<RetryAfter> for u32 {
    fn from(retry_after: RetryAfter) -> Self {
        retry_after.0
    }
}

impl TryFrom<u32> for RetryAfter {
    type Error = InvalidRetryAfter;

    fn try_from(seconds: u32) -> Result<Self, Self::Error> {
        Self::seconds(seconds)
    }
}

/// Whether new calls are admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Pause {
    Running,
    /// Every new call gets `503` with this `retry-after`.
    Paused {
        retry_after: RetryAfter,
    },
}

/// How long a session or user block lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "at")]
pub enum BlockedUntil {
    Unblocked,
    Time(UnixMillis),
}

impl BlockedUntil {
    /// A block lasting `duration` from `now`, or until unblocked.
    pub fn after(duration: Option<Duration>, now: UnixMillis) -> Self {
        match duration {
            None => Self::Unblocked,
            Some(duration) => Self::Time(UnixMillis::from_millis(
                now.as_millis()
                    .saturating_add(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
            )),
        }
    }

    fn applies(self, now: UnixMillis) -> bool {
        match self {
            Self::Unblocked => true,
            Self::Time(until) => now < until,
        }
    }
}

/// The operator's commands.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum OperatorCommand {
    Pause {
        retry_after: RetryAfter,
    },
    Resume,
    /// Block a client session until a time, or until unblocked.
    BlockSession {
        session: NativeSessionId,
        until: BlockedUntil,
    },
    UnblockSession {
        session: NativeSessionId,
    },
    /// Refuse a named user's new calls until a time, or until unblocked.
    BlockUser {
        user: UserId,
        until: BlockedUntil,
    },
    UnblockUser {
        user: UserId,
    },
    /// Stop a running call. It changes no state; it is recorded like every
    /// command, with its own revision.
    CancelCall {
        call: CallId,
    },
}

impl OperatorCommand {
    /// A session block, lasting `duration` from `now` if given.
    pub fn block_session(
        session: NativeSessionId,
        duration: Option<Duration>,
        now: UnixMillis,
    ) -> Self {
        Self::BlockSession {
            session,
            until: BlockedUntil::after(duration, now),
        }
    }

    /// A user block, lasting `duration` from `now` if given.
    pub fn block_user(user: UserId, duration: Option<Duration>, now: UnixMillis) -> Self {
        Self::BlockUser {
            user,
            until: BlockedUntil::after(duration, now),
        }
    }
}

/// One snapshot of operator state. Its JSON form is both the control API's
/// view and the stored snapshot a restart restores.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorState {
    revision: OperatorRevision,
    pause: Pause,
    blocked_sessions: BTreeMap<NativeSessionId, BlockedUntil>,
    /// Absent from snapshots stored before users could be blocked.
    #[serde(default)]
    blocked_users: BTreeMap<UserId, BlockedUntil>,
}

impl Default for OperatorState {
    fn default() -> Self {
        Self {
            revision: OperatorRevision::default(),
            pause: Pause::Running,
            blocked_sessions: BTreeMap::new(),
            blocked_users: BTreeMap::new(),
        }
    }
}

impl OperatorState {
    /// The state a stored snapshot describes: its JSON `document` and the
    /// `revision` stored beside it, which must agree.
    pub fn restore(revision: OperatorRevision, document: &str) -> Result<Self, RestoreError> {
        let state: Self = serde_json::from_str(document).map_err(RestoreError::Unreadable)?;
        if state.revision != revision {
            return Err(RestoreError::RevisionMismatch);
        }
        Ok(state)
    }

    pub fn revision(&self) -> OperatorRevision {
        self.revision
    }

    pub fn pause(&self) -> Pause {
        self.pause
    }

    /// How many sessions are blocked at `now`.
    pub fn blocked_sessions(&self, now: UnixMillis) -> usize {
        self.blocked_sessions
            .keys()
            .filter(|session| self.blocks(**session, now))
            .count()
    }

    /// How many users are blocked at `now`.
    pub fn blocked_users(&self, now: UnixMillis) -> usize {
        self.blocked_users
            .values()
            .filter(|until| until.applies(now))
            .count()
    }

    /// Whether `session` is blocked at `now`. An expired block no longer applies.
    pub fn blocks(&self, session: NativeSessionId, now: UnixMillis) -> bool {
        self.blocked_sessions
            .get(&session)
            .is_some_and(|until| until.applies(now))
    }

    /// Whether `user` is blocked at `now`. An expired block no longer applies.
    pub fn blocks_user(&self, user: &UserId, now: UnixMillis) -> bool {
        self.blocked_users
            .get(user)
            .is_some_and(|until| until.applies(now))
    }

    /// The state after `command`, with the next revision. Every command makes
    /// a revision, even one that changes nothing, so each is recorded.
    #[must_use]
    pub fn apply(&self, command: OperatorCommand) -> Self {
        let mut next = self.clone();
        next.revision = self.revision.next();
        match command {
            OperatorCommand::Pause { retry_after } => next.pause = Pause::Paused { retry_after },
            OperatorCommand::Resume => next.pause = Pause::Running,
            OperatorCommand::BlockSession { session, until } => {
                next.blocked_sessions.insert(session, until);
            }
            OperatorCommand::UnblockSession { session } => {
                next.blocked_sessions.remove(&session);
            }
            OperatorCommand::BlockUser { user, until } => {
                next.blocked_users.insert(user, until);
            }
            OperatorCommand::UnblockUser { user } => {
                next.blocked_users.remove(&user);
            }
            OperatorCommand::CancelCall { .. } => {}
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::{
        OperatorCommand, OperatorRevision, OperatorState, Pause, RestoreError, RetryAfter,
    };
    use ren_protocol::native::NativeSessionId;
    use ren_recorder::{caller::UserId, time::UnixMillis};
    use std::time::Duration;
    use uuid::Uuid;

    #[test]
    fn commands_make_revisions_and_blocks_expire() {
        let session = NativeSessionId::new(Uuid::from_u128(7)).unwrap();
        let now = UnixMillis::from_millis(1_000);
        let state = OperatorState::default();
        assert_eq!(state.revision().get(), 0);
        let paused = state.apply(OperatorCommand::Pause {
            retry_after: RetryAfter::seconds(30).unwrap(),
        });
        assert_eq!(paused.revision().get(), 1);
        assert!(matches!(paused.pause(), Pause::Paused { .. }));
        let resumed = paused.apply(OperatorCommand::Resume);
        assert_eq!(
            (resumed.revision().get(), resumed.pause()),
            (2, Pause::Running)
        );

        let blocked = resumed.apply(OperatorCommand::block_session(
            session,
            Some(Duration::from_secs(60)),
            now,
        ));
        assert!(blocked.blocks(session, now));
        assert!(!blocked.blocks(session, UnixMillis::from_millis(61_000)));
        let forever = blocked.apply(OperatorCommand::block_session(session, None, now));
        assert!(forever.blocks(session, UnixMillis::from_millis(u64::MAX)));
        let unblocked = forever.apply(OperatorCommand::UnblockSession { session });
        assert!(!unblocked.blocks(session, now));
        assert_eq!(unblocked.revision().get(), 5);
        // The original snapshot is unchanged.
        assert_eq!(state, OperatorState::default());
    }

    #[test]
    fn a_stored_snapshot_reads_back_and_is_validated() {
        let session = NativeSessionId::new(Uuid::from_u128(7)).unwrap();
        let now = UnixMillis::from_millis(1_000);
        let state = OperatorState::default()
            .apply(OperatorCommand::Pause {
                retry_after: RetryAfter::seconds(30).unwrap(),
            })
            .apply(OperatorCommand::block_session(session, None, now))
            .apply(OperatorCommand::block_session(
                NativeSessionId::new(Uuid::from_u128(8)).unwrap(),
                Some(Duration::from_secs(1)),
                now,
            ));
        let json = serde_json::to_string(&state).unwrap();
        let read = OperatorState::restore(state.revision(), &json).unwrap();
        assert_eq!(read, state);
        assert_eq!(read.blocked_sessions(now), 2);
        assert_eq!(read.blocked_sessions(UnixMillis::from_millis(2_000)), 1);
        // A stored retry-after is checked like a command's.
        let invalid = json.replace(r#""retry_after":30"#, r#""retry_after":0"#);
        assert!(matches!(
            OperatorState::restore(state.revision(), &invalid),
            Err(RestoreError::Unreadable(_))
        ));
        assert!(matches!(
            OperatorState::restore(state.revision(), "not json"),
            Err(RestoreError::Unreadable(_))
        ));
        // The revision stored beside the document must agree with it.
        assert!(matches!(
            OperatorState::restore(OperatorRevision::restore(5), &json),
            Err(RestoreError::RevisionMismatch)
        ));
    }

    #[test]
    fn user_blocks_expire_and_older_snapshots_have_none() {
        let user = UserId::parse("alice").unwrap();
        let now = UnixMillis::from_millis(1_000);
        let blocked = OperatorState::default().apply(OperatorCommand::block_user(
            user.clone(),
            Some(Duration::from_secs(60)),
            now,
        ));
        assert!(blocked.blocks_user(&user, now));
        assert!(!blocked.blocks_user(&user, UnixMillis::from_millis(61_000)));
        assert!(!blocked.blocks_user(&UserId::parse("bob").unwrap(), now));
        let unblocked = blocked.apply(OperatorCommand::UnblockUser { user: user.clone() });
        assert!(!unblocked.blocks_user(&user, now));
        let json = serde_json::to_string(&blocked).unwrap();
        assert_eq!(
            OperatorState::restore(blocked.revision(), &json).unwrap(),
            blocked
        );
        let older = r#"{"revision":0,"pause":{"state":"running"},"blocked_sessions":{}}"#;
        assert_eq!(
            OperatorState::restore(OperatorRevision::default(), older).unwrap(),
            OperatorState::default()
        );
    }

    #[test]
    fn retry_after_is_bounded() {
        assert!(RetryAfter::seconds(0).is_err());
        assert!(RetryAfter::seconds(3601).is_err());
        assert_eq!(RetryAfter::seconds(5).unwrap().get(), 5);
    }
}
