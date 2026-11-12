//! Puts a [`Frame`] on a ratatui buffer: the title line, the conversation
//! anchored to the bottom, completions over it, the message box, and the
//! hint line.

use crate::console::view::{Frame, Line};
use crate::{Drawn, Ink, Theme};
use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};

impl Drawn for Frame {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 5 {
            return;
        }
        buf.set_style(area, theme.ink(Ink::Ground));
        let bottom = area.y + area.height;
        // The message box: up to six lines, then the hint under it.
        let typed = u16::try_from(self.composer.len().clamp(1, 6)).unwrap_or(1);
        let hint_row = bottom - 1;
        let composer_top = hint_row.saturating_sub(typed);
        let lower_rule = composer_top.saturating_sub(1);
        let body = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: lower_rule.saturating_sub(area.y + 2),
        };

        self.line(&self.title, area.x, area.y, area.width, theme, buf);
        let strip_width = u16::try_from(self.columns_of(&self.strip)).unwrap_or(0);
        if strip_width < area.width / 2 {
            self.line(
                &self.strip,
                area.x + area.width - strip_width,
                area.y,
                strip_width,
                theme,
                buf,
            );
        }
        self.rule(area.x, area.y + 1, area.width, theme, buf);

        match &self.side {
            Some(side) => {
                let side_width = (area.width / 3).clamp(24, 48).min(area.width / 2);
                let preview = Rect {
                    width: body.width.saturating_sub(side_width + 1),
                    ..body
                };
                self.body(&self.body, self.scroll, preview, theme, buf);
                let x = preview.x + preview.width;
                for y in body.y..body.y + body.height {
                    buf.set_string(x, y, "│", theme.ink(Ink::Rule));
                }
                for (row, line) in side.iter().enumerate().take(usize::from(body.height)) {
                    let y = body.y + u16::try_from(row).unwrap_or(0);
                    self.line(line, x + 2, y, side_width.saturating_sub(2), theme, buf);
                }
            }
            None => self.body(&self.body, self.scroll, body, theme, buf),
        }

        self.popup(body, theme, buf);
        self.rule(area.x, lower_rule, area.width, theme, buf);
        self.composer(area.x, composer_top, area.width, typed, theme, buf);
        self.line(&self.hint, area.x, hint_row, area.width, theme, buf);
        let right = u16::try_from(self.columns_of(&self.hint_right)).unwrap_or(0);
        let left = u16::try_from(self.columns_of(&self.hint)).unwrap_or(0);
        if left + right + 2 <= area.width {
            self.line(
                &self.hint_right,
                area.x + area.width - right,
                hint_row,
                right,
                theme,
                buf,
            );
        }
    }
}

impl Frame {
    fn columns_of(&self, line: &Line) -> usize {
        line.iter().map(|span| Self::columns(&span.text)).sum()
    }

    fn line(&self, line: &Line, x: u16, y: u16, width: u16, theme: &Theme, buf: &mut Buffer) {
        let mut at = x;
        let end = x + width;
        for span in line {
            if at >= end {
                break;
            }
            let (next, _) = buf.set_stringn(
                at,
                y,
                &span.text,
                usize::from(end - at),
                theme.ink(span.ink),
            );
            at = next;
        }
    }

    fn rule(&self, x: u16, y: u16, width: u16, theme: &Theme, buf: &mut Buffer) {
        for column in x..x + width {
            buf.set_string(column, y, "─", theme.ink(Ink::Rule));
        }
    }

    /// The newest lines that fit, `scroll` lines up from the end.
    fn body(&self, lines: &[Line], scroll: usize, area: Rect, theme: &Theme, buf: &mut Buffer) {
        let rows = usize::from(area.height);
        let scroll = scroll.min(lines.len().saturating_sub(rows));
        let end = lines.len() - scroll;
        let start = end.saturating_sub(rows);
        for (row, line) in lines[start..end].iter().enumerate() {
            let y = area.y + u16::try_from(row).unwrap_or(0);
            self.line(line, area.x, y, area.width, theme, buf);
        }
        if scroll > 0 {
            let note = format!(" ↓ {scroll} more ");
            let width = u16::try_from(Self::columns(&note)).unwrap_or(0);
            if width < area.width {
                buf.set_string(
                    area.x + area.width - width,
                    area.y + area.height.saturating_sub(1),
                    note,
                    theme.ink(Ink::Bar),
                );
            }
        }
    }

    /// Completions, just above the message box.
    fn popup(&self, body: Rect, theme: &Theme, buf: &mut Buffer) {
        if self.popup.is_empty() {
            return;
        }
        let shown = self.popup.len().min(8).min(usize::from(body.height));
        let label_width = self
            .popup
            .iter()
            .map(|(suggestion, _)| Self::columns(&suggestion.label))
            .max()
            .unwrap_or(0);
        let width = body.width.saturating_sub(4).min(80);
        let top = body.y + body.height - u16::try_from(shown).unwrap_or(0);
        let chosen = self
            .popup
            .iter()
            .position(|(_, chosen)| *chosen)
            .unwrap_or(0);
        let first = chosen.saturating_sub(shown - 1);
        for (row, (suggestion, chosen)) in self.popup.iter().skip(first).take(shown).enumerate() {
            let y = top + u16::try_from(row).unwrap_or(0);
            let base = theme.ink(Ink::Bar);
            for x in body.x + 2..body.x + 2 + width {
                buf.set_string(x, y, " ", base);
            }
            let (mark, ink) = if *chosen {
                ("› ", Ink::Focus)
            } else {
                ("  ", Ink::Text)
            };
            let line = vec![
                crate::console::Styled {
                    text: format!(" {mark}{:<label_width$}  ", suggestion.label),
                    ink,
                },
                crate::console::Styled {
                    text: suggestion.detail.clone(),
                    ink: Ink::Muted,
                },
            ];
            self.line(&line, body.x + 2, y, width, theme, buf);
        }
    }

    fn composer(&self, x: u16, y: u16, width: u16, rows: u16, theme: &Theme, buf: &mut Buffer) {
        let prompt = theme.ink(Ink::Prompt);
        buf.set_string(x, y, " › ", prompt);
        if let Some(placeholder) = &self.placeholder {
            buf.set_stringn(
                x + 3,
                y,
                placeholder,
                usize::from(width.saturating_sub(3)),
                theme.ink(Ink::Muted),
            );
            buf.set_style(
                Rect {
                    x: x + 3,
                    y,
                    width: 1,
                    height: 1,
                },
                theme.ink(Ink::Text).add_modifier(Modifier::REVERSED),
            );
            return;
        }
        // The cursor's line stays in view.
        let first = self.cursor.0.saturating_sub(usize::from(rows) - 1);
        for (row, text) in self
            .composer
            .iter()
            .skip(first)
            .take(usize::from(rows))
            .enumerate()
        {
            let line_y = y + u16::try_from(row).unwrap_or(0);
            buf.set_stringn(
                x + 3,
                line_y,
                text,
                usize::from(width.saturating_sub(4)),
                theme.ink(Ink::Text),
            );
        }
        let cursor_y = y + u16::try_from(self.cursor.0 - first).unwrap_or(0);
        let cursor_x = x + 3 + u16::try_from(self.cursor.1).unwrap_or(0);
        if cursor_x < x + width {
            buf.set_style(
                Rect {
                    x: cursor_x,
                    y: cursor_y,
                    width: 1,
                    height: 1,
                },
                theme.ink(Ink::Text).add_modifier(Modifier::REVERSED),
            );
        }
    }
}
