//! The action menu: what can be done with the selected entry, opened with
//! space over the main screen. It lists only what applies now, grouped by
//! what it acts on, each with a letter that works inside the menu only; ↑
//! and ↓ with Enter work too. A delete asks once more inside the menu.

use super::Action;
use super::main::Verb;
use crate::Key;
use crate::ui::{Anchor, Emphasis, Ink, Line, Place, Span, Text, View};

/// The menu, as opened.
#[derive(Debug)]
pub(crate) struct Actions {
    title: String,
    entries: Vec<Entry>,
    /// The entry the cursor is on.
    at: usize,
    /// The entry that asks once more before it acts.
    confirming: Option<usize>,
}

/// One thing the menu can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub key: char,
    pub label: String,
    pub verb: Verb,
    pub section: Section,
}

/// What a group of entries acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    /// What the run asks.
    Answer,
    /// The run, or the session.
    Run,
    /// What was recorded of it.
    History,
    /// How Ren shows things.
    View,
}

impl Section {
    fn heading(self) -> &'static str {
        match self {
            Self::Answer => "it asks",
            Self::Run => "run",
            Self::History => "history",
            Self::View => "view",
        }
    }
}

impl Actions {
    pub fn new(title: String, entries: Vec<Entry>) -> Self {
        Self {
            title,
            entries,
            at: 0,
            confirming: None,
        }
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn press(&mut self, key: Key) -> Vec<Action> {
        if let Some(index) = self.confirming {
            return match key {
                Key::Enter => self.act(index),
                Key::Char(ch) if self.entries.get(index).is_some_and(|e| e.key == ch) => {
                    self.act(index)
                }
                // Anything else keeps it.
                Key::Esc
                | Key::Interrupt
                | Key::Char(_)
                | Key::Tab
                | Key::BackTab
                | Key::Left
                | Key::Right
                | Key::Up
                | Key::Down
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
                | Key::Newer => {
                    self.confirming = None;
                    Vec::new()
                }
            };
        }
        match key {
            Key::Esc | Key::Interrupt | Key::Char(' ') | Key::Left => vec![Action::Pop],
            Key::Up | Key::Char('k') => {
                self.at = self.at.saturating_sub(1);
                Vec::new()
            }
            Key::Down | Key::Char('j') => {
                self.at = (self.at + 1).min(self.entries.len().saturating_sub(1));
                Vec::new()
            }
            Key::Home => {
                self.at = 0;
                Vec::new()
            }
            Key::End => {
                self.at = self.entries.len().saturating_sub(1);
                Vec::new()
            }
            Key::Enter | Key::Right => self.choose(self.at),
            Key::Char(ch) => match self.entries.iter().position(|entry| entry.key == ch) {
                Some(index) => self.choose(index),
                None => Vec::new(),
            },
            Key::Tab
            | Key::BackTab
            | Key::Backspace
            | Key::Delete
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Newline
            | Key::Older
            | Key::Newer => Vec::new(),
        }
    }

    /// Choose an entry: one that asks first asks, the others act.
    fn choose(&mut self, index: usize) -> Vec<Action> {
        let Some(entry) = self.entries.get(index) else {
            return Vec::new();
        };
        self.at = index;
        if entry.verb.asks_first() {
            self.confirming = Some(index);
            return Vec::new();
        }
        self.act(index)
    }

    fn act(&self, index: usize) -> Vec<Action> {
        match self.entries.get(index) {
            Some(entry) => vec![Action::Pop, Action::Verb(entry.verb)],
            None => vec![Action::Pop],
        }
    }

    /// The menu over `under`.
    pub fn over(&self, under: View) -> View {
        let mut lines = Vec::new();
        let mut section = None;
        for (index, entry) in self.entries.iter().enumerate() {
            if section != Some(entry.section) {
                section = Some(entry.section);
                lines.push(Line::of(
                    format!(" {}", entry.section.heading()),
                    Ink::Muted,
                ));
            }
            let chosen = index == self.at;
            let mut key = Span::new(format!("  {}  ", entry.key), Ink::Tool);
            let mut label = Span::new(
                format!("{} ", entry.label),
                if entry.verb.asks_first() {
                    Ink::Error
                } else {
                    Ink::Text
                },
            );
            if chosen {
                key = key.with(Emphasis::Reversed);
                label = label.with(Emphasis::Reversed);
            }
            lines.push(Line::new(vec![key, label]));
        }
        lines.push(Line::default());
        lines.push(
            match self.confirming.and_then(|index| self.entries.get(index)) {
                Some(entry) => Line::of(
                    format!(
                        " {} for good? enter does it · any other key keeps it",
                        entry.label
                    ),
                    Ink::Error,
                ),
                None => Line::of(" a letter, or ↑↓ and enter · esc closes", Ink::Muted),
            },
        );
        let widest = lines
            .iter()
            .map(Line::width)
            .chain([Text(&self.title).width() + 4])
            .max()
            .unwrap_or(0);
        let width = u16::try_from(widest + 4).unwrap_or(u16::MAX);
        let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
        View::layer(
            under,
            View::Framed {
                title: format!(" {} ", self.title),
                ink: Ink::Bar,
                body: Box::new(View::lines(lines, Anchor::Top)),
            },
            Place::Center { width, height },
        )
    }
}
