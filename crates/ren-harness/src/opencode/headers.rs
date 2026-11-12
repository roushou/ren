//! opencode's attribution header: its session id, read on every route,
//! since opencode reaches Ren through the other clients' routes.

use crate::attribution::headers::{Claim, HeaderReading};
use crate::attribution::{HintIssue, HintSource};
use sha2::{Digest, Sha256};
use uuid::Uuid;

impl HeaderReading<'_> {
    const OPENCODE_SESSION: &'static str = "x-opencode-session-id";

    /// opencode's session header, on any route. opencode names no turn; a
    /// subagent's call names its own child session, as opencode does.
    pub(crate) fn opencode(&mut self) -> Option<Claim> {
        if !self.headers.contains_key(Self::OPENCODE_SESSION) {
            return None;
        }
        Some(Claim {
            source: Some(HintSource::OpenCodeHeaders),
            session: self.opencode_session(),
            turn: None,
            thread: None,
        })
    }

    fn opencode_session(&mut self) -> Option<Uuid> {
        let value = self.header(Self::OPENCODE_SESSION, OpenCodeSessionId::MAX_LEN)?;
        let id = OpenCodeSessionId::parse(value).map(OpenCodeSessionId::uuid);
        if id.is_none() {
            self.issue(HintIssue::InvalidOrDuplicateHeader);
        }
        id
    }
}

/// opencode's own session identifier: `ses_`, then ASCII letters and digits
/// (opencode 2.x writes 26). It is not a UUID, and Ren's native session
/// identifiers are, so the hint carries a UUID derived from it; the original
/// is never stored.
struct OpenCodeSessionId<'a>(&'a str);

impl<'a> OpenCodeSessionId<'a> {
    const PREFIX: &'static str = "ses_";
    const MAX_LEN: usize = 132;
    const NAMESPACE: &'static [u8] = b"ren:opencode-session:";

    fn parse(value: &'a str) -> Option<Self> {
        let rest = value.strip_prefix(Self::PREFIX)?;
        (!rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .then_some(Self(value))
    }

    /// A version 8 UUID from the namespaced SHA-256 of the identifier: the
    /// same session always gives the same UUID, and different sessions
    /// collide only as SHA-256 does.
    fn uuid(self) -> Uuid {
        let digest = Sha256::new()
            .chain_update(Self::NAMESPACE)
            .chain_update(self.0.as_bytes())
            .finalize();
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid::Builder::from_custom_bytes(bytes).into_uuid()
    }
}
