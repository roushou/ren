//! The wall: every live session's traffic, one pane per session, each
//! showing its newest turns as its calls stream, and a status line.
//!
//! [`Wall`] holds only how the wall is arranged: which session has which
//! pane, the focus, a zoomed session, and pins. What it shows comes from
//! the model; [`Wall::frame`] reads it at a given time, and `view` lays the
//! frame out.

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
pub use runs::{RunCounts, RunMark};

use super::{Action, Ctx, Look, Screen};
use crate::keys::HelpRow;
use crate::model::sessions::{Reading, Sessions, Spark};
use crate::model::{Control, Model, Read};
use crate::{Key, Size};
use grid::Placement;
use pane::SessionRead;
use ren_recorder::capture::CallState;
use ren_recorder::id::{RunId, SessionId};
use ren_recorder::time::UnixMillis;
use ren_store::activity::CallActivity;
use ren_store::view::{CallView, SessionView};

/// How the wall is arranged.
#[derive(Debug)]
pub struct Wall {
    /// The size the grid was made for.
    size: Size,
    grid: Grid,
    placement: Placement,
    /// Sessions that want a pane, most wanted first, as last arranged.
    wanted: Vec<SessionId>,
    /// Wanted sessions with no pane.
    hidden: usize,
    /// The focused pane's place, always within the grid.
    focus: usize,
    /// The session filling the screen, if any.
    zoom: Option<SessionId>,
    /// Sessions kept on the wall, first, even when they go quiet.
    pins: Vec<SessionId>,
    /// What it was last arranged from: the sessions' revision, the size,
    /// and the threads live runs hold.
    arranged: Option<(u64, Size, Vec<String>)>,
}

/// How the wall is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Every pane, one focused.
    Panes { focus: usize },
    /// One session fills the screen.
    Zoomed,
}

/// Everything to draw at one time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub size: Size,
    /// One pane across and down when zoomed.
    pub grid: Grid,
    /// In pane order: across, then down.
    pub panes: Vec<Pane>,
    pub layout: Layout,
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
    /// How many of a turnless session's newest calls a pane reads.
    pub const TURNLESS_CALLS: usize = 4;

    pub fn new(size: Size) -> Self {
        Self {
            size,
            grid: Grid::for_sessions(size, 0),
            placement: Placement::default(),
            wanted: Vec::new(),
            hidden: 0,
            focus: 0,
            zoom: None,
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

    /// The model or the size changed: arranged again when what it is
    /// arranged from moved.
    pub(crate) fn changed(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let now = (
            ctx.model.sessions.revision(),
            ctx.size,
            ctx.model.desk.threads(),
        );
        if self.arranged.as_ref() == Some(&now) {
            return Vec::new();
        }
        self.arranged = Some(now);
        self.size = ctx.size;
        self.rank(ctx.model);
        self.arrange(ctx.model)
    }

    /// Takes one key press.
    pub(crate) fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let Some(command) = Command::KEYS.find(key) else {
            return Vec::new();
        };
        let panes = self.grid.panes();
        let columns = usize::from(self.grid.columns);
        match command {
            Command::Theme => return vec![Action::CycleTheme],
            Command::Quit => return vec![Action::Quit],
            Command::Help => return vec![Action::Help],
            Command::Runs => {
                let run = self.focused_run(ctx.model);
                return vec![Action::Push(Screen::runs(run))];
            }
            Command::Next | Command::Previous if self.zoom.is_some() => {
                self.zoom = self.neighbour(command == Command::Next);
            }
            Command::Next => self.focus = (self.focus + 1) % panes,
            Command::Previous => self.focus = (self.focus + panes - 1) % panes,
            Command::Left if !self.focus.is_multiple_of(columns) => self.focus -= 1,
            Command::Right if self.focus % columns + 1 < columns => self.focus += 1,
            Command::Up if self.focus >= columns => self.focus -= columns,
            Command::Down if self.focus + columns < panes => self.focus += columns,
            Command::Left | Command::Right | Command::Up | Command::Down => {}
            Command::Zoom => {
                self.zoom = match self.zoom {
                    Some(_) => None,
                    None => self.focused(),
                };
            }
            Command::Back => self.zoom = None,
            Command::Pin => {
                if let Some(session) = self.zoom.or_else(|| self.focused()) {
                    match self.pins.iter().position(|pinned| *pinned == session) {
                        Some(at) => {
                            self.pins.remove(at);
                        }
                        None => self.pins.push(session),
                    }
                    self.rank(ctx.model);
                    return self.arrange(ctx.model);
                }
            }
        }
        Vec::new()
    }

    /// The live run the zoomed or focused session belongs to.
    fn focused_run(&self, model: &Model) -> Option<RunId> {
        let session = self.zoom.or_else(|| self.focused())?;
        let view = model
            .sessions
            .sessions()
            .iter()
            .find(|view| view.id == session)?;
        model.desk.driving(&view.native_id).map(|run| run.id)
    }

    fn focused(&self) -> Option<SessionId> {
        self.placement.panes().get(self.focus).copied().flatten()
    }

    /// The placed session after (or before) the zoomed one, in pane order.
    fn neighbour(&self, forward: bool) -> Option<SessionId> {
        let placed: Vec<SessionId> = self.placement.panes().iter().flatten().copied().collect();
        let at = placed
            .iter()
            .position(|session| Some(*session) == self.zoom)?;
        let count = placed.len();
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        placed.get(next).copied()
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

    /// Places sessions in panes; asks for the turns the panes show that
    /// changed since they were read, or are still being derived.
    fn arrange(&mut self, model: &Model) -> Vec<Action> {
        self.grid = Grid::for_sessions(self.size, self.wanted.len());
        self.hidden = self.placement.arrange(self.grid, &self.wanted);
        self.focus = self.focus.min(self.grid.panes().saturating_sub(1));
        // A zoomed session that left the wall leaves the zoom.
        if self
            .zoom
            .is_some_and(|session| !self.placement.panes().contains(&Some(session)))
        {
            self.zoom = None;
        }
        let mut reads = Vec::new();
        for session in self.placement.panes().iter().flatten().copied() {
            let calls = model.sessions.session_calls(session);
            let turns = SessionPane::recent_turns(&calls);
            if turns.is_empty() {
                // No turns: the newest finished calls, each read once its
                // text is derived.
                for call in calls
                    .iter()
                    .rev()
                    .filter(|call| call.state.known() != Some(CallState::Open))
                    .take(Self::TURNLESS_CALLS)
                {
                    if !model.sessions.call_settled(call.id) {
                        reads.push(Action::Read(Read::Call(call.id)));
                    }
                }
            }
            for turn in turns {
                if !model.sessions.settled(turn) {
                    reads.push(Action::Read(Read::Turn(turn)));
                }
            }
        }
        reads
    }

    /// What to draw at `now`.
    pub(crate) fn frame(&self, look: &Look<'_>) -> Frame {
        let model = look.model;
        let now = look.now;
        let sessions = model.sessions.sessions();
        let placed: Vec<Option<SessionId>> = match self.zoom {
            Some(session) => vec![Some(session)],
            None => self.placement.panes().to_vec(),
        };
        let panes: Vec<Pane> = placed
            .iter()
            .map(|pane| {
                let Some(session) =
                    pane.and_then(|id| sessions.iter().find(|session| session.id == id))
                else {
                    return Pane::Free;
                };
                let calls = model.sessions.session_calls(session.id);
                let recent = SessionPane::recent_turns(&calls);
                let held = |turn| model.sessions.turn(turn);
                let (earlier, turn) = match recent.as_slice() {
                    [earlier, newest] => (held(*earlier), held(*newest)),
                    [newest] => (None, held(*newest)),
                    _ => (None, None),
                };
                let activities: Vec<&CallActivity> = calls
                    .iter()
                    .filter_map(|call| model.sessions.activity(call.id))
                    .collect();
                let mut pane = SessionPane::read(&SessionRead {
                    activities: &activities,
                    session,
                    calls: &calls,
                    earlier,
                    turn,
                    progress: model.sessions.progress(),
                    now,
                });
                pane.pinned = self.pins.contains(&session.id);
                pane.run = model.desk.driving(&session.native_id).and_then(RunMark::of);
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
        let (grid, layout) = match self.zoom {
            Some(_) => (
                Grid {
                    columns: 1,
                    rows: 1,
                },
                Layout::Zoomed,
            ),
            None => (self.grid, Layout::Panes { focus: self.focus }),
        };
        Frame {
            size: look.size,
            grid,
            panes,
            layout,
            status,
        }
    }

    pub(crate) fn view(&self, look: &Look<'_>) -> crate::ui::View {
        self.frame(look).view()
    }
}
