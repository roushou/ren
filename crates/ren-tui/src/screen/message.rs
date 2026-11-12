//! The message box: what is being written to a run, each run its own
//! draft, and the run's earlier prompts to recall. It is open only while
//! writing; what is typed stays when it closes, as that run's draft.

use super::composer::{Composer, Wrapped};
use super::offer::Mention;
use crate::Key;
use ren_recorder::id::RunId;
use std::collections::HashMap;

/// The text being written to one run, and the other runs' drafts.
#[derive(Debug, Default)]
pub(crate) struct MessageBox {
    composer: Composer,
    /// The run the text is written to.
    owner: Option<RunId>,
    /// Other runs' drafts, kept as they were left.
    drafts: HashMap<RunId, String>,
    /// An earlier prompt shown again, until the text is edited.
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
    /// Enter: send what is written (queued while a turn runs).
    Submit,
    /// Alt-Enter: send it into the running turn now.
    SubmitNow,
    /// Not a key the box takes.
    Left(Key),
}

impl MessageBox {
    /// Write to `run`: the text written to another run is kept as its
    /// draft, and `run`'s own draft comes back.
    pub fn for_run(&mut self, run: RunId) {
        if self.owner == Some(run) {
            return;
        }
        let text = self.composer.take();
        if let Some(owner) = self.owner
            && !text.trim().is_empty()
        {
            self.drafts.insert(owner, text);
        }
        self.composer
            .set(&self.drafts.remove(&run).unwrap_or_default());
        self.owner = Some(run);
        self.recall = None;
    }

    /// The run the text is written to.
    pub fn owner(&self) -> Option<RunId> {
        self.owner
    }

    pub fn text(&self) -> String {
        self.composer.text()
    }

    pub fn is_empty(&self) -> bool {
        self.composer.is_empty()
    }

    /// What the word at the cursor asks to be offered.
    pub fn mention(&self) -> Option<Mention> {
        let (text, cursor) = self.composer.at_cursor();
        Mention::at(text, cursor)
    }

    /// The mention's word replaced by what was chosen.
    pub fn choose(&mut self, mention: &Mention, written: &str) {
        self.typed(|composer| composer.replace(mention.start, mention.end, written));
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

    /// Takes a key, with the text wrapped to `width`: recalling the run's
    /// earlier `prompts`, oldest first (ctrl-p and ctrl-n, or ↑ and ↓ with
    /// nothing typed or a recalled prompt shown), then editing; ↑ and ↓
    /// move through wrapped rows.
    pub fn press(&mut self, key: Key, width: usize, prompts: &[String]) -> Typed {
        let recallable = self.composer.is_empty() || self.recall.is_some();
        match key {
            Key::Older => {
                self.step(true, prompts);
            }
            Key::Newer => {
                self.step(false, prompts);
            }
            Key::Up if recallable && self.step(true, prompts) => {}
            Key::Down if self.recall.is_some() => {
                self.step(false, prompts);
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
            Key::SendNow => return Typed::SubmitNow,
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
            | Key::Client
            | Key::Model => return Typed::Left(key),
        }
        Typed::Took
    }

    /// The prompt before (or after) the one shown; after the newest, what
    /// was typed before recalling. Whether there was one.
    fn step(&mut self, older: bool, prompts: &[String]) -> bool {
        let next = match (&self.recall, older) {
            (None, true) if !prompts.is_empty() => Recall {
                back: 0,
                draft: self.composer.text(),
            },
            (None, true | false) => return false,
            (Some(recall), true) => Recall {
                back: (recall.back + 1).min(prompts.len().saturating_sub(1)),
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
        let Some(text) = prompts
            .len()
            .checked_sub(next.back + 1)
            .and_then(|at| prompts.get(at))
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
    use ren_recorder::id::RunId;

    const PROMPTS: [&str; 2] = ["fix the test", "also the docs"];

    fn prompts() -> Vec<String> {
        PROMPTS.map(str::to_owned).to_vec()
    }

    fn typed(message: &mut MessageBox, text: &str) {
        for ch in text.chars() {
            assert_eq!(message.press(Key::Char(ch), 40, &prompts()), Typed::Took);
        }
    }

    #[test]
    fn the_runs_prompts_are_recalled_newest_first_and_the_draft_comes_back() {
        let mut message = MessageBox::default();
        let prompts = prompts();
        typed(&mut message, "draft");
        // Ctrl-P goes back whatever is typed.
        let _ = message.press(Key::Older, 40, &prompts);
        assert_eq!(message.text(), "also the docs");
        let _ = message.press(Key::Up, 40, &prompts);
        assert_eq!(message.text(), "fix the test");
        let _ = message.press(Key::Up, 40, &prompts);
        assert_eq!(message.text(), "fix the test", "the oldest stays");
        let _ = message.press(Key::Down, 40, &prompts);
        let _ = message.press(Key::Newer, 40, &prompts);
        assert_eq!(message.text(), "draft");
        // ↑ recalls only with nothing typed (or a recalled prompt shown).
        assert_eq!(message.press(Key::Up, 40, &prompts), Typed::Left(Key::Up));
        let _ = message.take();
        let _ = message.press(Key::Up, 40, &prompts);
        assert_eq!(message.text(), "also the docs");
        // Editing it ends the recall: ↓ is the screen's again.
        typed(&mut message, "!");
        assert_eq!(
            message.press(Key::Down, 40, &prompts),
            Typed::Left(Key::Down)
        );
        assert_eq!(message.press(Key::SendNow, 40, &prompts), Typed::SubmitNow);
    }

    #[test]
    fn each_run_keeps_its_own_draft() {
        let (one, two) = (
            RunId::restore(uuid::Uuid::from_u128(1)),
            RunId::restore(uuid::Uuid::from_u128(2)),
        );
        let mut message = MessageBox::default();
        message.for_run(one);
        typed(&mut message, "for one");
        message.for_run(two);
        assert!(message.is_empty(), "two starts empty");
        typed(&mut message, "for two");
        message.for_run(one);
        assert_eq!(message.text(), "for one");
        message.for_run(two);
        assert_eq!(message.text(), "for two");
        assert_eq!(message.owner(), Some(two));
    }
}
