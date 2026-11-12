//! The wall as a view: panes in a grid, each a title bar, the session's
//! conversation, and a state line; and what the status line says of them.
//! Pure: the crate's one renderer draws it.

use super::pane::{CallFacts, Elapsed, Phase, SessionPane};
use super::runs::RunPhase;
use super::{Pane, Panes};
use crate::conversation::Density;
use crate::model::Reading;
use crate::ui::Tone;
use crate::ui::{Anchor, Bar, Grid, Ink, Keep, Line, Rule, Size, Span, Text, View};
use ren_harness::Harness;
use ren_harness::drive::ApprovalAsk;
use ren_recorder::time::UnixMillis;

impl Panes {
    /// The panes, in the room they have.
    pub fn view(&self) -> View {
        let (width, height) = (self.size.width, self.size.height);
        let grid = Grid {
            columns: self.grid.columns.max(1),
            rows: self.grid.rows.max(1),
            cells: Vec::new(),
        };
        let all_free = self.panes.iter().all(|pane| *pane == Pane::Free);
        let cells = self
            .panes
            .iter()
            .enumerate()
            .map(|(index, pane)| {
                let (place, focused) = (Some(index), self.focus == index);
                let (cell_width, cell_height) = grid.cell(width, height, index);
                match pane {
                    Pane::Free if all_free => View::lines(
                        vec![Line::of(
                            "no live sessions · a session appears here when its client calls",
                            Ink::Muted,
                        )],
                        Anchor::Middle,
                    ),
                    Pane::Free => View::Empty,
                    Pane::Session(session) => session.view(
                        place,
                        focused,
                        crate::Size {
                            width: cell_width,
                            height: cell_height,
                        },
                        self.status.now,
                        Density::Pane,
                    ),
                }
            })
            .collect();
        View::Grid(Grid { cells, ..grid })
    }

    /// The status line: what is stale or needs a look, each client's
    /// recent output, the counts, in segments split by rules.
    pub fn status_line(&self) -> Line {
        let status = &self.status;
        let rule = Span::new("│", Ink::Rule);
        let mut left = Line::default();
        if let Some(failure) = &status.failure {
            left.push(Span::new(format!(" ! stale: {failure} "), Ink::ErrorTitle));
        }
        if status.attention > 0 {
            let needs = if status.attention == 1 {
                "needs"
            } else {
                "need"
            };
            left.push(Span::new(
                format!(" ! {} {needs} a look ", status.attention),
                Ink::WarningTitle,
            ));
        }
        // The frame keeps the help hint and the clock at the right.
        let left_width = usize::from(self.size.width).saturating_sub(22);
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
        let counts = Span::plain(format!(" {} ", counts.join(" · ")));
        // Sparks share what the counts leave: each gets as many glyphs as
        // fit (merging bins, on one shared scale), or none at all.
        let room = left_width.saturating_sub(left.width() + 1 + counts.width());
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
            let ink = Ink::Client(Some(*harness));
            let mut spark = vec![Span::new(format!(" {} ", Self::short(*harness)), ink)];
            if glyphs > 0 {
                spark.push(Span::new(Self::spark(bins, peak), ink));
            }
            spark.push(Span::plain(format!(" {} tok ", Kilo(*total))));
            let width = 1 + spark.iter().map(Span::width).sum::<usize>();
            if used + width > room {
                break;
            }
            used += width;
            if !left.spans.is_empty() {
                left.push(rule.clone());
            }
            left.spans.extend(spark);
        }
        if !left.spans.is_empty() {
            left.push(rule);
        }
        left.push(counts);
        left
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
}

impl SessionPane {
    /// A session's pane: its title bar, a rule under it when the pane can
    /// spare a line, the conversation against the bottom, and a state line
    /// and a facts line at the foot when there is room.
    pub(crate) fn view(
        &self,
        place: Option<usize>,
        focused: bool,
        size: crate::Size,
        now: UnixMillis,
        density: Density,
    ) -> View {
        let (width, height) = (size.width, size.height);
        let inner = usize::from(width.saturating_sub(2));
        let lit = self.lit();
        let mut rows = vec![(Size::Exact(1), View::Bar(self.title(place, focused, lit)))];
        if height >= 7 {
            let rule = match lit {
                Some(ink) => Rule::Edge(ink),
                None => Rule::Plain,
            };
            rows.push((Size::Exact(1), View::Rule(rule)));
        }
        rows.push((
            Size::Fill,
            View::pad(
                1,
                1,
                View::lines(self.conversation().lines(inner, density), Anchor::Bottom),
            ),
        ));
        let foot = match height {
            0..=3 => 0,
            4..=5 => 1,
            _ => 2,
        };
        if foot >= 1 {
            let state = self.state_line(now);
            rows.push((
                Size::Exact(1),
                View::pad(1, 1, View::lines(vec![state], Anchor::Top)),
            ));
        }
        if foot >= 2 {
            let facts = self.facts.as_ref().map_or_else(Line::default, |facts| {
                Line::of(Text(&Self::facts_text(facts)).fit(inner), Ink::Muted)
            });
            rows.push((
                Size::Exact(1),
                View::pad(1, 1, View::lines(vec![facts], Anchor::Top)),
            ));
        }
        View::fill(Ink::Pane, View::Rows(rows))
    }

    /// The title bar: green while a response streams, amber or red with
    /// an alert; the place, session, client and provider, the turn's call
    /// count, and at the right the context and the session's output.
    pub(crate) fn title(&self, place: Option<usize>, focused: bool, lit: Option<Ink>) -> Bar {
        let harness = self.harness.as_ref().and_then(|harness| harness.known());
        let name = self.harness.as_ref().map_or_else(
            || "unknown".to_owned(),
            |harness| harness.label().replace('_', "-"),
        );
        let session = self.session.to_string();
        let short = session.get(..8).unwrap_or(&session);
        let mut left = Line::new(vec![Span::plain(" ")]);
        match place {
            Some(place) if focused => left.push(Span::new(format!("{place}"), Ink::Focus)),
            Some(place) => left.push(Span::plain(format!("{place}"))),
            None => {}
        }
        left.push(Span::plain(match place {
            Some(_) => format!(":{short}  "),
            None => format!("{short}  "),
        }));
        let route = match lit {
            Some(_) => Ink::Same,
            None => Ink::Client(harness),
        };
        left.push(Span::new(name, route));
        left.push(Span::new(format!(" → {}", self.provider.label()), route));
        if self.turn_calls > 0 {
            left.push(Span::plain(format!("  call {}", self.turn_calls)));
        }
        if self.open > 1 {
            left.push(Span::plain(format!(" · {} open", self.open)));
        }
        if self.pinned {
            left.push(Span::plain("  pinned"));
        }
        if let Some(run) = &self.run {
            left.push(Span::plain(match run.phase {
                RunPhase::Idle => "  run · idle",
                RunPhase::Working => "  run · working",
                RunPhase::Waiting { .. } => "  run · asks",
            }));
        }
        // Only what is known: context, then the session's output.
        let mut facts = Vec::new();
        if let Some(context) = self.context {
            facts.push(format!("ctx {}", Kilo(context)));
        }
        if let Some(output) = self.session_output {
            facts.push(format!("{} tok", Grouped(output)));
        }
        let right = if facts.is_empty() {
            Line::default()
        } else {
            Line::from(Span::plain(format!("{} ", facts.join(" · "))))
        };
        Bar {
            left,
            right,
            fill: lit.unwrap_or(Ink::Muted),
            keep: Keep::Left,
        }
    }

    /// A lit title bar's ink: live while a response streams, warning or
    /// error when the newest call needs a look; `None` when it is plain.
    pub(crate) fn lit(&self) -> Option<Ink> {
        if let Some(RunPhase::Waiting { .. }) = self.run.as_ref().map(|run| &run.phase) {
            return Some(Ink::WarningTitle);
        }
        match self.alerts.first().map(|alert| alert.tone) {
            Some(Tone::Error) => Some(Ink::ErrorTitle),
            Some(Tone::Warning) => Some(Ink::WarningTitle),
            Some(Tone::Live | Tone::Success | Tone::Neutral) | None => match self.phase {
                Phase::Responding { .. } => Some(Ink::LiveTitle),
                Phase::Awaiting { .. }
                | Phase::ClientsTurn { .. }
                | Phase::YourTurn { .. }
                | Phase::Stopped { .. }
                | Phase::Idle { .. } => None,
            },
        }
    }

    /// `! alert · state · detail`, as the mockup words it.
    pub(crate) fn state_line(&self, now: UnixMillis) -> Line {
        // A run waiting for the operator says what it asks, and how to answer.
        if let Some(RunPhase::Waiting { ask, more }) = self.run.as_ref().map(|run| &run.phase) {
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
            let mut line = Line::of(format!("? {what}"), Ink::of(Tone::Warning));
            if let Some(reason) = reason {
                line.push(Span::new(format!(" · {reason}"), Ink::Muted));
            }
            if *more > 0 {
                line.push(Span::new(format!(" (+{more})"), Ink::Muted));
            }
            line.push(Span::new("  enter to answer", Ink::Muted));
            return line;
        }
        let mut line = Line::default();
        if let Some(alert) = self.alerts.first() {
            let more = match self.alerts.len() {
                1 => String::new(),
                count => format!(" (+{})", count - 1),
            };
            let text = match alert.detail.as_str() {
                "" => format!("! {}{more}", alert.label),
                detail => format!("! {} · {detail}{more}", alert.label),
            };
            line.push(Span::new(text, Ink::of(alert.tone)));
            line.push(Span::new(" · ", Ink::Muted));
        }
        let since = |at: UnixMillis| Elapsed(now.as_millis().saturating_sub(at.as_millis()));
        let (label, tone, detail) = match &self.phase {
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
        if matches!(self.phase, Phase::Stopped { .. }) && !self.alerts.is_empty() {
            line.spans.pop();
            return line;
        }
        line.push(Span::new(label, Ink::of(tone)));
        if !detail.is_empty() {
            line.push(Span::new(format!(" · {detail}"), Ink::Muted));
        }
        line
    }

    /// `model · in 36.0k (93% cached) · out 122 · reasoning 63 · end_turn`
    pub(crate) fn facts_text(facts: &CallFacts) -> String {
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

#[cfg(test)]
mod tests {
    use super::{Grouped, Kilo, Panes};

    #[test]
    fn numbers_and_times_read_short() {
        let kilo: Vec<String> = [950, 1_234, 36_000, 148_000, 1_300_000]
            .map(|n| Kilo(n).to_string())
            .into();
        assert_eq!(kilo, ["950", "1.2k", "36.0k", "148k", "1.3M"]);
        let grouped: Vec<String> = [7, 2_426, 1_234_567].map(|n| Grouped(n).to_string()).into();
        assert_eq!(grouped, ["7", "2,426", "1,234,567"]);
    }

    #[test]
    fn sparks_share_one_scale() {
        assert_eq!(Panes::spark(&[0, 1, 4, 8], 8), " ▁▄█");
        assert_eq!(Panes::spark(&[0, 0], 0), "  ");
    }
}
