//! Keys the view reads, and what each asks it to do. Whatever reads the
//! terminal turns its key events into [`Key`]s; the help overlay lists
//! [`Command::ALL`] with the keys that give each.

/// A key, as the view reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Tab,
    BackTab,
    Left,
    Right,
    Up,
    Down,
    Enter,
    Esc,
    Backspace,
    /// Ctrl-C: in raw mode the terminal sends it as a key, not a signal.
    Interrupt,
}

/// What a key asks the view to do.
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
    /// Close the help, or leave a zoomed session.
    Back,
    /// Type a prompt for the focused session's run.
    Compose,
    /// Answer the run's oldest approval: allow it once.
    Accept,
    /// Allow it, and the same kind of action for the rest of the session.
    AcceptForSession,
    /// Refuse it; the turn goes on.
    Decline,
    /// Interrupt the run's turn.
    InterruptRun,
    /// Stop the run's process.
    StopRun,
    /// Start a new run: type the folder.
    StartRun,
}

impl Command {
    /// Every command, in the order the help lists them.
    pub const ALL: [Self; 19] = [
        Self::Next,
        Self::Previous,
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Zoom,
        Self::Pin,
        Self::StartRun,
        Self::Compose,
        Self::Accept,
        Self::AcceptForSession,
        Self::Decline,
        Self::InterruptRun,
        Self::StopRun,
        Self::Theme,
        Self::Back,
        Self::Help,
        Self::Quit,
    ];

    /// The command `key` gives, if any.
    pub fn of(key: Key) -> Option<Self> {
        match key {
            Key::Char('q') | Key::Interrupt => Some(Self::Quit),
            Key::Tab | Key::Char('n') => Some(Self::Next),
            Key::BackTab | Key::Char('N') => Some(Self::Previous),
            Key::Left | Key::Char('h') => Some(Self::Left),
            Key::Right | Key::Char('l') => Some(Self::Right),
            Key::Up | Key::Char('k') => Some(Self::Up),
            Key::Down | Key::Char('j') => Some(Self::Down),
            Key::Enter | Key::Char('z') => Some(Self::Zoom),
            Key::Char('p') => Some(Self::Pin),
            Key::Char('?') => Some(Self::Help),
            Key::Char('t') => Some(Self::Theme),
            Key::Esc => Some(Self::Back),
            Key::Char('s') => Some(Self::StartRun),
            Key::Char('c') => Some(Self::Compose),
            Key::Char('a') => Some(Self::Accept),
            Key::Char('A') => Some(Self::AcceptForSession),
            Key::Char('d') => Some(Self::Decline),
            Key::Char('x') => Some(Self::InterruptRun),
            Key::Char('X') => Some(Self::StopRun),
            Key::Char(_) | Key::Backspace => None,
        }
    }

    /// The keys that give it, as the help writes them.
    pub fn keys(self) -> &'static str {
        match self {
            Self::Quit => "q  ctrl-c",
            Self::Next => "tab  n",
            Self::Previous => "shift-tab  N",
            Self::Left => "←  h",
            Self::Right => "→  l",
            Self::Up => "↑  k",
            Self::Down => "↓  j",
            Self::Zoom => "enter  z",
            Self::Pin => "p",
            Self::Help => "?",
            Self::Theme => "t",
            Self::Back => "esc",
            Self::StartRun => "s",
            Self::Compose => "c",
            Self::Accept => "a",
            Self::AcceptForSession => "A",
            Self::Decline => "d",
            Self::InterruptRun => "x",
            Self::StopRun => "X",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Next => "next pane (zoomed: next session)",
            Self::Previous => "previous pane (zoomed: previous session)",
            Self::Left => "pane to the left",
            Self::Right => "pane to the right",
            Self::Up => "pane above",
            Self::Down => "pane below",
            Self::Zoom => "zoom into the focused session, or back to the panes",
            Self::Pin => "keep the focused session on the wall, even when quiet",
            Self::Help => "show or hide this help",
            Self::Theme => "cycle theme: terminal, dark, light (not saved)",
            Self::Back => "close the help, or leave a zoomed session",
            Self::StartRun => "start a Codex run: type its folder",
            Self::Compose => "prompt the focused session's run",
            Self::Accept => "approve the run's oldest request, once",
            Self::AcceptForSession => "approve it for the rest of the session",
            Self::Decline => "decline it; the turn goes on",
            Self::InterruptRun => "interrupt the run's turn",
            Self::StopRun => "stop the run",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Key};

    #[test]
    fn every_command_has_a_key_that_gives_it() {
        let keys = [
            Key::Char('q'),
            Key::Tab,
            Key::BackTab,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Enter,
            Key::Char('p'),
            Key::Char('?'),
            Key::Char('t'),
            Key::Esc,
            Key::Char('s'),
            Key::Char('c'),
            Key::Char('a'),
            Key::Char('A'),
            Key::Char('d'),
            Key::Char('x'),
            Key::Char('X'),
        ];
        for command in Command::ALL {
            assert!(
                keys.iter().any(|key| Command::of(*key) == Some(command)),
                "{command:?}"
            );
        }
        assert_eq!(Command::of(Key::Interrupt), Some(Command::Quit));
        assert_eq!(Command::of(Key::Char('w')), None);
    }
}
