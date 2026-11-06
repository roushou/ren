//! Client-supplied attribution: which session and turn a call claims to belong
//! to.
//!
//! These are hints, never authentication or proof of client identity. They are
//! kept as the client sent them and never inferred from timing or content.
//! Native identifiers have their own types, separate from TGV's `SessionId`
//! and `TurnId`.

mod headers;

use crate::Harness;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_protocol::native::{MessageHints, NativeSessionId, NativeThreadId, NativeTurnId};
use uuid::Uuid;

/// Which header convention supplied the hints.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum HintSource {
    ClaudeCodeHeaders,
    CodexHeaders,
    PiExtensionHeaders,
    /// The `client_metadata` of a Codex WebSocket `response.create`.
    CodexMessageMetadata,
}

impl HintSource {
    /// The harness whose requests use this convention.
    pub fn harness(self) -> Harness {
        match self {
            Self::ClaudeCodeHeaders => Harness::ClaudeCode,
            Self::CodexHeaders | Self::CodexMessageMetadata => Harness::Codex,
            Self::PiExtensionHeaders => Harness::Pi,
        }
    }
}

/// Why some hints were discarded.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum HintIssue {
    InvalidOrDuplicateHeader,
    InvalidTurnMetadata,
    ConflictingIdentifiers,
    TurnWithoutSession,
}

/// A session claim. A turn exists only inside a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionHint {
    pub source: HintSource,
    pub session: NativeSessionId,
    pub turn: Option<NativeTurnId>,
    pub thread: Option<NativeThreadId>,
}

/// Everything a call's headers said about attribution.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attribution {
    pub hint: Option<SessionHint>,
    pub issues: Vec<HintIssue>,
}

impl Attribution {
    /// A WebSocket call's attribution: the channel's handshake hints, with the
    /// turn its `response.create` named. A message that names a different
    /// session or thread than the handshake is a conflict, and neither is
    /// preferred: the call keeps the handshake's session without a turn.
    pub fn for_message(&self, message: MessageHints) -> Self {
        let mut attribution = self.clone();
        let mut issue = |issue| {
            if !attribution.issues.contains(&issue) {
                attribution.issues.push(issue);
            }
        };
        if message.is_malformed() {
            issue(HintIssue::InvalidTurnMetadata);
            return attribution;
        }
        let conflicts = |ours: Option<Uuid>, theirs: Option<Uuid>| matches!((ours, theirs), (Some(ours), Some(theirs)) if ours != theirs);
        let hint = match self.hint {
            Some(channel) => {
                if conflicts(
                    Some(channel.session.as_uuid()),
                    message.session().map(NativeSessionId::as_uuid),
                ) || conflicts(
                    channel.thread.map(NativeThreadId::as_uuid),
                    message.thread().map(NativeThreadId::as_uuid),
                ) {
                    issue(HintIssue::ConflictingIdentifiers);
                    Some(SessionHint {
                        turn: None,
                        ..channel
                    })
                } else {
                    Some(SessionHint {
                        turn: message.turn().or(channel.turn),
                        thread: channel.thread.or(message.thread()),
                        ..channel
                    })
                }
            }
            None => match message.session() {
                Some(session) => Some(SessionHint {
                    source: HintSource::CodexMessageMetadata,
                    session,
                    turn: message.turn(),
                    thread: message.thread(),
                }),
                None => {
                    if message.turn().is_some() {
                        issue(HintIssue::TurnWithoutSession);
                    }
                    None
                }
            },
        };
        attribution.hint = hint;
        attribution
    }
}

#[cfg(test)]
mod tests {
    use super::{Attribution, HintIssue, HintSource, SessionHint};
    use tgv_protocol::native::{MessageHints, NativeSessionId, NativeThreadId, NativeTurnId};
    use uuid::Uuid;

    fn session(n: u128) -> NativeSessionId {
        NativeSessionId::new(Uuid::from_u128(n)).unwrap()
    }

    fn turn(n: u128) -> NativeTurnId {
        NativeTurnId::new(Uuid::from_u128(n)).unwrap()
    }

    fn thread(n: u128) -> NativeThreadId {
        NativeThreadId::new(Uuid::from_u128(n)).unwrap()
    }

    fn channel() -> Attribution {
        Attribution {
            hint: Some(SessionHint {
                source: HintSource::CodexHeaders,
                session: session(1),
                turn: None,
                thread: Some(thread(2)),
            }),
            issues: Vec::new(),
        }
    }

    #[test]
    fn a_message_turn_joins_the_handshake_session() {
        let call =
            channel().for_message(MessageHints::valid(Some(session(1)), None, Some(turn(3))));
        let hint = call.hint.unwrap();
        assert_eq!(
            (hint.source, hint.session, hint.turn, hint.thread),
            (
                HintSource::CodexHeaders,
                session(1),
                Some(turn(3)),
                Some(thread(2))
            )
        );
        assert!(call.issues.is_empty());
    }

    #[test]
    fn conflicting_or_malformed_hints_never_pick_a_side() {
        let call =
            channel().for_message(MessageHints::valid(Some(session(9)), None, Some(turn(3))));
        assert_eq!(call.hint.unwrap().turn, None);
        assert_eq!(call.issues, vec![HintIssue::ConflictingIdentifiers]);

        let call = channel().for_message(MessageHints::MALFORMED);
        assert_eq!(call.hint, channel().hint);
        assert_eq!(call.issues, vec![HintIssue::InvalidTurnMetadata]);
    }

    #[test]
    fn without_handshake_hints_the_message_names_the_session() {
        let call = Attribution::default().for_message(MessageHints::valid(
            Some(session(1)),
            None,
            Some(turn(3)),
        ));
        assert_eq!(call.hint.unwrap().source, HintSource::CodexMessageMetadata);

        let call =
            Attribution::default().for_message(MessageHints::valid(None, None, Some(turn(3))));
        assert_eq!(call.hint, None);
        assert_eq!(call.issues, vec![HintIssue::TurnWithoutSession]);
    }
}
