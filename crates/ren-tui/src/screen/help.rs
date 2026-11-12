//! The help: the keys of the screen under it, from that screen's own key
//! table, in a box over it.

use super::Action;
use crate::Key;
use crate::keys::HelpRow;
use crate::ui::{Anchor, Emphasis, Ink, Line, Place, Span, Text, View};

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
            Key::Char('?') | Key::Esc | Key::Interrupt => vec![Action::Pop],
            Key::Char('t') => vec![Action::CycleTheme],
            Key::Char('q') => vec![Action::Quit],
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
            | Key::Newline
            | Key::Older
            | Key::Newer => Vec::new(),
        }
    }

    /// The box over `under`.
    pub fn over(&self, under: View) -> View {
        let keys = || {
            self.rows.iter().filter_map(|row| match row {
                HelpRow::Key { label, does } => Some((label, does)),
                HelpRow::Heading(_) => None,
            })
        };
        let label_width = keys()
            .map(|(label, _)| Text(label).width())
            .max()
            .unwrap_or(0);
        let text_width = keys()
            .map(|(_, does)| Text(does).width())
            .max()
            .unwrap_or(0);
        let mut lines = Vec::new();
        for row in &self.rows {
            match row {
                HelpRow::Heading(text) => lines.push(Line::new(vec![
                    Span::new(format!(" {text}"), Ink::Text).with(Emphasis::Bold),
                ])),
                HelpRow::Key { label, does } => lines.push(Line::new(vec![
                    Span::new(format!(" {label:<label_width$}   "), Ink::Tool),
                    Span::plain(does.clone()),
                ])),
            }
        }
        let width = u16::try_from(label_width + 4 + text_width + 4).unwrap_or(u16::MAX);
        let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
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
