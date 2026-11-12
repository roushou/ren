//! Claude Code's attribution headers: its session and prompt ids.

use crate::attribution::HintSource;
use crate::attribution::headers::{Claim, HeaderReading};

impl HeaderReading<'_> {
    /// Claude Code's session and prompt headers.
    pub(crate) fn claude_code(&mut self) -> Claim {
        let session = self.uuid("x-claude-code-session-id");
        let turn = self.uuid("x-claude-code-prompt-id");
        Claim {
            source: (session.is_some() || turn.is_some()).then_some(HintSource::ClaudeCodeHeaders),
            session,
            turn,
            thread: None,
        }
    }
}
