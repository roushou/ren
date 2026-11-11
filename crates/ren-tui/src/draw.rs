//! A wall rendered with resolved semantic styles. Drawing reads only the
//! wall and its theme, and can be tested without a terminal.

use crate::call::Tone;
use crate::keys::Command;
use crate::runs::{EntryLine, RunPhase};
use crate::session::{
    CallFacts, Line as PaneLine, Phase, ResultMark, SessionPane, Span as Elapsed, ToolLine,
};
use crate::{Help, Pane, Reading, Status, Theme, View, Wall};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Widget};
use ren_harness::Harness;
use ren_harness::drive::ApprovalAsk;
use ren_protocol::activity::ResultStatus;
use ren_recorder::time::UnixMillis;
use unicode_width::UnicodeWidthChar;

/// A wall and its resolved rendering styles.
#[must_use]
pub struct ThemedWall<'a> {
    wall: &'a Wall,
    theme: &'a Theme,
}

impl Wall {
    pub fn themed<'a>(&'a self, theme: &'a Theme) -> ThemedWall<'a> {
        ThemedWall { wall: self, theme }
    }
}

impl Widget for &Wall {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.themed(&Theme::default()).render(area, buf);
    }
}

impl Widget for ThemedWall<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        buf.set_style(area, self.theme.ground);
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
        match &self.wall.entry {
            Some(entry) => self.draw_entry(entry, status, buf),
            None => self.draw_status(&self.wall.status, status, buf),
        }
        if self.wall.help == Help::Shown {
            self.draw_help(area, buf);
        }
    }
}

impl ThemedWall<'_> {
    fn draw_panes(&self, area: Rect, buf: &mut Buffer) {
        let columns = self.wall.grid.columns.max(1);
        let rows = self.wall.grid.rows.max(1);
        // One-cell rules between columns and between rows.
        let width = area.width.saturating_sub(columns - 1);
        let height = area.height.saturating_sub(rows - 1);
        let rule = self.theme.rule;
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
        let all_free = self.wall.panes.iter().all(|pane| *pane == Pane::Free);
        for (index, pane) in self.wall.panes.iter().enumerate() {
            let Ok(index16) = u16::try_from(index) else {
                continue;
            };
            let (column, row) = (index16 % columns, index16 / columns);
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
            let (focused, place) = match self.wall.view {
                View::Panes { focus } => (focus == index, Some(index)),
                View::Zoomed => (false, None),
            };
            match pane {
                Pane::Free if all_free => self.draw_empty(rect, buf),
                Pane::Free => {}
                Pane::Session(session) => {
                    self.draw_session(rect, place, session, focused, self.wall.status.now, buf);
                }
            }
        }
    }

    /// Where the `part`-th of `parts` equal shares of `total` starts.
    fn start(total: u16, parts: u16, part: u16) -> u16 {
        let total = u32::from(total);
        u16::try_from(total * u32::from(part) / u32::from(parts.max(1))).unwrap_or(u16::MAX)
    }

    /// No session in the window at all.
    fn draw_empty(&self, rect: Rect, buf: &mut Buffer) {
        let text = "no live sessions · a session appears here when its client calls";
        let width = u16::try_from(Self::width(text)).unwrap_or(u16::MAX);
        let x = rect.x + rect.width.saturating_sub(width) / 2;
        buf.set_stringn(
            x,
            rect.y + rect.height / 2,
            text,
            usize::from(rect.width),
            self.theme.muted,
        );
    }

    fn draw_session(
        &self,
        rect: Rect,
        place: Option<usize>,
        pane: &SessionPane,
        focused: bool,
        now: UnixMillis,
        buf: &mut Buffer,
    ) {
        if rect.height == 0 || rect.width == 0 {
            return;
        }
        buf.set_style(rect, self.theme.pane);
        self.draw_title(rect, place, pane, focused, buf);
        let inner = rect.width.saturating_sub(2);
        // A state line and a facts line at the foot, when there is room.
        let foot = match rect.height {
            0..=3 => 0,
            4..=5 => 1,
            _ => 2,
        };
        if foot >= 1 {
            let y = rect.y + rect.height - foot;
            buf.set_line(rect.x + 1, y, &self.state_line(pane, now), inner);
        }
        if foot >= 2
            && let Some(facts) = &pane.facts
        {
            let text = Self::fit(&Self::facts(facts), usize::from(inner));
            buf.set_string(rect.x + 1, rect.y + rect.height - 1, text, self.theme.muted);
        }
        // A rule under the title, as the mockup's border: in the rule's
        // style, or a half block in a lit bar's own color, so the bar and
        // its edge read as one. Left out when the pane cannot spare a line.
        let head = if rect.height >= 7 {
            let (edge, style) = match self.lit(pane) {
                // A theme that leaves the bar unfilled lends the half block
                // the title's text color instead.
                Some(title) => match title.bg {
                    Some(color) if color != Color::Reset => ("▀", Style::new().fg(color)),
                    Some(_) | None => ("▀", Style::new().fg(title.fg.unwrap_or(Color::Reset))),
                },
                None => ("─", self.theme.rule),
            };
            let style = self.theme.pane.patch(style);
            for x in rect.x..rect.x + rect.width {
                buf.set_string(x, rect.y + 1, edge, style);
            }
            2
        } else {
            1
        };
        let body_top = rect.y + head;
        let body_height = usize::from(rect.height.saturating_sub(head + foot));
        let lines = self.body(pane, usize::from(inner));
        let skip = lines.len().saturating_sub(body_height);
        let top = body_top + u16::try_from(body_height - (lines.len() - skip)).unwrap_or(0);
        for (row, line) in lines.iter().skip(skip).enumerate() {
            let Ok(row) = u16::try_from(row) else {
                break;
            };
            buf.set_line(rect.x + 1, top + row, line, inner);
        }
    }

    /// The title bar: green while a response streams, amber or red with
    /// an alert; the place, session, client and provider, the turn's call
    /// count, and at the right the context and the session's output.
    fn draw_title(
        &self,
        rect: Rect,
        place: Option<usize>,
        pane: &SessionPane,
        focused: bool,
        buf: &mut Buffer,
    ) {
        let lit = self.lit(pane);
        let base = lit.unwrap_or(self.theme.pane.patch(self.theme.muted));
        buf.set_style(Rect { height: 1, ..rect }, base);
        let harness = pane.harness.as_ref().and_then(|harness| harness.known());
        let name = pane.harness.as_ref().map_or_else(
            || "unknown".to_owned(),
            |harness| harness.label().replace('_', "-"),
        );
        let session = pane.session.to_string();
        let short = session.get(..8).unwrap_or(&session);
        let mut spans = vec![Span::raw(" ")];
        match place {
            Some(place) if focused => {
                spans.push(Span::raw(format!("{place}")).style(self.theme.focus))
            }
            Some(place) => spans.push(Span::raw(format!("{place}"))),
            None => {}
        }
        spans.push(Span::raw(match place {
            Some(_) => format!(":{short}  "),
            None => format!("{short}  "),
        }));
        let route_style = match lit {
            Some(_) => Style::new(),
            None => self.theme.harness(harness),
        };
        spans.push(Span::raw(name).style(route_style));
        spans.push(Span::raw(format!(" → {}", pane.provider.label())).style(route_style));
        if pane.turn_calls > 0 {
            spans.push(Span::raw(format!("  call {}", pane.turn_calls)));
        }
        if pane.open > 1 {
            spans.push(Span::raw(format!(" · {} open", pane.open)));
        }
        if pane.pinned {
            spans.push(Span::raw("  pinned"));
        }
        if let Some(run) = &pane.run {
            spans.push(Span::raw(match run.phase {
                RunPhase::Idle => "  run · idle",
                RunPhase::Working => "  run · working",
                RunPhase::Waiting { .. } => "  run · asks",
            }));
        }
        let title = Line::from(spans);
        buf.set_line(rect.x, rect.y, &title, rect.width);
        // Only what is known: context, then the session's output.
        let mut facts = Vec::new();
        if let Some(context) = pane.context {
            facts.push(format!("ctx {}", Kilo(context)));
        }
        if let Some(output) = pane.session_output {
            facts.push(format!("{} tok", Grouped(output)));
        }
        let right = if facts.is_empty() {
            String::new()
        } else {
            format!("{} ", facts.join(" · "))
        };
        let right_width = u16::try_from(Self::width(&right)).unwrap_or(u16::MAX);
        let title_width = u16::try_from(title.width()).unwrap_or(u16::MAX);
        if title_width + 2 + right_width <= rect.width {
            buf.set_string(rect.x + rect.width - right_width, rect.y, right, base);
        }
    }

    /// A lit title bar's style: responding while a response streams,
    /// warning or error when the newest call needs a look; `None` when it
    /// is plain.
    fn lit(&self, pane: &SessionPane) -> Option<Style> {
        if let Some(RunPhase::Waiting { .. }) = pane.run.as_ref().map(|run| &run.phase) {
            return Some(self.theme.warning_title);
        }
        match pane.alerts.first().map(|alert| alert.tone) {
            Some(Tone::Error) => Some(self.theme.error_title),
            Some(Tone::Warning) => Some(self.theme.warning_title),
            Some(Tone::Live | Tone::Success | Tone::Neutral) | None => match pane.phase {
                Phase::Responding { .. } => Some(self.theme.responding_title),
                Phase::Awaiting { .. }
                | Phase::ClientsTurn { .. }
                | Phase::YourTurn { .. }
                | Phase::Stopped { .. }
                | Phase::Idle { .. } => None,
            },
        }
    }

    /// `! alert · state · detail`, as the mockup words it.
    fn state_line(&self, pane: &SessionPane, now: UnixMillis) -> Line<'static> {
        // A run waiting for the operator says what it asks, and how to answer.
        if let Some(RunPhase::Waiting { ask, more }) = pane.run.as_ref().map(|run| &run.phase) {
            let (what, reason) = match ask {
                ApprovalAsk::Command {
                    command,
                    cwd,
                    reason,
                } => (
                    match (command, cwd) {
                        (Some(command), Some(cwd)) => format!("run `{command}` in {cwd}"),
                        (Some(command), None) => format!("run `{command}`"),
                        (None, Some(_) | None) => "run a command".to_owned(),
                    },
                    reason,
                ),
                ApprovalAsk::FileChange { reason } => ("change files".to_owned(), reason),
                ApprovalAsk::Tool { tool, detail } => (format!("use {tool}"), detail),
            };
            let mut spans =
                vec![Span::raw(format!("? {what}")).style(self.theme.tone(Tone::Warning))];
            if let Some(reason) = reason {
                spans.push(Span::raw(format!(" · {reason}")).style(self.theme.muted));
            }
            if *more > 0 {
                spans.push(Span::raw(format!(" (+{more})")).style(self.theme.muted));
            }
            spans.push(Span::raw("  a yes · A always · d no").style(self.theme.muted));
            return Line::from(spans);
        }
        let mut spans = Vec::new();
        if let Some(alert) = pane.alerts.first() {
            let more = match pane.alerts.len() {
                1 => String::new(),
                count => format!(" (+{})", count - 1),
            };
            let text = match alert.detail.as_str() {
                "" => format!("! {}{more}", alert.label),
                detail => format!("! {} · {detail}{more}", alert.label),
            };
            spans.push(Span::raw(text).style(self.theme.tone(alert.tone)));
            spans.push(Span::raw(" · ").style(self.theme.muted));
        }
        let since = |at: UnixMillis| Elapsed(now.as_millis().saturating_sub(at.as_millis()));
        let (label, tone, detail) = match &pane.phase {
            Phase::Awaiting { since: at } => {
                let waited = now.as_millis().saturating_sub(at.as_millis());
                let slow =
                    waited >= u64::try_from(SessionPane::SLOW_HEAD.as_millis()).unwrap_or(u64::MAX);
                (
                    "waiting for first byte".to_owned(),
                    if slow { Tone::Warning } else { Tone::Live },
                    since(*at).to_string(),
                )
            }
            Phase::Responding { since: at } => {
                ("streaming".to_owned(), Tone::Live, since(*at).to_string())
            }
            Phase::ClientsTurn { since: at } => (
                "complete".to_owned(),
                Tone::Success,
                format!("client running tools · {}", since(*at)),
            ),
            Phase::YourTurn { since: at } => (
                "complete".to_owned(),
                Tone::Success,
                format!("waiting for you · {}", since(*at)),
            ),
            Phase::Stopped { end, status, .. } => (end.label(*status), end.tone(), String::new()),
            Phase::Idle { since: Some(at) } => {
                ("idle".to_owned(), Tone::Neutral, since(*at).to_string())
            }
            Phase::Idle { since: None } => ("idle".to_owned(), Tone::Neutral, String::new()),
        };
        // A stopped call's alert already says how it ended.
        if matches!(pane.phase, Phase::Stopped { .. }) && !pane.alerts.is_empty() {
            spans.pop();
            return Line::from(spans);
        }
        spans.push(Span::raw(label).style(self.theme.tone(tone)));
        if !detail.is_empty() {
            spans.push(Span::raw(format!(" · {detail}")).style(self.theme.muted));
        }
        Line::from(spans)
    }

    /// `model · in 36.0k (93% cached) · out 122 · reasoning 63 · end_turn`
    fn facts(facts: &CallFacts) -> String {
        let mut parts = Vec::new();
        if let Some(model) = &facts.model {
            parts.push(model.clone());
        }
        parts.push(match (facts.input, facts.cached) {
            (Some(input), Some(cached)) => format!("in {} ({cached}% cached)", Kilo(input)),
            (Some(input), None) => format!("in {}", Kilo(input)),
            (None, Some(_) | None) => "in —".to_owned(),
        });
        if let Some(output) = facts.output {
            parts.push(format!("out {}", Grouped(output)));
        }
        if let Some(reasoning) = facts.reasoning.filter(|tokens| *tokens > 0) {
            parts.push(format!("reasoning {}", Grouped(reasoning)));
        }
        if let Some(stop) = &facts.stop_reason {
            parts.push(stop.clone());
        }
        parts.join(" · ")
    }

    /// `text`, cut to `width` cells with an ellipsis.
    fn fit(text: &str, width: usize) -> String {
        if Self::width(text) <= width {
            return text.to_owned();
        }
        let mut out = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if used + ch_width + 1 > width {
                break;
            }
            out.push(ch);
            used += ch_width;
        }
        out.push('…');
        out
    }

    /// The pane's turn as a conversation, wrapped to `width`, oldest first.
    fn body(&self, pane: &SessionPane, width: usize) -> Vec<Line<'static>> {
        let dim = self.theme.muted;
        let mut lines = Vec::new();
        for line in &pane.lines {
            match line {
                PaneLine::Call { index } => lines.push(Line::styled(
                    match index {
                        Some(index) => format!("── call {index} ──"),
                        None => "── call ──".to_owned(),
                    },
                    dim,
                )),
                PaneLine::Prompt(text) => {
                    lines.extend(Self::hanging("> ", text, width, self.theme.prompt));
                }
                PaneLine::Result(tool) => lines.push(self.result(tool)),
                PaneLine::Reply(text) => lines.extend(self.markdown(text, width)),
                PaneLine::Tools(tool) => lines.push(self.tool(tool)),
                PaneLine::Waiting => {
                    lines.push(Line::styled("request sent · waiting for the provider", dim));
                }
            }
        }
        if let Some(streaming) = &pane.streaming {
            for tool in &streaming.tools {
                lines.push(Line::from(vec![
                    Span::raw("● ").style(self.theme.tool),
                    Span::raw(tool.clone())
                        .style(self.theme.tool)
                        .add_modifier(Modifier::BOLD),
                ]));
            }
            if streaming.thinking {
                lines.push(Line::styled(
                    "thinking…",
                    dim.add_modifier(Modifier::ITALIC),
                ));
            } else {
                let text = if streaming.cut {
                    format!("…{}", streaming.reply)
                } else {
                    streaming.reply.clone()
                };
                let mut reply = self.markdown(&text, width);
                // The cursor sits where the stream has got to.
                match reply.last_mut() {
                    Some(last) if last.width() < width => {
                        last.push_span(Span::raw("▌").style(self.theme.text));
                    }
                    Some(_) | None => reply.push(Line::raw("▌").style(self.theme.text)),
                }
                lines.extend(reply);
            }
        }
        lines
    }

    /// `prefix` then `text`, wrapped, later rows indented under the text.
    fn hanging(prefix: &str, text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
        let indent = Self::width(prefix);
        Self::wrap(text, width.saturating_sub(indent))
            .into_iter()
            .enumerate()
            .map(|(row, part)| {
                let lead = if row == 0 {
                    prefix.to_owned()
                } else {
                    " ".repeat(indent)
                };
                Line::styled(format!("{lead}{part}"), style)
            })
            .collect()
    }

    /// A reply's text with its markdown read lightly: fenced code drawn as
    /// a block behind a rule, headings and whole `**bold**` lines in bold,
    /// and emphasis markers left out.
    fn markdown(&self, text: &str, width: usize) -> Vec<Line<'static>> {
        let code = self.theme.code;
        let rule = self.theme.rule;
        let mut lines = Vec::new();
        let mut in_code = false;
        for source in text.lines() {
            if source.trim_start().starts_with("```") {
                in_code = !in_code;
                continue;
            }
            if in_code {
                for part in Self::wrap(source, width.saturating_sub(2)) {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", rule),
                        Span::styled(part, code),
                    ]));
                }
                continue;
            }
            let trimmed = source.trim_start();
            let heading = trimmed.starts_with('#');
            let shown = if heading {
                trimmed.trim_start_matches('#').trim_start()
            } else {
                source
            };
            let bold = heading || (trimmed.starts_with("**") && trimmed.contains(":**"));
            let plain = shown.replace("**", "").replace("__", "");
            let style = if bold {
                self.theme.text.add_modifier(Modifier::BOLD)
            } else {
                self.theme.text
            };
            for part in Self::wrap(&plain, width) {
                lines.push(Line::styled(part, style));
            }
        }
        lines
    }

    /// `● Bash cargo test ×2`
    fn tool(&self, tool: &ToolLine) -> Line<'static> {
        let mut spans = vec![
            Span::raw("● ").style(self.theme.tool),
            Span::raw(tool.name.clone())
                .style(self.theme.tool)
                .add_modifier(Modifier::BOLD),
        ];
        if let Some(target) = &tool.target {
            spans.push(Span::raw(format!(" {target}")).style(self.theme.tool));
        }
        if tool.count > 1 {
            spans.push(Span::raw(format!(" ×{}", tool.count)).style(self.theme.tool));
        }
        Line::from(spans)
    }

    /// `⎿ Read src/lib.rs → fn main() {}`: what the client sent back.
    fn result(&self, tool: &ToolLine) -> Line<'static> {
        let dim = self.theme.muted;
        let mut head = format!("⎿ {}", tool.name);
        if let Some(target) = &tool.target {
            head.push(' ');
            head.push_str(target);
        }
        if tool.count > 1 {
            head.push_str(&format!(" ×{}", tool.count));
        }
        let (outcome, style) = match &tool.result {
            ResultMark::Pending => ("…".to_owned(), self.theme.muted),
            ResultMark::Unreadable => ("result not readable".to_owned(), self.theme.muted),
            ResultMark::Missing => ("no result".to_owned(), self.theme.warning),
            ResultMark::Reported(status) => match status.known() {
                Some(ResultStatus::Succeeded | ResultStatus::Unreported) => (
                    tool.output.clone().unwrap_or_else(|| "ok".to_owned()),
                    self.theme.muted,
                ),
                Some(ResultStatus::Failed) => (
                    tool.output
                        .as_ref()
                        .map_or_else(|| "failed".to_owned(), |output| format!("failed: {output}")),
                    self.theme.error,
                ),
                None => (status.label().replace('_', " "), self.theme.muted),
            },
            ResultMark::Failures(count) => (format!("{count} failed"), self.theme.error),
            ResultMark::AllSucceeded(count) => (format!("{count} ok"), self.theme.muted),
        };
        Line::from(vec![
            Span::styled(head, dim),
            Span::styled(" → ", dim),
            Span::styled(outcome, style),
        ])
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

    fn spans_width(spans: &[Span<'static>]) -> usize {
        spans.iter().map(|span| Self::width(&span.content)).sum()
    }

    /// Segments split by rules: the tag, what is stale or needs a look,
    /// each client's recent output, the counts; the help hint and the clock
    /// keep their place at the right.
    fn draw_status(&self, status: &Status, area: Rect, buf: &mut Buffer) {
        let base = self.theme.bar;
        buf.set_style(area, base);
        let rule = Span::styled("│", self.theme.rule);
        let mut spans = vec![
            Span::raw(" ren ")
                .style(self.theme.responding_title)
                .add_modifier(Modifier::BOLD),
        ];
        if let Some(failure) = &status.failure {
            spans.push(Span::raw(format!(" ! stale: {failure} ")).style(self.theme.error_title));
        }
        if let Some(notice) = &status.notice {
            spans.push(Span::raw(format!(" {notice} ")).style(self.theme.warning_title));
        }
        if status.attention > 0 {
            let needs = if status.attention == 1 {
                "needs"
            } else {
                "need"
            };
            spans.push(
                Span::raw(format!(" ! {} {needs} a look ", status.attention))
                    .style(self.theme.warning_title),
            );
        }
        let right = format!(" ? help │ {} ", Clock(status.now));
        let right_width = u16::try_from(Self::width(&right)).unwrap_or(u16::MAX);
        let left_width = match area.width.checked_sub(right_width) {
            Some(left) => {
                buf.set_string(area.x + left, area.y, right, base.patch(self.theme.muted));
                left
            }
            None => area.width,
        };
        let mut counts = vec![format!(
            "{} session{}",
            status.sessions,
            if status.sessions == 1 { "" } else { "s" }
        )];
        counts.push(format!("{} streaming", status.responding));
        counts.push(format!("{} waiting", status.awaiting));
        if status.hidden > 0 {
            counts.push(format!("{} hidden", status.hidden));
        }
        if let Some(runs) = status.runs {
            counts.push(format!(
                "{} run{}",
                runs.live,
                if runs.live == 1 { "" } else { "s" }
            ));
            if runs.waiting > 0 {
                counts.push(format!("{} asking", runs.waiting));
            }
        }
        match status.reading {
            Reading::Pending => counts.push("reading…".to_owned()),
            Reading::Complete => {}
            Reading::Capped => counts.push("older calls not read".to_owned()),
        }
        let counts = vec![Span::raw(format!(" {} ", counts.join(" · ")))];
        // Sparks share what the counts leave: each gets as many glyphs as
        // fit (merging bins, on one shared scale), or none at all.
        let room = usize::from(left_width)
            .saturating_sub(Self::spans_width(&spans) + 1 + Self::spans_width(&counts));
        let sparks = status.output.len().max(1);
        // Each spark also takes its rule, name, and total: about 14 cells.
        let each = (room / sparks).saturating_sub(14);
        let glyphs = [24, 12, 8, 6, 4]
            .into_iter()
            .find(|glyphs| *glyphs <= each)
            .unwrap_or(0);
        let merged: Vec<(Harness, Vec<u64>, u64)> = status
            .output
            .iter()
            .map(|spark| {
                let per = spark.bins.len().div_ceil(glyphs.max(1)).max(1);
                let bins = spark
                    .bins
                    .chunks(per)
                    .map(|chunk| chunk.iter().sum::<u64>())
                    .collect();
                (spark.harness, bins, spark.bins.iter().sum())
            })
            .collect();
        let peak = merged
            .iter()
            .flat_map(|(_, bins, _)| bins.iter().copied())
            .max()
            .unwrap_or(0);
        let mut used = 0;
        for (harness, bins, total) in &merged {
            let style = self.theme.harness(Some(*harness));
            let mut spark = vec![Span::raw(format!(" {} ", Self::short(*harness))).style(style)];
            if glyphs > 0 {
                spark.push(Span::raw(Self::spark(bins, peak)).style(style));
            }
            spark.push(Span::raw(format!(" {} tok ", Kilo(*total))));
            let width = 1 + Self::spans_width(&spark);
            if used + width > room {
                break;
            }
            used += width;
            spans.push(rule.clone());
            spans.extend(spark);
        }
        spans.push(rule);
        spans.extend(counts);
        buf.set_line(area.x, area.y, &Line::from(spans), left_width);
    }

    /// The status line while text is typed: what it is for, the text with
    /// a cursor, and how to send or drop it.
    fn draw_entry(&self, entry: &EntryLine, area: Rect, buf: &mut Buffer) {
        let base = self.theme.bar;
        buf.set_style(area, base);
        let right = " enter send · esc cancel ";
        let right_width = u16::try_from(Self::width(right)).unwrap_or(u16::MAX);
        let left_width = match area.width.checked_sub(right_width) {
            Some(left) => {
                buf.set_string(area.x + left, area.y, right, base.patch(self.theme.muted));
                left
            }
            None => area.width,
        };
        let label = format!(" {} › ", entry.label);
        // The end of a long text stays in view, where the cursor is.
        let room = usize::from(left_width).saturating_sub(Self::width(&label) + 2);
        let mut shown: Vec<char> = Vec::new();
        let mut width = 0;
        for ch in entry.text.chars().rev() {
            let wide = ch.width().unwrap_or(0);
            if width + wide > room {
                break;
            }
            width += wide;
            shown.push(ch);
        }
        let text: String = shown.into_iter().rev().collect();
        let line = Line::from(vec![
            Span::raw(label).style(self.theme.responding_title),
            Span::raw(format!(" {text}")),
            Span::raw("▏").style(self.theme.focus),
        ]);
        buf.set_line(area.x, area.y, &line, left_width);
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

    fn draw_help(&self, area: Rect, buf: &mut Buffer) {
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
            .border_style(self.theme.rule)
            .style(self.theme.bar)
            .render(rect, buf);
        for (row, (keys, text)) in rows.iter().enumerate() {
            let Ok(row) = u16::try_from(row) else {
                break;
            };
            if row + 2 >= height {
                break;
            }
            let line = Line::from(vec![
                Span::raw(format!("{keys:<keys_width$}   ")).style(self.theme.tool),
                Span::raw(*text),
            ]);
            buf.set_line(rect.x + 2, rect.y + 1 + row, &line, width.saturating_sub(4));
        }
    }
}

/// A token count, short: `950`, `36.0k`, `148k`, `1.3M`.
struct Kilo(u64);

impl std::fmt::Display for Kilo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0;
        match n {
            0..1_000 => write!(f, "{n}"),
            1_000..100_000 => write!(f, "{}.{}k", n / 1000, n % 1000 / 100),
            100_000..1_000_000 => write!(f, "{}k", n / 1000),
            _ => write!(f, "{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
        }
    }
}

/// A count with thousands grouped: `2,426`.
struct Grouped(u64);

impl std::fmt::Display for Grouped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digits = self.0.to_string();
        let mut out = String::new();
        for (index, ch) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                out.push(',');
            }
            out.push(ch);
        }
        f.write_str(&out)
    }
}

/// `HH:MM:SS UTC`.
struct Clock(UnixMillis);

impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.0.as_millis() / 1000 % 86_400;
        write!(
            f,
            "{:02}:{:02}:{:02} UTC",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, Grouped, Kilo, ThemedWall};
    use crate::{Size, Theme, Tui};
    use ren_recorder::time::UnixMillis;

    #[test]
    fn numbers_and_times_read_short() {
        let kilo: Vec<String> = [950, 1_234, 36_000, 148_000, 1_300_000]
            .map(|n| Kilo(n).to_string())
            .into();
        assert_eq!(kilo, ["950", "1.2k", "36.0k", "148k", "1.3M"]);
        let grouped: Vec<String> = [7, 2_426, 1_234_567].map(|n| Grouped(n).to_string()).into();
        assert_eq!(grouped, ["7", "2,426", "1,234,567"]);
        assert_eq!(
            Clock(UnixMillis::from_millis(3_723_000)).to_string(),
            "01:02:03 UTC"
        );
    }

    #[test]
    fn text_wraps_at_spaces_and_breaks_long_words() {
        assert_eq!(ThemedWall::wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(ThemedWall::wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(ThemedWall::wrap("a\n\nb", 4), ["a", "", "b"]);
        assert_eq!(ThemedWall::wrap("日本語の文", 4), ["日本", "語の", "文"]);
    }

    #[test]
    fn replies_read_their_markdown_lightly() {
        let wall = Tui::new(Size {
            width: 80,
            height: 24,
        })
        .wall(UnixMillis::from_millis(0));
        let theme = Theme::default();
        let lines: Vec<String> = wall
            .themed(&theme)
            .markdown(
                "## Plan\n**What to look for:** the A\n```rust\nlet x = 1;\n```\n- one",
                40,
            )
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            ["Plan", "What to look for: the A", "│ let x = 1;", "- one"]
        );
    }

    #[test]
    fn sparks_share_one_scale() {
        assert_eq!(ThemedWall::spark(&[0, 1, 4, 8], 8), " ▁▄█");
        assert_eq!(ThemedWall::spark(&[0, 0], 0), "  ");
    }
}
