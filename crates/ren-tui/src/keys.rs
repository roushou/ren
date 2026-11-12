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

/// One row of the help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    pub label: String,
    pub does: String,
}

impl<C: Copy> Keymap<C> {
    /// The command `key` gives, if any.
    pub fn find(self, key: Key) -> Option<C> {
        self.0
            .iter()
            .find(|bind| bind.keys.contains(&key))
            .map(|bind| bind.command)
    }

    /// The help's rows.
    pub fn help(self) -> Vec<HelpRow> {
        self.0
            .iter()
            .map(|bind| HelpRow {
                label: bind.label.to_owned(),
                does: bind.does.to_owned(),
            })
            .collect()
    }
}
