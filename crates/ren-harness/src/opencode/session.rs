//! opencode's session identifier, and the UUID Ren reads it as: in the
//! attribution header of each model request, and as the thread of a driven
//! run, so a run and its recorded calls name the same conversation.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// opencode's own session identifier: `ses_`, then ASCII letters and digits
/// (opencode 2.x writes 26). It is not a UUID, and Ren's native session and
/// thread identifiers are, so Ren carries a UUID derived from it; the
/// original is never stored. A driven run finds its session again by
/// listing the folder's sessions and deriving each one's UUID.
pub(crate) struct OpenCodeSessionId<'a>(&'a str);

impl<'a> OpenCodeSessionId<'a> {
    const PREFIX: &'static str = "ses_";
    pub(crate) const MAX_LEN: usize = 132;
    const NAMESPACE: &'static [u8] = b"ren:opencode-session:";

    pub(crate) fn parse(value: &'a str) -> Option<Self> {
        let rest = value.strip_prefix(Self::PREFIX)?;
        (!rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .then_some(Self(value))
    }

    /// A version 8 UUID from the namespaced SHA-256 of the identifier: the
    /// same session always gives the same UUID, and different sessions
    /// collide only as SHA-256 does.
    pub(crate) fn uuid(&self) -> Uuid {
        let digest = Sha256::new()
            .chain_update(Self::NAMESPACE)
            .chain_update(self.0.as_bytes())
            .finalize();
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid::Builder::from_custom_bytes(bytes).into_uuid()
    }

    pub(crate) fn as_str(&self) -> &'a str {
        self.0
    }
}
