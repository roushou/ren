//! The help: the keys of the screen under it, from that screen's own key
//! table, in a box over it.

use super::Action;
use crate::Key;
use crate::keys::HelpRow;
use crate::ui::{Anchor, Ink, Line, Place, Span, Text, View};

/// The keys of the screen it was opened over.
#[derive(Debug)]
pub(crate) struct Help {
    rows: Vec<HelpRow>,
}

impl Help {
    pub fn new(rows: Vec<HelpRow>) -> Self {
        Self { rows }
    }

    /// It closes, previews themes, and quits; other keys do nothing.
    pub fn press(&self, key: Key) -> Vec<Action> {
        match key {
            Key::Char('?') | Key::Esc => vec![Action::Pop],
            Key::Char('t') => vec![Action::CycleTheme],
            Key::Char('q') | Key::Interrupt => vec![Action::Quit],
            Key::Char(_)
            | Key::Tab
            | Key::BackTab
            | Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
            | Key::Enter
            | Key::Backspace
            | Key::Delete
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Newline => Vec::new(),
        }
    }

    /// The box over `under`.
    pub fn over(&self, under: View) -> View {
        let label_width = self
            .rows
            .iter()
            .map(|row| Text(&row.label).width())
            .max()
            .unwrap_or(0);
        let text_width = self
            .rows
            .iter()
            .map(|row| Text(&row.does).width())
            .max()
            .unwrap_or(0);
        let lines = self
            .rows
            .iter()
            .map(|row| {
                Line::new(vec![
                    Span::new(format!("{:<label_width$}   ", row.label), Ink::Tool),
                    Span::plain(row.does.clone()),
                ])
            })
            .collect();
        let width = u16::try_from(label_width + 3 + text_width + 4).unwrap_or(u16::MAX);
        let height = u16::try_from(self.rows.len() + 2).unwrap_or(u16::MAX);
        View::layer(
            under,
            View::Framed {
                title: " keys ".to_owned(),
                ink: Ink::Bar,
                body: Box::new(View::lines(lines, Anchor::Top)),
            },
            Place::Center { width, height },
        )
    }
}
