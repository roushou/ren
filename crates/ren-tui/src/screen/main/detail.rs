//! The selected session: a header saying what it is and how it stands,
//! its tabs, and what the chosen tab shows. A run's conversation is its
//! turns, the newest open and each earlier one folded to a line; a session
//! watched is shown as it is read from its calls.

use super::list::{Item, Listing};
use super::watched::{Elapsed, Kilo, SessionPane};
use super::{Focus, Main, Mode, Tab};
use crate::conversation::{Block, Conversation, Density};
use crate::model::desk::{Attention, Run};
use crate::model::{CallEnd, Model};
use crate::ui::{Anchor, Bar, Ink, Keep, Line, Lines, Rule, Size, Span, Text, View};
use ren_harness::drive::ApprovalAsk;
use ren_recorder::id::SessionId;
use ren_recorder::time::UnixMillis;
use ren_store::activity::TurnActivity;
use ren_store::activity::report::CallMetrics;
use std::collections::HashMap;

/// One turn of a run: recorded, or the one streaming now.
struct Turn {
    prompt: String,
    /// `3m · 6 tools · 1 failed`, once it is recorded.
    summary: String,
    blocks: Vec<Block>,
    files: Files,
}

/// The files a turn touched.
#[derive(Default)]
struct Files {
    written: Vec<String>,
    read: Vec<String>,
}

/// `HH:MM:SS UTC`.
pub(crate) struct Clock(pub UnixMillis);

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

impl Main {
    /// The selected session, `width` cells wide.
    pub(super) fn detail(&self, look: &crate::screen::Look<'_>, width: u16) -> View {
        let model = look.model;
        let Some(item) = self.selected else {
            return View::lines(
                vec![
                    Line::of("No sessions yet.", Ink::Text),
                    Line::of(
                        "n starts a run · sessions started elsewhere appear when their client calls",
                        Ink::Muted,
                    ),
                ],
                Anchor::Middle,
            );
        };
        let text = usize::from(width).saturating_sub(4);
        let (title, right, lit) = self.header(model, item, look.now);
        let body = match self.tab {
            Tab::Conversation => self.conversation(model, item, text, look.now),
            Tab::Activity => self.activity(model, item, text, look.now),
            Tab::Files => self.files(model, item, look.now),
            Tab::Calls => self.calls(model, item),
        };
        let mut rows = vec![
            (
                Size::Exact(1),
                View::Bar(Bar {
                    left: title,
                    right,
                    fill: lit.unwrap_or(Ink::Muted),
                    keep: Keep::Left,
                }),
            ),
            (Size::Exact(1), View::Bar(self.tabs())),
            (
                Size::Exact(1),
                View::Rule(match lit {
                    Some(ink) => Rule::Edge(ink),
                    None => Rule::Plain,
                }),
            ),
            (
                Size::Fill,
                View::pad(
                    1,
                    1,
                    View::Lines(Lines {
                        lines: body,
                        anchor: Anchor::Newest,
                        scroll: self.scroll,
                    }),
                ),
            ),
        ];
        if self.mode == Mode::Writing || !look.message.is_empty() {
            let (composer, cursor) = look.message.lines();
            let typed = u16::try_from(composer.len().clamp(1, 6)).unwrap_or(1);
            rows.push((Size::Exact(1), View::Rule(Rule::Plain)));
            rows.push((
                Size::Exact(typed),
                View::lines(
                    Self::composer(
                        &composer,
                        cursor,
                        self.mode == Mode::Writing,
                        usize::from(typed),
                    ),
                    Anchor::Top,
                ),
            ));
        }
        View::fill(Ink::Pane, View::Rows(rows))
    }

    /// The message box: what is written, the cursor while writing.
    fn composer(lines: &[String], cursor: (usize, usize), writing: bool, rows: usize) -> Vec<Line> {
        let first = cursor.0.saturating_sub(rows - 1);
        let lead = if writing { " › " } else { " ✎ " };
        lines
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(row, text)| {
                let lead = Span::new(if row == 0 { lead } else { "   " }, Ink::Prompt);
                if !writing || row != cursor.0 {
                    return Line::new(vec![lead, Span::new(text.clone(), Ink::Text)]);
                }
                let mut before = String::new();
                let mut used = 0;
                let mut chars = text.chars();
                while let Some(ch) = chars.next() {
                    if used >= cursor.1 {
                        return Line::new(vec![
                            lead,
                            Span::new(before, Ink::Text),
                            Span::new(ch.to_string(), Ink::Text)
                                .with(crate::ui::Emphasis::Reversed),
                            Span::new(chars.as_str(), Ink::Text),
                        ]);
                    }
                    used += Text(&ch.to_string()).width();
                    before.push(ch);
                }
                Line::new(vec![
                    lead,
                    Span::new(before, Ink::Text),
                    Span::new(" ", Ink::Text).with(crate::ui::Emphasis::Reversed),
                ])
            })
            .collect()
    }

    /// The tabs, the chosen one marked.
    fn tabs(&self) -> Bar {
        let mut left = Line::default();
        for tab in Tab::ALL {
            let ink = if tab == self.tab {
                Ink::Focus
            } else {
                Ink::Muted
            };
            left.push(Span::plain(" "));
            left.push(Span::new(format!(" {} ", tab.name()), ink));
        }
        let focus = match self.focus {
            Focus::List => "",
            Focus::Session => "j/k turns · enter open/fold ",
        };
        Bar {
            left,
            right: Line::of(focus, Ink::Muted),
            fill: Ink::Pane,
            keep: Keep::Left,
        }
    }

    /// The header: what the session is, how it stands, and a fact or two;
    /// lit by what it waits for.
    fn header(&self, model: &Model, item: Item, now: UnixMillis) -> (Line, Line, Option<Ink>) {
        match item {
            Item::Run(id) => {
                let Some(run) = model.desk.get(id) else {
                    return (Line::of(" run", Ink::Same), Line::default(), None);
                };
                let lit = match run.attention() {
                    Attention::NeedsYou => Some(Ink::WarningTitle),
                    Attention::Working => Some(Ink::LiveTitle),
                    Attention::YourTurn | Attention::Starting | Attention::Ended => None,
                };
                let title = Line::new(vec![
                    Span::plain(format!(" {}", Listing::run_name(model, run))),
                    Span::plain(format!(" · {} · {}", run.client(), run.phase())),
                ]);
                (
                    title,
                    Line::from(Span::plain(format!("{} ", run.workdir))),
                    lit,
                )
            }
            Item::Watched(session) => {
                let (_, name, _, _) = Listing::describe(model, item, now);
                let pane = Self::pane(model, session, now);
                let lit = pane.as_ref().and_then(SessionPane::lit);
                let right = pane
                    .as_ref()
                    .and_then(|pane| pane.facts.as_ref())
                    .map_or_else(String::new, SessionPane::facts_text);
                (
                    Line::from(Span::plain(format!(" {name} · watched"))),
                    Line::from(Span::plain(format!("{right} "))),
                    lit,
                )
            }
        }
    }

    fn pane(model: &Model, session: SessionId, now: UnixMillis) -> Option<SessionPane> {
        model
            .sessions
            .sessions()
            .iter()
            .find(|view| view.id == session)
            .map(|view| SessionPane::of(model, view, now))
    }

    /// A run's turns, oldest first: those recorded, then the one streaming.
    fn turns(run: &Run) -> Vec<Turn> {
        let mut turns: Vec<Turn> = run.history.iter().map(Self::recorded).collect();
        if !run.transcript.is_empty() {
            let blocks = Conversation::live(run.transcript.entries()).blocks;
            let prompt = blocks
                .iter()
                .rev()
                .find_map(|block| {
                    if let Block::Prompt(text) = block {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            let written = blocks
                .iter()
                .filter_map(|block| {
                    if let Block::Step(step) = block
                        && step.kind == crate::conversation::StepKind::Edit
                    {
                        Some(step.target.clone())
                    } else {
                        None
                    }
                })
                .collect();
            turns.push(Turn {
                prompt,
                summary: match run.attention() {
                    Attention::Working | Attention::NeedsYou | Attention::Starting => {
                        "now".to_owned()
                    }
                    Attention::YourTurn | Attention::Ended => "just now".to_owned(),
                },
                blocks,
                files: Files {
                    written,
                    read: Vec::new(),
                },
            });
        }
        turns
    }

    fn recorded(turn: &TurnActivity) -> Turn {
        let blocks = Conversation::turn(turn, &HashMap::new()).blocks;
        let mut summary = Vec::new();
        if let (Some(start), Some(end)) = (turn.started_at, turn.ended_at) {
            summary.push(Elapsed(end.as_millis().saturating_sub(start.as_millis())).to_string());
        }
        if turn.tool_calls > 0 {
            summary.push(format!(
                "{} tool{}",
                turn.tool_calls,
                if turn.tool_calls == 1 { "" } else { "s" }
            ));
        }
        if turn.failed_tool_calls > 0 {
            summary.push(format!("{} failed", turn.failed_tool_calls));
        }
        Turn {
            prompt: turn
                .prompt
                .as_ref()
                .map(|prompt| prompt.text.clone())
                .unwrap_or_default(),
            summary: summary.join(" · "),
            blocks,
            files: Files {
                written: turn.files_written.clone(),
                read: turn.files_read.clone(),
            },
        }
    }

    /// Whether turn `index` of `count` is open: the newest unless folded by
    /// hand, an earlier one if opened by hand.
    fn open(&self, index: usize, count: usize) -> bool {
        (index + 1 == count) != self.toggled.contains(&index)
    }

    fn conversation(&self, model: &Model, item: Item, width: usize, now: UnixMillis) -> Vec<Line> {
        let run = match item {
            Item::Run(id) => model.desk.get(id),
            Item::Watched(session) => {
                return Self::pane(model, session, now).map_or_else(Vec::new, |pane| {
                    let mut lines = pane.conversation().lines(width, Density::Page);
                    lines.push(Line::default());
                    lines.push(pane.state_line(now));
                    lines
                });
            }
        };
        let Some(run) = run else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        if let Some(handoff) = &self.handoff {
            lines.push(Line::of(format!("↳ {handoff}"), Ink::Muted));
        }
        let turns = Self::turns(run);
        let count = turns.len();
        let cursor = (self.focus == Focus::Session).then(|| self.turn.min(count.saturating_sub(1)));
        for (index, turn) in turns.iter().enumerate() {
            let open = self.open(index, count);
            let here = cursor == Some(index);
            lines.push(Line::default());
            let mark = match (open, here) {
                (true, true) => "▼ ",
                (true, false) => "▾ ",
                (false, true) => "▶ ",
                (false, false) => "▸ ",
            };
            let room = width.saturating_sub(Text(&turn.summary).width() + 6);
            let mut head = Line::new(vec![
                Span::new(mark, if here { Ink::Focus } else { Ink::Muted }),
                Span::new(
                    Text(turn.prompt.lines().next().unwrap_or_default()).fit(room),
                    Ink::Prompt,
                ),
            ]);
            if !turn.summary.is_empty() {
                head.push(Span::new(format!("  {}", turn.summary), Ink::Muted));
            }
            lines.push(head);
            if open {
                let blocks: Vec<Block> = turn
                    .blocks
                    .iter()
                    .filter(|block| !matches!(block, Block::Prompt(_)))
                    .cloned()
                    .collect();
                lines.extend(Conversation { blocks }.lines(width, Density::Page));
            }
        }
        if let Some(waiting) = run.waiting() {
            lines.push(Line::default());
            Self::card(&run.client(), &waiting.ask, width, &mut lines);
        }
        if count == 0 && run.waiting().is_none() {
            lines.push(Line::default());
            let text = match run.attention() {
                Attention::Starting => format!("starting {} in {}…", run.client(), run.workdir),
                Attention::YourTurn => format!(
                    "{} is ready in {}. i writes to it.",
                    run.client(),
                    run.workdir
                ),
                Attention::Working | Attention::NeedsYou => "working…".to_owned(),
                Attention::Ended => "this run ended".to_owned(),
            };
            lines.push(Line::of(text, Ink::Muted));
        }
        lines
    }

    /// Every step taken, turn by turn: commands, edits, tools.
    fn activity(&self, model: &Model, item: Item, width: usize, now: UnixMillis) -> Vec<Line> {
        let turns: Vec<(String, Vec<Block>)> = match item {
            Item::Run(id) => model.desk.get(id).map_or_else(Vec::new, |run| {
                Self::turns(run)
                    .into_iter()
                    .map(|turn| (turn.prompt, turn.blocks))
                    .collect()
            }),
            Item::Watched(session) => Self::pane(model, session, now)
                .map_or_else(Vec::new, |pane| {
                    vec![(String::new(), pane.conversation().blocks)]
                }),
        };
        let mut lines = Vec::new();
        for (prompt, blocks) in turns {
            let steps: Vec<Block> = blocks
                .into_iter()
                .filter(|block| {
                    matches!(block, Block::Tools(_) | Block::Step(_) | Block::Result(_))
                })
                .collect();
            if steps.is_empty() {
                continue;
            }
            lines.push(Line::default());
            if !prompt.is_empty() {
                lines.push(Line::of(
                    format!(
                        "› {}",
                        Text(prompt.lines().next().unwrap_or_default()).fit(width)
                    ),
                    Ink::Muted,
                ));
            }
            lines.extend(Conversation { blocks: steps }.lines(width, Density::Pane));
        }
        if lines.is_empty() {
            lines.push(Line::of("no steps yet", Ink::Muted));
        }
        lines
    }

    /// The files written and read, newest turn first.
    fn files(&self, model: &Model, item: Item, now: UnixMillis) -> Vec<Line> {
        let files: Vec<Files> = match item {
            Item::Run(id) => model.desk.get(id).map_or_else(Vec::new, |run| {
                Self::turns(run)
                    .into_iter()
                    .map(|turn| turn.files)
                    .collect()
            }),
            Item::Watched(session) => {
                let calls = model.sessions.session_calls(session);
                crate::model::sessions::Sessions::recent_turns(&calls)
                    .into_iter()
                    .filter_map(|turn| model.sessions.turn(turn))
                    .map(|turn| Files {
                        written: turn.files_written.clone(),
                        read: turn.files_read.clone(),
                    })
                    .collect()
            }
        };
        let _ = now;
        let mut written: Vec<String> = Vec::new();
        let mut read: Vec<String> = Vec::new();
        for turn in files.into_iter().rev() {
            for path in turn.written {
                if !written.contains(&path) {
                    written.push(path);
                }
            }
            for path in turn.read {
                if !read.contains(&path) && !written.contains(&path) {
                    read.push(path);
                }
            }
        }
        let mut lines = vec![
            Line::default(),
            Line::of(format!("WRITTEN  {}", written.len()), Ink::Muted),
        ];
        lines.extend(
            written.into_iter().map(|path| {
                Line::new(vec![Span::new("✎ ", Ink::Tool), Span::new(path, Ink::Code)])
            }),
        );
        lines.push(Line::default());
        lines.push(Line::of(format!("READ  {}", read.len()), Ink::Muted));
        lines.extend(read.into_iter().map(|path| {
            Line::new(vec![
                Span::new("· ", Ink::Muted),
                Span::new(path, Ink::Text),
            ])
        }));
        lines
    }

    /// The session's model calls through the gateway, newest last.
    fn calls(&self, model: &Model, item: Item) -> Vec<Line> {
        let session = match item {
            Item::Watched(session) => Some(session),
            Item::Run(id) => model
                .desk
                .get(id)
                .and_then(|run| run.recorded.as_ref())
                .and_then(|recorded| recorded.session),
        };
        let Some(session) = session else {
            return vec![Line::of("no calls recorded yet", Ink::Muted)];
        };
        let calls = model.sessions.session_calls(session);
        if calls.is_empty() {
            return vec![Line::of("no calls in the last few minutes", Ink::Muted)];
        }
        let mut lines = vec![Line::of(
            "STARTED        MODEL                 ENDED                 TIME      IN      OUT",
            Ink::Muted,
        )];
        for call in calls {
            let end = CallEnd::of(call);
            let metrics = CallMetrics::of(call);
            let time = call.ended_at.map_or_else(
                || "…".to_owned(),
                |ended| {
                    Elapsed(
                        ended
                            .as_millis()
                            .saturating_sub(call.started_at.as_millis()),
                    )
                    .to_string()
                },
            );
            let tokens = |tokens: Option<u64>| {
                tokens.map_or_else(|| "—".to_owned(), |tokens| Kilo(tokens).to_string())
            };
            lines.push(Line::new(vec![
                Span::new(format!("{}   ", Clock(call.started_at)), Ink::Muted),
                Span::new(
                    format!(
                        "{:<21} ",
                        Text(call.model.as_deref().unwrap_or("—")).fit(21)
                    ),
                    Ink::Text,
                ),
                Span::new(
                    format!("{:<21} ", Text(&end.label(call.status)).fit(21)),
                    Ink::of(end.tone()),
                ),
                Span::new(
                    format!(
                        "{time:>6}  {:>6}  {:>6}",
                        tokens(metrics.context_tokens),
                        tokens(metrics.output_tokens)
                    ),
                    Ink::Muted,
                ),
            ]));
        }
        lines
    }

    fn wrap(text: &str, width: usize) -> Vec<String> {
        Text(text).wrap(width.max(8))
    }

    /// The approval card: what the client wants, and the four answers.
    fn card(client: &str, ask: &ApprovalAsk, width: usize, lines: &mut Vec<Line>) {
        let (verb, detail, reason) = match ask {
            ApprovalAsk::Command {
                command,
                cwd,
                reason,
            } => (
                "run".to_owned(),
                command.clone().unwrap_or_else(|| "a command".to_owned()),
                match (reason, cwd) {
                    (Some(reason), _) => Some(reason.clone()),
                    (None, Some(cwd)) => Some(format!("in {cwd}")),
                    (None, None) => None,
                },
            ),
            ApprovalAsk::FileChange { reason } => (
                "change files".to_owned(),
                reason.clone().unwrap_or_else(|| "files".to_owned()),
                None,
            ),
            ApprovalAsk::Tool { tool, detail } => (
                format!("use {tool}"),
                detail.clone().unwrap_or_default(),
                None,
            ),
        };
        let inner = width.saturating_sub(4).clamp(20, 88);
        let title = format!("┌ {client} wants to {verb} ");
        let fill = (inner + 3).saturating_sub(Text(&title).width());
        lines.push(Line::of(
            format!("  {title}{}┐", "─".repeat(fill)),
            Ink::Warning,
        ));
        let row = |text: &str, ink: Ink| -> Line {
            let pad = inner.saturating_sub(Text(text).width());
            Line::new(vec![
                Span::new("  │ ", Ink::Warning),
                Span::new(format!("{text}{}", " ".repeat(pad)), ink),
                Span::new(" │", Ink::Warning),
            ])
        };
        for text in Self::wrap(&detail, inner) {
            lines.push(row(&text, Ink::Code));
        }
        if let Some(reason) = reason {
            for text in Self::wrap(&reason, inner) {
                lines.push(row(&text, Ink::Muted));
            }
        }
        lines.push(row("", Ink::Text));
        let choices = "y allow once   a allow for this run   d deny   D deny and stop the turn";
        for text in Self::wrap(choices, inner) {
            lines.push(row(&text, Ink::Focus));
        }
        lines.push(Line::of(
            format!("  └{}┘", "─".repeat(inner + 2)),
            Ink::Warning,
        ));
    }
}
