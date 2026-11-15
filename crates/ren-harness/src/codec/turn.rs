//! The turn under way in a driven conversation, the ids a client that names
//! no turns gets, and what each prompt's images are until it is echoed.

use crate::{
    attachment::Attachment,
    drive::{CommandRefused, DriveEvent, ItemId, Prompt},
};
use ren_protocol::native::NativeTurnId;
use std::collections::VecDeque;
use uuid::Uuid;

/// Where a conversation's turn stands. `P` is what a requested turn keeps
/// until it starts (the prompt, for a client that never echoes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnSlot<P = ()> {
    Idle,
    /// A prompt was sent; the client has not started its turn.
    Requested(P),
    Running(NativeTurnId),
    /// Ren asked the running turn to stop; it has not ended.
    Stopping(NativeTurnId),
}

/// Whether Ren asked a turn to stop before it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Unasked,
    Asked,
}

impl<P> TurnSlot<P> {
    /// A prompt is sent, keeping `pending`; refused while a turn is under
    /// way or requested.
    pub(crate) fn request(&mut self, pending: P) -> Result<(), CommandRefused> {
        match self {
            Self::Idle => {
                *self = Self::Requested(pending);
                Ok(())
            }
            Self::Requested(_) | Self::Running(_) | Self::Stopping(_) => {
                Err(CommandRefused::TurnRunning)
            }
        }
    }

    /// The turn under way, for a command that needs one.
    pub(crate) fn running(&self) -> Result<NativeTurnId, CommandRefused> {
        match self {
            Self::Running(turn) | Self::Stopping(turn) => Ok(*turn),
            Self::Requested(_) => Err(CommandRefused::TurnNotStarted),
            Self::Idle => Err(CommandRefused::NoTurn),
        }
    }

    /// Whether a turn is under way.
    pub(crate) fn under_way(&self) -> bool {
        self.running().is_ok()
    }

    /// Ren asks the turn under way to stop.
    pub(crate) fn stop(&mut self) -> Result<NativeTurnId, CommandRefused> {
        let turn = self.running()?;
        *self = Self::Stopping(turn);
        Ok(turn)
    }

    /// The client reported `turn` started: reported to Ren unless it is the
    /// one under way.
    pub(crate) fn start(&mut self, turn: NativeTurnId) -> Option<DriveEvent> {
        match self {
            Self::Running(running) | Self::Stopping(running) if *running == turn => None,
            Self::Idle | Self::Requested(_) | Self::Running(_) | Self::Stopping(_) => {
                *self = Self::Running(turn);
                Some(DriveEvent::TurnStarted { turn })
            }
        }
    }

    /// The turn under way ended: it, and whether Ren asked it to stop. None,
    /// and nothing changed, when no turn is under way.
    pub(crate) fn end(&mut self) -> Option<(NativeTurnId, Stop)> {
        let ended = match self {
            Self::Running(turn) => (*turn, Stop::Unasked),
            Self::Stopping(turn) => (*turn, Stop::Asked),
            Self::Idle | Self::Requested(_) => return None,
        };
        *self = Self::Idle;
        Some(ended)
    }

    /// Back to idle, whatever stood: what stood.
    pub(crate) fn clear(&mut self) -> Self {
        std::mem::replace(self, Self::Idle)
    }

    /// A requested turn that will not start: what it kept, and back to idle.
    pub(crate) fn withdraw(&mut self) -> Option<P> {
        match self.clear() {
            Self::Requested(pending) => Some(pending),
            other @ (Self::Idle | Self::Running(_) | Self::Stopping(_)) => {
                *self = other;
                None
            }
        }
    }

    /// An item named `name`, within the turn under way (`turn:name`).
    pub(crate) fn scope(&self, name: String) -> ItemId {
        match self {
            Self::Running(turn) | Self::Stopping(turn) => ItemId::new(format!("{turn}:{name}")),
            Self::Idle | Self::Requested(_) => ItemId::new(name),
        }
    }
}

/// Turn ids for a client that names no turns (Pi, opencode): a fresh seed
/// mixed with how many were minted.
#[derive(Debug)]
pub(crate) struct TurnMint {
    seed: Uuid,
    minted: u64,
}

impl TurnMint {
    pub(crate) fn new(seed: Uuid) -> Self {
        Self { seed, minted: 0 }
    }

    /// The next turn id: never nil, never one this mint gave before.
    pub(crate) fn mint(&mut self) -> NativeTurnId {
        loop {
            self.minted += 1;
            let mixed = Uuid::from_u128(self.seed.as_u128() ^ u128::from(self.minted));
            if let Some(turn) = NativeTurnId::new(mixed) {
                return turn;
            }
        }
    }

    /// How many were minted.
    pub(crate) fn minted(&self) -> u64 {
        self.minted
    }
}

/// What the images of each prompt sent are, in order, until the client
/// echoes it: the echo then shows them without their bytes.
#[derive(Debug, Default)]
pub(crate) struct EchoQueue(VecDeque<Vec<Attachment>>);

impl EchoQueue {
    /// `prompt` was sent; kept when it has images.
    pub(crate) fn push(&mut self, prompt: &Prompt) {
        if !prompt.images().is_empty() {
            self.0.push_back(prompt.sent());
        }
    }

    /// `prompt` was sent, and only its echo is awaited.
    pub(crate) fn only(&mut self, prompt: &Prompt) {
        self.0.clear();
        self.push(prompt);
    }

    /// The images of the oldest prompt not yet echoed, kept for its echo.
    pub(crate) fn front(&self) -> Vec<Attachment> {
        self.0.front().cloned().unwrap_or_default()
    }

    /// The images of the oldest prompt not yet echoed, now echoed.
    pub(crate) fn pop(&mut self) -> Vec<Attachment> {
        self.0.pop_front().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Stop, TurnMint, TurnSlot};
    use crate::drive::{CommandRefused, DriveEvent};

    #[test]
    fn a_turn_is_requested_started_stopped_and_ended() {
        let mut mint = TurnMint::new(uuid::Uuid::from_u128(7));
        let turn = mint.mint();
        let mut slot = TurnSlot::Idle;
        assert_eq!(slot.running(), Err(CommandRefused::NoTurn));
        assert_eq!(slot.end(), None);
        slot.request(()).unwrap();
        assert_eq!(slot.request(()), Err(CommandRefused::TurnRunning));
        assert_eq!(slot.stop(), Err(CommandRefused::TurnNotStarted));
        assert_eq!(slot.start(turn), Some(DriveEvent::TurnStarted { turn }));
        assert_eq!(slot.start(turn), None);
        assert_eq!(slot.scope("a".into()).as_str(), format!("{turn}:a"));
        assert_eq!(slot.stop(), Ok(turn));
        assert_eq!(slot.start(turn), None, "still stopping");
        assert_eq!(slot.end(), Some((turn, Stop::Asked)));
        assert_eq!(slot, TurnSlot::Idle);
        assert_eq!(slot.scope("a".into()).as_str(), "a");
    }

    #[test]
    fn a_withdrawn_request_gives_back_what_it_kept() {
        let mut slot = TurnSlot::Idle;
        slot.request("prompt").unwrap();
        assert_eq!(slot.withdraw(), Some("prompt"));
        assert_eq!(slot.withdraw(), None);
        let mut mint = TurnMint::new(uuid::Uuid::nil());
        let turn = mint.mint();
        assert_ne!(mint.mint(), turn);
        assert_eq!(mint.minted(), 2);
        let _ = slot.start(turn);
        assert_eq!(slot.withdraw(), None);
        assert_eq!(slot, TurnSlot::Running(turn));
    }
}
