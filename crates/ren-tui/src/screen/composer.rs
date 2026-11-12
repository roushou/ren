//! The editor every prompt is written in: text and a cursor, edited a key
//! (or a paste) at a time, and wrapped to the width it is shown at, so the
//! cursor moves up and down through wrapped rows as it is drawn.

use crate::ui::{Emphasis, Ink, Line, Span, Text};

/// Text being typed, with the cursor between characters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composer {
    chars: Vec<char>,
    /// The cursor's position, in characters.
    cursor: usize,
}

/// One row of wrapped text: characters `start..end` of the text, without
/// the line break that may follow them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    start: usize,
    end: usize,
    /// It ends its line: a line break or the end of the text follows.
    last: bool,
}

/// The text wrapped to a width, and where the cursor is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    pub rows: Vec<String>,
    /// The cursor's row, and its column in cells.
    pub cursor: (usize, usize),
}

impl Wrapped {
    /// The rows of a window `height` rows tall that keeps the cursor in
    /// view: from the top while it fits, else ending at the cursor.
    pub fn window(&self, height: usize) -> std::ops::Range<usize> {
        let height = height.max(1);
        let first = (self.cursor.0 + 1).saturating_sub(height);
        first..(first + height).min(self.rows.len())
    }

    /// The rows in a window `height` rows tall, each after the span `lead`
    /// gives for its row, with the cursor drawn when `cursor` holds.
    pub fn lines(&self, height: usize, cursor: bool, lead: impl Fn(usize) -> Span) -> Vec<Line> {
        self.window(height)
            .map(|row| {
                let text = &self.rows[row];
                let mut line = Line::new(vec![lead(row)]);
                if !cursor || row != self.cursor.0 {
                    line.push(Span::new(text.clone(), Ink::Text));
                    return line;
                }
                let mut before = String::new();
                let mut used = 0;
                let mut chars = text.chars();
                while let Some(ch) = chars.next() {
                    if used >= self.cursor.1 {
                        line.push(Span::new(before, Ink::Text));
                        line.push(Span::new(ch.to_string(), Ink::Text).with(Emphasis::Reversed));
                        line.push(Span::new(chars.as_str(), Ink::Text));
                        return line;
                    }
                    used += Composer::cell(ch);
                    before.push(ch);
                }
                line.push(Span::new(before, Ink::Text));
                line.push(Span::new(" ", Ink::Text).with(Emphasis::Reversed));
                line
            })
            .collect()
    }
}

impl Composer {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Its lines, as written, and its characters.
    pub fn counts(&self) -> (usize, usize) {
        let lines = self.chars.iter().filter(|ch| **ch == '\n').count() + 1;
        (lines, self.chars.len())
    }

    /// The cursor's position, in characters.
    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, ch: char) {
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    /// Text pasted at the cursor, with its line breaks, as one edit.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let pasted: Vec<char> = text.chars().collect();
        let count = pasted.len();
        self.chars.splice(self.cursor..self.cursor, pasted);
        self.cursor += count;
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

    /// Up a row as wrapped to `width`; whether there was one.
    pub fn up(&mut self, width: usize) -> bool {
        self.step(width, -1)
    }

    /// Down a row as wrapped to `width`; whether there was one.
    pub fn down(&mut self, width: usize) -> bool {
        self.step(width, 1)
    }

    /// Up or down `rows` rows as wrapped to `width`.
    pub fn page(&mut self, width: usize, rows: usize, down: bool) {
        for _ in 0..rows.max(1) {
            let moved = if down {
                self.down(width)
            } else {
                self.up(width)
            };
            if !moved {
                break;
            }
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

    /// The text wrapped to `width` cells, and the cursor in it.
    pub fn wrap(&self, width: usize) -> Wrapped {
        let rows = self.rows(width);
        let at = self.row_of(&rows);
        let row = rows[at];
        Wrapped {
            cursor: (at, self.cells(row.start, self.cursor)),
            rows: rows
                .iter()
                .map(|row| self.chars[row.start..row.end].iter().collect())
                .collect(),
        }
    }

    fn cell(ch: char) -> usize {
        let mut buf = [0; 4];
        Text(ch.encode_utf8(&mut buf)).width()
    }

    fn cells(&self, from: usize, to: usize) -> usize {
        self.chars[from..to].iter().map(|ch| Self::cell(*ch)).sum()
    }

    /// The rows the text wraps to: at a space where one fits, else
    /// mid-word. Never empty.
    fn rows(&self, width: usize) -> Vec<Row> {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut start = 0;
        loop {
            let line_end = self.chars[start..]
                .iter()
                .position(|ch| *ch == '\n')
                .map_or(self.chars.len(), |at| start + at);
            let mut row_start = start;
            loop {
                let mut used = 0;
                let mut end = row_start;
                let mut space = None;
                while end < line_end && used + Self::cell(self.chars[end]) <= width {
                    used += Self::cell(self.chars[end]);
                    if self.chars[end] == ' ' {
                        space = Some(end + 1);
                    }
                    end += 1;
                }
                if end >= line_end {
                    rows.push(Row {
                        start: row_start,
                        end: line_end,
                        last: true,
                    });
                    break;
                }
                // Break after the last space that fits, keeping words whole.
                let end = match space {
                    Some(after) if after > row_start => after,
                    Some(_) | None => end.max(row_start + 1),
                };
                rows.push(Row {
                    start: row_start,
                    end,
                    last: false,
                });
                row_start = end;
            }
            if line_end >= self.chars.len() {
                return rows;
            }
            start = line_end + 1;
        }
    }

    /// The row the cursor is on: at a wrap, the row it starts.
    fn row_of(&self, rows: &[Row]) -> usize {
        rows.iter()
            .position(|row| self.cursor < row.end || (self.cursor == row.end && row.last))
            .unwrap_or(rows.len() - 1)
    }

    fn step(&mut self, width: usize, by: isize) -> bool {
        let rows = self.rows(width);
        let at = self.row_of(&rows);
        let Some(target) = at.checked_add_signed(by).and_then(|to| rows.get(to)) else {
            return false;
        };
        let column = self.cells(rows[at].start, self.cursor);
        // The same column in the target row, or its end; a wrapped row's
        // end is the next row's start, so it stops before it.
        let limit = if target.last || target.end == target.start {
            target.end
        } else {
            target.end - 1
        };
        let mut position = target.start;
        let mut used = 0;
        while position < limit && used + Self::cell(self.chars[position]) <= column {
            used += Self::cell(self.chars[position]);
            position += 1;
        }
        self.cursor = position;
        true
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

    #[test]
    fn long_text_wraps_at_words_and_the_cursor_moves_through_its_rows() {
        let mut composer = Composer::default();
        composer.paste("the upload test is flaky\r\nfix it");
        assert_eq!(composer.counts(), (2, 31));
        let wrapped = composer.wrap(10);
        assert_eq!(
            wrapped.rows,
            ["the ", "upload ", "test is ", "flaky", "fix it"]
        );
        assert_eq!(wrapped.cursor, (4, 6));
        // Up keeps the column, clamped to a shorter row.
        assert!(composer.up(10));
        assert_eq!(composer.wrap(10).cursor, (3, 5));
        assert!(composer.up(10));
        assert_eq!(composer.wrap(10).cursor, (2, 5));
        composer.page(10, 9, false);
        assert_eq!(composer.wrap(10).cursor, (0, 3));
        assert!(!composer.up(10));
        assert!(composer.down(10));
        assert_eq!(composer.wrap(10).cursor, (1, 3));
        // A word longer than the width is cut.
        let mut long = Composer::default();
        long.paste("abcdefghij");
        assert_eq!(long.wrap(4).rows, ["abcd", "efgh", "ij"]);
        // The window keeps the cursor in view.
        let window = composer.wrap(10).window(2);
        assert_eq!(window, 0..2);
        composer.page(10, 9, true);
        assert_eq!(composer.wrap(10).window(2), 3..5);
        // Empty text is one empty row.
        assert_eq!(Composer::default().wrap(5).rows, [""]);
    }
}
