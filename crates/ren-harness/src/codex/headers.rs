//! Codex's attribution headers: its session and thread ids, and the turn
//! metadata header that also names them.

use crate::attribution::HintIssue;
use crate::attribution::HintSource;
use crate::attribution::headers::{Claim, HeaderReading};
use serde::Deserialize;

impl HeaderReading<'_> {
    const MAX_TURN_METADATA: usize = 8192;

    pub(crate) fn codex(&mut self) -> Claim {
        let mut session = self.uuid("session-id");
        let mut thread = self.uuid("thread-id");
        let mut turn = None;
        if let Some(raw) = self.header("x-codex-turn-metadata", Self::MAX_TURN_METADATA) {
            match serde_json::from_str::<TurnMetadata>(raw) {
                Ok(metadata) => {
                    turn = self.metadata_id(metadata.turn_id);
                    let claimed_session = self.metadata_id(metadata.session_id);
                    let claimed_thread = self.metadata_id(metadata.thread_id);
                    // Do not silently prefer one source when identifiers disagree.
                    if Self::conflicts(session, claimed_session)
                        || Self::conflicts(thread, claimed_thread)
                    {
                        self.issue(HintIssue::ConflictingIdentifiers);
                        session = None;
                        thread = None;
                        turn = None;
                    }
                }
                Err(_) => self.issue(HintIssue::InvalidTurnMetadata),
            }
        }
        Claim {
            source: session.is_some().then_some(HintSource::CodexHeaders),
            session,
            turn,
            thread,
        }
    }
}

// Serde rejects duplicate known fields. Ignore all other fields: account,
// installation, machine, tool policy, and user-defined names never enter storage.
#[derive(Deserialize)]
struct TurnMetadata {
    turn_id: Option<String>,
    session_id: Option<String>,
    thread_id: Option<String>,
}
