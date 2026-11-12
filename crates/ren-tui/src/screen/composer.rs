//! The message box: text and a cursor, edited a key at a time.

/// Text being typed, with the cursor between characters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composer {
    chars: Vec<char>,
    /// The cursor's position, in characters.
    cursor: usize,
}

impl Composer {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The cursor's position, in characters.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether the cursor is after the last character.
    pub fn at_end(&self) -> bool {
        self.cursor == self.chars.len()
    }

    pub fn insert(&mut self, ch: char) {
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    /// To the start of the cursor's line.
    pub fn home(&mut self) {
        while self.cursor > 0 && self.chars[self.cursor - 1] != '\n' {
            self.cursor -= 1;
        }
    }

    /// To the end of the cursor's line.
    pub fn end(&mut self) {
        while self.cursor < self.chars.len() && self.chars[self.cursor] != '\n' {
            self.cursor += 1;
        }
    }

    /// Replace the text, with the cursor at its end.
    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
    }

    /// The text, leaving the box empty.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.chars).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Composer;

    #[test]
    fn edits_at_the_cursor() {
        let mut composer = Composer::default();
        for ch in "fix it".chars() {
            composer.insert(ch);
        }
        composer.home();
        composer.insert('>');
        composer.end();
        composer.backspace();
        composer.left();
        composer.delete();
        assert_eq!(composer.text(), ">fix ");
        composer.insert('\n');
        composer.insert('b');
        composer.home();
        assert_eq!(composer.cursor(), 6);
        assert_eq!(composer.take(), ">fix \nb");
        assert!(composer.is_empty());
    }
}
