//! What the terminal tells the views: keys and sizes. Whatever reads the
//! terminal turns its events into these.

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
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    /// Alt (Option) with ← or →.
    AltLeft,
    AltRight,
    /// Ctrl-J: a new line in typed text, as terminals cannot tell
    /// Shift-Enter from Enter.
    Newline,
    /// Ctrl-C: in raw mode the terminal sends it as a key, not a signal.
    Interrupt,
}

/// A terminal's size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}
