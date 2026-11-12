//! The terminal itself: the one part of the view that performs I/O. Opening
//! a [`Terminal`] puts the terminal in raw mode on its alternate screen;
//! dropping it puts the terminal back, whether the view ended, failed, or
//! panicked. [`TerminalEvents`] turns the terminal's events into the keys and
//! sizes the view reads.

use crate::ui::View;
use crate::{Key, Size, Theme};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use crossterm::{cursor, execute, terminal};
use futures_util::StreamExt;
use ratatui::Terminal as Ratatui;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};

/// The terminal, taken over until dropped.
#[must_use = "dropping it puts the terminal back"]
pub struct Terminal {
    terminal: Ratatui<CrosstermBackend<Stdout>>,
    theme: Theme,
}

/// Why the terminal could not be used.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("cannot take over the terminal")]
    Open(#[source] io::Error),
    #[error("cannot draw to the terminal")]
    Draw(#[source] io::Error),
    #[error("cannot read the terminal's events")]
    Events(#[source] io::Error),
}

/// What the terminal reports that the view reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Key(Key),
    /// Text pasted, as one block with its line breaks (bracketed paste),
    /// not as keys: an Enter in it does not send.
    Paste(String),
    Resize(Size),
}

/// The terminal's events, as the view reads them.
pub struct TerminalEvents {
    stream: EventStream,
}

impl Terminal {
    /// Raw mode, the alternate screen, and a hidden cursor.
    pub fn open() -> Result<Self, TerminalError> {
        // The caller resolves color policy (including NO_COLOR). Let the
        // backend emit that theme, including resets and explicit overrides.
        crossterm::style::force_color_output(true);
        terminal::enable_raw_mode().map_err(TerminalError::Open)?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(
            stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            EnableBracketedPaste
        ) {
            Self::restore();
            return Err(TerminalError::Open(error));
        }
        match Ratatui::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self {
                terminal,
                theme: Theme::default(),
            }),
            Err(error) => {
                Self::restore();
                Err(TerminalError::Open(error))
            }
        }
    }

    pub fn size(&self) -> Result<Size, TerminalError> {
        let size = self.terminal.size().map_err(TerminalError::Draw)?;
        Ok(Size {
            width: size.width,
            height: size.height,
        })
    }

    /// Draws the view `view` builds for the terminal's size.
    pub fn draw(&mut self, view: impl FnOnce(Size) -> View) -> Result<(), TerminalError> {
        let theme = &self.theme;
        self.terminal
            .draw(|frame| {
                let area = frame.area();
                view(Size {
                    width: area.width,
                    height: area.height,
                })
                .render(theme, area, frame.buffer_mut());
            })
            .map(|_| ())
            .map_err(TerminalError::Draw)
    }

    /// Used on the next draw; the root style resets the previous theme.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn events(&self) -> TerminalEvents {
        TerminalEvents {
            stream: EventStream::new(),
        }
    }

    /// Hands the terminal to another program (the operator's editor) until
    /// [`Self::resume`]. Drop the [`TerminalEvents`] first, or they read
    /// that program's keys.
    pub fn suspend(&mut self) -> Result<(), TerminalError> {
        Self::restore();
        Ok(())
    }

    /// Takes the terminal back after [`Self::suspend`] and draws it whole.
    pub fn resume(&mut self) -> Result<(), TerminalError> {
        terminal::enable_raw_mode().map_err(TerminalError::Open)?;
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            EnableBracketedPaste
        )
        .map_err(TerminalError::Open)?;
        // A fresh surface draws every cell on the next draw. (Clearing the
        // old one asks the terminal where its cursor is, which a terminal
        // just handed back may not answer in time.)
        execute!(io::stdout(), terminal::Clear(terminal::ClearType::All))
            .map_err(TerminalError::Open)?;
        self.terminal =
            Ratatui::new(CrosstermBackend::new(io::stdout())).map_err(TerminalError::Open)?;
        Ok(())
    }

    /// Puts the terminal back as best it can: nothing is left to report
    /// a failure to.
    fn restore() {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        Self::restore();
    }
}

impl TerminalEvents {
    /// The next key, paste, or size change; `None` when the terminal's
    /// events end. Other events (mouse, focus, key releases) are skipped.
    pub async fn next(&mut self) -> Option<Result<TerminalEvent, TerminalError>> {
        loop {
            let event = match self.stream.next().await? {
                Ok(event) => event,
                Err(error) => return Some(Err(TerminalError::Events(error))),
            };
            if let Some(event) = TerminalEvent::of(&event) {
                return Some(Ok(event));
            }
        }
    }
}

impl TerminalEvent {
    fn of(event: &Event) -> Option<Self> {
        match event {
            Event::Key(key) => Self::key(key).map(Self::Key),
            Event::Resize(width, height) => Some(Self::Resize(Size {
                width: *width,
                height: *height,
            })),
            Event::Paste(text) => Some(Self::Paste(text.clone())),
            Event::FocusGained | Event::FocusLost | Event::Mouse(_) => None,
        }
    }

    fn key(event: &KeyEvent) -> Option<Key> {
        if event.kind != KeyEventKind::Press {
            return None;
        }
        let control = event.modifiers.contains(KeyModifiers::CONTROL);
        let alt = event.modifiers.contains(KeyModifiers::ALT);
        match event.code {
            KeyCode::Char('c') if control => Some(Key::Interrupt),
            KeyCode::Enter if alt => Some(Key::SendNow),
            KeyCode::Char('j') if control => Some(Key::Newline),
            KeyCode::Char('p') if control => Some(Key::Older),
            KeyCode::Char('n') if control => Some(Key::Newer),
            KeyCode::Char('e') if control => Some(Key::Editor),
            KeyCode::Char('o') if control => Some(Key::Folder),
            KeyCode::Char('l') if control => Some(Key::Client),
            KeyCode::Char('k') if control => Some(Key::Model),
            KeyCode::Char(_) if control => None,
            // macOS terminals send Option-← and Option-→ as Alt-b and Alt-f.
            KeyCode::Char('b') if alt => Some(Key::AltLeft),
            KeyCode::Char('f') if alt => Some(Key::AltRight),
            KeyCode::Char(ch) => Some(Key::Char(ch)),
            KeyCode::Tab => Some(Key::Tab),
            KeyCode::BackTab => Some(Key::BackTab),
            KeyCode::Left if alt => Some(Key::AltLeft),
            KeyCode::Right if alt => Some(Key::AltRight),
            KeyCode::Left => Some(Key::Left),
            KeyCode::Right => Some(Key::Right),
            KeyCode::Up => Some(Key::Up),
            KeyCode::Down => Some(Key::Down),
            KeyCode::Enter => Some(Key::Enter),
            KeyCode::Esc => Some(Key::Esc),
            KeyCode::Backspace => Some(Key::Backspace),
            KeyCode::Delete => Some(Key::Delete),
            KeyCode::Home => Some(Key::Home),
            KeyCode::End => Some(Key::End),
            KeyCode::PageUp => Some(Key::PageUp),
            KeyCode::PageDown => Some(Key::PageDown),
            KeyCode::Insert
            | KeyCode::F(_)
            | KeyCode::Null
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalEvent;
    use crate::{Key, Size};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    #[test]
    fn terminal_events_become_keys_and_sizes() {
        let press = |code, modifiers| Event::Key(KeyEvent::new(code, modifiers));
        let of = |event: Event| TerminalEvent::of(&event);
        assert_eq!(
            of(press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(TerminalEvent::Key(Key::Interrupt))
        );
        assert_eq!(
            of(press(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(TerminalEvent::Key(Key::Char('q')))
        );
        for (ch, key) in [
            ('e', Key::Editor),
            ('o', Key::Folder),
            ('l', Key::Client),
            ('k', Key::Model),
            ('j', Key::Newline),
        ] {
            assert_eq!(
                of(press(KeyCode::Char(ch), KeyModifiers::CONTROL)),
                Some(TerminalEvent::Key(key))
            );
        }
        // A paste is one block, its line breaks kept.
        assert_eq!(
            of(Event::Paste("one\ntwo".into())),
            Some(TerminalEvent::Paste("one\ntwo".into()))
        );
        assert_eq!(of(press(KeyCode::Char('x'), KeyModifiers::CONTROL)), None);
        assert_eq!(of(press(KeyCode::F(1), KeyModifiers::NONE)), None);
        let mut release = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(of(Event::Key(release)), None);
        assert_eq!(
            of(Event::Resize(120, 40)),
            Some(TerminalEvent::Resize(Size {
                width: 120,
                height: 40
            }))
        );
        assert_eq!(of(Event::FocusLost), None);
    }
}
