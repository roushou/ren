//! Key tables as data. Each screen declares its keys once, as a
//! [`Keymap`] of [`Bind`]s: what keys give which of its commands, and how
//! the help words them. Looking a key up and listing the help both read the
//! same table, so the two never disagree.

use crate::Key;

/// Keys that give one command, and how the help writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bind<C: 'static> {
    pub keys: &'static [Key],
    /// The keys as the help writes them.
    pub label: &'static str,
    pub does: &'static str,
    pub command: C,
}

/// A screen's keys, in the order the help lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keymap<C: 'static>(pub &'static [Bind<C>]);

/// One row of the help: a key and what it does, or a heading over the
/// keys of one layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpRow {
    Key { label: String, does: String },
    Heading(String),
}

impl HelpRow {
    pub fn key(label: &str, does: &str) -> Self {
        Self::Key {
            label: label.to_owned(),
            does: does.to_owned(),
        }
    }

    pub fn heading(text: &str) -> Self {
        Self::Heading(text.to_owned())
    }
}

impl<C: Copy> Keymap<C> {
    /// The command `key` gives, if any.
    pub fn find(self, key: Key) -> Option<C> {
        self.0
            .iter()
            .find(|bind| bind.keys.contains(&key))
            .map(|bind| bind.command)
    }

    /// The help's rows; a bind without words shares the row before.
    pub fn help(self) -> Vec<HelpRow> {
        self.0
            .iter()
            .filter(|bind| !bind.does.is_empty())
            .map(|bind| HelpRow::key(bind.label, bind.does))
            .collect()
    }
}
