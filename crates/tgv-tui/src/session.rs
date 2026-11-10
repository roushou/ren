//! One session as a pane shows it: what it is doing now, the newest turn's
//! replies and tool calls, and what in it needs a look. The same readings
//! as the web UI's Live wall, so the two never disagree about a session.

use crate::call::{CallEnd, Tone};
use std::collections::HashMap;
use std::time::Duration;
use tgv_harness::Harness;
use tgv_harness::tools::EffectKind;
use tgv_protocol::activity::{ActivityLimitation, ResultStatus};
use tgv_protocol::label::Stored;
use tgv_protocol::route::Provider;
use tgv_recorder::id::{CallId, SessionId, TurnId};
use tgv_recorder::progress::Progress;
use tgv_recorder::time::UnixMillis;
use tgv_store::activity::report::{CallMetrics, Finding, RawFinding};
use tgv_store::activity::{CallActivity, ToolCallView, TurnActivity};
use tgv_store::view::{CallView, SessionView};

/// What a session is doing, from its calls. The first two are seen on the
/// wire; `ClientsTurn` and `YourTurn` are inferred from how the last call
/// ended, and `Idle` only means no call is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// A call was sent and no response head has arrived.
    Awaiting { since: UnixMillis },
    /// The response head arrived; the body is still streaming.
    Responding { since: UnixMillis },
    /// The last call completed asking for tools. Inferred: TGV does not see
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

/// How a tool call's result reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultMark {
    /// No later call of the turn has been read: the result is still to come.
    Pending,
    /// The request that would carry it could not be read whole.
    Unreadable,
    /// A later call was read and did not answer it.
    Missing,
    Reported(Stored<ResultStatus>),
    /// A run of like calls with this many failures.
    Failures(usize),
    /// A run of like calls that all succeeded.
    AllSucceeded(usize),
}

/// One tool call, or a run of like ones with nothing said between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLine {
    pub name: String,
    pub effect: Option<Stored<EffectKind>>,
    /// What it touched, relative to the turn's working directory.
    pub target: Option<String>,
    pub result: ResultMark,
    /// The first line of what the client reported back.
    pub output: Option<String>,
    /// How many like calls this line stands for.
    pub count: usize,
}

/// A line of a pane, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// The start of a model call: its place among the turn's calls.
    Call {
        index: Option<usize>,
    },
    Reply(String),
    Tools(ToolLine),
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
pub struct SessionPane {
    pub session: SessionId,
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
    pub lines: Vec<Line>,
    pub streaming: Option<Streaming>,
    pub tally: Option<Tally>,
    pub alerts: Vec<Alert>,
    /// Kept ahead of other sessions by the viewer.
    pub pinned: bool,
}

/// A session's calls and its newest turn, as read.
pub(crate) struct SessionRead<'a> {
    pub session: &'a SessionView,
    /// Its calls in the window and its open ones, oldest first.
    pub calls: &'a [&'a CallView],
    pub turn: Option<&'a TurnActivity>,
    pub progress: &'a HashMap<CallId, Progress>,
    pub now: UnixMillis,
}

impl SessionPane {
    /// The most lines a pane keeps; it shows the newest that fit.
    pub const MAX_LINES: usize = 60;
    /// A last call that completed its turn this recently is still your turn.
    pub const YOUR_TURN: Duration = Duration::from_secs(120);
    /// Waiting this long for a response head is worth a look.
    pub const SLOW_HEAD: Duration = Duration::from_secs(30);

    pub(crate) fn read(read: &SessionRead<'_>) -> Self {
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
        let lines = turn.map_or_else(Vec::new, |turn| Self::lines(turn, &order));
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
            streaming: Self::streaming(calls, &order, read.progress),
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

    fn is_open(call: &CallView) -> bool {
        call.state.known() == Some(tgv_recorder::capture::CallState::Open)
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

    /// The newest turn's lines: each read call's start, its reply, then its
    /// tool calls, like ones in a row merged. Calls whose text is not
    /// derived yet are left to `streaming`.
    fn lines(turn: &TurnActivity, order: &HashMap<CallId, usize>) -> Vec<Line> {
        let workdir = Workdir::of(turn);
        let mut lines = Vec::new();
        for (index, call) in turn.calls.iter().enumerate() {
            if Self::underived(call) {
                continue;
            }
            lines.push(Line::Call {
                index: order.get(&call.call).copied(),
            });
            if let Some(reply) = call
                .reply
                .as_ref()
                .map(|reply| reply.text.trim())
                .filter(|text| !text.is_empty())
            {
                lines.push(Line::Reply(reply.to_owned()));
            }
            let later = turn.calls.get(index + 1..).unwrap_or_default();
            let mut run: Option<Run> = None;
            for tool in &call.tool_calls {
                let line = Self::tool_line(tool, later, workdir.as_ref());
                run = Some(match run {
                    Some(mut same) if same.takes(&line) => {
                        same.push(line);
                        same
                    }
                    Some(other) => {
                        lines.push(Line::Tools(other.finish()));
                        Run::new(line)
                    }
                    None => Run::new(line),
                });
            }
            lines.extend(run.map(|run| Line::Tools(run.finish())));
        }
        let skip = lines.len().saturating_sub(Self::MAX_LINES);
        lines.into_iter().skip(skip).collect()
    }

    fn underived(call: &CallActivity) -> bool {
        call.limitations
            .iter()
            .any(|limitation| limitation.known() == Some(ActivityLimitation::NotYetDerived))
    }

    /// Whether TGV could not read a call's text whole.
    fn unreadable(call: &CallActivity) -> bool {
        call.limitations
            .iter()
            .any(|limitation| match limitation.known() {
                Some(
                    ActivityLimitation::CaptureGap
                    | ActivityLimitation::Truncated
                    | ActivityLimitation::NotJson
                    | ActivityLimitation::DecodedTooLarge
                    | ActivityLimitation::UnsupportedEncoding
                    | ActivityLimitation::DecodingError
                    | ActivityLimitation::UnexpectedShape,
                ) => true,
                Some(
                    ActivityLimitation::UnsupportedEndpoint
                    | ActivityLimitation::UnfinishedToolCall
                    | ActivityLimitation::TooManyToolCalls
                    | ActivityLimitation::NotYetDerived,
                )
                | None => false,
            })
    }

    fn tool_line(
        tool: &ToolCallView,
        later: &[CallActivity],
        workdir: Option<&Workdir>,
    ) -> ToolLine {
        let effect = tool.effects.first();
        // A result rides in the client's next request: until a later call
        // is read, a missing one is still to come.
        let result = match &tool.result {
            Some(result) => ResultMark::Reported(result.status.clone()),
            None if later.iter().all(Self::underived) => ResultMark::Pending,
            None if later.first().is_some_and(Self::unreadable) => ResultMark::Unreadable,
            None => ResultMark::Missing,
        };
        ToolLine {
            name: tool.name.clone(),
            effect: effect.map(|effect| effect.kind.clone()),
            target: effect.map(|effect| match workdir {
                Some(workdir) => workdir.shorten(&effect.target),
                None => effect.target.clone(),
            }),
            result,
            output: tool
                .result
                .as_ref()
                .and_then(|result| Self::first_line(&result.output.text)),
            count: 1,
        }
    }

    /// The first line with text in it, cut to 160 characters.
    fn first_line(text: &str) -> Option<String> {
        let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
        Some(line.chars().take(160).collect())
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
                    detail: format!("waiting {}", Span(waited)),
                    at: since,
                });
            }
        }
        // Rate limits come in bursts: one alert, with the count.
        let limited: Vec<&&CallView> = calls
            .iter()
            .filter(|call| call.status == Some(429))
            .collect();
        if let Some(newest) = limited.last() {
            alerts.push(Alert {
                tone: Tone::Warning,
                label: match limited.len() {
                    1 => "429".to_owned(),
                    count => format!("{count} × 429"),
                },
                detail: "rate limited".to_owned(),
                at: newest.started_at,
            });
        }
        for call in calls {
            if call.status == Some(429) {
                continue;
            }
            let end = CallEnd::of(call);
            if !end.troubled() {
                continue;
            }
            alerts.push(Alert {
                tone: match end.tone() {
                    Tone::Error => Tone::Error,
                    Tone::Live | Tone::Warning | Tone::Success | Tone::Neutral => Tone::Warning,
                },
                label: match call.status {
                    Some(status) if status >= 400 => status.to_string(),
                    Some(_) | None => end.label(None),
                },
                detail: call
                    .model
                    .clone()
                    .unwrap_or_else(|| call.endpoint.label().to_owned()),
                at: call.ended_at.unwrap_or(call.started_at),
            });
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

/// Like tool calls in a row, with nothing said between them: never empty.
struct Run {
    newest: ToolLine,
    count: usize,
    failed: usize,
    succeeded: usize,
}

impl Run {
    fn new(line: ToolLine) -> Self {
        let mut run = Self {
            count: 0,
            failed: 0,
            succeeded: 0,
            newest: line.clone(),
        };
        run.push(line);
        run
    }

    /// Whether `line` is like the run's calls: the same tool, the same kind
    /// of effect.
    fn takes(&self, line: &ToolLine) -> bool {
        self.newest.name == line.name && self.newest.effect == line.effect
    }

    fn push(&mut self, line: ToolLine) {
        self.count += 1;
        match &line.result {
            ResultMark::Reported(status) => match status.known() {
                Some(ResultStatus::Failed) => self.failed += 1,
                Some(ResultStatus::Succeeded) => self.succeeded += 1,
                Some(ResultStatus::Unreported) | None => {}
            },
            ResultMark::Pending
            | ResultMark::Unreadable
            | ResultMark::Missing
            | ResultMark::Failures(_)
            | ResultMark::AllSucceeded(_) => {}
        }
        self.newest = line;
    }

    /// The run as one line: the newest call, the count, and a tally of
    /// their results when there is more than one.
    fn finish(self) -> ToolLine {
        let mut line = self.newest;
        if self.count > 1 {
            if self.failed > 0 {
                line.result = ResultMark::Failures(self.failed);
            } else if self.succeeded == self.count {
                line.result = ResultMark::AllSucceeded(self.count);
            }
        }
        line.count = self.count;
        line
    }
}

/// A duration as a pane writes it: `12 s`, `4 min`, `1 h 5 min`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span(pub u64);

impl std::fmt::Display for Span {
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

/// The directory most of a turn's commands change into first.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Workdir(String);

/// `cd <dir> && …` or `cd <dir>; …` at the start of a command.
#[derive(Debug, PartialEq, Eq)]
struct ChangeDirectory<'a> {
    dir: &'a str,
    rest: &'a str,
}

impl<'a> ChangeDirectory<'a> {
    fn parse(command: &'a str) -> Option<Self> {
        let after = command.strip_prefix("cd")?;
        let trimmed = after.trim_start();
        if trimmed.len() == after.len() {
            return None;
        }
        let (dir, rest) = match trimmed.chars().next()? {
            quote @ ('"' | '\'') => {
                let inner = &trimmed[1..];
                let end = inner.find(quote)?;
                (&inner[..end], &inner[end + 1..])
            }
            _ => {
                let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
                let token = &trimmed[..end];
                // `cd dir&&…` or `cd dir;…` without a space.
                match token.find("&&").or_else(|| token.find(';')) {
                    Some(at) => (&token[..at], &trimmed[at..]),
                    None => (token, &trimmed[end..]),
                }
            }
        };
        if dir.is_empty() {
            return None;
        }
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("&&")
            .or_else(|| rest.strip_prefix(';'))?
            .trim_start();
        Some(Self { dir, rest })
    }
}

impl Workdir {
    fn of(turn: &TurnActivity) -> Option<Self> {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for command in &turn.commands {
            let Some(cd) = ChangeDirectory::parse(command) else {
                continue;
            };
            match counts.iter_mut().find(|(dir, _)| *dir == cd.dir) {
                Some((_, count)) => *count += 1,
                None => counts.push((cd.dir, 1)),
            }
        }
        // The first of the most frequent, as the order they appeared.
        let mut best: Option<(&str, usize)> = None;
        for (dir, count) in counts {
            if best.is_none_or(|(_, most)| count > most) {
                best = Some((dir, count));
            }
        }
        best.map(|(dir, _)| Self(dir.to_owned()))
    }

    /// A target without a `cd` into the workdir, with paths relative to it.
    fn shorten(&self, target: &str) -> String {
        let text = match ChangeDirectory::parse(target) {
            Some(cd) if cd.dir == self.0 => cd.rest,
            Some(_) | None => target,
        };
        let root = self.0.trim_end_matches('/');
        if root.is_empty() {
            return text.to_owned();
        }
        text.replace(&format!("{root}/"), "").replace(root, ".")
    }
}

#[cfg(test)]
mod tests {
    use super::{ChangeDirectory, Span};

    #[test]
    fn spans_read_like_the_web_ui() {
        assert_eq!(Span(0).to_string(), "0 s");
        assert_eq!(Span(59_400).to_string(), "59 s");
        assert_eq!(Span(90_000).to_string(), "2 min");
        assert_eq!(Span(3_600_000).to_string(), "1 h");
        assert_eq!(Span(3_900_000).to_string(), "1 h 5 min");
    }

    #[test]
    fn a_leading_cd_is_read_with_or_without_quotes() {
        let cd = |text| ChangeDirectory::parse(text).map(|cd| (cd.dir, cd.rest));
        assert_eq!(
            cd("cd /w/app && cargo test"),
            Some(("/w/app", "cargo test"))
        );
        assert_eq!(cd("cd \"/w/my app\"; ls"), Some(("/w/my app", "ls")));
        assert_eq!(cd("cd '/w/a' && ls"), Some(("/w/a", "ls")));
        assert_eq!(cd("cd /w/app&&ls"), Some(("/w/app", "ls")));
        assert_eq!(cd("cd /w/app"), None);
        assert_eq!(cd("cdx /w && ls"), None);
        assert_eq!(cd("ls && cd /w"), None);
    }
}
