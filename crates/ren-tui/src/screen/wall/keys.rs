//! The wall's keys and what each asks it to do.

use crate::Key;
use crate::keys::{Bind, Keymap};

/// What a key asks the wall to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    /// Focus the next pane, or zoom into the next session.
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
    /// Fill the screen with the focused session, or go back to the panes.
    Zoom,
    /// Keep the focused session on the wall even when it goes quiet, or
    /// stop keeping it.
    Pin,
    Help,
    Theme,
    /// Leave a zoomed session.
    Back,
    /// Open the console, on the focused session's run if it drives one.
    Runs,
}

impl Command {
    /// The wall's keys, in the order the help lists them.
    pub const KEYS: Keymap<Self> = Keymap(&[
        Bind {
            keys: &[Key::Tab, Key::Char('n')],
            label: "tab  n",
            does: "next pane (zoomed: next session)",
            command: Self::Next,
        },
        Bind {
            keys: &[Key::BackTab, Key::Char('N')],
            label: "shift-tab  N",
            does: "previous pane (zoomed: previous session)",
            command: Self::Previous,
        },
        Bind {
            keys: &[Key::Left, Key::Char('h')],
            label: "←  h",
            does: "pane to the left",
            command: Self::Left,
        },
        Bind {
            keys: &[Key::Right, Key::Char('l')],
            label: "→  l",
            does: "pane to the right",
            command: Self::Right,
        },
        Bind {
            keys: &[Key::Up, Key::Char('k')],
            label: "↑  k",
            does: "pane above",
            command: Self::Up,
        },
        Bind {
            keys: &[Key::Down, Key::Char('j')],
            label: "↓  j",
            does: "pane below",
            command: Self::Down,
        },
        Bind {
            keys: &[Key::Enter, Key::Char('z')],
            label: "enter  z",
            does: "zoom into the focused session, or back to the panes",
            command: Self::Zoom,
        },
        Bind {
            keys: &[Key::Char('p')],
            label: "p",
            does: "keep the focused session on the wall, even when quiet",
            command: Self::Pin,
        },
        Bind {
            keys: &[Key::Char('r')],
            label: "r",
            does: "runs: open the focused run, or the console",
            command: Self::Runs,
        },
        Bind {
            keys: &[Key::Char('t')],
            label: "t",
            does: "cycle theme: terminal, dark, light (not saved)",
            command: Self::Theme,
        },
        Bind {
            keys: &[Key::Esc],
            label: "esc",
            does: "close the help, or leave a zoomed session",
            command: Self::Back,
        },
        Bind {
            keys: &[Key::Char('?')],
            label: "?",
            does: "show or hide this help",
            command: Self::Help,
        },
        Bind {
            keys: &[Key::Char('q'), Key::Interrupt],
            label: "q  ctrl-c",
            does: "quit",
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
        assert_eq!(Command::KEYS.find(Key::Char('q')), Some(Command::Quit));
        assert_eq!(Command::KEYS.find(Key::Interrupt), Some(Command::Quit));
        assert_eq!(Command::KEYS.find(Key::Enter), Some(Command::Zoom));
        assert_eq!(Command::KEYS.find(Key::Char('w')), None);
        assert_eq!(Command::KEYS.help().len(), 13);
    }
}
