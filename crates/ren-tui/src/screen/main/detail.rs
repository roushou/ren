//! The selected session: a header saying what it is and how it stands,
//! its tabs, and what the chosen tab shows. A run's conversation is its
//! turns, the newest open and each earlier one folded to a line; a session
//! watched is shown as it is read from its calls.

use super::list::{Item, Listing};
use super::recording::{Elapsed, Kilo, RecordingPane};
use super::{Focus, Main, Mode, Tab};
use crate::conversation::{Block, Conversation, Density};
use crate::model::desk::{Attention, Run};
use crate::model::{CallEnd, Model};
use crate::screen::Look;
use crate::screen::offer::{Asks, Offering};
use crate::ui::{Anchor, Bar, Ink, Keep, Line, Lines, Rule, Size, Span, Text, View};
use ren_harness::drive::ApprovalAsk;
use ren_recorder::id::RecordingId;
use ren_recorder::time::UnixMillis;
use ren_store::activity::report::CallMetrics;

/// One turn of a run: recorded, or the one streaming now.
struct Turn {
    prompt: String,
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
    /// The cells the message box's text wraps to, in a detail `width` wide:
    /// less its lead and a margin.
    pub(super) fn message_width(width: u16) -> usize {
        usize::from(width).saturating_sub(4)
    }

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
            // A plain rule: the header's own color says the state.
            (Size::Exact(1), View::Rule(Rule::Plain)),
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
        // What waits for the turn to end, one line: `e` in the action menu
        // takes it back.
        if let Some(queued) = self
            .run()
            .and_then(|run| model.desk.get(run))
            .and_then(|run| run.queued())
        {
            let first = queued.as_str().lines().next().unwrap_or_default();
            rows.push((Size::Exact(1), View::Rule(Rule::Plain)));
            rows.push((
                Size::Exact(1),
                View::lines(
                    vec![Line::new(vec![
                        Span::new(" ⏎ after this turn  ", Ink::Muted),
                        Span::new(first.to_owned(), Ink::Text),
                    ])],
                    Anchor::Top,
                ),
            ));
        }
        // A draft shows under the run it was written to.
        let drafted = !look.message.is_empty() && look.message.owner() == self.run();
        if self.mode == Mode::Writing || drafted {
            // The box grows with what is written, up to half the screen,
            // then scrolls to keep the cursor in view.
            let wrapped = look.message.wrap(Self::message_width(width));
            let most = usize::from(look.size.height / 2).max(3);
            let shown = wrapped.rows.len().clamp(1, most);
            let writing = self.mode == Mode::Writing;
            let first = wrapped.window(shown).start;
            if writing {
                rows.extend(self.offers(look, width));
            }
            rows.push((Size::Exact(1), View::Rule(Rule::Plain)));
            rows.push((
                Size::Exact(u16::try_from(shown).unwrap_or(1)),
                View::lines(
                    wrapped.lines(shown, writing, |row| {
                        let lead = match (row == first, writing) {
                            (true, true) => " › ",
                            (true, false) => " ✎ ",
                            (false, true | false) => "   ",
                        };
                        Span::new(lead, Ink::Prompt)
                    }),
                    Anchor::Top,
                ),
            ));
        }
        View::fill(Ink::Pane, View::Rows(rows))
    }

    /// The menu of what the word at the cursor asks for: a heading, then
    /// a window of offers around the lit one, each with what it does.
    fn offers(&self, look: &Look<'_>, width: u16) -> Vec<(Size, View)> {
        const SHOWN: usize = 6;
        let (Some(run), Some(mention)) = (self.run(), self.offering.open(look.message)) else {
            return Vec::new();
        };
        let offers = Offering::offers(look.model, run, &mention);
        let heading = match mention.asks {
            Asks::Commands => "commands",
            Asks::Skills => "skills",
            Asks::Files => "files in the run's folder",
        };
        let mut lines = vec![Line::new(vec![
            Span::new(format!(" {heading}"), Ink::Muted),
            Span::new("  ↑↓ choose · tab or enter insert · esc close", Ink::Muted),
        ])];
        if offers.is_empty() {
            lines.push(Line::of(
                format!("   {}", Offering::note(look.model, run, &mention)),
                Ink::Muted,
            ));
        }
        let active = self.offering.active.min(offers.len().saturating_sub(1));
        let first = (active + 1).saturating_sub(SHOWN);
        let cells = usize::from(width).saturating_sub(4);
        for (index, offer) in offers.iter().enumerate().skip(first).take(SHOWN) {
            let lit = index == active;
            let mut line = Line::new(vec![Span::new(
                if lit { " › " } else { "   " },
                if lit { Ink::Focus } else { Ink::Muted },
            )]);
            for (at, ch) in offer.label.chars().enumerate() {
                let ink = if offer.hits.contains(&at) {
                    Ink::Focus
                } else {
                    Ink::Prompt
                };
                line.push(Span::new(ch.to_string(), ink));
            }
            if let Some(hint) = &offer.hint {
                line.push(Span::new(format!(" {hint}"), Ink::Muted));
            }
            if let Some(tag) = offer.tag {
                line.push(Span::new(format!("  {tag}"), Ink::Muted));
            }
            if !offer.detail.is_empty() {
                let room = cells.saturating_sub(line.width() + 2);
                let detail: String = offer.detail.chars().take(room).collect();
                if !detail.is_empty() {
                    line.push(Span::new(format!("  {detail}"), Ink::Muted));
                }
            }
            lines.push(line);
        }
        vec![
            (Size::Exact(1), View::Rule(Rule::Plain)),
            (
                Size::Exact(u16::try_from(lines.len()).unwrap_or(1)),
                View::lines(lines, Anchor::Top),
            ),
        ]
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
            Focus::Session => "↑↓ scroll · pgup pgdn page ",
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
                // Its name, then how it stands in words; how it is set up
                // and where it works on the right, quieter.
                let client = match run.setup() {
                    Some(setup) => format!("{} · {setup}", run.client()),
                    None => run.client(),
                };
                let title = Line::new(vec![
                    Span::plain(format!(" {}", Listing::run_name(model, run)))
                        .with(crate::ui::Emphasis::Bold),
                    Span::plain(format!("  {}", run.status())),
                ]);
                (
                    title,
                    Line::from(Span::new(
                        format!("{client} · {} ", model.place(&run.workdir)),
                        Ink::Muted,
                    )),
                    lit,
                )
            }
            Item::Recording(session) => {
                let (_, name, _, _) = Listing::describe(model, item, now);
                let pane = Self::pane(model, session, now);
                let lit = pane.as_ref().and_then(RecordingPane::lit);
                let right = pane
                    .as_ref()
                    .and_then(|pane| pane.facts.as_ref())
                    .map_or_else(String::new, RecordingPane::facts_text);
                (
                    Line::from(Span::plain(format!(" {name} · watched"))),
                    Line::from(Span::plain(format!("{right} "))),
                    lit,
                )
            }
        }
    }

    fn pane(model: &Model, session: RecordingId, now: UnixMillis) -> Option<RecordingPane> {
        model
            .recordings
            .recordings()
            .iter()
            .find(|view| view.id == session)
            .map(|view| RecordingPane::of(model, view, now))
    }

    /// A run's turns, oldest first, from its whole conversation: each
    /// starts at its prompt.
    fn turns(run: &Run) -> Vec<Turn> {
        let blocks = Conversation::live(run.transcript.entries()).blocks;
        let mut turns: Vec<Turn> = Vec::new();
        for block in blocks {
            if let Block::Prompt(text) = &block {
                turns.push(Turn {
                    prompt: text.clone(),
                    blocks: Vec::new(),
                    files: Files::default(),
                });
            }
            let turn = match turns.last_mut() {
                Some(turn) => turn,
                None => {
                    turns.push(Turn {
                        prompt: String::new(),
                        blocks: Vec::new(),
                        files: Files::default(),
                    });
                    let at = turns.len() - 1;
                    &mut turns[at]
                }
            };
            if let Block::Step(step) = &block
                && step.kind == crate::conversation::StepKind::Edit
                && !turn.files.written.contains(&step.target)
            {
                turn.files.written.push(step.target.clone());
            }
            turn.blocks.push(block);
        }
        turns
    }

    fn conversation(&self, model: &Model, item: Item, width: usize, now: UnixMillis) -> Vec<Line> {
        let run = match item {
            Item::Run(id) => model.desk.get(id),
            Item::Recording(session) => {
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
        if run.transcript.earlier() {
            lines.push(Line::of(
                "earlier turns are not shown: the conversation is longer than one read",
                Ink::Muted,
            ));
        }
        // The whole conversation, every turn open, as one scroll.
        let blocks = Conversation::live(run.transcript.entries()).blocks;
        let count = blocks.len();
        lines.extend(Conversation { blocks }.lines(width, Density::Page));
        if let Some(waiting) = run.waiting() {
            lines.push(Line::default());
            Self::card(&run.client(), &waiting.ask, width, &mut lines);
        }
        if count == 0 && run.waiting().is_none() && run.transcript.is_read() {
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
            Item::Recording(session) => Self::pane(model, session, now)
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
            Item::Recording(session) => {
                let calls = model.recordings.recording_calls(session);
                crate::model::recordings::Recordings::recent_turns(&calls)
                    .into_iter()
                    .filter_map(|turn| model.recordings.turn(turn))
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
            Item::Recording(session) => Some(session),
            Item::Run(id) => model
                .desk
                .get(id)
                .and_then(|run| run.recorded.as_ref())
                .and_then(|recorded| recorded.session),
        };
        let Some(session) = session else {
            return vec![Line::of("no calls recorded yet", Ink::Muted)];
        };
        let calls = model.recordings.recording_calls(session);
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
        let choices = "y allow once   a allow for this run   d deny   space more";
        for text in Self::wrap(choices, inner) {
            lines.push(row(&text, Ink::Focus));
        }
        lines.push(Line::of(
            format!("  └{}┘", "─".repeat(inner + 2)),
            Ink::Warning,
        ));
    }
}
