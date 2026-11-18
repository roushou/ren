//! Identifiers and hints in the client's or provider's own namespace, as they
//! appear on the wire. They are claims, never proof of identity, and have
//! their own types, separate from Ren's own identities.

use crate::text::{WireText, WireTextError};
use uuid::Uuid;

/// Text that is not a native identifier: a non-nil UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a {kind} id is a non-nil UUID")]
pub struct InvalidNativeId {
    kind: &'static str,
}

macro_rules! native_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// A client-supplied identifier. Nil UUIDs carry no identity.
            pub fn new(uuid: Uuid) -> Option<Self> {
                (!uuid.is_nil()).then_some(Self(uuid))
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = InvalidNativeId;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Uuid::try_parse(text)
                    .ok()
                    .and_then(Self::new)
                    .ok_or(InvalidNativeId { kind: $kind })
            }
        }
    };
}

native_id!(
    /// A client's own session identifier.
    NativeSessionId,
    "session"
);
native_id!(
    /// A client's own turn or prompt identifier.
    NativeTurnId,
    "turn"
);
native_id!(
    /// A client's own thread identifier (Codex).
    NativeThreadId,
    "thread"
);

macro_rules! wire_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(WireText);

        impl $name {
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        /// An identifier read from the wire: never empty, bounded, and free
        /// of control characters ([`WireText::identifier`]).
        impl std::str::FromStr for $name {
            type Err = WireTextError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                WireText::identifier(text).map(Self)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

wire_id!(
    /// The identifier a client answers a tool call with: Anthropic's block
    /// `id`, OpenAI's `call_id`, a Chat Completions call's `id`.
    ToolCallId
);
wire_id!(
    /// An OpenAI Responses output item's own `id`, distinct from the
    /// [`ToolCallId`] the client answers with.
    OutputItemId
);

/// The hints one WebSocket message carried, already parsed. Malformed hints
/// carry no identifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessageHints {
    session: Option<NativeSessionId>,
    thread: Option<NativeThreadId>,
    turn: Option<NativeTurnId>,
    malformed: Malformed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Malformed {
    #[default]
    No,
    Yes,
}

impl MessageHints {
    pub const MALFORMED: Self = Self {
        session: None,
        thread: None,
        turn: None,
        malformed: Malformed::Yes,
    };

    pub fn valid(
        session: Option<NativeSessionId>,
        thread: Option<NativeThreadId>,
        turn: Option<NativeTurnId>,
    ) -> Self {
        Self {
            session,
            thread,
            turn,
            malformed: Malformed::No,
        }
    }

    pub fn session(&self) -> Option<NativeSessionId> {
        self.session
    }

    pub fn thread(&self) -> Option<NativeThreadId> {
        self.thread
    }

    pub fn turn(&self) -> Option<NativeTurnId> {
        self.turn
    }

    pub fn is_malformed(&self) -> bool {
        self.malformed == Malformed::Yes
    }
}

#[cfg(test)]
mod tests {
    use super::{InvalidNativeId, NativeSessionId, NativeTurnId};

    #[test]
    fn native_ids_parse_from_non_nil_uuids() {
        let text = "0b6a3c1e-7f2d-4a8b-9c5e-1d2f3a4b5c6d";
        let id: NativeSessionId = text.parse().unwrap();
        assert_eq!(id.to_string(), text);
        for bad in ["", "not-a-uuid", "00000000-0000-0000-0000-000000000000"] {
            assert_eq!(
                bad.parse::<NativeSessionId>(),
                Err(InvalidNativeId { kind: "session" }),
                "{bad:?}"
            );
        }
        assert_eq!(
            "nope".parse::<NativeTurnId>().unwrap_err().to_string(),
            "a turn id is a non-nil UUID"
        );
    }
}
