//! The console as a view: the title line, the conversation anchored to the
//! bottom (beside `/resume`'s list), completions over it, the message box,
//! and the hint line. Pure: the crate's one renderer draws it.

use crate::console::view::Frame;
use crate::ui::{Anchor, Emphasis, Ink, Keep, Line, Lines, Place, Rule, Size, Span, Text, View};

impl Frame {
    /// What to draw.
    pub fn view(&self) -> View {
        if self.width < 4 || self.height < 5 {
            return View::fill(Ink::Ground, View::Empty);
        }
        // The message box: up to six lines, then the hint under it.
        let typed = u16::try_from(self.composer.len().clamp(1, 6)).unwrap_or(1);
        let lines = |lines: &[Vec<Span>]| -> Vec<Line> {
            lines.iter().map(|line| Line::new(line.clone())).collect()
        };
        let conversation = View::Lines(Lines {
            lines: lines(&self.body),
            anchor: Anchor::Newest,
            scroll: self.scroll,
        });
        let body = match &self.side {
            Some(side) => {
                let side_width = (self.width / 3).clamp(24, 48).min(self.width / 2);
                View::Columns(vec![
                    (Size::Fill, conversation),
                    (
                        Size::Exact(side_width),
                        View::pad(1, 0, View::lines(lines(side), Anchor::Top)),
                    ),
                ])
            }
            None => conversation,
        };
        View::fill(
            Ink::Ground,
            View::Rows(vec![
                (
                    Size::Exact(1),
                    View::bar(
                        Line::new(self.title.clone()),
                        Line::new(self.strip.clone()),
                        Ink::Ground,
                        Keep::Left,
                    ),
                ),
                (Size::Exact(1), View::Rule(Rule::Plain)),
                (Size::Fill, self.popup(body)),
                (Size::Exact(1), View::Rule(Rule::Plain)),
                (Size::Exact(typed), self.composer(typed)),
                (
                    Size::Exact(1),
                    View::bar(
                        Line::new(self.hint.clone()),
                        Line::new(self.hint_right.clone()),
                        Ink::Ground,
                        Keep::Left,
                    ),
                ),
            ]),
        )
    }

    /// Completions, just above the message box.
    fn popup(&self, body: View) -> View {
        if self.popup.is_empty() {
            return body;
        }
        let shown = self.popup.len().min(8);
        let label_width = self
            .popup
            .iter()
            .map(|(suggestion, _)| Text(&suggestion.label).width())
            .max()
            .unwrap_or(0);
        let chosen = self
            .popup
            .iter()
            .position(|(_, chosen)| *chosen)
            .unwrap_or(0);
        let first = chosen.saturating_sub(shown - 1);
        let rows = self
            .popup
            .iter()
            .skip(first)
            .take(shown)
            .map(|(suggestion, chosen)| {
                let (mark, ink) = if *chosen {
                    ("› ", Ink::Focus)
                } else {
                    ("  ", Ink::Text)
                };
                Line::new(vec![
                    Span::new(format!(" {mark}{:<label_width$}  ", suggestion.label), ink),
                    Span::new(suggestion.detail.clone(), Ink::Muted),
                ])
            })
            .collect();
        View::layer(
            body,
            View::fill(Ink::Bar, View::lines(rows, Anchor::Top)),
            Place::Bottom {
                inset: 2,
                width: self.width.saturating_sub(4).min(80),
                height: u16::try_from(shown).unwrap_or(0),
            },
        )
    }

    /// The message box, its cursor's line kept in view.
    fn composer(&self, rows: u16) -> View {
        let prompt = Span::new(" › ", Ink::Prompt);
        let blank = Span::plain("   ");
        if let Some(placeholder) = &self.placeholder {
            let mut chars = placeholder.chars();
            let first = chars
                .next()
                .map(String::from)
                .unwrap_or_else(|| " ".to_owned());
            return View::lines(
                vec![Line::new(vec![
                    prompt,
                    Span::new(first, Ink::Text).with(Emphasis::Reversed),
                    Span::new(chars.as_str(), Ink::Muted),
                ])],
                Anchor::Top,
            );
        }
        let first = self.cursor.0.saturating_sub(usize::from(rows) - 1);
        let lines = self
            .composer
            .iter()
            .enumerate()
            .skip(first)
            .take(usize::from(rows))
            .map(|(row, text)| {
                let lead = if row == 0 {
                    prompt.clone()
                } else {
                    blank.clone()
                };
                if row != self.cursor.0 {
                    return Line::new(vec![lead, Span::new(text.clone(), Ink::Text)]);
                }
                // The cell under the cursor, reversed.
                let mut before = String::new();
                let mut used = 0;
                let mut chars = text.chars();
                for ch in chars.by_ref() {
                    if used >= self.cursor.1 {
                        let under = ch.to_string();
                        return Line::new(vec![
                            lead,
                            Span::new(before, Ink::Text),
                            Span::new(under, Ink::Text).with(Emphasis::Reversed),
                            Span::new(chars.as_str(), Ink::Text),
                        ]);
                    }
                    used += Text(&ch.to_string()).width();
                    before.push(ch);
                }
                Line::new(vec![
                    lead,
                    Span::new(before, Ink::Text),
                    Span::new(" ", Ink::Text).with(Emphasis::Reversed),
                ])
            })
            .collect();
        View::lines(lines, Anchor::Top)
    }
}
