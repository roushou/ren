//! One session full screen, in the frame: a run Ren drives, whose message
//! box sends to it, or a session watched from the wall, read the same way
//! and shown the same way, whose box only takes commands. A run's approval
//! is answered with 1–4, and answering moves on to the next run that waits.

use super::frame::{Frame, Here};
use super::message::Typed;
use super::slash::Slash;
use super::wall::SessionPane;
use super::{Action, Ctx, Look, News, Notice, Screen};
use crate::Key;
use crate::conversation::{Conversation, Density};
use crate::keys::{Bind, HelpRow, Keymap};
use crate::model::desk::{Attention, Run};
use crate::model::{Command, Control, Model};
use crate::ui::{Anchor, Bar, Ink, Keep, Line, Lines, Rule, Size, Span, Text, View};
use ren_harness::drive::{ApprovalAsk, ApprovalId, Decision, Prompt};
use ren_protocol::label::Labeled;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::{RunId, SessionId};
use ren_supervisor::RunState;
use std::collections::HashMap;

/// Which session is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subject {
    /// A run Ren drives.
    Run(RunId),
    /// A session started outside Ren, watched.
    Watched(SessionId),
}

/// One session, open.
#[derive(Debug)]
pub(crate) struct SessionScreen {
    subject: Subject,
    /// Lines scrolled up from the newest.
    scroll: usize,
    /// What happened to the run just left, shown atop this one.
    handoff: Option<String>,
    /// Whether a run's recorded turns were asked for.
    asked: bool,
    /// The sessions' revision a watched session's reads were asked at.
    read_at: Option<u64>,
}

/// What a session screen's own keys do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionKey {
    Answer,
    Send,
    NewLine,
    Complete,
    Back,
    PreviousPlace,
    NextPlace,
    PageUp,
    PageDown,
    LineUp,
    LineDown,
    Quit,
}

impl SessionKey {
    const KEYS: Keymap<Self> = Keymap(&[
        Bind {
            keys: &[
                Key::Char('1'),
                Key::Char('2'),
                Key::Char('3'),
                Key::Char('4'),
            ],
            label: "1–4",
            does: "answer an approval: allow, allow for the run, deny, deny and stop",
            command: Self::Answer,
        },
        Bind {
            keys: &[Key::Enter],
            label: "enter",
            does: "send; while a turn runs, it waits for the turn to end",
            command: Self::Send,
        },
        Bind {
            keys: &[Key::Newline],
            label: "ctrl-j",
            does: "a new line",
            command: Self::NewLine,
        },
        Bind {
            keys: &[Key::Tab],
            label: "tab",
            does: "complete a /command",
            command: Self::Complete,
        },
        Bind {
            keys: &[Key::Esc],
            label: "esc",
            does: "interrupt the turn; with none running, back to the wall",
            command: Self::Back,
        },
        Bind {
            keys: &[Key::AltLeft],
            label: "alt-←",
            does: "the place before: the wall, then each live run",
            command: Self::PreviousPlace,
        },
        Bind {
            keys: &[Key::AltRight],
            label: "alt-→",
            does: "the next place",
            command: Self::NextPlace,
        },
        Bind {
            keys: &[Key::PageUp],
            label: "pgup",
            does: "scroll up a page",
            command: Self::PageUp,
        },
        Bind {
            keys: &[Key::PageDown],
            label: "pgdn",
            does: "scroll down a page",
            command: Self::PageDown,
        },
        Bind {
            keys: &[Key::Up],
            label: "↑",
            does: "scroll up a line",
            command: Self::LineUp,
        },
        Bind {
            keys: &[Key::Down],
            label: "↓",
            does: "scroll down a line",
            command: Self::LineDown,
        },
        Bind {
            keys: &[Key::Interrupt],
            label: "ctrl-c",
            does: "clear what is typed, or quit (runs keep going)",
            command: Self::Quit,
        },
    ]);
}

impl SessionScreen {
    fn new(subject: Subject) -> Self {
        Self {
            subject,
            scroll: 0,
            handoff: None,
            asked: false,
            read_at: None,
        }
    }

    pub fn run(run: RunId) -> Self {
        Self::new(Subject::Run(run))
    }

    pub fn watching(session: SessionId) -> Self {
        Self::new(Subject::Watched(session))
    }

    /// `run`, with what happened to the run left for it.
    pub fn after(run: RunId, handoff: String) -> Self {
        Self {
            handoff: Some(handoff),
            ..Self::run(run)
        }
    }

    /// The session it watches, if it watches one.
    #[cfg(test)]
    pub fn watched(&self) -> Option<SessionId> {
        match self.subject {
            Subject::Watched(session) => Some(session),
            Subject::Run(_) => None,
        }
    }

    /// The run it shows, if it shows one.
    pub fn shown_run(&self) -> Option<RunId> {
        match self.subject {
            Subject::Run(run) => Some(run),
            Subject::Watched(_) => None,
        }
    }

    /// Its keys, for the help.
    pub fn keys() -> Vec<HelpRow> {
        SessionKey::KEYS.help()
    }

    /// Shown: what it shows is read.
    pub fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        self.scroll = 0;
        match self.subject {
            Subject::Run(run) => {
                self.asked = Self::session_known(ctx.model, run);
                vec![Action::Read(ctx.model.history(run, false))]
            }
            Subject::Watched(_) => {
                self.read_at = None;
                self.changed(ctx, &[])
            }
        }
    }

    fn session_known(model: &Model, run: RunId) -> bool {
        model
            .desk
            .get(run)
            .and_then(|run| run.recorded.as_ref())
            .is_some_and(|recorded| recorded.session.is_some())
    }

    pub fn changed(&mut self, ctx: &mut Ctx<'_>, news: &[News]) -> Vec<Action> {
        if let Some(News::Started(run)) = news.first() {
            return vec![Action::Replace(Screen::Session(Self::run(*run)))];
        }
        match self.subject {
            Subject::Run(run) => {
                // Its recorded turns, once its session is known.
                if !self.asked && Self::session_known(ctx.model, run) {
                    self.asked = true;
                    let empty = ctx
                        .model
                        .desk
                        .get(run)
                        .is_some_and(|run| run.history.is_empty());
                    if empty {
                        return vec![Action::Read(ctx.model.history(run, false))];
                    }
                }
                Vec::new()
            }
            Subject::Watched(session) => {
                let revision = ctx.model.sessions.revision();
                if self.read_at == Some(revision) {
                    return Vec::new();
                }
                self.read_at = Some(revision);
                ctx.model
                    .sessions
                    .reads_for(session)
                    .into_iter()
                    .map(Action::Read)
                    .collect()
            }
        }
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        // An approval waiting takes 1-4 while nothing is typed.
        if let (Key::Char(digit @ '1'..='4'), true, Some(run)) =
            (key, ctx.message.is_empty(), self.shown_run())
            && let Some((approval, _)) = ctx.model.desk.get(run).and_then(Run::approval)
        {
            let decision = match digit {
                '1' => Decision::Accept,
                '2' => Decision::AcceptForSession,
                '3' => Decision::Decline,
                _ => Decision::Cancel,
            };
            return Self::answer(ctx, run, approval, decision, None);
        }
        let key = match ctx.message.press(key, ctx.model.projects()) {
            Typed::Took => return Vec::new(),
            Typed::Submit => return self.submit(ctx),
            Typed::Left(key) => key,
        };
        let page = usize::from(ctx.size.height.saturating_sub(8).max(1));
        match SessionKey::KEYS.find(key) {
            Some(SessionKey::Back) => return self.back(ctx.model),
            Some(SessionKey::PreviousPlace) => return vec![Action::Step { forward: false }],
            Some(SessionKey::NextPlace) => return vec![Action::Step { forward: true }],
            Some(SessionKey::PageUp) => self.scroll = self.scroll.saturating_add(page),
            Some(SessionKey::PageDown) => self.scroll = self.scroll.saturating_sub(page),
            Some(SessionKey::LineUp) => self.scroll = self.scroll.saturating_add(1),
            Some(SessionKey::LineDown) => self.scroll = self.scroll.saturating_sub(1),
            Some(SessionKey::Quit) => return vec![Action::Quit],
            Some(
                SessionKey::Answer | SessionKey::Send | SessionKey::NewLine | SessionKey::Complete,
            )
            | None => {}
        }
        Vec::new()
    }

    /// Esc interrupts a running turn, as in the clients themselves; with
    /// none running, it goes back to the wall.
    fn back(&self, model: &Model) -> Vec<Action> {
        match self.shown_run().and_then(|run| model.desk.get(run)) {
            Some(run) => match run.attention() {
                Attention::Working | Attention::NeedsYou => {
                    vec![Action::Send(Command::Interrupt(run.id))]
                }
                Attention::YourTurn | Attention::Starting | Attention::Ended => {
                    vec![Action::Wall]
                }
            },
            None => vec![Action::Wall],
        }
    }

    fn submit(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let text = ctx.message.text();
        if text.trim().is_empty() {
            return Vec::new();
        }
        if let Some(slash) = Slash::parse(&text) {
            ctx.message.take();
            return slash.act(ctx, self.shown_run());
        }
        let Some(id) = self.shown_run() else {
            return vec![Action::Say(Notice::info(
                "Ren did not start this session, so it cannot send to it · /new starts a run",
            ))];
        };
        if let Control::Unavailable(why) = &ctx.model.control {
            return vec![Action::Say(Notice::refusal(format!(
                "runs cannot be driven from here: {why}"
            )))];
        }
        let Some(run) = ctx.model.desk.get(id) else {
            return Vec::new();
        };
        let Ok(prompt) = Prompt::parse(text) else {
            return Vec::new();
        };
        let approval = run.approval();
        let attention = run.attention();
        let resume = match (run.harness(), run.thread()) {
            (Some(harness), Some(thread)) => Some((
                harness,
                run.workdir.clone(),
                thread
                    .parse::<uuid::Uuid>()
                    .ok()
                    .and_then(NativeThreadId::new),
            )),
            (Some(_) | None, Some(_) | None) => None,
        };
        ctx.message.take();
        // Typing while an approval waits declines it; the words go next.
        if let Some((approval, _)) = approval {
            return Self::answer(ctx, id, approval, Decision::Decline, Some(prompt));
        }
        match attention {
            Attention::YourTurn => {
                self.scroll = 0;
                vec![Action::Send(ctx.model.prompt(id, prompt))]
            }
            Attention::Working | Attention::Starting | Attention::NeedsYou => {
                ctx.model.queue(id, prompt);
                Vec::new()
            }
            // A message to a run that ended continues its conversation in a
            // new run.
            Attention::Ended => match resume {
                Some((harness, dir, resume)) => vec![Action::Send(ctx.model.start(
                    harness,
                    dir,
                    resume,
                    Some(prompt),
                ))],
                None => vec![Action::Say(Notice::info(
                    "this run ended before it opened a conversation: /new starts another",
                ))],
            },
        }
    }

    fn answer(
        ctx: &mut Ctx<'_>,
        id: RunId,
        approval: ApprovalId,
        decision: Decision,
        then: Option<Prompt>,
    ) -> Vec<Action> {
        let Some(run) = ctx.model.desk.get(id) else {
            return Vec::new();
        };
        let what = Run::asked(run.approval().map(|(_, ask)| ask).as_ref());
        let left = run.title();
        let project = run.project().to_owned();
        // The run waits for nothing else: move on to the next that does.
        let more = match &run.live {
            Some(RunState::Working { approvals, .. }) => approvals.len() > 1,
            Some(_) | None => false,
        };
        let mut actions = vec![Action::Send(Command::Answer {
            run: id,
            approval,
            decision,
        })];
        if let Some(prompt) = then {
            ctx.model.queue(id, prompt);
        }
        if !more && let Some(next) = ctx.model.desk.next_needing(Some(id)) {
            let verb = match decision {
                Decision::Accept | Decision::AcceptForSession => "allowed",
                Decision::Decline => "denied",
                Decision::Cancel => "denied and stopped",
            };
            actions.push(Action::Replace(Screen::Session(Self::after(
                next,
                format!("from {project} · {left}: {verb} {what}"),
            ))));
        }
        actions
    }

    /// Where it is, as the places line names it.
    pub fn here(&self, model: &Model) -> Here {
        match self.subject {
            Subject::Run(run) => Here::Run(run),
            Subject::Watched(session) => Here::Watching(Self::watched_name(model, session)),
        }
    }

    fn watched_name(model: &Model, session: SessionId) -> String {
        let id = session.to_string();
        let short = id.get(..8).unwrap_or(&id).to_owned();
        let client = model
            .sessions
            .sessions()
            .iter()
            .find(|view| view.id == session)
            .and_then(|view| view.source.known())
            .map(|source| source.harness().label().replace('_', "-"));
        match client {
            Some(client) => format!("{client} {short}"),
            None => short,
        }
    }

    pub fn view(&self, look: &Look<'_>) -> View {
        let room = Frame::body(look);
        let here = self.here(look.model);
        match self.subject {
            Subject::Run(id) => {
                let (body, status, placeholder) = match look.model.desk.get(id) {
                    Some(run) => (
                        self.run_body(run, room),
                        Frame::trouble(look).unwrap_or_else(|| Self::keys_line(run)),
                        Self::placeholder(run),
                    ),
                    None => (
                        View::Empty,
                        Line::default(),
                        "/new <project> <prompt> starts a run".to_owned(),
                    ),
                };
                Frame::new(look, &here, body, status, &placeholder).view()
            }
            Subject::Watched(session) => {
                let view = look
                    .model
                    .sessions
                    .sessions()
                    .iter()
                    .find(|view| view.id == session);
                let body = match view {
                    Some(view) => SessionPane::of(look.model, view, look.now).view(
                        None,
                        false,
                        room,
                        look.now,
                        Density::Page,
                    ),
                    None => View::lines(
                        vec![Line::of(
                            "this session is no longer on the wall",
                            Ink::Muted,
                        )],
                        Anchor::Middle,
                    ),
                };
                Frame::new(
                    look,
                    &here,
                    body,
                    Line::of(" watching · esc back to the wall", Ink::Muted),
                    "Watching: Ren did not start this session · /new starts a run",
                )
                .view()
            }
        }
    }

    /// A run's body: a title bar lit by what it waits for, then its
    /// conversation.
    fn run_body(&self, run: &Run, room: crate::Size) -> View {
        let lit = match run.attention() {
            Attention::NeedsYou => Some(Ink::WarningTitle),
            Attention::Working => Some(Ink::LiveTitle),
            Attention::YourTurn | Attention::Starting | Attention::Ended => None,
        };
        let title = Bar {
            left: Line::new(vec![
                Span::plain(format!(" {}", run.project())),
                Span::plain(format!(" · {} · ", run.client())),
                Span::plain(run.phase()),
            ]),
            right: Line::from(Span::plain(format!("{} ", Text(&run.title()).summary(40)))),
            fill: lit.unwrap_or(Ink::Muted),
            keep: Keep::Left,
        };
        let width = usize::from(room.width);
        View::fill(
            Ink::Pane,
            View::Rows(vec![
                (Size::Exact(1), View::Bar(title)),
                (
                    Size::Exact(1),
                    View::Rule(match lit {
                        Some(ink) => Rule::Edge(ink),
                        None => Rule::Plain,
                    }),
                ),
                (
                    Size::Fill,
                    View::Lines(Lines {
                        lines: Self::conversation(
                            run,
                            self.handoff.as_deref(),
                            width.saturating_sub(4),
                        ),
                        anchor: Anchor::Newest,
                        scroll: self.scroll,
                    }),
                ),
            ]),
        )
    }

    fn placeholder(run: &Run) -> String {
        match run.attention() {
            Attention::NeedsYou => "1–4 answers, or type to reply instead".to_owned(),
            Attention::Working | Attention::Starting => {
                format!("Message {}: it reads it when this turn ends", run.client())
            }
            Attention::YourTurn => format!("Message {}…", run.client()),
            Attention::Ended => "Type to continue this conversation in a new run".to_owned(),
        }
    }

    fn keys_line(run: &Run) -> Line {
        match (&run.queued, run.attention()) {
            (Some(queued), _) => Line::of(
                format!(
                    " queued: {} · sends when this turn ends",
                    Text(queued.as_str()).summary(40)
                ),
                Ink::Muted,
            ),
            (None, Attention::NeedsYou) => Line::of(" 1–4 answers · esc interrupts", Ink::Muted),
            (None, Attention::Working | Attention::Starting) => {
                Line::of(" esc interrupts", Ink::Muted)
            }
            (None, Attention::YourTurn | Attention::Ended) => Line::of(
                " enter sends · ctrl-j new line · esc back to the wall",
                Ink::Muted,
            ),
        }
    }

    /// A run's conversation: what happened to the run left for it, its
    /// recorded turns, then what streamed since, then what it waits for.
    pub fn conversation(run: &Run, handoff: Option<&str>, width: usize) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        if let Some(handoff) = handoff {
            lines.push(Line::of(format!("  ↳ {handoff}"), Ink::Muted));
        }
        let mut conversation = Conversation::default();
        for turn in &run.history {
            conversation
                .blocks
                .extend(Conversation::turn(turn, &HashMap::new()).blocks);
        }
        conversation
            .blocks
            .extend(Conversation::live(run.transcript.entries()).blocks);
        lines.extend(conversation.lines(width, Density::Page));
        if let Some(waiting) = run.waiting() {
            lines.push(Line::default());
            Self::card(&run.client(), &waiting.ask, width, &mut lines);
        }
        if run.history.is_empty() && run.transcript.is_empty() && run.waiting().is_none() {
            lines.push(Line::default());
            let text = match run.attention() {
                Attention::Starting => format!("starting {} in {}…", run.client(), run.workdir),
                Attention::YourTurn => {
                    format!(
                        "{} is ready in {}. What should it do?",
                        run.client(),
                        run.workdir
                    )
                }
                Attention::Working | Attention::NeedsYou => "working…".to_owned(),
                Attention::Ended => "this run ended".to_owned(),
            };
            lines.push(Line::of(format!("  {text}"), Ink::Muted));
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
        let choices = "1 Allow once   2 Allow for this run   3 Deny   4 Deny and stop the turn";
        for text in Self::wrap(choices, inner) {
            lines.push(row(&text, Ink::Focus));
        }
        lines.push(Line::of(
            format!("  └{}┘", "─".repeat(inner + 2)),
            Ink::Warning,
        ));
    }
}
