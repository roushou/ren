//! The message box the console's screens share: text being typed, and the
//! slash completions offered for it. Typing carries over as screens change.

use super::composer::Composer;
use super::slash::{Slash, Suggestion};
use crate::Key;
use crate::ui::Text;
use ren_supervisor::Project;

/// Text being typed, and its completions.
#[derive(Debug, Default)]
pub(crate) struct MessageBox {
    composer: Composer,
    /// Which completion is chosen, while some are offered.
    choice: usize,
    /// Completions closed with esc until the text changes.
    dismissed: bool,
}

/// What a key did to the message box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Typed {
    /// It edited the text or the completions.
    Took,
    /// Enter: the screen sends what is typed.
    Submit,
    /// Not a key the box takes: the screen's own.
    Left(Key),
}

impl MessageBox {
    pub fn text(&self) -> String {
        self.composer.text()
    }

    pub fn is_empty(&self) -> bool {
        self.composer.is_empty()
    }

    /// The text, leaving the box empty.
    pub fn take(&mut self) -> String {
        self.dismissed = false;
        self.choice = 0;
        self.composer.take()
    }

    /// Completions for what is typed, unless closed.
    pub fn suggestions(&self, projects: &[Project]) -> Vec<Suggestion> {
        if self.dismissed {
            return Vec::new();
        }
        Slash::complete(&self.composer.text(), projects)
    }

    /// Completions, the chosen one marked.
    pub fn popup(&self, projects: &[Project]) -> Vec<(Suggestion, bool)> {
        let suggestions = self.suggestions(projects);
        let choice = self.choice.min(suggestions.len().saturating_sub(1));
        suggestions
            .into_iter()
            .enumerate()
            .map(|(index, suggestion)| (suggestion, index == choice))
            .collect()
    }

    /// Its lines, and where the cursor is in them: a line and a column.
    pub fn lines(&self) -> (Vec<String>, (usize, usize)) {
        let text = self.composer.text();
        let before: String = text.chars().take(self.composer.cursor()).collect();
        let line = before.matches('\n').count();
        let column = Text(before.rsplit('\n').next().unwrap_or_default()).width();
        (
            text.split('\n').map(str::to_owned).collect(),
            (line, column),
        )
    }

    /// Takes a key: completions first while offered, then editing.
    pub fn press(&mut self, key: Key, projects: &[Project]) -> Typed {
        let suggestions = self.suggestions(projects);
        if !suggestions.is_empty() {
            match key {
                Key::Up => {
                    self.choice = self.choice.saturating_sub(1);
                    return Typed::Took;
                }
                Key::Down => {
                    self.choice = (self.choice + 1).min(suggestions.len() - 1);
                    return Typed::Took;
                }
                Key::Tab => {
                    if let Some(chosen) = suggestions.get(self.choice) {
                        self.composer.set(&chosen.replacement);
                        self.choice = 0;
                    }
                    return Typed::Took;
                }
                Key::Esc => {
                    self.dismissed = true;
                    return Typed::Took;
                }
                Key::Char(_)
                | Key::BackTab
                | Key::Left
                | Key::Right
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
                | Key::Interrupt => {}
            }
        }
        match key {
            Key::Char(ch) => self.typed(|composer| composer.insert(ch)),
            Key::Newline => self.typed(|composer| composer.insert('\n')),
            Key::Backspace => self.typed(Composer::backspace),
            Key::Delete => self.typed(Composer::delete),
            Key::Left => self.composer.left(),
            Key::Right => self.composer.right(),
            Key::Home => self.composer.home(),
            Key::End => self.composer.end(),
            Key::Enter => return Typed::Submit,
            // Ctrl-C clears what is typed; with nothing typed it is the
            // screen's.
            Key::Interrupt if !self.composer.is_empty() => {
                self.take();
            }
            Key::Interrupt
            | Key::Up
            | Key::Down
            | Key::Tab
            | Key::BackTab
            | Key::Esc
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight => return Typed::Left(key),
        }
        Typed::Took
    }

    fn typed(&mut self, edit: impl FnOnce(&mut Composer)) {
        edit(&mut self.composer);
        self.dismissed = false;
        self.choice = 0;
    }
}
