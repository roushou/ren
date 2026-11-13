//! One session as it is watched: what it is doing now, the newest turn's
//! replies and tool calls, and what in it needs a look. The same readings
//! as the web UI's Live wall, so the two never disagree about a session.

use crate::conversation::{Block, Conversation, ToolLine};
use crate::model::CallEnd;
use crate::model::Model;
use crate::model::recordings::Recordings;
use crate::ui::{Ink, Line, Span, Tone};
use ren_harness::Harness;
use ren_protocol::label::Stored;
use ren_protocol::route::Provider;
use ren_recorder::id::{CallId, RecordingId, TurnId};
use ren_recorder::progress::Progress;
use ren_recorder::time::UnixMillis;
use ren_store::activity::report::{CallMetrics, Finding, RawFinding};
use ren_store::activity::{CallActivity, TurnActivity};
use ren_store::view::{CallView, RecordingView};
use std::collections::HashMap;
use std::time::Duration;

/// What a session is doing, from its calls. The first two are seen on the
/// wire; `ClientsTurn` and `YourTurn` are inferred from how the last call
/// ended, and `Idle` only means no call is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// A call was sent and no response head has arrived.
    Awaiting { since: UnixMillis },
    /// The response head arrived; the body is still streaming.
    Responding { since: UnixMillis },
    /// The last call completed asking for tools. Inferred: Ren does not see
    /// tools run.
    ClientsTurn { since: UnixMillis },
    /// The last call completed without asking for tools, lately. Inferred.
    YourTurn { since: UnixMillis },
    /// The last call did not end as a completed model call.
    Stopped {
        since: UnixMillis,
        end: CallEnd,
        status: Option<u16>,
    },
    /// No call open, and the last one completed a while ago (or there is
    /// none in the window).
    Idle { since: Option<UnixMillis> },
}

/// What the newest call measured, as the wire reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFacts {
    pub model: Option<String>,
    /// The prompt the model saw, in tokens; `None` until usage is reported.
    pub input: Option<u64>,
    /// The share of `input` read from the provider's cache, in percent.
    pub cached: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
    pub stop_reason: Option<String>,
}

/// What a streaming call has said so far, from progress updates: never
/// stored, and replaced by the call's activity once it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Streaming {
    pub call: CallId,
    /// Its place among the turn's calls.
    pub index: Option<usize>,
    pub reply: String,
    /// Whether the reply so far is longer than `reply`, which keeps its end.
    pub cut: bool,
    pub tools: Vec<String>,
    pub thinking: bool,
}

/// What the turn has done so far, as the client reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub read: usize,
    pub wrote: usize,
    pub ran: usize,
    pub failed: u64,
}

/// Something in a session worth a look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub tone: Tone,
    pub label: String,
    pub detail: String,
    pub at: UnixMillis,
}

/// One session, read for its pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingPane {
    pub session: RecordingId,
    pub harness: Option<Stored<Harness>>,
    pub provider: Stored<Provider>,
    pub phase: Phase,
    /// Model calls open at once (subagents run in parallel).
    pub open: usize,
    /// Calls in the newest turn so far.
    pub turn_calls: usize,
    pub model: Option<String>,
    /// Context the newest call that reported usage carried, in tokens.
    pub context: Option<u64>,
    /// That call's output tokens.
    pub output: Option<u64>,
    /// Output tokens over the session's calls, provider-reported.
    pub session_output: Option<u64>,
    /// The newest call's measurements.
    pub facts: Option<CallFacts>,
    /// The newest turns as a conversation, oldest first.
    pub lines: Vec<Block>,
    pub streaming: Option<Streaming>,
    pub tally: Option<Tally>,
    pub alerts: Vec<Alert>,
    /// Kept ahead of other sessions by the viewer.
    pub pinned: bool,
}

/// A session's calls and its newest turn, as read.
pub(crate) struct RecordingRead<'a> {
    pub session: &'a RecordingView,
    /// Its calls in the window and its open ones, oldest first.
    pub calls: &'a [&'a CallView],
    pub turn: Option<&'a TurnActivity>,
    /// The turn before it, when read: the pane shows how the session got
    /// here.
    pub earlier: Option<&'a TurnActivity>,
    /// For a session whose client sends no turns: its newest calls' activity,
    /// as read, oldest first.
    pub activities: &'a [&'a CallActivity],
    pub progress: &'a HashMap<CallId, Progress>,
    pub now: UnixMillis,
}

impl RecordingPane {
    /// `session` as read now, with what the model holds of it.
    pub(crate) fn of(model: &Model, session: &RecordingView, now: UnixMillis) -> Self {
        let calls = model.recordings.recording_calls(session.id);
        let recent = Recordings::recent_turns(&calls);
        let held = |turn| model.recordings.turn(turn);
        let (earlier, turn) = match recent.as_slice() {
            [earlier, newest] => (held(*earlier), held(*newest)),
            [newest] => (None, held(*newest)),
            _ => (None, None),
        };
        let activities: Vec<&CallActivity> = calls
            .iter()
            .filter_map(|call| model.recordings.activity(call.id))
            .collect();
        Self::read(&RecordingRead {
            activities: &activities,
            session,
            calls: &calls,
            earlier,
            turn,
            progress: model.recordings.progress(),
            now,
        })
    }

    /// The conversation it shows, with what a streaming call has said so
    /// far.
    pub(crate) fn conversation(&self) -> Conversation {
        let mut blocks = self.lines.clone();
        if let Some(streaming) = &self.streaming {
            for tool in &streaming.tools {
                blocks.push(Block::Tools(ToolLine::named(tool)));
            }
            blocks.push(if streaming.thinking {
                Block::Thinking
            } else if streaming.cut {
                Block::Streaming(format!("…{}", streaming.reply))
            } else {
                Block::Streaming(streaming.reply.clone())
            });
        }
        Conversation { blocks }
    }

    /// The most lines a pane keeps; it shows the newest that fit.
    pub const MAX_LINES: usize = 60;
    /// A last call that completed its turn this recently is still your turn.
    pub const YOUR_TURN: Duration = Duration::from_secs(120);
    /// Waiting this long for a response head is worth a look.
    pub const SLOW_HEAD: Duration = Duration::from_secs(30);

    pub(crate) fn read(read: &RecordingRead<'_>) -> Self {
        let calls = read.calls;
        let turn_id = Self::newest_turn(calls);
        let turn_calls: Vec<&CallView> = calls
            .iter()
            .copied()
            .filter(|call| turn_id.is_some() && call.turn == turn_id)
            .collect();
        let order: HashMap<CallId, usize> = turn_calls
            .iter()
            .enumerate()
            .map(|(index, call)| (call.id, index + 1))
            .collect();
        let turn = read.turn.filter(|turn| Some(turn.turn) == turn_id);
        // Context and output come from the newest call that reported usage.
        let measured = calls
            .iter()
            .rev()
            .find(|call| call.usage.is_some())
            .map(|call| CallMetrics::of(call));
        let phase = Self::phase(calls, read.now);
        let earlier = read.earlier.filter(|earlier| Some(earlier.turn) != turn_id);
        let mut lines = earlier.map_or_else(Vec::new, |earlier| {
            let order: HashMap<CallId, usize> = calls
                .iter()
                .filter(|call| call.turn == Some(earlier.turn))
                .enumerate()
                .map(|(index, call)| (call.id, index + 1))
                .collect();
            Conversation::turn(earlier, &order).blocks
        });
        lines.extend(turn.map_or_else(Vec::new, |turn| Conversation::turn(turn, &order).blocks));
        if turn_id.is_none() {
            let order: HashMap<CallId, usize> = calls
                .iter()
                .enumerate()
                .map(|(index, call)| (call.id, index + 1))
                .collect();
            lines.extend(Conversation::calls(read.activities, None, &order).blocks);
        }
        let skip = lines.len().saturating_sub(Self::MAX_LINES);
        lines.drain(..skip);
        // The newest open call, when its text is not read yet: its rule,
        // then what it has streamed (`streaming`) or that it is waiting.
        let open = calls.iter().rev().find(|call| Self::is_open(call));
        let streaming = Self::streaming(calls, &order, read.progress);
        if let Some(open) = open {
            let read_already = turn
                .map_or(read.activities, |_| &[])
                .iter()
                .copied()
                .chain(turn.iter().flat_map(|turn| &turn.calls))
                .any(|call| call.call == open.id && !Conversation::underived(call));
            if !read_already {
                lines.push(Block::Call {
                    index: order.get(&open.id).copied(),
                });
                if open.status.is_none() && streaming.is_none() {
                    lines.push(Block::Waiting);
                }
            }
        }
        let mut pane = Self {
            session: read.session.id,
            harness: read
                .session
                .source
                .known()
                .map(|source| source.harness().into()),
            provider: read.session.provider.clone(),
            open: calls.iter().filter(|call| Self::is_open(call)).count(),
            turn_calls: turn_calls.len(),
            model: calls.iter().rev().find_map(|call| call.model.clone()),
            context: measured.as_ref().and_then(|metrics| metrics.context_tokens),
            output: measured.as_ref().and_then(|metrics| metrics.output_tokens),
            session_output: read.session.tokens.output,
            facts: calls.last().map(|call| {
                // An open call names its model only once it ends.
                let mut facts = Self::facts(call);
                if facts.model.is_none() {
                    facts.model = calls.iter().rev().find_map(|call| call.model.clone());
                }
                facts
            }),
            streaming,
            tally: turn.map(|turn| Tally {
                read: turn.files_read.len(),
                wrote: turn.files_written.len(),
                ran: turn.commands.len(),
                failed: turn.failed_tool_calls,
            }),
            alerts: Vec::new(),
            pinned: false,
            lines,
            phase,
        };
        pane.alerts = pane.alerts_from(calls, turn, read.now);
        pane
    }

    fn facts(call: &CallView) -> CallFacts {
        let metrics = CallMetrics::of(call);
        CallFacts {
            model: call.model.clone(),
            input: metrics.context_tokens,
            cached: match (metrics.cache_read_tokens, metrics.context_tokens) {
                (Some(read), Some(context)) if context > 0 => {
                    Some(read.saturating_mul(100) / context)
                }
                (Some(_) | None, Some(_) | None) => None,
            },
            output: metrics.output_tokens,
            reasoning: call.usage.as_ref().and_then(|usage| usage.reasoning_tokens),
            stop_reason: call.stop_reason.clone(),
        }
    }

    fn is_open(call: &CallView) -> bool {
        call.state.known() == Some(ren_recorder::capture::CallState::Open)
    }

    /// The newest turn a call carried.
    pub(crate) fn newest_turn(calls: &[&CallView]) -> Option<TurnId> {
        calls.iter().rev().find_map(|call| call.turn)
    }

    fn phase(calls: &[&CallView], now: UnixMillis) -> Phase {
        if let Some(open) = calls.iter().rev().find(|call| Self::is_open(call)) {
            return match open.status {
                None => Phase::Awaiting {
                    since: open.started_at,
                },
                Some(_) => Phase::Responding {
                    since: open.started_at,
                },
            };
        }
        let Some(last) = calls.last() else {
            return Phase::Idle { since: None };
        };
        let since = last.ended_at.unwrap_or(last.started_at);
        match CallEnd::of(last) {
            CallEnd::ModelCompleted if !last.tool_names.is_empty() => Phase::ClientsTurn { since },
            CallEnd::ModelCompleted
                if now.as_millis().saturating_sub(since.as_millis())
                    < Self::millis(Self::YOUR_TURN) =>
            {
                Phase::YourTurn { since }
            }
            CallEnd::ModelCompleted | CallEnd::HttpComplete | CallEnd::Open => {
                Phase::Idle { since: Some(since) }
            }
            end @ (CallEnd::Rejected(_)
            | CallEnd::Error(_)
            | CallEnd::Stopped(_)
            | CallEnd::Recovered
            | CallEnd::Incomplete(_)
            | CallEnd::Unknown) => Phase::Stopped {
                since,
                end,
                status: last.status,
            },
        }
    }

    fn millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }
    /// The newest open call's progress, when it has any.
    fn streaming(
        calls: &[&CallView],
        order: &HashMap<CallId, usize>,
        progress: &HashMap<CallId, Progress>,
    ) -> Option<Streaming> {
        let (call, update) = calls
            .iter()
            .rev()
            .filter(|call| Self::is_open(call))
            .find_map(|call| progress.get(&call.id).map(|update| (call, update)))?;
        Some(Streaming {
            call: call.id,
            index: order.get(&call.id).copied(),
            reply: update.reply.clone(),
            cut: update.cut,
            tools: update.tools.clone(),
            thinking: update.thinking,
        })
    }

    /// Slow response heads, rate limits, bad endings, and the findings that
    /// matter while a turn runs.
    fn alerts_from(
        &self,
        calls: &[&CallView],
        turn: Option<&TurnActivity>,
        now: UnixMillis,
    ) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if let Phase::Awaiting { since } = self.phase {
            let waited = now.as_millis().saturating_sub(since.as_millis());
            if waited >= Self::millis(Self::SLOW_HEAD) {
                alerts.push(Alert {
                    tone: Tone::Warning,
                    label: "no response head".to_owned(),
                    detail: format!("waiting {}", Elapsed(waited)),
                    at: since,
                });
            }
        }
        // How the newest call ended, when it ended badly: an earlier
        // failure the session has moved past is not shown again.
        if let Some(call) = calls.last() {
            let end = CallEnd::of(call);
            if end.troubled() {
                let limited = calls.iter().filter(|call| call.status == Some(429)).count();
                let (label, detail) = match call.status {
                    Some(429) if limited > 1 => (
                        "429 rate limited".to_owned(),
                        format!("{limited} in the window"),
                    ),
                    Some(429) => ("429 rate limited".to_owned(), String::new()),
                    Some(_) | None => (
                        end.label(call.status),
                        call.model
                            .clone()
                            .unwrap_or_else(|| call.endpoint.label().to_owned()),
                    ),
                };
                alerts.push(Alert {
                    tone: match end.tone() {
                        Tone::Error => Tone::Error,
                        Tone::Live | Tone::Warning | Tone::Success | Tone::Neutral => Tone::Warning,
                    },
                    label,
                    detail,
                    at: call.ended_at.unwrap_or(call.started_at),
                });
            }
        }
        if let Some(turn) = turn {
            let at = turn.ended_at.or(turn.started_at).unwrap_or(now);
            alerts.extend(turn.findings.iter().filter_map(|finding| match finding {
                Finding::SecretExposure(exposure) => Some(Alert {
                    tone: Tone::Error,
                    label: "secret sent".to_owned(),
                    detail: format!("{} in {}", exposure.secret, exposure.field),
                    at,
                }),
                Finding::RepeatedFailure(failure) => Some(Alert {
                    tone: Tone::Warning,
                    label: format!("{} failed {}×", failure.tool, failure.failures),
                    detail: failure.target.clone().unwrap_or_default(),
                    at,
                }),
                Finding::CacheRebuild(_)
                | Finding::ContextGrowth(_)
                | Finding::LargeToolOutput(_)
                | Finding::Unknown(RawFinding { .. }) => None,
            }));
        }
        alerts.sort_by_key(|alert| std::cmp::Reverse(alert.at));
        alerts
    }
}

/// A duration as a pane writes it: `12 s`, `4 min`, `1 h 5 min`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed(pub u64);

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.0.saturating_add(500) / 1000;
        if seconds < 60 {
            return write!(f, "{seconds} s");
        }
        let minutes = seconds.saturating_add(30) / 60;
        if minutes < 60 {
            return write!(f, "{minutes} min");
        }
        match (minutes / 60, minutes % 60) {
            (hours, 0) => write!(f, "{hours} h"),
            (hours, rest) => write!(f, "{hours} h {rest} min"),
        }
    }
}

impl RecordingPane {
    /// `! alert · state · detail`, as the mockup words it.
    /// A lit header's ink: live while a response streams, warning or error
    /// when the newest call needs a look; `None` when it is plain.
    pub(crate) fn lit(&self) -> Option<Ink> {
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

    pub(crate) fn state_line(&self, now: UnixMillis) -> Line {
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
                let slow = waited
                    >= u64::try_from(RecordingPane::SLOW_HEAD.as_millis()).unwrap_or(u64::MAX);
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
    /// Bins as block glyphs on a shared scale; an empty bin is a space.
    pub(crate) fn spark(bins: &[u64], peak: u64) -> String {
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

    pub(crate) fn short(harness: Harness) -> &'static str {
        match harness {
            Harness::ClaudeCode => "cc",
            Harness::Codex => "codex",
            Harness::Pi => "pi",
            Harness::OpenCode => "oc",
        }
    }
}

/// A token count, short: `950`, `36.0k`, `148k`, `1.3M`.
pub(crate) struct Kilo(pub u64);

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
pub(crate) struct Grouped(pub u64);

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
    use super::Elapsed;

    #[test]
    fn spans_read_like_the_web_ui() {
        assert_eq!(Elapsed(0).to_string(), "0 s");
        assert_eq!(Elapsed(59_400).to_string(), "59 s");
        assert_eq!(Elapsed(90_000).to_string(), "2 min");
        assert_eq!(Elapsed(3_600_000).to_string(), "1 h");
        assert_eq!(Elapsed(3_900_000).to_string(), "1 h 5 min");
    }
}
