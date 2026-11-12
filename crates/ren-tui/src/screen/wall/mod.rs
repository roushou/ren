//! The wall: every live session's traffic, one pane per session, each
//! showing its newest turns as its calls stream. It is the first place on
//! the places line and always under every other screen.
//!
//! [`Wall`] holds only how the wall is arranged: which session has which
//! pane, the focused pane, and pins. Enter opens the focused session full
//! screen; typing goes to the message box, as everywhere.

mod grid;
mod keys;
mod pane;
mod runs;
#[cfg(test)]
mod tests;
mod view;

pub use grid::Grid;
pub use keys::Command;
pub use pane::SessionPane;
pub use runs::RunCounts;

use super::frame::{Frame, Here};
use super::message::Typed;
use super::session::SessionScreen;
use super::slash::Slash;
use super::{Action, Ctx, Look, Notice, Screen};
use crate::keys::HelpRow;
use crate::model::sessions::{Reading, Sessions, Spark};
use crate::model::{Control, Model, Read};
use crate::{Key, Size};
use grid::Placement;
use ren_recorder::capture::CallState;
use ren_recorder::id::SessionId;
use ren_recorder::time::UnixMillis;
use ren_store::view::{CallView, SessionView};

/// How the wall is arranged.
#[derive(Debug)]
pub struct Wall {
    /// The room the grid was made for.
    size: Size,
    grid: Grid,
    placement: Placement,
    /// Sessions that want a pane, most wanted first, as last arranged.
    wanted: Vec<SessionId>,
    /// Wanted sessions with no pane.
    hidden: usize,
    /// The focused pane's place, always within the grid.
    focus: usize,
    /// Sessions kept on the wall, first, even when they go quiet.
    pins: Vec<SessionId>,
    /// What it was last arranged from: the sessions' revision, the room,
    /// and the threads live runs hold.
    arranged: Option<(u64, Size, Vec<String>)>,
}

/// What the wall shows at one time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panes {
    /// The room the panes have.
    pub size: Size,
    pub grid: Grid,
    /// In pane order: across, then down.
    pub panes: Vec<Pane>,
    pub focus: usize,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pane {
    /// No session to show: the next one to start appears here.
    Free,
    Session(Box<SessionPane>),
}

/// The status line's facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub now: UnixMillis,
    pub reading: Reading,
    /// Why the last read failed: what is shown is from an earlier one.
    pub failure: Option<String>,
    /// Sessions active in the window.
    pub sessions: usize,
    /// Calls with a response streaming, and calls waiting for its head.
    pub responding: usize,
    pub awaiting: usize,
    /// Active sessions with no pane.
    pub hidden: usize,
    /// Sessions on the wall with something worth a look.
    pub attention: usize,
    /// Output tokens of finished calls, per client, over the last few
    /// minutes. Measured when each call ends: not a live rate.
    pub output: Vec<Spark>,
    /// Live runs, once read.
    pub runs: Option<RunCounts>,
}

impl Wall {
    pub fn new(size: Size) -> Self {
        Self {
            size,
            grid: Grid::for_sessions(size, 0),
            placement: Placement::default(),
            wanted: Vec::new(),
            hidden: 0,
            focus: 0,
            pins: Vec::new(),
            arranged: None,
        }
    }

    pub(crate) fn keys(&self) -> Vec<HelpRow> {
        Command::KEYS.help()
    }

    /// Shown: arranged afresh, with a new snapshot.
    pub(crate) fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        self.arranged = None;
        let mut actions = vec![Action::Read(Read::Snapshot)];
        actions.extend(self.changed(ctx));
        actions
    }

    /// The model or the room changed: arranged again when what it is
    /// arranged from moved.
    pub(crate) fn changed(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let room = Size {
            width: ctx.size.width,
            height: ctx.size.height.saturating_sub(Frame::ROWS),
        };
        let now = (
            ctx.model.sessions.revision(),
            room,
            ctx.model.desk.threads(),
        );
        if self.arranged.as_ref() == Some(&now) {
            return Vec::new();
        }
        self.arranged = Some(now);
        self.size = room;
        self.rank(ctx.model);
        self.arrange(ctx.model)
    }

    /// Takes one key press: the panes' keys while nothing is typed, else
    /// the message box's.
    pub(crate) fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        if ctx.message.is_empty()
            && let Some(command) = Command::KEYS.find(key)
        {
            return self.command(command, ctx.model);
        }
        match ctx.message.press(key, ctx.model.projects()) {
            Typed::Took => Vec::new(),
            Typed::Submit => {
                let text = ctx.message.text();
                match Slash::parse(&text) {
                    Some(slash) => {
                        ctx.message.take();
                        slash.act(ctx, None)
                    }
                    None => vec![Action::Say(Notice::info(
                        "open a session to reply to it · /new <project> <prompt> starts a run",
                    ))],
                }
            }
            Typed::Left(key) => match Command::KEYS.find(key) {
                Some(command @ (Command::PreviousPlace | Command::NextPlace | Command::Quit)) => {
                    self.command(command, ctx.model)
                }
                Some(_) | None => Vec::new(),
            },
        }
    }

    fn command(&mut self, command: Command, model: &Model) -> Vec<Action> {
        let panes = self.grid.panes();
        let columns = usize::from(self.grid.columns);
        match command {
            Command::Next => self.focus = (self.focus + 1) % panes,
            Command::Previous => self.focus = (self.focus + panes - 1) % panes,
            Command::Left if !self.focus.is_multiple_of(columns) => self.focus -= 1,
            Command::Right if self.focus % columns + 1 < columns => self.focus += 1,
            Command::Up if self.focus >= columns => self.focus -= columns,
            Command::Down if self.focus + columns < panes => self.focus += columns,
            Command::Left | Command::Right | Command::Up | Command::Down => {}
            Command::Open => return self.open(model),
            Command::PreviousPlace => return vec![Action::Step { forward: false }],
            Command::NextPlace => return vec![Action::Step { forward: true }],
            Command::Quit => return vec![Action::Quit],
        }
        Vec::new()
    }

    /// The focused session, full screen: driven when a live run drives it,
    /// else watched.
    fn open(&self, model: &Model) -> Vec<Action> {
        let Some(session) = self.focused() else {
            return Vec::new();
        };
        let run = model
            .sessions
            .sessions()
            .iter()
            .find(|view| view.id == session)
            .and_then(|view| model.desk.driving(&view.native_id))
            .map(|run| run.id);
        let screen = match run {
            Some(run) => SessionScreen::run(run),
            None => SessionScreen::watching(session),
        };
        vec![Action::Push(Screen::Session(screen))]
    }

    /// Keeps the focused session on the wall even when it goes quiet, or
    /// stops keeping it.
    pub(crate) fn pin(&mut self, model: &Model) -> Vec<Action> {
        let Some(session) = self.focused() else {
            return vec![Action::Say(Notice::info("no session in the focused pane"))];
        };
        let said = match self.pins.iter().position(|pinned| *pinned == session) {
            Some(at) => {
                self.pins.remove(at);
                "unpinned: it leaves the wall when it goes quiet"
            }
            None => {
                self.pins.push(session);
                "pinned: it stays on the wall, even when quiet"
            }
        };
        self.rank(model);
        let mut actions = self.arrange(model);
        actions.push(Action::Say(Notice::info(said)));
        actions
    }

    fn focused(&self) -> Option<SessionId> {
        self.placement.panes().get(self.focus).copied().flatten()
    }

    /// Ranks the active sessions.
    fn rank(&mut self, model: &Model) {
        self.wanted = match model.sessions.read_at() {
            Some(now) => Self::wanted(
                model.sessions.sessions(),
                model.sessions.calls(),
                &self.pins,
                &model.desk.threads(),
                now,
            ),
            None => Vec::new(),
        };
    }

    /// Sessions that want a pane, most wanted first: pinned ones and those
    /// a live run drives (by their thread), however quiet, then active ones
    /// with an open call, then the most recently seen.
    fn wanted(
        sessions: &[SessionView],
        calls: &[CallView],
        pins: &[SessionId],
        threads: &[String],
        now: UnixMillis,
    ) -> Vec<SessionId> {
        let open = |session: &SessionView| {
            calls.iter().any(|call| {
                call.session == Some(session.id) && call.state.known() == Some(CallState::Open)
            })
        };
        let window = Sessions::millis(Sessions::WINDOW);
        let pinned = |session: &SessionView| {
            pins.contains(&session.id) || threads.contains(&session.native_id)
        };
        let mut active: Vec<(&SessionView, bool)> = sessions
            .iter()
            .map(|session| (session, open(session)))
            .filter(|(session, open)| {
                *open
                    || pinned(session)
                    || now
                        .as_millis()
                        .saturating_sub(session.last_seen.as_millis())
                        <= window
            })
            .collect();
        active.sort_by(|(a, a_open), (b, b_open)| {
            pinned(b)
                .cmp(&pinned(a))
                .then(b_open.cmp(a_open))
                .then(b.last_seen.cmp(&a.last_seen))
                .then(a.id.cmp(&b.id))
        });
        active.into_iter().map(|(session, _)| session.id).collect()
    }

    /// Places sessions in panes; asks for what the panes show that changed
    /// since it was read, or is still being derived.
    fn arrange(&mut self, model: &Model) -> Vec<Action> {
        self.grid = Grid::for_sessions(self.size, self.wanted.len());
        self.hidden = self.placement.arrange(self.grid, &self.wanted);
        self.focus = self.focus.min(self.grid.panes().saturating_sub(1));
        self.placement
            .panes()
            .iter()
            .flatten()
            .flat_map(|session| model.sessions.reads_for(*session))
            .map(Action::Read)
            .collect()
    }

    /// What the wall shows at `now`, in `size`.
    pub(crate) fn panes(&self, model: &Model, now: UnixMillis, size: Size) -> Panes {
        let sessions = model.sessions.sessions();
        let panes: Vec<Pane> = self
            .placement
            .panes()
            .iter()
            .map(|pane| {
                let Some(session) =
                    pane.and_then(|id| sessions.iter().find(|session| session.id == id))
                else {
                    return Pane::Free;
                };
                let mut pane = SessionPane::of(model, session, now);
                pane.pinned = self.pins.contains(&session.id);
                Pane::Session(Box::new(pane))
            })
            .collect();
        let calls: &[CallView] = model.sessions.calls();
        let open = calls
            .iter()
            .filter(|call| call.state.known() == Some(CallState::Open));
        let responding = open.clone().filter(|call| call.status.is_some()).count();
        let status = Status {
            now,
            failure: model.sessions.failure().map(str::to_owned),
            reading: model.sessions.reading(),
            sessions: self.wanted.len(),
            responding,
            awaiting: open.count() - responding,
            hidden: self.hidden,
            attention: panes
                .iter()
                .filter(|pane| match pane {
                    Pane::Session(session) => !session.alerts.is_empty(),
                    Pane::Free => false,
                })
                .count(),
            output: model.sessions.output(now),
            runs: match model.control {
                Control::Ready => Some(RunCounts::of(&model.desk)),
                Control::Unread | Control::Unavailable(_) => None,
            },
        };
        Panes {
            size,
            grid: self.grid,
            panes,
            focus: self.focus,
            status,
        }
    }

    pub(crate) fn view(&self, look: &Look<'_>) -> crate::ui::View {
        let panes = self.panes(look.model, look.now, Frame::body(look));
        let status = panes.status_line();
        Frame::new(
            look,
            &Here::Wall,
            panes.view(),
            status,
            "/new <project> <prompt> starts a run · enter opens the focused session",
        )
        .view()
    }
}
