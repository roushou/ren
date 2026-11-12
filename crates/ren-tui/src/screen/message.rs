//! The message box every screen shares: text being typed, the slash
//! completions offered for it, and what was sent before, to recall. Typing
//! carries over as screens change.

use super::composer::Composer;
use super::slash::{Known, Slash, Suggestion};
use crate::Key;
use crate::ui::Text;

/// Text being typed, its completions, and what was sent.
#[derive(Debug, Default)]
pub(crate) struct MessageBox {
    composer: Composer,
    /// Which completion is chosen, while some are offered.
    choice: usize,
    /// Completions closed with esc until the text changes.
    dismissed: bool,
    /// What was sent, oldest first.
    sent: Vec<String>,
    /// A sent message shown again, until the text is edited.
    recall: Option<Recall>,
}

/// Which sent message is shown, and what was typed before recalling.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Recall {
    /// Back from the newest: 0 is the newest.
    back: usize,
    draft: String,
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

    /// How many sent messages are kept to recall.
    const KEPT: usize = 100;

    /// The text, sent: kept to recall, and the box left empty.
    pub fn send(&mut self) -> String {
        let text = self.take();
        if !text.trim().is_empty() && self.sent.last() != Some(&text) {
            self.sent.push(text.clone());
            if self.sent.len() > Self::KEPT {
                self.sent.remove(0);
            }
        }
        text
    }

    /// Whether a sent message is shown and not edited since.
    pub fn recalling(&self) -> bool {
        self.recall.is_some()
    }

    /// The sent message before (or after) the one shown; after the newest,
    /// what was typed before recalling. Whether there was one.
    fn step_back(&mut self, older: bool) -> bool {
        let next = match (&self.recall, older) {
            (None, true) if !self.sent.is_empty() => Some(Recall {
                back: 0,
                draft: self.composer.text(),
            }),
            (None, true | false) => None,
            (Some(recall), true) if recall.back + 1 < self.sent.len() => Some(Recall {
                back: recall.back + 1,
                draft: recall.draft.clone(),
            }),
            (Some(recall), true) => Some(recall.clone()),
            (Some(recall), false) if recall.back > 0 => Some(Recall {
                back: recall.back - 1,
                draft: recall.draft.clone(),
            }),
            (Some(recall), false) => {
                // Past the newest: back to what was being typed.
                let draft = recall.draft.clone();
                self.recall = None;
                self.composer.set(&draft);
                return true;
            }
        };
        let Some(next) = next else {
            return false;
        };
        let Some(text) = self
            .sent
            .len()
            .checked_sub(next.back + 1)
            .and_then(|at| self.sent.get(at))
        else {
            return false;
        };
        self.composer.set(text);
        self.recall = Some(next);
        self.dismissed = true;
        true
    }

    /// The text, leaving the box empty.
    pub fn take(&mut self) -> String {
        self.recall = None;
        self.dismissed = false;
        self.choice = 0;
        self.composer.take()
    }

    /// Completions for what is typed, unless closed.
    pub fn suggestions(&self, known: &Known<'_>) -> Vec<Suggestion> {
        if self.dismissed {
            return Vec::new();
        }
        Slash::complete(&self.composer.text(), known)
    }

    /// Completions, the chosen one marked.
    pub fn popup(&self, known: &Known<'_>) -> Vec<(Suggestion, bool)> {
        let suggestions = self.suggestions(known);
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
    pub fn ghost(&self, known: &Known<'_>) -> Option<String> {
        if !self.composer.at_end() {
            return None;
        }
        let text = self.composer.text();
        let suggestions = self.suggestions(known);
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

    /// Takes a key: completions first while offered, then recalling what
    /// was sent (ctrl-p and ctrl-n, or ↑ and ↓ with nothing typed but a
    /// recalled message), then editing.
    pub fn press(&mut self, key: Key, known: &Known<'_>) -> Typed {
        let suggestions = self.suggestions(known);
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
                Key::Right if self.ghost(known).is_some() => {
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
                | Key::Interrupt
                | Key::Older
                | Key::Newer => {}
            }
        }
        let recallable = self.composer.is_empty() || self.recalling();
        match key {
            Key::Older => {
                self.step_back(true);
            }
            Key::Newer => {
                self.step_back(false);
            }
            Key::Up if recallable && self.step_back(true) => {}
            Key::Down if self.recalling() => {
                self.step_back(false);
            }
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
            self.recall = None;
        }
    }

    fn typed(&mut self, edit: impl FnOnce(&mut Composer)) {
        edit(&mut self.composer);
        self.recall = None;
        self.dismissed = false;
        self.choice = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{MessageBox, Typed};
    use crate::Key;
    use crate::screen::slash::Known;
    use ren_supervisor::{Project, ProjectGit};

    fn typed(message: &mut MessageBox, text: &str, known: &Known<'_>) {
        for ch in text.chars() {
            assert_eq!(message.press(Key::Char(ch), known), Typed::Took);
        }
    }

    #[test]
    fn the_chosen_completion_is_filled_in_and_right_takes_it() {
        let projects = vec![Project {
            path: "/dev/acme-api".into(),
            name: "acme-api".into(),
            git: ProjectGit::None,
        }];
        let known = Known {
            projects: &projects,
            runs: Vec::new(),
        };
        let mut message = MessageBox::default();
        typed(&mut message, "/re", &known);
        assert_eq!(message.ghost(&known).as_deref(), Some("sume "));
        let _ = message.press(Key::Right, &known);
        assert_eq!(message.text(), "/resume ");
        assert_eq!(message.ghost(&known), None);

        // A project is filled in once its name carries on what is typed.
        let mut message = MessageBox::default();
        typed(&mut message, "/new ac", &known);
        assert_eq!(message.ghost(&known).as_deref(), Some("me-api "));
        let _ = message.press(Key::Tab, &known);
        assert_eq!(message.text(), "/new acme-api ");

        // Matched inside the name: offered, but nothing to fill in.
        let mut message = MessageBox::default();
        typed(&mut message, "/new api", &known);
        assert_eq!(message.ghost(&known), None);

        // Away from the end, → moves the cursor.
        let mut message = MessageBox::default();
        typed(&mut message, "/re", &known);
        let _ = message.press(Key::Left, &known);
        assert_eq!(message.ghost(&known), None);
        let _ = message.press(Key::Right, &known);
        assert_eq!(message.text(), "/re");
    }

    #[test]
    fn what_was_sent_is_recalled_newest_first_and_the_draft_comes_back() {
        let known = Known::default();
        let mut message = MessageBox::default();
        for text in ["fix the test", "also the docs", "also the docs", "  "] {
            typed(&mut message, text, &known);
            let _ = message.send();
        }
        typed(&mut message, "draft", &known);
        // Ctrl-P goes back whatever is typed; repeats are kept once.
        let _ = message.press(Key::Older, &known);
        assert_eq!(message.text(), "also the docs");
        let _ = message.press(Key::Up, &known);
        assert_eq!(message.text(), "fix the test");
        let _ = message.press(Key::Up, &known);
        assert_eq!(message.text(), "fix the test", "the oldest stays");
        let _ = message.press(Key::Down, &known);
        let _ = message.press(Key::Newer, &known);
        assert_eq!(message.text(), "draft");
        // ↑ recalls only with nothing typed (or a recalled message shown).
        assert_eq!(message.press(Key::Up, &known), Typed::Left(Key::Up));
        let _ = message.take();
        let _ = message.press(Key::Up, &known);
        assert_eq!(message.text(), "also the docs");
        // Editing it ends the recall: ↓ is the screen's again.
        typed(&mut message, "!", &known);
        assert_eq!(message.press(Key::Down, &known), Typed::Left(Key::Down));
    }
}
