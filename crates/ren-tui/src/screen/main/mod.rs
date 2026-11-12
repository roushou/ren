//! The one screen: the session list on the left, the selected session on
//! the right, a top bar with the traffic, and a key bar at the bottom.
//!
//! Keys act; they do not type. j and k (or the arrows) move through the
//! list, or through a session's turns once the session has the focus; a
//! digit jumps to an entry; tab changes what the session shows. `i` opens
//! the message box to write to a run, `n` starts one, and an approval is
//! answered with `y`, `a`, `d`, or `D`. Only the message box, the search,
//! and the new-run form take typed text, and Esc leaves each of them.

mod detail;
mod list;
pub(crate) mod watched;
#[cfg(test)]
mod watched_tests;

pub(crate) use list::{Grouping, Item, Listing};

use super::message::Typed;
use super::new_run::NewRun;
use super::{Action, Ctx, Look, News, Notice, Screen};
use crate::Key;
use crate::keys::{Bind, HelpRow, Keymap};
use crate::model::desk::{Attention, Run};
use crate::model::{Command, Control, Model};
use crate::ui::{Anchor, Ink, Joint, Keep, Line, Lines, Rule, Size, Span, View};
use ren_harness::drive::{ApprovalId, Decision, Prompt};
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::RunId;
use ren_supervisor::RunState;

/// The session list and the selected session.
#[derive(Debug, Default)]
pub(crate) struct Main {
    selected: Option<Item>,
    focus: Focus,
    tab: Tab,
    mode: Mode,
    /// Narrows the list, by name, project, or title.
    filter: String,
    /// How the list is grouped.
    grouping: Grouping,
    /// The turn the cursor is on, once the session has the focus: an index
    /// into its turns, oldest first.
    turn: usize,
    /// Turns opened or closed by hand: the newest is open unless closed.
    toggled: Vec<usize>,
    /// Lines scrolled up from the newest.
    scroll: usize,
    /// What happened to the run just left, shown atop the next.
    handoff: Option<String>,
    /// The entry whose record was last asked for, and the sessions'
    /// revision it was asked at.
    read: Option<(Item, u64, bool)>,
    /// Whether the operator chose the selection; until then it follows
    /// the top of the list, what needs you first.
    chosen: bool,
    /// Ctrl-C was pressed once: pressed again, right away, it quits.
    leaving: bool,
}

/// Which side has the keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Focus {
    #[default]
    List,
    Session,
}

/// What the selected session shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Conversation,
    Activity,
    Files,
    Calls,
}

/// What typed text goes to, if anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Keys act.
    #[default]
    Normal,
    /// Writing to the selected run.
    Writing,
    /// Narrowing the list.
    Searching,
}

/// What a key does while keys act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Down,
    Up,
    IntoSession,
    IntoList,
    NextTab,
    PreviousTab,
    Write,
    NewRun,
    Allow,
    AllowForRun,
    Deny,
    DenyAndStop,
    Interrupt,
    Stop,
    Search,
    Group,
    PageUp,
    PageDown,
    Theme,
    Help,
    Quit,
    /// Ctrl-C: quits when pressed twice in a row, so a stray press does not.
    Leave,
}

impl Act {
    const KEYS: Keymap<Self> = Keymap(&[
        Bind {
            keys: &[Key::Char('j'), Key::Down],
            label: "j ↓",
            does: "next session, or next turn in a session",
            command: Self::Down,
        },
        Bind {
            keys: &[Key::Char('k'), Key::Up],
            label: "k ↑",
            does: "previous session, or previous turn",
            command: Self::Up,
        },
        Bind {
            keys: &[Key::Char('l'), Key::Right, Key::Enter],
            label: "l → enter",
            does: "into the session; on a turn, open or fold it",
            command: Self::IntoSession,
        },
        Bind {
            keys: &[Key::Char('h'), Key::Left, Key::Esc],
            label: "h ← esc",
            does: "back to the list",
            command: Self::IntoList,
        },
        Bind {
            keys: &[Key::Tab],
            label: "tab",
            does: "conversation, activity, files, calls",
            command: Self::NextTab,
        },
        Bind {
            keys: &[Key::BackTab],
            label: "shift-tab",
            does: "the tab before",
            command: Self::PreviousTab,
        },
        Bind {
            keys: &[Key::Char('i')],
            label: "i",
            does: "write to the run (enter sends, esc keeps a draft)",
            command: Self::Write,
        },
        Bind {
            keys: &[Key::Char('n')],
            label: "n",
            does: "start a run",
            command: Self::NewRun,
        },
        Bind {
            keys: &[Key::Char('y')],
            label: "y",
            does: "allow what the run asks, once",
            command: Self::Allow,
        },
        Bind {
            keys: &[Key::Char('a')],
            label: "a",
            does: "allow it for the rest of the run",
            command: Self::AllowForRun,
        },
        Bind {
            keys: &[Key::Char('d')],
            label: "d",
            does: "deny it",
            command: Self::Deny,
        },
        Bind {
            keys: &[Key::Char('D')],
            label: "D",
            does: "deny it and stop the turn",
            command: Self::DenyAndStop,
        },
        Bind {
            keys: &[Key::Char('x')],
            label: "x",
            does: "interrupt the turn",
            command: Self::Interrupt,
        },
        Bind {
            keys: &[Key::Char('S')],
            label: "S",
            does: "stop the run's process",
            command: Self::Stop,
        },
        Bind {
            keys: &[Key::Char('/')],
            label: "/",
            does: "search the sessions (esc clears)",
            command: Self::Search,
        },
        Bind {
            keys: &[Key::Char('g')],
            label: "g",
            does: "group the list by folder, or by what each needs",
            command: Self::Group,
        },
        Bind {
            keys: &[Key::PageUp],
            label: "pgup",
            does: "scroll the session up",
            command: Self::PageUp,
        },
        Bind {
            keys: &[Key::PageDown],
            label: "pgdn",
            does: "scroll it down",
            command: Self::PageDown,
        },
        Bind {
            keys: &[Key::Char('t')],
            label: "t",
            does: "next theme: terminal, dark, light (not saved)",
            command: Self::Theme,
        },
        Bind {
            keys: &[Key::Char('?')],
            label: "?",
            does: "these keys",
            command: Self::Help,
        },
        Bind {
            keys: &[Key::Char('q')],
            label: "q",
            does: "quit (runs keep going)",
            command: Self::Quit,
        },
        Bind {
            keys: &[Key::Interrupt],
            label: "ctrl-c ×2",
            does: "quit, pressed twice in a row",
            command: Self::Leave,
        },
    ]);
}

impl Main {
    /// The keys, for the help: the key table, then the digits and the
    /// message box.
    pub fn keys() -> Vec<HelpRow> {
        let mut rows = Act::KEYS.help();
        for (label, does) in [
            ("1–9", "jump to that session in the list"),
            (
                "ctrl-p/n",
                "while writing: the messages sent before and after",
            ),
        ] {
            rows.push(HelpRow {
                label: label.to_owned(),
                does: does.to_owned(),
            });
        }
        rows
    }

    pub fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        self.changed(ctx, &[])
    }

    /// The model changed: the selection kept on a listed entry, and what
    /// the selected entry shows read.
    pub fn changed(&mut self, ctx: &mut Ctx<'_>, news: &[News]) -> Vec<Action> {
        if let Some(News::Started(run)) = news.first() {
            self.select(Item::Run(*run));
            self.focus = Focus::Session;
        }
        let listing = Listing::of(ctx.model, &self.filter, self.grouping);
        let lost = self
            .selected
            .is_none_or(|item| listing.position(item).is_none());
        if lost || !self.chosen {
            let first = listing.items().first().copied();
            if first != self.selected {
                self.selected = first;
                self.reset_session();
            }
        }
        self.reads(ctx.model)
    }

    /// What the selected entry shows, read once it changed.
    fn reads(&mut self, model: &Model) -> Vec<Action> {
        let Some(item) = self.selected else {
            return Vec::new();
        };
        let revision = model.sessions.revision();
        match item {
            Item::Run(run) => {
                let known = Self::session_known(model, run);
                let asked = matches!(self.read, Some((read, _, true)) if read == item);
                let asked_unknown = matches!(self.read, Some((read, _, false)) if read == item);
                if asked || (asked_unknown && !known) {
                    return Vec::new();
                }
                self.read = Some((item, revision, known));
                vec![Action::Read(model.history(run, false))]
            }
            Item::Watched(session) => {
                if self.read == Some((item, revision, true)) {
                    return Vec::new();
                }
                self.read = Some((item, revision, true));
                model
                    .sessions
                    .reads_for(session)
                    .into_iter()
                    .map(Action::Read)
                    .collect()
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

    /// The operator's choice of entry.
    fn select(&mut self, item: Item) {
        self.chosen = true;
        if self.selected != Some(item) {
            self.selected = Some(item);
            self.reset_session();
        }
    }

    fn reset_session(&mut self) {
        self.turn = usize::MAX;
        self.toggled.clear();
        self.scroll = 0;
        self.handoff = None;
    }

    /// The selected run, if a run is selected.
    pub fn run(&self) -> Option<RunId> {
        match self.selected {
            Some(Item::Run(run)) => Some(run),
            Some(Item::Watched(_)) | None => None,
        }
    }

    #[cfg(test)]
    pub fn selected(&self) -> Option<Item> {
        self.selected
    }

    #[cfg(test)]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        // Ctrl-C quits only when pressed again right away.
        let leaving = std::mem::take(&mut self.leaving);
        if key == Key::Interrupt && self.mode == Mode::Normal {
            if leaving {
                return vec![Action::Quit];
            }
            self.leaving = true;
            return vec![Action::Say(Notice::info(
                "press ctrl-c again to quit · runs keep going",
            ))];
        }
        match self.mode {
            Mode::Writing => self.write(key, ctx),
            Mode::Searching => self.search(key, ctx),
            Mode::Normal => {
                if let Key::Char(digit @ '1'..='9') = key {
                    let index = usize::from(u8::try_from(digit).unwrap_or(b'1') - b'1');
                    if let Some(item) = Listing::of(ctx.model, &self.filter, self.grouping)
                        .items()
                        .get(index)
                    {
                        self.select(*item);
                        return self.reads(ctx.model);
                    }
                    return Vec::new();
                }
                match Act::KEYS.find(key) {
                    Some(act) => self.act(act, ctx),
                    None => Vec::new(),
                }
            }
        }
    }

    fn act(&mut self, act: Act, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let page = usize::from(ctx.size.height.saturating_sub(8).max(1));
        match act {
            Act::Down | Act::Up => {
                let forward = act == Act::Down;
                match self.focus {
                    Focus::List => return self.step(ctx.model, forward),
                    Focus::Session => self.step_turn(ctx.model, forward),
                }
            }
            Act::IntoSession => match self.focus {
                Focus::List => self.focus = Focus::Session,
                Focus::Session => self.toggle_turn(ctx.model),
            },
            Act::IntoList => self.focus = Focus::List,
            Act::NextTab => self.tab = self.tab.next(),
            Act::PreviousTab => self.tab = self.tab.previous(),
            Act::Write => return self.open_writing(ctx.model),
            Act::NewRun => {
                return vec![Action::Push(Screen::NewRun(NewRun::new(ctx.model.client)))];
            }
            Act::Allow => return self.answer(ctx, Decision::Accept, None),
            Act::AllowForRun => return self.answer(ctx, Decision::AcceptForSession, None),
            Act::Deny => return self.answer(ctx, Decision::Decline, None),
            Act::DenyAndStop => return self.answer(ctx, Decision::Cancel, None),
            Act::Interrupt => return self.interrupt(ctx.model),
            Act::Stop => {
                return match self.live_run(ctx.model) {
                    Some(run) => vec![Action::Send(Command::Stop(run))],
                    None => vec![Action::Say(Notice::info("no live run selected to stop"))],
                };
            }
            Act::Search => {
                self.mode = Mode::Searching;
                self.focus = Focus::List;
            }
            Act::Group => {
                self.grouping = match self.grouping {
                    Grouping::Folder => Grouping::State,
                    Grouping::State => Grouping::Folder,
                };
                return vec![Action::Say(Notice::info(match self.grouping {
                    Grouping::Folder => "grouped by folder · g groups by what each needs",
                    Grouping::State => "grouped by what each needs · g groups by folder",
                }))];
            }
            Act::PageUp => self.scroll = self.scroll.saturating_add(page),
            Act::PageDown => self.scroll = self.scroll.saturating_sub(page),
            Act::Theme => return vec![Action::CycleTheme],
            Act::Help => return vec![Action::Help],
            Act::Quit => return vec![Action::Quit],
            // Taken in `press`, before any key is acted on.
            Act::Leave => {}
        }
        Vec::new()
    }

    /// The next (or previous) entry of the list.
    fn step(&mut self, model: &Model, forward: bool) -> Vec<Action> {
        let items = Listing::of(model, &self.filter, self.grouping).items();
        if items.is_empty() {
            return Vec::new();
        }
        let at = self
            .selected
            .and_then(|item| items.iter().position(|row| *row == item));
        let next = match (at, forward) {
            (None, _) => 0,
            (Some(at), true) => (at + 1).min(items.len() - 1),
            (Some(at), false) => at.saturating_sub(1),
        };
        if let Some(item) = items.get(next) {
            self.select(*item);
        }
        self.reads(model)
    }

    fn step_turn(&mut self, model: &Model, forward: bool) {
        let count = self.turn_count(model);
        if count == 0 {
            return;
        }
        let at = self.turn.min(count - 1);
        self.turn = if forward {
            (at + 1).min(count - 1)
        } else {
            at.saturating_sub(1)
        };
    }

    fn toggle_turn(&mut self, model: &Model) {
        let count = self.turn_count(model);
        if count == 0 {
            return;
        }
        let at = self.turn.min(count - 1);
        match self.toggled.iter().position(|turn| *turn == at) {
            Some(index) => {
                self.toggled.remove(index);
            }
            None => self.toggled.push(at),
        }
    }

    /// The live run selected, if one is.
    fn live_run(&self, model: &Model) -> Option<RunId> {
        let run = model.desk.get(self.run()?)?;
        match run.attention() {
            Attention::Ended => None,
            Attention::NeedsYou
            | Attention::YourTurn
            | Attention::Working
            | Attention::Starting => Some(run.id),
        }
    }

    fn interrupt(&self, model: &Model) -> Vec<Action> {
        match self.run().and_then(|run| model.desk.get(run)) {
            Some(run) => match run.attention() {
                Attention::Working | Attention::NeedsYou => {
                    vec![Action::Send(Command::Interrupt(run.id))]
                }
                Attention::YourTurn | Attention::Starting | Attention::Ended => {
                    vec![Action::Say(Notice::info("no turn is running"))]
                }
            },
            None => vec![Action::Say(Notice::info("select a run to interrupt"))],
        }
    }

    fn open_writing(&mut self, model: &Model) -> Vec<Action> {
        match self.selected {
            Some(Item::Run(_)) => {
                if let Control::Unavailable(why) = &model.control {
                    return vec![Action::Say(Notice::refusal(format!(
                        "runs cannot be driven from here: {why}"
                    )))];
                }
                self.mode = Mode::Writing;
                self.focus = Focus::Session;
                Vec::new()
            }
            Some(Item::Watched(_)) => vec![Action::Say(Notice::info(
                "Ren did not start this session, so it cannot write to it · n starts a run",
            ))],
            None => vec![Action::Say(Notice::info("n starts a run"))],
        }
    }

    fn search(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match key {
            Key::Char(ch) => self.filter.push(ch),
            Key::Backspace => {
                self.filter.pop();
            }
            Key::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
            }
            Key::Enter | Key::Down | Key::Up => {
                self.mode = Mode::Normal;
                if key != Key::Enter {
                    return self.act(if key == Key::Down { Act::Down } else { Act::Up }, ctx);
                }
            }
            Key::Interrupt => {
                self.filter.clear();
                self.mode = Mode::Normal;
            }
            Key::Tab
            | Key::BackTab
            | Key::Left
            | Key::Right
            | Key::Delete
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Newline
            | Key::Older
            | Key::Newer => {}
        }
        // The selection stays on a match.
        self.changed(ctx, &[])
    }

    fn write(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match ctx.message.press(key) {
            Typed::Took => Vec::new(),
            Typed::Submit => {
                let actions = self.send(ctx);
                self.mode = Mode::Normal;
                actions
            }
            Typed::Left(Key::Esc) => {
                // What is typed stays, as a draft.
                self.mode = Mode::Normal;
                Vec::new()
            }
            Typed::Left(Key::Interrupt) => {
                if ctx.message.is_empty() {
                    self.mode = Mode::Normal;
                } else {
                    ctx.message.take();
                }
                Vec::new()
            }
            Typed::Left(Key::PageUp) => {
                self.scroll = self.scroll.saturating_add(8);
                Vec::new()
            }
            Typed::Left(Key::PageDown) => {
                self.scroll = self.scroll.saturating_sub(8);
                Vec::new()
            }
            Typed::Left(_) => Vec::new(),
        }
    }

    /// What is written, to the selected run: sent now, kept until the turn
    /// ends, or (to a run that ended) a new run continuing it.
    fn send(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let Some(id) = self.run() else {
            return Vec::new();
        };
        let Some(run) = ctx.model.desk.get(id) else {
            return Vec::new();
        };
        let Ok(prompt) = Prompt::parse(ctx.message.text()) else {
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
        ctx.message.send();
        self.scroll = 0;
        // Writing while an approval waits declines it; the words go next.
        if approval.is_some() {
            return self.answer(ctx, Decision::Decline, Some(prompt));
        }
        match attention {
            Attention::YourTurn => vec![Action::Send(ctx.model.prompt(id, prompt))],
            Attention::Working | Attention::Starting | Attention::NeedsYou => {
                ctx.model.queue(id, prompt);
                vec![Action::Say(Notice::info(
                    "kept: it is sent when this turn ends",
                ))]
            }
            Attention::Ended => match resume {
                Some((harness, dir, resume)) => vec![Action::Send(ctx.model.start(
                    harness,
                    dir,
                    resume,
                    Some(prompt),
                ))],
                None => vec![Action::Say(Notice::info(
                    "this run ended before it opened a conversation: n starts another",
                ))],
            },
        }
    }

    /// Answers what the selected run asks; with nothing else waiting on
    /// it, moves on to the next run that needs you.
    fn answer(
        &mut self,
        ctx: &mut Ctx<'_>,
        decision: Decision,
        then: Option<Prompt>,
    ) -> Vec<Action> {
        let Some((id, approval)) = self.waiting(ctx.model) else {
            return vec![Action::Say(Notice::info(
                "nothing waits for an answer here",
            ))];
        };
        let Some(run) = ctx.model.desk.get(id) else {
            return Vec::new();
        };
        let what = Run::asked(run.approval().map(|(_, ask)| ask).as_ref());
        let name = Listing::run_name(ctx.model, run);
        let more = match &run.live {
            Some(RunState::Working { approvals, .. }) => approvals.len() > 1,
            Some(_) | None => false,
        };
        if let Some(prompt) = then {
            ctx.model.queue(id, prompt);
        }
        let actions = vec![Action::Send(Command::Answer {
            run: id,
            approval,
            decision,
        })];
        if !more && let Some(next) = ctx.model.desk.next_needing(Some(id)) {
            let verb = match decision {
                Decision::Accept | Decision::AcceptForSession => "allowed",
                Decision::Decline => "denied",
                Decision::Cancel => "denied and stopped",
            };
            self.select(Item::Run(next));
            self.handoff = Some(format!("from {name}: {verb} {what}"));
            let mut actions = actions;
            actions.extend(self.reads(ctx.model));
            return actions;
        }
        actions
    }

    fn waiting(&self, model: &Model) -> Option<(RunId, ApprovalId)> {
        let run = model.desk.get(self.run()?)?;
        run.approval().map(|(approval, _)| (run.id, approval))
    }

    pub fn view(&self, look: &Look<'_>) -> View {
        let width = look.size.width;
        let side = (width / 4).clamp(24, 36).min(width / 2);
        let listing = Listing::of(look.model, &self.filter, self.grouping);
        let mut list = listing.lines(look.model, self.selected, usize::from(side), look.now);
        if self.mode == Mode::Searching || !self.filter.is_empty() {
            list.insert(
                0,
                Line::new(vec![
                    Span::new(" / ", Ink::Prompt),
                    Span::new(format!("{}▏", self.filter), Ink::Text),
                ]),
            );
        }
        let list_view = View::Lines(Lines {
            lines: list,
            anchor: Anchor::Top,
            scroll: 0,
        });
        let detail = self.detail(look, width.saturating_sub(side + 1));
        View::fill(
            Ink::Ground,
            View::Rows(vec![
                (Size::Exact(1), View::Bar(self.top_bar(look))),
                // The list and the session are framed above and below, the
                // rule between them meeting both.
                (
                    Size::Exact(1),
                    View::Rule(Rule::Joined {
                        at: side,
                        joint: Joint::Above,
                    }),
                ),
                (
                    Size::Fill,
                    View::Columns(vec![(Size::Exact(side), list_view), (Size::Fill, detail)]),
                ),
                (
                    Size::Exact(1),
                    View::Rule(Rule::Joined {
                        at: side,
                        joint: Joint::Below,
                    }),
                ),
                (Size::Exact(1), View::Bar(self.key_bar(look))),
            ]),
        )
    }

    /// The key bar: a notice, or the keys that matter here and now.
    fn key_bar(&self, look: &Look<'_>) -> crate::ui::Bar {
        let right = Line::of(
            format!(" ? keys │ {} ", detail::Clock(look.now)),
            Ink::Muted,
        );
        let left = match look.notice {
            Some(notice) => Line::of(
                format!(" {}", notice.text),
                match notice.tone {
                    super::NoticeTone::Info => Ink::Muted,
                    super::NoticeTone::Refusal => Ink::Error,
                },
            ),
            None => Line::of(format!(" {}", self.hints(look.model)), Ink::Muted),
        };
        crate::ui::Bar {
            left,
            right,
            fill: Ink::Bar,
            keep: Keep::Right,
        }
    }

    fn hints(&self, model: &Model) -> &'static str {
        match self.mode {
            Mode::Writing => {
                "enter sends · ctrl-j new line · ctrl-p/n earlier messages · esc keeps a draft"
            }
            Mode::Searching => "type to search · enter keeps it · esc clears",
            Mode::Normal => match self.waiting(model) {
                Some(_) => {
                    "y allow · a allow for the run · d deny · D deny and stop · i reply instead"
                }
                None => match (self.focus, self.selected) {
                    (_, None) => "n start a run · ? keys",
                    (Focus::List, Some(_)) => {
                        "j/k sessions · enter open · i write · n new run · / search · g group"
                    }
                    (Focus::Session, Some(_)) => {
                        "j/k turns · enter open or fold · tab views · i write · esc list"
                    }
                },
            },
        }
    }

    /// The top bar: Ren, the gateway's traffic, and how many runs need you.
    fn top_bar(&self, look: &Look<'_>) -> crate::ui::Bar {
        let traffic = look.model.sessions.traffic(look.now);
        let mut left = Line::new(vec![
            Span::new(" ren ", Ink::LiveTitle).with(crate::ui::Emphasis::Bold),
            Span::plain(format!(
                "  {} session{} · {} streaming · {} waiting",
                traffic.sessions,
                if traffic.sessions == 1 { "" } else { "s" },
                traffic.responding,
                traffic.awaiting,
            )),
        ]);
        // Each client's output over the last minutes, on one scale.
        let merged: Vec<(ren_harness::Harness, Vec<u64>)> = traffic
            .output
            .iter()
            .map(|spark| {
                let bins = spark
                    .bins
                    .chunks(3)
                    .map(|chunk| chunk.iter().sum())
                    .collect();
                (spark.harness, bins)
            })
            .collect();
        let peak = merged
            .iter()
            .flat_map(|(_, bins)| bins.iter().copied())
            .max()
            .unwrap_or(0);
        for (harness, bins) in &merged {
            let total: u64 = bins.iter().sum();
            left.push(Span::new(
                format!(
                    "  {} {} {} tok",
                    watched::SessionPane::short(*harness),
                    watched::SessionPane::spark(bins, peak),
                    watched::Kilo(total)
                ),
                Ink::Client(Some(*harness)),
            ));
        }
        if let Some(failure) = &traffic.failure {
            left.push(Span::new(format!("  ! stale: {failure} "), Ink::ErrorTitle));
        }
        let desk = &look.model.desk;
        let needing = desk.needing();
        let live = desk.ordered().len();
        let right = match needing {
            0 => Line::of(format!("{live} live ",), Ink::Muted),
            n => Line::new(vec![
                Span::new(
                    format!(" {n} need{} you ", if n == 1 { "s" } else { "" }),
                    Ink::WarningTitle,
                ),
                Span::new(format!(" {live} live "), Ink::Muted),
            ]),
        };
        crate::ui::Bar {
            left,
            right,
            fill: Ink::Bar,
            keep: Keep::Right,
        }
    }

    /// The turns of the selected session.
    fn turn_count(&self, model: &Model) -> usize {
        match self.selected {
            Some(Item::Run(run)) => model.desk.get(run).map_or(0, |run| {
                run.history.len() + usize::from(!run.transcript.is_empty())
            }),
            Some(Item::Watched(_)) | None => 0,
        }
    }
}

impl Tab {
    const ALL: [Self; 4] = [Self::Conversation, Self::Activity, Self::Files, Self::Calls];

    fn next(self) -> Self {
        let at = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::ALL
            .get((at + 1) % Self::ALL.len())
            .copied()
            .unwrap_or(self)
    }

    fn previous(self) -> Self {
        let at = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::ALL
            .get((at + Self::ALL.len() - 1) % Self::ALL.len())
            .copied()
            .unwrap_or(self)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Activity => "Activity",
            Self::Files => "Files",
            Self::Calls => "Calls",
        }
    }
}
