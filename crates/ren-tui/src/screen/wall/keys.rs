//! The wall's keys, while nothing is typed: moving between panes, opening
//! the focused session, and moving between places.

use crate::Key;
use crate::keys::{Bind, Keymap};

/// What a key asks the wall to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
    /// The focused session, full screen.
    Open,
    PreviousPlace,
    NextPlace,
    Quit,
}

impl Command {
    /// The wall's keys, in the order the help lists them.
    pub const KEYS: Keymap<Self> = Keymap(&[
        Bind {
            keys: &[Key::Tab],
            label: "tab",
            does: "next pane",
            command: Self::Next,
        },
        Bind {
            keys: &[Key::BackTab],
            label: "shift-tab",
            does: "previous pane",
            command: Self::Previous,
        },
        Bind {
            keys: &[Key::Left],
            label: "←",
            does: "pane to the left",
            command: Self::Left,
        },
        Bind {
            keys: &[Key::Right],
            label: "→",
            does: "pane to the right",
            command: Self::Right,
        },
        Bind {
            keys: &[Key::Up],
            label: "↑",
            does: "pane above",
            command: Self::Up,
        },
        Bind {
            keys: &[Key::Down],
            label: "↓",
            does: "pane below",
            command: Self::Down,
        },
        Bind {
            keys: &[Key::Enter],
            label: "enter",
            does: "open the focused session",
            command: Self::Open,
        },
        Bind {
            keys: &[Key::AltLeft],
            label: "alt-←",
            does: "the place before: the wall, then each live run",
            command: Self::PreviousPlace,
        },
        Bind {
            keys: &[Key::AltRight],
            label: "alt-→",
            does: "the next place",
            command: Self::NextPlace,
        },
        Bind {
            keys: &[Key::Interrupt],
            label: "ctrl-c",
            does: "clear what is typed, or quit (runs keep going)",
            command: Self::Quit,
        },
    ]);
}

#[cfg(test)]
mod tests {
    use super::Command;
    use crate::Key;

    #[test]
    fn keys_give_their_commands() {
        assert_eq!(Command::KEYS.find(Key::Interrupt), Some(Command::Quit));
        assert_eq!(Command::KEYS.find(Key::Enter), Some(Command::Open));
        assert_eq!(Command::KEYS.find(Key::AltRight), Some(Command::NextPlace));
        assert_eq!(Command::KEYS.find(Key::Char('q')), None);
    }
}
