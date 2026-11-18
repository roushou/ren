//! What Ren answers a client in the provider's place: the codes of its
//! refusals, the same over HTTP and WebSocket.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Why Ren answered a call itself. On the wire each code is prefixed `ren_`
/// (`RenCode::wire`), so it is never mistaken for the provider's.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum RenCode {
    Paused,
    SessionBlocked,
    UserBlocked,
    Blocked,
    UserNotAllowed,
    UserBusy,
    IdentityRefused,
    OperatorCancelled,
    ShuttingDown,
    TooLarge,
    BufferFull,
    UploadTimeout,
    UploadFailed,
}

impl RenCode {
    /// The error type a client reads: the label, prefixed `ren_`.
    pub fn wire(self) -> String {
        format!("ren_{self}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_prefixed_on_the_wire() {
        assert_eq!(RenCode::Paused.wire(), "ren_paused");
        assert_eq!(RenCode::UserNotAllowed.wire(), "ren_user_not_allowed");
    }
}
