//! One run's conversation fills the screen, with the message box under it.
//! Typing replies; an approval is answered with 1–4, and answering one
//! moves on to the next run that waits for you.

use super::chrome::Chrome;
use super::message::Typed;
use super::slash::Slash;
use super::{Action, Ctx, Look, News, Notice, Screen};
use crate::Key;
use crate::keys::{Bind, HelpRow, Keymap};
use crate::model::desk::{Attention, Run};
use crate::model::transcript::Entry;
use crate::model::{Command, Control, Model};
use crate::ui::{Ink, Line, Span, Text, View};
use ren_harness::drive::{ApprovalAsk, ApprovalId, Decision, ItemStatus, Prompt, TurnEnd};
use ren_protocol::activity::ResultStatus;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::RunId;
use ren_store::activity::TurnActivity;
use ren_supervisor::RunState;

/// One run, open.
#[derive(Debug)]
pub(crate) struct RunScreen {
    run: RunId,
    /// Lines scrolled up from the newest.
    scroll: usize,
    /// What happened to the run just left, shown atop this one.
    handoff: Option<String>,
    /// Whether its recorded turns were asked for.
    asked: bool,
}

/// What a run screen's own keys do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunKey {
    Answer,
    Send,
    NewLine,
    Complete,
    Interrupt,
    Previous,
    Next,
    PageUp,
    PageDown,
    LineUp,
    LineDown,
    Quit,
}

impl RunKey {
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
            does: "interrupt the turn",
            command: Self::Interrupt,
        },
        Bind {
            keys: &[Key::AltLeft],
            label: "alt-←",
            does: "the run before, what needs you first",
            command: Self::Previous,
        },
        Bind {
            keys: &[Key::AltRight],
            label: "alt-→",
            does: "the next run",
            command: Self::Next,
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

impl RunScreen {
    pub fn new(run: RunId) -> Self {
        Self {
            run,
            scroll: 0,
            handoff: None,
            asked: false,
        }
    }

    /// `run`, with what happened to the run left for it.
    pub fn after(run: RunId, handoff: String) -> Self {
        Self {
            handoff: Some(handoff),
            ..Self::new(run)
        }
    }

    #[cfg(test)]
    pub fn run(&self) -> RunId {
        self.run
    }

    /// The console's keys and commands, for the help.
    pub fn help() -> Vec<HelpRow> {
        let mut rows = RunKey::KEYS.help();
        rows.extend(Slash::commands().map(|(command, does)| HelpRow {
            label: format!("/{command}"),
            does: does.to_owned(),
        }));
        rows
    }

    /// Shown: its recorded turns are read.
    pub fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        self.scroll = 0;
        self.asked = Self::session_known(ctx.model, self.run);
        vec![Action::Read(ctx.model.history(self.run, false))]
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
            return vec![Action::Replace(Screen::Run(Self::new(*run)))];
        }
        // Its recorded turns, once its session is known.
        if !self.asked && Self::session_known(ctx.model, self.run) {
            self.asked = true;
            let empty = ctx
                .model
                .desk
                .get(self.run)
                .is_some_and(|run| run.history.is_empty());
            if empty {
                return vec![Action::Read(ctx.model.history(self.run, false))];
            }
        }
        Vec::new()
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        // An approval waiting takes 1-4 while nothing is typed.
        if let (Key::Char(digit @ '1'..='4'), true) = (key, ctx.message.is_empty())
            && let Some((approval, _)) = ctx.model.desk.get(self.run).and_then(Run::approval)
        {
            let decision = match digit {
                '1' => Decision::Accept,
                '2' => Decision::AcceptForSession,
                '3' => Decision::Decline,
                _ => Decision::Cancel,
            };
            return self.answer(ctx, approval, decision, None);
        }
        let key = match ctx.message.press(key, ctx.model.projects()) {
            Typed::Took => return Vec::new(),
            Typed::Submit => return self.submit(ctx),
            Typed::Left(key) => key,
        };
        let page = usize::from(ctx.size.height.saturating_sub(8).max(1));
        match RunKey::KEYS.find(key) {
            Some(RunKey::Interrupt) => return self.interrupt(ctx.model),
            Some(RunKey::Previous) => return Self::step(ctx.model, Some(self.run), false),
            Some(RunKey::Next) => return Self::step(ctx.model, Some(self.run), true),
            Some(RunKey::PageUp) => self.scroll = self.scroll.saturating_add(page),
            Some(RunKey::PageDown) => self.scroll = self.scroll.saturating_sub(page),
            Some(RunKey::LineUp) => self.scroll = self.scroll.saturating_add(1),
            Some(RunKey::LineDown) => self.scroll = self.scroll.saturating_sub(1),
            Some(RunKey::Quit) => return vec![Action::Quit],
            Some(RunKey::Answer | RunKey::Send | RunKey::NewLine | RunKey::Complete) | None => {}
        }
        Vec::new()
    }

    /// The live run after (or before) `from`, what needs you first.
    pub fn step(model: &Model, from: Option<RunId>, forward: bool) -> Vec<Action> {
        match model.desk.step(from, forward) {
            Some(run) => vec![Action::Replace(Screen::Run(Self::new(run)))],
            None => vec![Action::Say(Notice::info("no live runs: /new starts one"))],
        }
    }

    /// Esc interrupts the running turn, as in the clients themselves.
    fn interrupt(&self, model: &Model) -> Vec<Action> {
        match model.desk.get(self.run).map(Run::attention) {
            Some(Attention::Working | Attention::NeedsYou) => {
                vec![Action::Send(Command::Interrupt(self.run))]
            }
            Some(Attention::YourTurn | Attention::Starting | Attention::Ended) | None => Vec::new(),
        }
    }

    fn submit(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let text = ctx.message.text();
        if text.trim().is_empty() {
            return Vec::new();
        }
        if let Some(slash) = Slash::parse(&text) {
            ctx.message.take();
            return slash.act(ctx, Some(self.run));
        }
        if let Control::Unavailable(why) = &ctx.model.control {
            return vec![Action::Say(Notice::refusal(format!(
                "runs cannot be driven from here: {why}"
            )))];
        }
        let Some(run) = ctx.model.desk.get(self.run) else {
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
            return self.answer(ctx, approval, Decision::Decline, Some(prompt));
        }
        match attention {
            Attention::YourTurn => {
                self.scroll = 0;
                vec![Action::Send(ctx.model.prompt(self.run, prompt))]
            }
            Attention::Working | Attention::Starting | Attention::NeedsYou => {
                ctx.model.queue(self.run, prompt);
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
        &mut self,
        ctx: &mut Ctx<'_>,
        approval: ApprovalId,
        decision: Decision,
        then: Option<Prompt>,
    ) -> Vec<Action> {
        let Some(run) = ctx.model.desk.get(self.run) else {
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
            run: self.run,
            approval,
            decision,
        })];
        if let Some(prompt) = then {
            ctx.model.queue(self.run, prompt);
        }
        if !more && let Some(next) = ctx.model.desk.next_needing(Some(self.run)) {
            let verb = match decision {
                Decision::Accept | Decision::AcceptForSession => "allowed",
                Decision::Decline => "denied",
                Decision::Cancel => "denied and stopped",
            };
            actions.push(Action::Replace(Screen::Run(Self::after(
                next,
                format!("from {project} · {left}: {verb} {what}"),
            ))));
        }
        actions
    }

    pub fn view(&self, look: &Look<'_>) -> View {
        let width = usize::from(look.size.width);
        let Some(run) = look.model.desk.get(self.run) else {
            return Chrome::new(
                look,
                Line::of(" run", Ink::Text),
                Vec::new(),
                Some(self.run),
                Self::idle_placeholder(),
                Line::default(),
            )
            .view();
        };
        let mut chrome = Chrome::new(
            look,
            Self::title(run),
            Self::conversation(run, self.handoff.as_deref(), width.saturating_sub(4)),
            Some(self.run),
            Self::placeholder(run),
            Self::keys(run),
        );
        chrome.scroll = self.scroll;
        chrome.view()
    }

    fn title(run: &Run) -> Line {
        let ink = match run.attention() {
            Attention::NeedsYou => Ink::Warning,
            Attention::YourTurn | Attention::Working | Attention::Starting => Ink::Text,
            Attention::Ended => Ink::Muted,
        };
        Line::new(vec![
            Span::new(format!(" {}", run.project()), Ink::Text),
            Span::new(format!(" · {} · ", run.client()), Ink::Muted),
            Span::new(run.phase(), ink),
        ])
    }

    pub fn idle_placeholder() -> String {
        "/new <project> <prompt> starts a run · /help".to_owned()
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

    fn keys(run: &Run) -> Line {
        match (&run.queued, run.attention()) {
            (Some(queued), _) => Line::of(
                format!(
                    " queued: {} · sends when this turn ends",
                    Text(queued.as_str()).summary(40)
                ),
                Ink::Muted,
            ),
            (None, Attention::NeedsYou) => Line::of(" 1–4 answers", Ink::Muted),
            (None, Attention::Working | Attention::Starting) => {
                Line::of(" esc interrupts", Ink::Muted)
            }
            (None, Attention::YourTurn | Attention::Ended) => {
                Line::of(" enter sends · ctrl-j new line", Ink::Muted)
            }
        }
    }

    /// A run's conversation: what happened to the run left for it, its
    /// recorded turns, then what streamed since, then what it waits for.
    pub fn conversation(run: &Run, handoff: Option<&str>, width: usize) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        if let Some(handoff) = handoff {
            lines.push(Line::of(format!("  ↳ {handoff}"), Ink::Muted));
        }
        for turn in &run.history {
            lines.push(Line::default());
            Self::recorded_turn(turn, width, &mut lines);
        }
        for entry in run.transcript.entries() {
            Self::entry(entry, width, &mut lines);
        }
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

    fn recorded_turn(turn: &TurnActivity, width: usize, lines: &mut Vec<Line>) {
        if let Some(prompt) = &turn.prompt {
            for (index, text) in Self::wrap(&prompt.text, width.saturating_sub(4))
                .into_iter()
                .enumerate()
            {
                let lead = if index == 0 { "  › " } else { "    " };
                lines.push(Line::of(format!("{lead}{text}"), Ink::Prompt));
            }
        }
        for call in &turn.calls {
            for tool in &call.tool_calls {
                let target = tool
                    .effects
                    .first()
                    .map_or_else(|| tool.input.text.clone(), |effect| effect.target.clone());
                let failed = tool
                    .result
                    .as_ref()
                    .is_some_and(|result| result.status.known() == Some(ResultStatus::Failed));
                Self::tool_line(
                    "●",
                    &tool.name,
                    &target,
                    if failed { "failed" } else { "" },
                    width,
                    lines,
                );
            }
            if let Some(reply) = &call.reply
                && !reply.text.trim().is_empty()
            {
                for text in Self::wrap(&reply.text, width.saturating_sub(4)) {
                    lines.push(Line::of(format!("    {text}"), Ink::Text));
                }
            }
        }
    }

    fn tool_line(
        mark: &str,
        name: &str,
        target: &str,
        end: &str,
        width: usize,
        lines: &mut Vec<Line>,
    ) {
        let name = format!("  {mark} {name:<6} ");
        let room = width.saturating_sub(Text(&name).width() + Text(end).width() + 2);
        let target = Text(target.lines().next().unwrap_or_default()).summary(room);
        let gap =
            width.saturating_sub(Text(&name).width() + Text(&target).width() + Text(end).width());
        lines.push(Line::new(vec![
            Span::new(name, Ink::Tool),
            Span::new(target, Ink::Code),
            Span::new(format!("{}{end}", " ".repeat(gap)), Ink::Muted),
        ]));
    }

    fn entry(entry: &Entry, width: usize, lines: &mut Vec<Line>) {
        let ended = |status: &ItemStatus, running: &'static str| -> String {
            match status {
                ItemStatus::InProgress => running.to_owned(),
                ItemStatus::Completed => String::new(),
                ItemStatus::Failed => "failed".to_owned(),
                ItemStatus::Declined => "denied".to_owned(),
                ItemStatus::Other(other) => other.clone(),
            }
        };
        match entry {
            Entry::Prompt(text) => {
                lines.push(Line::default());
                for (index, text) in Self::wrap(text, width.saturating_sub(4))
                    .into_iter()
                    .enumerate()
                {
                    let lead = if index == 0 { "  › " } else { "    " };
                    lines.push(Line::of(format!("{lead}{text}"), Ink::Prompt));
                }
            }
            Entry::Message { text, done, .. } => {
                let mut wrapped = Self::wrap(text, width.saturating_sub(4));
                if !done && let Some(last) = wrapped.last_mut() {
                    last.push('▍');
                }
                for text in wrapped {
                    lines.push(Line::of(format!("    {text}"), Ink::Text));
                }
            }
            Entry::Reasoning { done: false, .. } => {
                lines.push(Line::of("    thinking…", Ink::Muted));
            }
            Entry::Reasoning { done: true, .. } => {}
            Entry::Command {
                command,
                status,
                output,
                exit_code,
                ..
            } => {
                let end = match (status, exit_code) {
                    (ItemStatus::InProgress, _) => "running…".to_owned(),
                    (_, Some(code)) => format!("exit {code}"),
                    (ItemStatus::Completed, None) => "done".to_owned(),
                    (ItemStatus::Failed, None) => "failed".to_owned(),
                    (ItemStatus::Declined, None) => "denied".to_owned(),
                    (ItemStatus::Other(other), None) => other.clone(),
                };
                Self::tool_line("●", "Bash", command, &end, width, lines);
                // A failed command shows the end of its output.
                let failed =
                    matches!(status, ItemStatus::Failed) || exit_code.is_some_and(|code| code != 0);
                if failed && let Some(output) = output {
                    let tail: Vec<&str> = output.lines().rev().take(3).collect();
                    for text in tail.into_iter().rev() {
                        lines.push(Line::of(
                            format!(
                                "           {}",
                                Text(text).summary(width.saturating_sub(12))
                            ),
                            Ink::Muted,
                        ));
                    }
                }
            }
            Entry::FileChange { paths, status, .. } => {
                let target = if paths.is_empty() {
                    "files".to_owned()
                } else {
                    paths.join(", ")
                };
                Self::tool_line(
                    "✎",
                    "Edit",
                    &target,
                    &ended(status, "editing…"),
                    width,
                    lines,
                );
            }
            Entry::Tool { name, status, .. } => {
                let (tool, target) = name.split_once(' ').unwrap_or((name.as_str(), ""));
                Self::tool_line("●", tool, target, &ended(status, "running…"), width, lines);
            }
            Entry::Notice { text, retrying } => {
                let (text, ink) = if *retrying {
                    (format!("    ! {text} · retrying"), Ink::Warning)
                } else {
                    (format!("    ! {text}"), Ink::Error)
                };
                lines.push(Line::of(text, ink));
            }
            Entry::TurnEnd(end) => match end {
                TurnEnd::Completed => {}
                TurnEnd::Interrupted => lines.push(Line::of("    turn interrupted", Ink::Muted)),
                TurnEnd::Failed { message } => lines.push(Line::of(
                    format!(
                        "    turn failed{}",
                        message
                            .as_ref()
                            .map_or_else(String::new, |message| format!(": {message}"))
                    ),
                    Ink::Error,
                )),
                TurnEnd::Unknown { status } => {
                    lines.push(Line::of(format!("    turn ended: {status}"), Ink::Muted));
                }
            },
        }
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
