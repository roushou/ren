//! Where an operator's session stands: a draft until its first prompt goes
//! out, active after, and archived when the operator shelves it. Pure: the
//! supervisor applies each input and carries out what it returns.
//!
//! A session is not its process. Its client starts, idles, and ends any
//! number of times (`run`); the session goes on until it is archived, and
//! comes back when it is written to again.

use ren_harness::Harness;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Whether a prompt has gone out in the session.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum SessionPhase {
    /// Nothing sent yet: its client can still be changed.
    Draft,
    /// A prompt went out: its client is fixed.
    Active,
}

/// Where a session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Listed with the current sessions.
    Open(SessionPhase),
    /// Shelved by the operator, in the phase it had.
    Archived(SessionPhase),
}

/// What happens to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionInput {
    /// The operator chose the client it works with.
    ChooseClient(Harness),
    /// A prompt went out to its client.
    Prompted,
    Archive,
    Unarchive,
}

/// What the supervisor does for a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    /// Run with this client from its first process on.
    UseClient(Harness),
    /// End the client process serving the session, if one runs: nothing is
    /// served while it is shelved.
    StopProcess,
}

/// Why an input was refused in the session's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionRefused {
    #[error("the session's client is fixed once a prompt went out")]
    ClientFixed,
    #[error("the session is archived")]
    Archived,
    #[error("the session is not archived")]
    NotArchived,
}

impl SessionState {
    /// A session as it is created: an open draft.
    pub const NEW: Self = Self::Open(SessionPhase::Draft);

    pub fn phase(self) -> SessionPhase {
        match self {
            Self::Open(phase) | Self::Archived(phase) => phase,
        }
    }

    /// The state after `input`, and what to do for it.
    pub fn apply(self, input: SessionInput) -> Result<(Self, Vec<SessionAction>), SessionRefused> {
        match (self, input) {
            (Self::Open(SessionPhase::Draft), SessionInput::ChooseClient(harness)) => {
                Ok((self, vec![SessionAction::UseClient(harness)]))
            }
            (Self::Open(SessionPhase::Active), SessionInput::ChooseClient(_)) => {
                Err(SessionRefused::ClientFixed)
            }
            (Self::Archived(_), SessionInput::ChooseClient(_)) => Err(SessionRefused::Archived),
            // Written to, a session is current again, as a recording with
            // new traffic is.
            (Self::Open(_) | Self::Archived(_), SessionInput::Prompted) => {
                Ok((Self::Open(SessionPhase::Active), Vec::new()))
            }
            (Self::Open(phase), SessionInput::Archive) => {
                Ok((Self::Archived(phase), vec![SessionAction::StopProcess]))
            }
            (Self::Archived(_), SessionInput::Archive) => Err(SessionRefused::Archived),
            (Self::Archived(phase), SessionInput::Unarchive) => Ok((Self::Open(phase), Vec::new())),
            (Self::Open(_), SessionInput::Unarchive) => Err(SessionRefused::NotArchived),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(state: SessionState, inputs: &[SessionInput]) -> SessionState {
        inputs.iter().fold(state, |state, input| {
            state.apply(*input).map(|(next, _)| next).unwrap_or(state)
        })
    }

    #[test]
    fn a_draft_takes_a_client_until_its_first_prompt_fixes_it() {
        let draft = SessionState::NEW;
        assert_eq!(
            draft.apply(SessionInput::ChooseClient(Harness::Pi)),
            Ok((draft, vec![SessionAction::UseClient(Harness::Pi)]))
        );
        let (active, actions) = draft.apply(SessionInput::Prompted).unwrap();
        assert_eq!(active, SessionState::Open(SessionPhase::Active));
        assert!(actions.is_empty());
        assert_eq!(
            active.apply(SessionInput::ChooseClient(Harness::Codex)),
            Err(SessionRefused::ClientFixed)
        );
        // Prompting again keeps it active.
        assert_eq!(
            active.apply(SessionInput::Prompted),
            Ok((active, Vec::new()))
        );
    }

    #[test]
    fn archiving_stops_the_process_and_keeps_the_phase() {
        for phase in [SessionPhase::Draft, SessionPhase::Active] {
            let open = SessionState::Open(phase);
            assert_eq!(
                open.apply(SessionInput::Archive),
                Ok((
                    SessionState::Archived(phase),
                    vec![SessionAction::StopProcess]
                ))
            );
            let archived = SessionState::Archived(phase);
            assert_eq!(
                archived.apply(SessionInput::Unarchive),
                Ok((open, Vec::new()))
            );
            assert_eq!(archived.phase(), phase);
        }
    }

    #[test]
    fn illegal_inputs_are_refused_with_why() {
        let archived = SessionState::Archived(SessionPhase::Draft);
        assert_eq!(
            archived.apply(SessionInput::Archive),
            Err(SessionRefused::Archived)
        );
        assert_eq!(
            archived.apply(SessionInput::ChooseClient(Harness::Pi)),
            Err(SessionRefused::Archived)
        );
        assert_eq!(
            SessionState::NEW.apply(SessionInput::Unarchive),
            Err(SessionRefused::NotArchived)
        );
    }

    #[test]
    fn writing_to_an_archived_session_brings_it_back_active() {
        let state = after(
            SessionState::NEW,
            &[SessionInput::Archive, SessionInput::Prompted],
        );
        assert_eq!(state, SessionState::Open(SessionPhase::Active));
    }
}
