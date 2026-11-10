//! A terminal view of a TGV gateway's live sessions: one pane per session,
//! each showing its newest turn as its calls stream, and a status line.
//!
//! [`Tui`] is sans-IO. It takes what was read ([`Input`]: snapshots of
//! sessions and calls, turn activity, progress updates, live observations,
//! the terminal's size) and answers with the reads it needs next
//! ([`Read`]); [`Tui::wall`] is what to draw at a given time. Whoever
//! drives it performs the reads, through the management API or this
//! host's storage.

mod call;
mod draw;
mod grid;
mod keys;
#[cfg(test)]
mod purity;
mod screen;
mod session;
#[cfg(test)]
mod tests;

pub use call::{CallEnd, ErrorCause, Shortfall, Stop, Tone};
pub use grid::{Grid, Size};
pub use keys::{Command, Key};
pub use screen::{Screen, ScreenError, ScreenEvent, ScreenEvents};
pub use session::{Alert, Line, Phase, ResultMark, SessionPane, Span, Streaming, Tally, ToolLine};

use grid::Placement;
use session::SessionRead;
use std::collections::HashMap;
use std::time::Duration;
use tgv_harness::Harness;
use tgv_protocol::label::{Labeled, Stored};
use tgv_recorder::Observation;
use tgv_recorder::batch::ObservationKind;
use tgv_recorder::capture::CallState;
use tgv_recorder::id::{CallId, SessionId, TurnId};
use tgv_recorder::progress::Progress;
use tgv_recorder::time::UnixMillis;
use tgv_store::activity::TurnActivity;
use tgv_store::query::{CallFilter, CallQuery, KindFilter, PageLimit, StatusFilter, TimeRange};
use tgv_store::view::{CallPage, CallView, SessionView};

/// The live sessions of one gateway, arranged in panes.
#[derive(Debug)]
pub struct Tui {
    grid: Grid,
    placement: Placement,
    /// Sessions that want a pane, most wanted first, as last read.
    wanted: Vec<SessionId>,
    /// Wanted sessions with no pane.
    hidden: usize,
    snapshot: Option<Held>,
    turns: HashMap<TurnId, HeldTurn>,
    progress: HashMap<CallId, Progress>,
    /// The focused pane's place, always within the grid.
    focus: usize,
    /// The session filling the screen, if any.
    zoom: Option<SessionId>,
    /// Sessions kept on the wall, first, even when they go quiet.
    pins: Vec<SessionId>,
    help: Help,
}

/// Whether the help is over the wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    Hidden,
    Shown,
}

/// What a key press left to do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Pressed {
    /// Go on, with these reads.
    Continue(Vec<Read>),
    Quit,
}

/// How the wall is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Every pane, one focused.
    Panes { focus: usize },
    /// One session fills the screen.
    Zoomed,
}

/// One read of the gateway's sessions and calls.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// When the reads were made.
    pub read_at: UnixMillis,
    pub sessions: Vec<SessionView>,
    /// Model calls still open.
    pub open: CallPage,
    /// Model calls started in the window, newest first.
    pub recent: CallPage,
}

/// The reads that make a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotQuery {
    pub sessions: PageLimit,
    pub open: CallQuery,
    pub recent: CallQuery,
}

/// What was read, or what happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Snapshot(Snapshot),
    /// A turn's activity, as asked for with [`Read::Turn`].
    Turn(Box<TurnActivity>),
    /// An update on a streaming call.
    Progress(Progress),
    Observation(Observation),
    Resize(Size),
}

/// A read the view needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// A new [`Snapshot`], from [`Tui::snapshot_query`].
    Snapshot,
    Turn(TurnId),
}

/// Everything to draw at one time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wall {
    /// One pane across and down when zoomed.
    pub grid: Grid,
    /// In pane order: across, then down.
    pub panes: Vec<Pane>,
    pub view: View,
    pub help: Help,
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
}

/// How complete the view's reads are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Nothing read yet.
    Pending,
    Complete,
    /// More calls started in the window than one read returns: older ones
    /// are left out.
    Capped,
}

/// One client's output over time, oldest bin first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spark {
    pub harness: Harness,
    pub bins: Vec<u64>,
}

#[derive(Debug)]
struct Held {
    snapshot: Snapshot,
    /// Open and recent calls, each once.
    calls: Vec<CallView>,
}

#[derive(Debug)]
struct HeldTurn {
    /// The turn's calls and their states when it was read: it changes only
    /// when one of them does.
    signature: Vec<(CallId, Stored<CallState>)>,
    activity: TurnActivity,
}

impl Tui {
    /// Sessions seen this recently are active.
    pub const WINDOW: Duration = Duration::from_secs(5 * 60);
    /// How far back the status line's output reaches, and in how many bins.
    pub const OUTPUT_SPAN: Duration = Duration::from_secs(4 * 60);
    pub const OUTPUT_BINS: usize = 24;

    pub fn new(size: Size) -> Self {
        Self {
            grid: Grid::fit(size),
            placement: Placement::default(),
            wanted: Vec::new(),
            hidden: 0,
            snapshot: None,
            turns: HashMap::new(),
            progress: HashMap::new(),
            focus: 0,
            zoom: None,
            pins: Vec::new(),
            help: Help::Hidden,
        }
    }

    /// Takes one key press.
    pub fn press(&mut self, key: Key) -> Pressed {
        let Some(command) = Command::of(key) else {
            return Pressed::Continue(Vec::new());
        };
        // The help takes no command but closing it, or quitting.
        if self.help == Help::Shown {
            match command {
                Command::Quit => return Pressed::Quit,
                Command::Help | Command::Back => self.help = Help::Hidden,
                Command::Next
                | Command::Previous
                | Command::Left
                | Command::Right
                | Command::Up
                | Command::Down
                | Command::Zoom
                | Command::Pin => {}
            }
            return Pressed::Continue(Vec::new());
        }
        let panes = self.grid.panes();
        let columns = usize::from(self.grid.columns);
        match command {
            Command::Quit => return Pressed::Quit,
            Command::Help => self.help = Help::Shown,
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
                    self.rank();
                    return Pressed::Continue(self.arrange());
                }
            }
        }
        Pressed::Continue(Vec::new())
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

    /// The reads for a snapshot at `now`: open model calls, and those
    /// started in the window.
    pub fn snapshot_query(now: UnixMillis) -> SnapshotQuery {
        let model = CallFilter {
            kind: Some(KindFilter::Model),
            ..CallFilter::default()
        };
        let from =
            UnixMillis::from_millis(now.as_millis().saturating_sub(Self::millis(Self::WINDOW)));
        SnapshotQuery {
            sessions: PageLimit::DEFAULT,
            open: CallQuery {
                filter: CallFilter {
                    status: Some(StatusFilter::Active),
                    ..model.clone()
                },
                limit: PageLimit::DEFAULT,
                after: None,
            },
            recent: CallQuery {
                filter: CallFilter {
                    // A range from a time with no end is always valid.
                    time: TimeRange::new(Some(from), None).unwrap_or_default(),
                    ..model
                },
                limit: PageLimit::LARGEST,
                after: None,
            },
        }
    }

    /// Takes one input; returns the reads it calls for.
    #[must_use]
    pub fn apply(&mut self, input: Input) -> Vec<Read> {
        match input {
            Input::Snapshot(snapshot) => self.take_snapshot(snapshot),
            Input::Turn(activity) => {
                let signature = self.signature(activity.turn);
                self.turns.insert(
                    activity.turn,
                    HeldTurn {
                        signature,
                        activity: *activity,
                    },
                );
                Vec::new()
            }
            Input::Progress(progress) => {
                // A late update on a call already read as ended is dropped.
                let ended = self
                    .call(progress.call)
                    .is_some_and(|call| call.state.known() != Some(CallState::Open));
                if !ended {
                    self.progress.insert(progress.call, progress);
                }
                Vec::new()
            }
            Input::Observation(observation) => self.observe(&observation),
            Input::Resize(size) => {
                self.grid = Grid::fit(size);
                self.arrange()
            }
        }
    }

    fn observe(&mut self, observation: &Observation) -> Vec<Read> {
        match observation.kind.known() {
            Some(ObservationKind::CallFinished | ObservationKind::CallRecovered) => {
                if let Some(call) = observation.call {
                    self.progress.remove(&call);
                }
                vec![Read::Snapshot]
            }
            Some(ObservationKind::CallOpened | ObservationKind::ResponseHead) => {
                vec![Read::Snapshot]
            }
            Some(
                ObservationKind::AttemptStarted
                | ObservationKind::AttemptFinished
                | ObservationKind::ChannelClosed
                | ObservationKind::OperatorAction,
            )
            | None => Vec::new(),
        }
    }

    fn take_snapshot(&mut self, snapshot: Snapshot) -> Vec<Read> {
        let mut calls: Vec<CallView> = Vec::new();
        for call in snapshot.open.calls.iter().chain(&snapshot.recent.calls) {
            if !calls.iter().any(|held| held.id == call.id) {
                calls.push(call.clone());
            }
        }
        calls.sort_by_key(|call| call.started_at);
        // Progress is kept only for calls that may still be streaming.
        self.progress.retain(|id, _| {
            calls
                .iter()
                .find(|call| call.id == *id)
                .is_none_or(|call| call.state.known() == Some(CallState::Open))
        });
        self.snapshot = Some(Held { snapshot, calls });
        self.rank();
        self.arrange()
    }

    /// Ranks the active sessions from the held snapshot.
    fn rank(&mut self) {
        self.wanted = self.snapshot.as_ref().map_or_else(Vec::new, |held| {
            Self::wanted(
                &held.snapshot.sessions,
                &held.calls,
                &self.pins,
                held.snapshot.read_at,
            )
        });
    }

    /// Sessions that want a pane, most wanted first: pinned ones, however
    /// quiet, then active ones with an open call, then the most recently
    /// seen.
    fn wanted(
        sessions: &[SessionView],
        calls: &[CallView],
        pins: &[SessionId],
        now: UnixMillis,
    ) -> Vec<SessionId> {
        let open = |session: &SessionView| {
            calls.iter().any(|call| {
                call.session == Some(session.id) && call.state.known() == Some(CallState::Open)
            })
        };
        let window = Self::millis(Self::WINDOW);
        let pinned = |session: &SessionView| pins.contains(&session.id);
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
    fn arrange(&mut self) -> Vec<Read> {
        self.hidden = self.placement.arrange(self.grid, &self.wanted);
        self.focus = self.focus.min(self.grid.panes().saturating_sub(1));
        // A zoomed session that left the wall leaves the zoom.
        if self
            .zoom
            .is_some_and(|session| !self.placement.panes().contains(&Some(session)))
        {
            self.zoom = None;
        }
        let shown: Vec<SessionId> = self.placement.panes().iter().flatten().copied().collect();
        let mut reads = Vec::new();
        for session in shown.iter().copied() {
            let calls = self.session_calls(session);
            let Some(turn) = session::SessionPane::newest_turn(&calls) else {
                continue;
            };
            let signature = self.signature(turn);
            let settled = self.turns.get(&turn).is_some_and(|held| {
                held.signature == signature
                    && held.activity.calls.iter().all(|call| {
                        !call.limitations.iter().any(|limitation| {
                            limitation.known()
                                == Some(tgv_protocol::activity::ActivityLimitation::NotYetDerived)
                        })
                    })
            });
            if !settled {
                reads.push(Read::Turn(turn));
            }
        }
        // Turns no pane shows any more are let go.
        self.turns
            .retain(|_, held| shown.contains(&held.activity.session));
        reads
    }

    fn signature(&self, turn: TurnId) -> Vec<(CallId, Stored<CallState>)> {
        self.snapshot
            .iter()
            .flat_map(|held| &held.calls)
            .filter(|call| call.turn == Some(turn))
            .map(|call| (call.id, call.state.clone()))
            .collect()
    }

    fn call(&self, id: CallId) -> Option<&CallView> {
        self.snapshot
            .iter()
            .flat_map(|held| &held.calls)
            .find(|call| call.id == id)
    }

    /// A session's held calls, oldest first.
    fn session_calls(&self, session: SessionId) -> Vec<&CallView> {
        self.snapshot
            .iter()
            .flat_map(|held| &held.calls)
            .filter(|call| call.session == Some(session))
            .collect()
    }

    /// What to draw at `now`.
    pub fn wall(&self, now: UnixMillis) -> Wall {
        let sessions: &[SessionView] = self
            .snapshot
            .as_ref()
            .map_or(&[], |held| held.snapshot.sessions.as_slice());
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
                let calls = self.session_calls(session.id);
                let turn = session::SessionPane::newest_turn(&calls)
                    .and_then(|turn| self.turns.get(&turn))
                    .map(|held| &held.activity);
                let mut pane = session::SessionPane::read(&SessionRead {
                    session,
                    calls: &calls,
                    turn,
                    progress: &self.progress,
                    now,
                });
                pane.pinned = self.pins.contains(&session.id);
                Pane::Session(Box::new(pane))
            })
            .collect();
        let calls: &[CallView] = self.snapshot.as_ref().map_or(&[], |held| &held.calls);
        let open = calls
            .iter()
            .filter(|call| call.state.known() == Some(CallState::Open));
        let responding = open.clone().filter(|call| call.status.is_some()).count();
        let status = Status {
            now,
            reading: match &self.snapshot {
                None => Reading::Pending,
                Some(held) if held.snapshot.recent.next.is_some() => Reading::Capped,
                Some(_) => Reading::Complete,
            },
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
            output: Self::output(calls, now),
        };
        let (grid, view) = match self.zoom {
            Some(_) => (
                Grid {
                    columns: 1,
                    rows: 1,
                },
                View::Zoomed,
            ),
            None => (self.grid, View::Panes { focus: self.focus }),
        };
        Wall {
            grid,
            panes,
            view,
            help: self.help,
            status,
        }
    }

    /// Output tokens of finished calls, per client, binned by when each
    /// ended. Clients with no output in the span are left out.
    fn output(calls: &[CallView], now: UnixMillis) -> Vec<Spark> {
        let span = Self::millis(Self::OUTPUT_SPAN);
        let bins = u64::try_from(Self::OUTPUT_BINS).unwrap_or(1).max(1);
        let width = (span / bins).max(1);
        let start = now.as_millis().saturating_sub(span);
        Harness::ALL
            .iter()
            .filter_map(|harness| {
                let mut spark = Spark {
                    harness: *harness,
                    bins: vec![0; Self::OUTPUT_BINS],
                };
                for call in calls {
                    let attributed = call
                        .attribution
                        .hint
                        .as_ref()
                        .and_then(|hint| hint.source.known())
                        .map(|source| source.harness());
                    let (Some(ended), Some(output)) = (
                        call.ended_at,
                        call.usage.as_ref().and_then(|usage| usage.output_tokens),
                    ) else {
                        continue;
                    };
                    if attributed != Some(*harness) || ended.as_millis() < start || ended > now {
                        continue;
                    }
                    let bin = usize::try_from((ended.as_millis() - start) / width)
                        .unwrap_or(usize::MAX)
                        .min(Self::OUTPUT_BINS - 1);
                    if let Some(total) = spark.bins.get_mut(bin) {
                        *total = total.saturating_add(output);
                    }
                }
                spark.bins.iter().any(|total| *total > 0).then_some(spark)
            })
            .collect()
    }

    fn millis(duration: Duration) -> u64 {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    }
}
