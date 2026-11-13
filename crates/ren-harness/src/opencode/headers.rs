//! opencode's attribution header: its session id, read on every route,
//! since opencode reaches Ren through the other clients' routes.

use super::session::OpenCodeSessionId;
use crate::attribution::headers::{Claim, HeaderReading};
use crate::attribution::{HintIssue, HintSource};
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
        let id = OpenCodeSessionId::parse(value).map(|id| id.uuid());
        if id.is_none() {
            self.issue(HintIssue::InvalidOrDuplicateHeader);
        }
        id
    }
}
