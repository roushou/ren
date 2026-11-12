//! The message box: what is being written to a run, and what was sent
//! before, to recall. It is open only while writing; what is typed stays
//! when it closes, as a draft.

use super::composer::{Composer, Wrapped};
use crate::Key;

/// Text being written, and what was sent.
#[derive(Debug, Default)]
pub(crate) struct MessageBox {
    composer: Composer,
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
    /// It edited the text, or recalled a sent message.
    Took,
    /// Enter: send what is written.
    Submit,
    /// Not a key the box takes.
    Left(Key),
}

impl MessageBox {
    /// How many sent messages are kept to recall.
    const KEPT: usize = 100;

    pub fn text(&self) -> String {
        self.composer.text()
    }

    pub fn is_empty(&self) -> bool {
        self.composer.is_empty()
    }

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

    /// The text, leaving the box empty.
    pub fn take(&mut self) -> String {
        self.recall = None;
        self.composer.take()
    }

    /// Its text wrapped to `width`, and the cursor in it.
    pub fn wrap(&self, width: usize) -> Wrapped {
        self.composer.wrap(width)
    }

    /// Text pasted at the cursor, with its line breaks.
    pub fn paste(&mut self, text: &str) {
        self.typed(|composer| composer.paste(text));
    }

    /// The text as the operator's editor gave it back.
    pub fn edited(&mut self, text: &str) {
        self.typed(|composer| composer.set(text.trim_end_matches('\n')));
    }

    /// Takes a key, with the text wrapped to `width`: recalling what was
    /// sent (ctrl-p and ctrl-n, or ↑ and ↓ with nothing typed or a recalled
    /// message shown), then editing; ↑ and ↓ move through wrapped rows.
    pub fn press(&mut self, key: Key, width: usize) -> Typed {
        let recallable = self.composer.is_empty() || self.recall.is_some();
        match key {
            Key::Older => {
                self.step(true);
            }
            Key::Newer => {
                self.step(false);
            }
            Key::Up if recallable && self.step(true) => {}
            Key::Down if self.recall.is_some() => {
                self.step(false);
            }
            Key::Up if self.composer.up(width) => {}
            Key::Down if self.composer.down(width) => {}
            Key::Char(ch) => self.typed(|composer| composer.insert(ch)),
            Key::Newline => self.typed(|composer| composer.insert('\n')),
            Key::Backspace => self.typed(Composer::backspace),
            Key::Delete => self.typed(Composer::delete),
            Key::Left => self.composer.left(),
            Key::Right => self.composer.right(),
            Key::Home => self.composer.home(),
            Key::End => self.composer.end(),
            Key::Enter => return Typed::Submit,
            Key::Up
            | Key::Down
            | Key::Interrupt
            | Key::Tab
            | Key::BackTab
            | Key::Esc
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Editor
            | Key::Folder
            | Key::Client => return Typed::Left(key),
        }
        Typed::Took
    }

    /// The sent message before (or after) the one shown; after the newest,
    /// what was typed before recalling. Whether there was one.
    fn step(&mut self, older: bool) -> bool {
        let next = match (&self.recall, older) {
            (None, true) if !self.sent.is_empty() => Recall {
                back: 0,
                draft: self.composer.text(),
            },
            (None, true | false) => return false,
            (Some(recall), true) => Recall {
                back: (recall.back + 1).min(self.sent.len().saturating_sub(1)),
                draft: recall.draft.clone(),
            },
            (Some(recall), false) if recall.back > 0 => Recall {
                back: recall.back - 1,
                draft: recall.draft.clone(),
            },
            (Some(recall), false) => {
                // Past the newest: back to what was being typed.
                let draft = recall.draft.clone();
                self.recall = None;
                self.composer.set(&draft);
                return true;
            }
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
        true
    }

    fn typed(&mut self, edit: impl FnOnce(&mut Composer)) {
        edit(&mut self.composer);
        self.recall = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{MessageBox, Typed};
    use crate::Key;

    fn typed(message: &mut MessageBox, text: &str) {
        for ch in text.chars() {
            assert_eq!(message.press(Key::Char(ch), 40), Typed::Took);
        }
    }

    #[test]
    fn what_was_sent_is_recalled_newest_first_and_the_draft_comes_back() {
        let mut message = MessageBox::default();
        for text in ["fix the test", "also the docs", "also the docs", "  "] {
            typed(&mut message, text);
            let _ = message.send();
        }
        typed(&mut message, "draft");
        // Ctrl-P goes back whatever is typed; repeats are kept once.
        let _ = message.press(Key::Older, 40);
        assert_eq!(message.text(), "also the docs");
        let _ = message.press(Key::Up, 40);
        assert_eq!(message.text(), "fix the test");
        let _ = message.press(Key::Up, 40);
        assert_eq!(message.text(), "fix the test", "the oldest stays");
        let _ = message.press(Key::Down, 40);
        let _ = message.press(Key::Newer, 40);
        assert_eq!(message.text(), "draft");
        // ↑ recalls only with nothing typed (or a recalled message shown).
        assert_eq!(message.press(Key::Up, 40), Typed::Left(Key::Up));
        let _ = message.take();
        let _ = message.press(Key::Up, 40);
        assert_eq!(message.text(), "also the docs");
        // Editing it ends the recall: ↓ is the screen's again.
        typed(&mut message, "!");
        assert_eq!(message.press(Key::Down, 40), Typed::Left(Key::Down));
    }
}
