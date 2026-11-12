//! Text as a terminal lays it out: measured in cells, cut to fit, and
//! wrapped at spaces. The one place either is done.

use unicode_width::UnicodeWidthChar;

/// Text measured in terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text<'a>(pub &'a str);

impl Text<'_> {
    /// How many cells it takes.
    pub fn width(self) -> usize {
        self.0.chars().map(|ch| ch.width().unwrap_or(0)).sum()
    }

    /// Cut to `width` cells, the last of them an ellipsis when cut.
    pub fn fit(self, width: usize) -> String {
        if self.width() <= width {
            return self.0.to_owned();
        }
        let mut out = String::new();
        let mut used = 0;
        for ch in self.0.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if used + ch_width + 1 > width {
                break;
            }
            out.push(ch);
            used += ch_width;
        }
        if width > 0 {
            out.push('…');
        }
        out
    }

    /// Trimmed, then cut to `width` cells with an ellipsis: a one-line
    /// summary.
    pub fn summary(self, width: usize) -> String {
        Text(self.0.trim()).fit(width)
    }

    /// Rows of at most `width` cells, broken at spaces where they can be,
    /// keeping the text's own line breaks; a word wider than a row is cut.
    pub fn wrap(self, width: usize) -> Vec<String> {
        let width = width.max(1);
        let mut rows = Vec::new();
        for paragraph in self.0.split('\n') {
            let mut row = String::new();
            let mut row_width = 0;
            for word in paragraph.split_inclusive(' ') {
                // A word's trailing space may fall past the row's end.
                let word_width = Text(word.trim_end_matches(' ')).width();
                if row_width + word_width.min(width) > width && !row.is_empty() {
                    rows.push(row.trim_end().to_owned());
                    row = String::new();
                    row_width = 0;
                }
                for ch in word.chars() {
                    let ch_width = ch.width().unwrap_or(0);
                    // A space at the row's end is the break itself.
                    if ch == ' ' && row_width + ch_width > width {
                        continue;
                    }
                    if row_width + ch_width > width {
                        rows.push(std::mem::take(&mut row));
                        row_width = 0;
                    }
                    row.push(ch);
                    row_width += ch_width;
                }
            }
            rows.push(row.trim_end().to_owned());
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::Text;

    #[test]
    fn text_wraps_at_spaces_and_breaks_long_words() {
        assert_eq!(Text("one two three").wrap(7), ["one two", "three"]);
        assert_eq!(Text("abcdefghij").wrap(4), ["abcd", "efgh", "ij"]);
        assert_eq!(Text("a\n\nb").wrap(4), ["a", "", "b"]);
        assert_eq!(Text("日本語の文").wrap(4), ["日本", "語の", "文"]);
    }

    #[test]
    fn text_is_cut_with_an_ellipsis() {
        assert_eq!(Text("abcdef").fit(4), "abc…");
        assert_eq!(Text("abc").fit(4), "abc");
        assert_eq!(Text("日本語").fit(4), "日…");
        assert_eq!(Text("  padded  ").summary(10), "padded");
        assert_eq!(Text("abcdef").width(), 6);
    }
}
