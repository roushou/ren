//! A [`Wall`] drawn into a ratatui buffer: panes in a grid, split by
//! one-cell rules, each with a title bar, its lines kept to the bottom as
//! they stream, and a footer; then the status line. Drawing reads only the
//! wall, so it is tested on a buffer without a terminal.

use crate::call::Tone;
use crate::keys::Command;
use crate::session::{Line as PaneLine, Phase, ResultMark, SessionPane, Span as Elapsed, ToolLine};
use crate::{Help, Pane, Reading, Status, View, Wall};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};
use tgv_harness::Harness;
use tgv_protocol::activity::ResultStatus;
use tgv_recorder::time::UnixMillis;
use unicode_width::UnicodeWidthChar;

/// Colors the wall uses, in the terminal's 256-color palette so they read
/// the same on dark and light themes' backgrounds.
struct Palette;

impl Palette {
    const RULE: Color = Color::Indexed(238);
    const DIM: Color = Color::Indexed(244);
    const TITLE: Color = Color::Indexed(236);
    const LIVE: Color = Color::Indexed(71);
    const ERROR: Color = Color::Indexed(167);
    const WARNING: Color = Color::Indexed(179);
    const TOOL: Color = Color::Indexed(110);
    const INK: Color = Color::Indexed(16);

    fn tone(tone: Tone) -> Color {
        match tone {
            Tone::Live | Tone::Success => Self::LIVE,
            Tone::Error => Self::ERROR,
            Tone::Warning => Self::WARNING,
            Tone::Neutral => Self::DIM,
        }
    }

    fn harness(harness: Option<Harness>) -> Color {
        match harness {
            Some(Harness::ClaudeCode) => Color::Indexed(173),
            Some(Harness::Codex) => Color::Indexed(110),
            Some(Harness::Pi) => Color::Indexed(73),
            Some(Harness::OpenCode) => Color::Indexed(141),
            None => Self::DIM,
        }
    }
}

impl Widget for &Wall {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let panes = Rect {
            height: area.height - 1,
            ..area
        };
        let status = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        self.draw_panes(panes, buf);
        Wall::draw_status(&self.status, status, buf);
        if self.help == Help::Shown {
            Wall::draw_help(area, buf);
        }
    }
}

impl Wall {
    fn draw_panes(&self, area: Rect, buf: &mut Buffer) {
        let columns = self.grid.columns.max(1);
        let rows = self.grid.rows.max(1);
        let rules = columns - 1;
        let width = area.width.saturating_sub(rules);
        for (index, pane) in self.panes.iter().enumerate() {
            let Ok(index16) = u16::try_from(index) else {
                continue;
            };
            let (column, row) = (index16 % columns, index16 / columns);
            if row >= rows {
                continue;
            }
            // Columns and rows share the leftover cells from the left and top.
            let x = area.x + Self::start(width, columns, column) + column;
            let y = area.y + Self::start(area.height, rows, row);
            let rect = Rect {
                x,
                y,
                width: Self::start(width, columns, column + 1)
                    - Self::start(width, columns, column),
                height: Self::start(area.height, rows, row + 1)
                    - Self::start(area.height, rows, row),
            };
            if column + 1 < columns {
                for line in rect.y..rect.y + rect.height {
                    buf.set_string(
                        rect.x + rect.width,
                        line,
                        "│",
                        Style::new().fg(Palette::RULE),
                    );
                }
            }
            let focused = match self.view {
                View::Panes { focus } => focus == index,
                View::Zoomed => false,
            };
            match pane {
                Pane::Free => Self::draw_free(rect, index, focused, buf),
                Pane::Session(session) => {
                    Self::draw_session(rect, index, session, focused, self.status.now, buf);
                }
            }
        }
    }

    /// Where the `part`-th of `parts` equal shares of `total` starts.
    fn start(total: u16, parts: u16, part: u16) -> u16 {
        let total = u32::from(total);
        u16::try_from(total * u32::from(part) / u32::from(parts.max(1))).unwrap_or(u16::MAX)
    }

    fn draw_free(rect: Rect, index: usize, focused: bool, buf: &mut Buffer) {
        if rect.height == 0 {
            return;
        }
        let dim = Style::new().fg(Palette::DIM);
        let title = Style::new().bg(Palette::TITLE).fg(Palette::DIM);
        buf.set_style(Rect { height: 1, ..rect }, title);
        let marker = if focused { "▶" } else { " " };
        buf.set_stringn(
            rect.x,
            rect.y,
            format!("{marker}{index}: free"),
            usize::from(rect.width),
            title,
        );
        if rect.height > 2 {
            buf.set_stringn(
                rect.x + 1,
                rect.y + rect.height / 2,
                "the next session to start appears here",
                usize::from(rect.width.saturating_sub(1)),
                dim,
            );
        }
    }

    fn draw_session(
        rect: Rect,
        index: usize,
        pane: &SessionPane,
        focused: bool,
        now: UnixMillis,
        buf: &mut Buffer,
    ) {
        if rect.height == 0 {
            return;
        }
        let width = usize::from(rect.width);
        let title_style = Style::new().bg(Self::title_color(pane)).fg(Color::White);
        let title_area = Rect { height: 1, ..rect };
        buf.set_style(title_area, title_style);
        buf.set_line(
            rect.x,
            rect.y,
            &Self::title(index, pane, focused),
            rect.width,
        );
        let facts = Self::facts(pane);
        let facts_width = u16::try_from(Self::width(&facts)).unwrap_or(u16::MAX);
        let title_width =
            u16::try_from(Self::width(&Self::title(index, pane, focused).to_string()))
                .unwrap_or(u16::MAX);
        // Facts sit at the right when they fit beside the title.
        if title_width + 2 + facts_width <= rect.width {
            buf.set_string(
                rect.x + rect.width - facts_width - 1,
                rect.y,
                &facts,
                title_style,
            );
        }
        if rect.height < 2 {
            return;
        }
        let footer_y = rect.y + rect.height - 1;
        buf.set_line(
            rect.x + 1,
            footer_y,
            &Self::footer(pane, now),
            rect.width.saturating_sub(1),
        );
        let body_height = usize::from(rect.height.saturating_sub(2));
        let lines = Self::body(pane, width.saturating_sub(2));
        let skip = lines.len().saturating_sub(body_height);
        let top = rect.y + 1 + u16::try_from(body_height - (lines.len() - skip)).unwrap_or(0);
        for (row, line) in lines.iter().skip(skip).enumerate() {
            let Ok(row) = u16::try_from(row) else {
                break;
            };
            buf.set_line(rect.x + 1, top + row, line, rect.width.saturating_sub(2));
        }
    }

    fn title_color(pane: &SessionPane) -> Color {
        match pane.alerts.first().map(|alert| alert.tone) {
            Some(Tone::Error) => Color::Indexed(52),
            Some(Tone::Warning) => Color::Indexed(58),
            Some(Tone::Live | Tone::Success | Tone::Neutral) | None => match pane.phase {
                Phase::Responding { .. } => Color::Indexed(22),
                Phase::Awaiting { .. }
                | Phase::ClientsTurn { .. }
                | Phase::YourTurn { .. }
                | Phase::Stopped { .. }
                | Phase::Idle { .. } => Palette::TITLE,
            },
        }
    }

    /// `▶0:dadfe1c5 claude-code → anthropic  call 3  pinned`
    fn title(index: usize, pane: &SessionPane, focused: bool) -> Line<'static> {
        let harness = pane.harness.as_ref().and_then(|harness| harness.known());
        let name = pane.harness.as_ref().map_or_else(
            || "unknown client".to_owned(),
            |harness| harness.label().replace('_', "-"),
        );
        let session = pane.session.to_string();
        let mut spans = vec![
            Span::raw(if focused { "▶" } else { " " }).add_modifier(Modifier::BOLD),
            Span::raw(format!("{index}:{} ", session.get(..8).unwrap_or(&session))),
            Span::raw(name)
                .fg(Palette::harness(harness))
                .add_modifier(Modifier::BOLD),
            Span::raw(format!(" → {}", pane.provider.label())),
        ];
        if pane.turn_calls > 0 {
            spans.push(Span::raw(format!("  call {}", pane.turn_calls)));
        }
        if pane.open > 1 {
            spans.push(Span::raw(format!(" · {} open", pane.open)));
        }
        if pane.pinned {
            spans.push(Span::raw("  pinned").add_modifier(Modifier::ITALIC));
        }
        Line::from(spans)
    }

    /// `model-a · ctx 61k · out 1.2k`
    fn facts(pane: &SessionPane) -> String {
        let mut facts = Vec::new();
        if let Some(model) = &pane.model {
            facts.push(model.clone());
        }
        if let Some(context) = pane.context {
            facts.push(format!("ctx {}", Tokens(context)));
        }
        if let Some(output) = pane.output {
            facts.push(format!("out {}", Tokens(output)));
        }
        facts.join(" · ")
    }

    /// Alerts first, then the phase, then the turn's tally.
    fn footer(pane: &SessionPane, now: UnixMillis) -> Line<'static> {
        let mut spans = Vec::new();
        if let Some(alert) = pane.alerts.first() {
            let more = match pane.alerts.len() {
                1 => String::new(),
                count => format!(" (+{})", count - 1),
            };
            spans.push(
                Span::raw(
                    format!("! {} {}{more}", alert.label, alert.detail)
                        .trim_end()
                        .to_owned(),
                )
                .fg(Palette::tone(alert.tone)),
            );
            spans.push(Span::raw(" · ").fg(Palette::DIM));
        }
        let (phase, tone) = Self::phase(&pane.phase, now);
        spans.push(Span::raw(phase).fg(Palette::tone(tone)));
        if let Some(tally) = pane.tally {
            let mut parts = vec![
                format!("read {}", tally.read),
                format!("wrote {}", tally.wrote),
                format!("ran {}", tally.ran),
            ];
            if tally.failed > 0 {
                parts.push(format!("{} failed", tally.failed));
            }
            spans.push(Span::raw(format!(" · {}", parts.join(" · "))).fg(Palette::DIM));
        }
        Line::from(spans)
    }

    fn phase(phase: &Phase, now: UnixMillis) -> (String, Tone) {
        let since = |at: UnixMillis| Elapsed(now.as_millis().saturating_sub(at.as_millis()));
        match phase {
            Phase::Awaiting { since: at } => {
                let waited = now.as_millis().saturating_sub(at.as_millis());
                let slow =
                    waited >= u64::try_from(SessionPane::SLOW_HEAD.as_millis()).unwrap_or(u64::MAX);
                (
                    format!("waiting for response · {}", since(*at)),
                    if slow { Tone::Warning } else { Tone::Live },
                )
            }
            Phase::Responding { since: at } => (format!("responding · {}", since(*at)), Tone::Live),
            Phase::ClientsTurn { since: at } => {
                (format!("client's turn · {}", since(*at)), Tone::Neutral)
            }
            Phase::YourTurn { since: at } => (format!("your turn · {}", since(*at)), Tone::Neutral),
            Phase::Stopped { end, status, .. } => (end.label(*status), end.tone()),
            Phase::Idle { since: Some(at) } => (format!("idle · {}", since(*at)), Tone::Neutral),
            Phase::Idle { since: None } => ("idle".to_owned(), Tone::Neutral),
        }
    }

    /// The pane's lines, wrapped to `width`, oldest first.
    fn body(pane: &SessionPane, width: usize) -> Vec<Line<'static>> {
        let dim = Style::new().fg(Palette::DIM);
        let mut lines = Vec::new();
        for line in &pane.lines {
            match line {
                PaneLine::Call { index } => {
                    lines.push(Line::styled(Self::rule(*index, None, width), dim))
                }
                PaneLine::Reply(text) => {
                    lines.extend(Self::wrap(text, width).into_iter().map(Line::raw));
                }
                PaneLine::Tools(tool) => lines.push(Self::tool(tool)),
            }
        }
        if let Some(streaming) = &pane.streaming {
            lines.push(Line::styled(
                Self::rule(streaming.index, Some("streaming"), width),
                dim,
            ));
            for tool in &streaming.tools {
                lines.push(Line::from(vec![
                    Span::raw("⏺ ").fg(Palette::TOOL),
                    Span::raw(tool.clone())
                        .fg(Palette::TOOL)
                        .add_modifier(Modifier::BOLD),
                    Span::raw(" …").fg(Palette::DIM),
                ]));
            }
            if !streaming.reply.is_empty() {
                let text = if streaming.cut {
                    format!("…{}", streaming.reply)
                } else {
                    streaming.reply.clone()
                };
                lines.extend(Self::wrap(&text, width).into_iter().map(Line::raw));
            }
            if streaming.thinking {
                lines.push(Line::styled(
                    "thinking…",
                    dim.add_modifier(Modifier::ITALIC),
                ));
            }
        }
        lines
    }

    /// `── call 3 ──`, filled with rule to `width`.
    fn rule(index: Option<usize>, note: Option<&str>, width: usize) -> String {
        let mut label = match (index, note) {
            (Some(index), Some(note)) => format!("── call {index} · {note}"),
            (Some(index), None) => format!("── call {index}"),
            (None, Some(note)) => format!("── {note}"),
            (None, None) => "── call".to_owned(),
        };
        label.push(' ');
        let fill = width.saturating_sub(Self::width(&label));
        label + &"─".repeat(fill.min(width))
    }

    /// `⏺ Bash ×2 cargo test  1 failed`
    fn tool(tool: &ToolLine) -> Line<'static> {
        let mut spans = vec![
            Span::raw("⏺ ").fg(Palette::TOOL),
            Span::raw(tool.name.clone())
                .fg(Palette::TOOL)
                .add_modifier(Modifier::BOLD),
        ];
        if tool.count > 1 {
            spans.push(Span::raw(format!(" ×{}", tool.count)).fg(Palette::TOOL));
        }
        if let Some(target) = &tool.target {
            spans.push(Span::raw(format!(" {target}")));
        }
        let (result, tone) = Self::result(&tool.result);
        spans.push(Span::raw(format!("  {result}")).fg(Palette::tone(tone)));
        Line::from(spans)
    }

    fn result(result: &ResultMark) -> (String, Tone) {
        match result {
            ResultMark::Pending => ("…".to_owned(), Tone::Neutral),
            ResultMark::Unreadable => ("result not readable".to_owned(), Tone::Neutral),
            ResultMark::Missing => ("no result".to_owned(), Tone::Warning),
            ResultMark::Reported(status) => match status.known() {
                Some(ResultStatus::Succeeded) => ("ok".to_owned(), Tone::Success),
                Some(ResultStatus::Failed) => ("failed".to_owned(), Tone::Error),
                Some(ResultStatus::Unreported) => ("not reported".to_owned(), Tone::Neutral),
                None => (status.label().replace('_', " "), Tone::Neutral),
            },
            ResultMark::Failures(count) => (format!("{count} failed"), Tone::Error),
            ResultMark::AllSucceeded(count) => (format!("{count} ok"), Tone::Success),
        }
    }

    /// `text` broken into rows of at most `width` cells, at spaces where it
    /// can, keeping its own line breaks.
    fn wrap(text: &str, width: usize) -> Vec<String> {
        let width = width.max(1);
        let mut rows = Vec::new();
        for paragraph in text.lines() {
            let mut row = String::new();
            let mut row_width = 0;
            for word in paragraph.split_inclusive(' ') {
                // A word's trailing space may fall past the row's end.
                let word_width = Self::width(word.trim_end_matches(' '));
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

    fn width(text: &str) -> usize {
        text.chars().map(|ch| ch.width().unwrap_or(0)).sum()
    }

    fn draw_status(status: &Status, area: Rect, buf: &mut Buffer) {
        let base = Style::new().bg(Color::Indexed(234)).fg(Color::Indexed(252));
        buf.set_style(area, base);
        let mut spans = vec![
            Span::raw(" tgv ")
                .bg(Palette::LIVE)
                .fg(Palette::INK)
                .add_modifier(Modifier::BOLD),
        ];
        // A failed read comes first: everything else on the wall is stale.
        if let Some(failure) = &status.failure {
            spans.push(
                Span::raw(format!(" ! stale: {failure} "))
                    .bg(Palette::ERROR)
                    .fg(Palette::INK),
            );
        }
        if status.attention > 0 {
            let needs = if status.attention == 1 {
                "needs"
            } else {
                "need"
            };
            spans.push(
                Span::raw(format!(" ! {} {needs} a look ", status.attention))
                    .bg(Palette::WARNING)
                    .fg(Palette::INK),
            );
        }
        // Sparks share one scale; narrow terminals get half as many glyphs.
        let peak = status
            .output
            .iter()
            .flat_map(|spark| spark.bins.iter().copied())
            .max()
            .unwrap_or(0);
        let pair = area.width < 120;
        for spark in &status.output {
            let bins: Vec<u64> = if pair {
                spark
                    .bins
                    .chunks(2)
                    .map(|chunk| chunk.iter().sum::<u64>() / 2)
                    .collect()
            } else {
                spark.bins.clone()
            };
            let color = Palette::harness(Some(spark.harness));
            spans.push(Span::raw(format!(" {} ", Self::short(spark.harness))).fg(color));
            spans.push(Span::raw(Self::spark(&bins, peak)).fg(color));
        }
        // The clock and the help hint keep their place at the right; the
        // facts give way to them, whole, most telling first.
        let right = format!(" ? help  {} UTC ", Clock(status.now));
        let right_width = u16::try_from(Self::width(&right)).unwrap_or(u16::MAX);
        let left_width = match area.width.checked_sub(right_width) {
            Some(left) => {
                buf.set_string(area.x + left, area.y, right, base.fg(Palette::DIM));
                left
            }
            None => area.width,
        };
        let mut facts = vec![format!(
            "{} session{}",
            status.sessions,
            if status.sessions == 1 { "" } else { "s" }
        )];
        match status.reading {
            Reading::Pending => facts.push("reading…".to_owned()),
            Reading::Complete => {}
            Reading::Capped => facts.push("older calls not read".to_owned()),
        }
        if status.hidden > 0 {
            facts.push(format!("{} without a pane", status.hidden));
        }
        facts.push(format!("{} responding", status.responding));
        facts.push(format!("{} waiting", status.awaiting));
        let mut used: usize = spans.iter().map(|span| Self::width(&span.content)).sum();
        let room = usize::from(left_width).saturating_sub(1);
        let mut text = String::new();
        for fact in facts {
            let piece = if text.is_empty() {
                format!("  {fact}")
            } else {
                format!(" · {fact}")
            };
            if used + Self::width(&piece) > room {
                break;
            }
            used += Self::width(&piece);
            text.push_str(&piece);
        }
        spans.push(Span::raw(text));
        buf.set_line(area.x, area.y, &Line::from(spans), left_width);
    }

    fn short(harness: Harness) -> &'static str {
        match harness {
            Harness::ClaudeCode => "cc",
            Harness::Codex => "codex",
            Harness::Pi => "pi",
            Harness::OpenCode => "oc",
        }
    }

    /// Bins as block glyphs on a shared scale; an empty bin is a space.
    fn spark(bins: &[u64], peak: u64) -> String {
        const GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        bins.iter()
            .map(|value| {
                if *value == 0 || peak == 0 {
                    return ' ';
                }
                let level = (value.saturating_mul(8).div_ceil(peak)).clamp(1, 8);
                GLYPHS[usize::try_from(level - 1).unwrap_or(0).min(7)]
            })
            .collect()
    }

    fn draw_help(area: Rect, buf: &mut Buffer) {
        let rows: Vec<(String, &str)> = Command::ALL
            .iter()
            .map(|command| (command.keys().to_owned(), command.describe()))
            .collect();
        let keys_width = rows
            .iter()
            .map(|(keys, _)| Self::width(keys))
            .max()
            .unwrap_or(0);
        let text_width = rows
            .iter()
            .map(|(_, text)| Self::width(text))
            .max()
            .unwrap_or(0);
        let inner_width = u16::try_from(keys_width + 3 + text_width).unwrap_or(u16::MAX);
        let inner_height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
        let width = (inner_width + 4).min(area.width);
        let height = (inner_height + 2).min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        Clear.render(rect, buf);
        Block::bordered()
            .title(" keys ")
            .border_style(Style::new().fg(Palette::DIM))
            .render(rect, buf);
        for (row, (keys, text)) in rows.iter().enumerate() {
            let Ok(row) = u16::try_from(row) else {
                break;
            };
            if row + 2 >= height {
                break;
            }
            let line = Line::from(vec![
                Span::raw(format!("{keys:<keys_width$}   ")).fg(Palette::TOOL),
                Span::raw(*text),
            ]);
            buf.set_line(rect.x + 2, rect.y + 1 + row, &line, width.saturating_sub(4));
        }
    }
}

/// A token count, short: `950`, `1.2k`, `61k`, `1.3M`.
struct Tokens(u64);

impl std::fmt::Display for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0;
        match n {
            0..1_000 => write!(f, "{n}"),
            1_000..10_000 => write!(f, "{}.{}k", n / 1000, n % 1000 / 100),
            10_000..1_000_000 => write!(f, "{}k", n / 1000),
            _ => write!(f, "{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
        }
    }
}

/// `HH:MM:SS`, in UTC.
struct Clock(UnixMillis);

impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.0.as_millis() / 1000 % 86_400;
        write!(
            f,
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, Tokens, Wall};
    use tgv_recorder::time::UnixMillis;

    #[test]
    fn numbers_and_times_read_short() {
        let tokens: Vec<String> = [950, 1_234, 61_000, 1_300_000]
            .map(|n| Tokens(n).to_string())
            .into();
        assert_eq!(tokens, ["950", "1.2k", "61k", "1.3M"]);
        assert_eq!(
            Clock(UnixMillis::from_millis(3_723_000)).to_string(),
            "01:02:03"
        );
    }

    #[test]
    fn text_wraps_at_spaces_and_breaks_long_words() {
        assert_eq!(Wall::wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(Wall::wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(Wall::wrap("a\n\nb", 4), ["a", "", "b"]);
        assert_eq!(Wall::wrap("日本語の文", 4), ["日本", "語の", "文"]);
    }

    #[test]
    fn sparks_share_one_scale() {
        assert_eq!(Wall::spark(&[0, 1, 4, 8], 8), " ▁▄█");
        assert_eq!(Wall::spark(&[0, 0], 0), "  ");
    }
}
