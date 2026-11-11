//! The terminal itself: the one part of the view that performs I/O. Opening
//! a [`Screen`] puts the terminal in raw mode on its alternate screen;
//! dropping it puts the terminal back, whether the view ended, failed, or
//! panicked. [`ScreenEvents`] turns the terminal's events into the keys and
//! sizes the view reads.

use crate::{Key, Size, Theme};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::io::{self, Stdout};

/// The terminal, taken over until dropped.
#[must_use = "dropping the screen puts the terminal back"]
pub struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    theme: Theme,
}

/// Why the terminal could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ScreenError {
    #[error("cannot take over the terminal")]
    Open(#[source] io::Error),
    #[error("cannot draw to the terminal")]
    Draw(#[source] io::Error),
    #[error("cannot read the terminal's events")]
    Events(#[source] io::Error),
}

/// What the terminal reports that the view reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEvent {
    Key(Key),
    Resize(Size),
}

/// A view the screen can draw: the wall, or another screen built on it.
pub trait Drawn {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer);
}

/// The terminal's events, as the view reads them.
pub struct ScreenEvents {
    stream: EventStream,
}

impl Screen {
    /// Raw mode, the alternate screen, and a hidden cursor.
    pub fn open() -> Result<Self, ScreenError> {
        // The caller resolves color policy (including NO_COLOR). Let the
        // backend emit that theme, including resets and explicit overrides.
        crossterm::style::force_color_output(true);
        terminal::enable_raw_mode().map_err(ScreenError::Open)?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide) {
            Self::restore();
            return Err(ScreenError::Open(error));
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self {
                terminal,
                theme: Theme::default(),
            }),
            Err(error) => {
                Self::restore();
                Err(ScreenError::Open(error))
            }
        }
    }

    pub fn size(&self) -> Result<Size, ScreenError> {
        let size = self.terminal.size().map_err(ScreenError::Draw)?;
        Ok(Size {
            width: size.width,
            height: size.height,
        })
    }

    pub fn draw(&mut self, view: &impl Drawn) -> Result<(), ScreenError> {
        let theme = &self.theme;
        self.terminal
            .draw(|frame| {
                let area = frame.area();
                view.render(theme, area, frame.buffer_mut());
            })
            .map(|_| ())
            .map_err(ScreenError::Draw)
    }

    /// Used on the next draw; the root style resets the previous theme.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn events(&self) -> ScreenEvents {
        ScreenEvents {
            stream: EventStream::new(),
        }
    }

    /// Puts the terminal back as best it can: nothing is left to report
    /// a failure to.
    fn restore() {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        Self::restore();
    }
}

impl ScreenEvents {
    /// The next key or size change; `None` when the terminal's events end.
    /// Other events (mouse, focus, paste, key releases) are skipped.
    pub async fn next(&mut self) -> Option<Result<ScreenEvent, ScreenError>> {
        loop {
            let event = match self.stream.next().await? {
                Ok(event) => event,
                Err(error) => return Some(Err(ScreenError::Events(error))),
            };
            if let Some(event) = ScreenEvent::of(&event) {
                return Some(Ok(event));
            }
        }
    }
}

impl ScreenEvent {
    fn of(event: &Event) -> Option<Self> {
        match event {
            Event::Key(key) => Self::key(key).map(Self::Key),
            Event::Resize(width, height) => Some(Self::Resize(Size {
                width: *width,
                height: *height,
            })),
            Event::FocusGained | Event::FocusLost | Event::Mouse(_) | Event::Paste(_) => None,
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
            KeyCode::Char('j') if control => Some(Key::Newline),
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
    use super::ScreenEvent;
    use crate::{Key, Size};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    #[test]
    fn terminal_events_become_keys_and_sizes() {
        let press = |code, modifiers| Event::Key(KeyEvent::new(code, modifiers));
        let of = |event: Event| ScreenEvent::of(&event);
        assert_eq!(
            of(press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(ScreenEvent::Key(Key::Interrupt))
        );
        assert_eq!(
            of(press(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(ScreenEvent::Key(Key::Char('q')))
        );
        assert_eq!(of(press(KeyCode::Char('x'), KeyModifiers::CONTROL)), None);
        assert_eq!(of(press(KeyCode::F(1), KeyModifiers::NONE)), None);
        let mut release = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(of(Event::Key(release)), None);
        assert_eq!(
            of(Event::Resize(120, 40)),
            Some(ScreenEvent::Resize(Size {
                width: 120,
                height: 40
            }))
        );
        assert_eq!(of(Event::FocusLost), None);
    }
}
