//! Identifiers and hints in the client's or provider's own namespace, as they
//! appear on the wire. They are claims, never proof of identity, and have
//! their own types, separate from Ren's own identities.

use uuid::Uuid;

macro_rules! native_id {
    ($(#[$meta:meta])* $name:ident) => {
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
    };
}

native_id!(
    /// A client's own session identifier.
    NativeSessionId
);
native_id!(
    /// A client's own turn or prompt identifier.
    NativeTurnId
);
native_id!(
    /// A client's own thread identifier (Codex).
    NativeThreadId
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
