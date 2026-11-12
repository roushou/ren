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

    /// The rest of the chosen completion, filled in after the cursor: only
    /// while the cursor is at the end and the completion carries on what is
    /// typed.
    pub fn ghost(&self, projects: &[Project]) -> Option<String> {
        if !self.composer.at_end() {
            return None;
        }
        let text = self.composer.text();
        let suggestions = self.suggestions(projects);
        let chosen = suggestions.get(self.choice.min(suggestions.len().saturating_sub(1)))?;
        chosen
            .replacement
            .strip_prefix(&text)
            .filter(|rest| !rest.is_empty())
            .map(str::to_owned)
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
                    self.accept(&suggestions);
                    return Typed::Took;
                }
                // At the end, → takes the filled-in rest, as in a shell.
                Key::Right if self.ghost(projects).is_some() => {
                    self.accept(&suggestions);
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

    /// The chosen completion, in place of what is typed.
    fn accept(&mut self, suggestions: &[Suggestion]) {
        if let Some(chosen) = suggestions.get(self.choice) {
            self.composer.set(&chosen.replacement);
            self.choice = 0;
        }
    }

    fn typed(&mut self, edit: impl FnOnce(&mut Composer)) {
        edit(&mut self.composer);
        self.dismissed = false;
        self.choice = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{MessageBox, Typed};
    use crate::Key;
    use ren_supervisor::{Project, ProjectGit};

    fn typed(text: &str, projects: &[Project]) -> MessageBox {
        let mut message = MessageBox::default();
        for ch in text.chars() {
            assert_eq!(message.press(Key::Char(ch), projects), Typed::Took);
        }
        message
    }

    #[test]
    fn the_chosen_completion_is_filled_in_and_right_takes_it() {
        let projects = vec![Project {
            path: "/dev/acme-api".into(),
            name: "acme-api".into(),
            git: ProjectGit::None,
        }];
        let mut message = typed("/re", &projects);
        assert_eq!(message.ghost(&projects).as_deref(), Some("sume "));
        let _ = message.press(Key::Right, &projects);
        assert_eq!(message.text(), "/resume ");
        assert_eq!(message.ghost(&projects), None);

        // A project is filled in once its name carries on what is typed.
        let mut message = typed("/new ac", &projects);
        assert_eq!(message.ghost(&projects).as_deref(), Some("me-api "));
        let _ = message.press(Key::Tab, &projects);
        assert_eq!(message.text(), "/new acme-api ");

        // Matched inside the name: offered, but nothing to fill in.
        let message = typed("/new api", &projects);
        assert_eq!(message.ghost(&projects), None);

        // Away from the end, → moves the cursor.
        let mut message = typed("/re", &projects);
        let _ = message.press(Key::Left, &projects);
        assert_eq!(message.ghost(&projects), None);
        let _ = message.press(Key::Right, &projects);
        assert_eq!(message.text(), "/re");
    }
}
