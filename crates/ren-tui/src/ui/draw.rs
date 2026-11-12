//! The one renderer: any [`View`] onto a ratatui buffer, with a resolved
//! theme. Tested by drawing views to a buffer, without a terminal.

use super::{Anchor, Bar, Emphasis, Grid, Ink, Keep, Line, Lines, Place, Rule, Size, Text, View};
use crate::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Clear, Widget};

impl View {
    /// Draws the view into `area` of `buf`.
    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        match self {
            Self::Empty => {}
            Self::Lines(lines) => lines.render(theme, area, buf),
            Self::Bar(bar) => bar.render(theme, area, buf),
            Self::Rule(rule) => rule.render(theme, area, buf),
            Self::Rows(rows) => {
                let heights = Self::split(area.height, rows.iter().map(|(size, _)| *size));
                let mut y = area.y;
                for ((_, view), height) in rows.iter().zip(heights) {
                    view.render(theme, Rect { y, height, ..area }, buf);
                    y += height;
                }
            }
            Self::Columns(columns) => {
                let rules = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(0);
                let widths = Self::split(
                    area.width.saturating_sub(rules),
                    columns.iter().map(|(size, _)| *size),
                );
                let mut x = area.x;
                for (index, ((_, view), width)) in columns.iter().zip(widths).enumerate() {
                    if index > 0 && x < area.x + area.width {
                        for y in area.y..area.y + area.height {
                            buf.set_string(x, y, "│", theme.ink(Ink::Rule));
                        }
                        x += 1;
                    }
                    view.render(theme, Rect { x, width, ..area }, buf);
                    x += width;
                }
            }
            Self::Grid(grid) => grid.render(theme, area, buf),
            Self::Fill(ink, view) => {
                buf.set_style(area, theme.ink(*ink));
                view.render(theme, area, buf);
            }
            Self::Pad { left, right, view } => {
                let inner = Rect {
                    x: area.x + (*left).min(area.width),
                    width: area.width.saturating_sub(left + right),
                    ..area
                };
                view.render(theme, inner, buf);
            }
            Self::Layer { under, over, place } => {
                under.render(theme, area, buf);
                over.render(theme, place.within(area), buf);
            }
            Self::Framed { title, ink, body } => {
                Clear.render(area, buf);
                Block::bordered()
                    .title(title.as_str())
                    .border_style(theme.ink(Ink::Rule))
                    .style(theme.ink(*ink))
                    .render(area, buf);
                let inner = Rect {
                    x: area.x + 2,
                    y: area.y + 1,
                    width: area.width.saturating_sub(4),
                    height: area.height.saturating_sub(2),
                };
                body.render(theme, inner, buf);
            }
        }
    }

    /// `total` split among sizes: exact ones first, in order, then the
    /// rest shared by the fills.
    fn split(total: u16, sizes: impl Iterator<Item = Size> + Clone) -> Vec<u16> {
        let exact: u16 = sizes
            .clone()
            .map(|size| match size {
                Size::Exact(cells) => cells,
                Size::Fill => 0,
            })
            .fold(0, u16::saturating_add);
        let fills = u16::try_from(
            sizes
                .clone()
                .filter(|size| matches!(size, Size::Fill))
                .count(),
        )
        .unwrap_or(1);
        let mut left = total.saturating_sub(exact);
        let mut fill_index = 0;
        let mut budget = total;
        sizes
            .map(|size| {
                let cells = match size {
                    Size::Exact(cells) => cells,
                    Size::Fill => {
                        let share = Grid::start(left, fills - fill_index, 1);
                        fill_index += 1;
                        left -= share;
                        share
                    }
                };
                let cells = cells.min(budget);
                budget -= cells;
                cells
            })
            .collect()
    }

    /// Puts `line` at `x`, `y`, cut at `width` cells.
    fn line(line: &Line, theme: &Theme, x: u16, y: u16, width: u16, buf: &mut Buffer) {
        let end = x + width;
        let mut at = x;
        for span in &line.spans {
            if at >= end {
                break;
            }
            let style = theme.ink(span.ink).add_modifier(match span.emphasis {
                Emphasis::Plain => Modifier::empty(),
                Emphasis::Bold => Modifier::BOLD,
                Emphasis::Italic => Modifier::ITALIC,
                Emphasis::Reversed => Modifier::REVERSED,
            });
            let (next, _) = buf.set_stringn(at, y, &span.text, usize::from(end - at), style);
            at = next;
        }
    }
}

impl Lines {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let rows = usize::from(area.height);
        match self.anchor {
            Anchor::Top => {
                for (row, line) in self.lines.iter().take(rows).enumerate() {
                    let y = area.y + u16::try_from(row).unwrap_or(0);
                    View::line(line, theme, area.x, y, area.width, buf);
                }
            }
            Anchor::Middle => {
                let shown = self.lines.len().min(rows);
                let top = area.y + u16::try_from((rows - shown) / 2).unwrap_or(0);
                for (row, line) in self.lines.iter().take(shown).enumerate() {
                    let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
                    let x = area.x + area.width.saturating_sub(width) / 2;
                    let y = top + u16::try_from(row).unwrap_or(0);
                    View::line(line, theme, x, y, area.width, buf);
                }
            }
            Anchor::Bottom | Anchor::Newest => {
                let scroll = self.scroll.min(self.lines.len().saturating_sub(rows));
                let end = self.lines.len() - scroll;
                let start = end.saturating_sub(rows);
                let top = match self.anchor {
                    Anchor::Bottom => area.y + u16::try_from(rows - (end - start)).unwrap_or(0),
                    Anchor::Top | Anchor::Middle | Anchor::Newest => area.y,
                };
                for (row, line) in self.lines[start..end].iter().enumerate() {
                    let y = top + u16::try_from(row).unwrap_or(0);
                    View::line(line, theme, area.x, y, area.width, buf);
                }
                if scroll > 0 {
                    let note = format!(" ↓ {scroll} more ");
                    let width = u16::try_from(Text(&note).width()).unwrap_or(u16::MAX);
                    if width < area.width {
                        buf.set_string(
                            area.x + area.width - width,
                            area.y + area.height - 1,
                            note,
                            theme.ink(Ink::Bar),
                        );
                    }
                }
            }
        }
    }
}

impl Bar {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let row = Rect { height: 1, ..area };
        buf.set_style(row, theme.ink(self.fill));
        let left = u16::try_from(self.left.width()).unwrap_or(u16::MAX);
        let right = u16::try_from(self.right.width()).unwrap_or(u16::MAX);
        match self.keep {
            Keep::Left => {
                View::line(&self.left, theme, area.x, area.y, area.width, buf);
                if right > 0 && left + 2 + right <= area.width {
                    View::line(
                        &self.right,
                        theme,
                        area.x + area.width - right,
                        area.y,
                        right,
                        buf,
                    );
                }
            }
            Keep::Right => {
                let room = match area.width.checked_sub(right) {
                    Some(room) => {
                        View::line(&self.right, theme, area.x + room, area.y, right, buf);
                        room
                    }
                    None => area.width,
                };
                View::line(&self.left, theme, area.x, area.y, room, buf);
            }
        }
    }
}

impl Rule {
    fn render(self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let (glyph, style) = match self {
            Self::Plain => ("─", theme.ink(Ink::Rule)),
            Self::Edge(ink) => {
                // A theme that leaves the bar unfilled lends the half block
                // the title's text color instead.
                let title = theme.ink(ink);
                let color = match title.bg {
                    Some(color) if color != Color::Reset => color,
                    Some(_) | None => title.fg.unwrap_or(Color::Reset),
                };
                ("▀", theme.ink(Ink::Pane).patch(Style::new().fg(color)))
            }
        };
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, glyph, style);
        }
    }
}

impl Grid {
    fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let columns = self.columns.max(1);
        let rows = self.rows.max(1);
        // One-cell rules between columns and between rows.
        let width = area.width.saturating_sub(columns - 1);
        let height = area.height.saturating_sub(rows - 1);
        let rule = theme.ink(Ink::Rule);
        let rule_row = |row: u16| area.y + Self::start(height, rows, row) + row - 1;
        for row in 1..rows {
            for x in area.x..area.x + area.width {
                buf.set_string(x, rule_row(row), "─", rule);
            }
        }
        for column in 1..columns {
            let x = area.x + Self::start(width, columns, column) + column - 1;
            for y in area.y..area.y + area.height {
                let crossing = (1..rows).any(|row| y == rule_row(row));
                buf.set_string(x, y, if crossing { "┼" } else { "│" }, rule);
            }
        }
        for (index, cell) in self.cells.iter().enumerate() {
            let Ok(index) = u16::try_from(index) else {
                continue;
            };
            let (column, row) = (index % columns, index / columns);
            if row >= rows {
                continue;
            }
            let rect = Rect {
                x: area.x + Self::start(width, columns, column) + column,
                y: area.y + Self::start(height, rows, row) + row,
                width: Self::start(width, columns, column + 1)
                    - Self::start(width, columns, column),
                height: Self::start(height, rows, row + 1) - Self::start(height, rows, row),
            };
            cell.render(theme, rect, buf);
        }
    }
}

impl Place {
    fn within(self, area: Rect) -> Rect {
        match self {
            Self::Center { width, height } => {
                let width = width.min(area.width);
                let height = height.min(area.height);
                Rect {
                    x: area.x + (area.width - width) / 2,
                    y: area.y + (area.height - height) / 2,
                    width,
                    height,
                }
            }
            Self::Bottom {
                inset,
                width,
                height,
            } => {
                let height = height.min(area.height);
                Rect {
                    x: area.x + inset.min(area.width),
                    y: area.y + area.height - height,
                    width: width.min(area.width.saturating_sub(inset)),
                    height,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Anchor, Bar, Ink, Keep, Line, Lines, Size, Span, View};
    use crate::Theme;
    use ratatui::{buffer::Buffer, layout::Rect};

    fn drawn(view: &View, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(&Theme::default(), area, &mut buf);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn rows_give_fills_what_exact_rows_leave() {
        let view = View::Rows(vec![
            (
                Size::Exact(1),
                View::Bar(Bar {
                    left: Line::of("top", Ink::Text),
                    right: Line::of("r", Ink::Muted),
                    fill: Ink::Bar,
                    keep: Keep::Left,
                }),
            ),
            (
                Size::Fill,
                View::Lines(Lines {
                    lines: vec![Line::of("a", Ink::Text), Line::of("b", Ink::Text)],
                    anchor: Anchor::Bottom,
                    scroll: 0,
                }),
            ),
            (
                Size::Exact(1),
                View::Lines(Lines {
                    lines: vec![Line::from(Span::plain("end"))],
                    anchor: Anchor::Top,
                    scroll: 0,
                }),
            ),
        ]);
        assert_eq!(drawn(&view, 8, 5), ["top    r", "", "a", "b", "end"]);
    }

    #[test]
    fn columns_have_a_rule_between() {
        let view = View::Columns(vec![
            (
                Size::Fill,
                View::lines(vec![Line::of("left", Ink::Text)], Anchor::Top),
            ),
            (
                Size::Exact(3),
                View::lines(vec![Line::of("ri", Ink::Text)], Anchor::Top),
            ),
        ]);
        assert_eq!(drawn(&view, 10, 1), ["left  │ri"]);
    }
}
