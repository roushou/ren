//! Ren in the terminal: one program over one gateway, made of screens.
//!
//! [`Tui`] is sans-IO. It holds the model (everything read from the
//! gateway: sessions and their calls, runs and their conversations,
//! projects) and a stack of screens (`screen`) drawn in one frame: the
//! wall of every session's traffic at the bottom, then a session full
//! screen (a run Ren drives, or one watched), `/resume`'s list, and the
//! help. The wall and each live run are the places the frame's top line
//! lists and Alt-← and Alt-→ move between. Keys go to the screen on top; what was read goes to the
//! model, then to that screen. Both answer with [`Out`]s: the reads to
//! make and the commands to send through the control API. [`Tui::view`]
//! is what to draw, built from the drawing primitives in [`ui`];
//! [`Terminal`] is the only part that touches the terminal.

mod conversation;
mod event;
mod keys;
mod model;
#[cfg(test)]
mod purity;
mod screen;
mod terminal;
#[cfg(test)]
mod tests;
mod theme;
pub mod ui;

pub use event::{Key, Size};
pub use model::{Answered, Command, Input, Read, Snapshot, SnapshotQuery};
pub use terminal::{Terminal, TerminalError, TerminalEvent, TerminalEvents};
pub use theme::{Theme, ThemeName};

use model::{Effect, Model};
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;
use screen::{Action, Ctx, Help, Look, MessageBox, News, Notice, Screen, SessionScreen, Wall};
use std::collections::VecDeque;
use ui::View;

/// The terminal's screens over one gateway, the top one shown.
#[derive(Debug)]
pub struct Tui {
    model: Model,
    stack: Stack,
    /// The message box the console's screens share.
    message: MessageBox,
    /// Shown under the message box until the next key.
    notice: Option<Notice>,
    size: Size,
    /// Opened on the runs: the run that needs you opens once runs are read.
    opening: Option<Opening>,
}

/// Which screen the terminal opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// The wall of every session's traffic (`ren tui`).
    Wall,
    /// The run that needs you, else the newest live one (`ren runs`).
    Runs,
}

/// What the shell does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Read(Read),
    /// A command for the control API.
    Send(Command),
    /// Preview the next theme; the shell owns the local preference.
    CycleTheme,
    Quit,
}

/// Screens, one on another, over the wall.
#[derive(Debug)]
struct Stack {
    bottom: Screen,
    above: Vec<Screen>,
}

impl Stack {
    fn top(&self) -> &Screen {
        self.above.last().unwrap_or(&self.bottom)
    }

    fn top_mut(&mut self) -> &mut Screen {
        self.above.last_mut().unwrap_or(&mut self.bottom)
    }

    /// The screen under the top one.
    fn under(&self) -> Option<&Screen> {
        match self.above.len() {
            0 => None,
            1 => Some(&self.bottom),
            above => self.above.get(above - 2),
        }
    }

    /// Closes the top screen; the bottom one stays.
    fn pop(&mut self) {
        self.above.pop();
    }

    fn replace(&mut self, screen: Screen) {
        *self.top_mut() = screen;
    }

    /// Closes every screen over the wall.
    fn back_to_wall(&mut self) {
        self.above.clear();
    }

    fn wall_mut(&mut self) -> Option<&mut Wall> {
        match &mut self.bottom {
            Screen::Wall(wall) => Some(wall),
            Screen::Session(_) | Screen::Resume(_) | Screen::Help(_) => None,
        }
    }
}

impl Tui {
    pub fn new(size: Size, opening: Opening) -> Self {
        Self {
            model: Model::new(),
            stack: Stack {
                bottom: Screen::Wall(Wall::new(size)),
                above: Vec::new(),
            },
            message: MessageBox::default(),
            notice: None,
            size,
            opening: Some(opening),
        }
    }

    /// The reads for a snapshot at `now`.
    pub fn snapshot_query(now: UnixMillis) -> SnapshotQuery {
        Model::snapshot_query(now)
    }

    /// The reads to make first: everything the screens show, so each is
    /// ready when shown, then what the first screen asks for.
    pub fn start(&mut self) -> Vec<Out> {
        let mut outs = vec![
            Out::Read(Read::Snapshot),
            Out::Read(Read::Runs),
            Out::Read(Read::Recorded),
            Out::Read(Read::Projects),
        ];
        let actions = self.with_top(Screen::shown);
        outs.extend(self.act(actions));
        Self::once(outs)
    }

    /// Takes one key press, for the screen on top.
    pub fn press(&mut self, key: Key) -> Vec<Out> {
        self.notice = None;
        let actions = self.with_top(|screen, ctx| screen.press(key, ctx));
        let outs = self.act(actions);
        Self::once(outs)
    }

    /// Takes one input; returns what it calls for.
    pub fn apply(&mut self, input: Input) -> Vec<Out> {
        if let Input::Resize(size) = input {
            self.size = size;
        }
        let mut outs = Vec::new();
        let mut news = Vec::new();
        for effect in self.model.apply(input) {
            match effect {
                Effect::Read(read) => outs.push(Out::Read(read)),
                Effect::Send(command) => outs.push(Out::Send(command)),
                Effect::Started(run) => news.push(News::Started(run)),
                Effect::Refused(why) => self.notice = Some(Notice::refusal(why)),
            }
        }
        let mut actions = self.with_top(|screen, ctx| screen.changed(ctx, &news));
        // Opened on the runs: once they are read, the one that needs you,
        // else the newest live one.
        if let (Some(Opening::Runs), model::Control::Ready) = (self.opening, &self.model.control) {
            self.opening = None;
            let desk = &self.model.desk;
            if let Some(run) = desk
                .next_needing(None)
                .or_else(|| desk.ordered().first().map(|run| run.id))
            {
                actions.push(Action::Open(run));
            }
        }
        outs.extend(self.act(actions));
        Self::once(outs)
    }

    /// Each read asked for once: the model and a screen may both ask for
    /// the same one.
    fn once(outs: Vec<Out>) -> Vec<Out> {
        let mut kept: Vec<Out> = Vec::with_capacity(outs.len());
        for out in outs {
            let repeated = match &out {
                Out::Read(_) => kept.contains(&out),
                Out::Send(_) | Out::CycleTheme | Out::Quit => false,
            };
            if !repeated {
                kept.push(out);
            }
        }
        kept
    }

    /// What to draw at `now`.
    pub fn view(&self, now: UnixMillis) -> View {
        let look = self.look(now);
        match (self.stack.top(), self.stack.under()) {
            (Screen::Help(help), Some(under)) => help.over(under.view(&look)),
            (top, _) => top.view(&look),
        }
    }

    fn look(&self, now: UnixMillis) -> Look<'_> {
        Look {
            model: &self.model,
            message: &self.message,
            notice: self.notice.as_ref(),
            size: self.size,
            now,
        }
    }

    fn with_top(
        &mut self,
        act: impl FnOnce(&mut Screen, &mut Ctx<'_>) -> Vec<Action>,
    ) -> Vec<Action> {
        let mut ctx = Ctx {
            model: &mut self.model,
            message: &mut self.message,
            size: self.size,
        };
        act(self.stack.top_mut(), &mut ctx)
    }

    /// Carries out what a screen asked for, and what each screen it shows
    /// asks for when shown.
    fn act(&mut self, actions: Vec<Action>) -> Vec<Out> {
        let mut outs = Vec::new();
        let mut pending: VecDeque<Action> = actions.into();
        while let Some(action) = pending.pop_front() {
            let shown = match action {
                Action::Read(read) => {
                    outs.push(Out::Read(read));
                    false
                }
                Action::Send(command) => {
                    outs.push(Out::Send(command));
                    false
                }
                Action::CycleTheme => {
                    outs.push(Out::CycleTheme);
                    false
                }
                Action::Quit => {
                    outs.push(Out::Quit);
                    false
                }
                Action::Say(notice) => {
                    self.notice = Some(notice);
                    false
                }
                Action::Push(screen) => {
                    self.stack.above.push(screen);
                    true
                }
                Action::Replace(screen) => {
                    self.stack.replace(screen);
                    true
                }
                Action::Pop => {
                    self.stack.pop();
                    true
                }
                Action::Wall => {
                    self.stack.back_to_wall();
                    true
                }
                Action::Open(run) => {
                    self.open(run);
                    true
                }
                Action::Step { forward } => self.step(forward),
                Action::Pin => {
                    let model = &self.model;
                    if let Some(wall) = self.stack.wall_mut() {
                        pending.extend(wall.pin(model));
                    }
                    false
                }
                Action::Help => {
                    let rows = self.stack.top().keys();
                    self.stack.above.push(Screen::Help(Help::new(rows)));
                    false
                }
            };
            if shown {
                pending.extend(self.with_top(Screen::shown));
            }
        }
        outs
    }

    /// Shows `run`: in place of the session shown, or over the wall.
    fn open(&mut self, run: RunId) {
        let screen = Screen::Session(SessionScreen::run(run));
        match self.stack.top() {
            Screen::Session(_) => self.stack.replace(screen),
            Screen::Wall(_) => self.stack.above.push(screen),
            Screen::Resume(_) | Screen::Help(_) => {
                self.stack.pop();
                self.open(run);
            }
        }
    }

    /// The next (or previous) place: the wall, then each live run, what
    /// needs you first. Whether a screen was shown.
    fn step(&mut self, forward: bool) -> bool {
        let mut places: Vec<Option<RunId>> = vec![None];
        places.extend(self.model.desk.ordered().iter().map(|run| Some(run.id)));
        let at = places
            .iter()
            .position(|place| *place == self.stack.top().run())
            .unwrap_or(0);
        let count = places.len();
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        match places.get(next).copied().flatten() {
            Some(run) => self.open(run),
            None if at == next => {
                self.notice = Some(Notice::info("no live runs: /new starts one"));
                return false;
            }
            None => self.stack.back_to_wall(),
        }
        true
    }

    /// The run the screen on top shows.
    #[cfg(test)]
    pub(crate) fn shown_run(&self) -> Option<ren_recorder::id::RunId> {
        self.stack.top().run()
    }

    /// The wall's frame, when the wall is shown: on top, or under the help.
    #[cfg(test)]
    pub(crate) fn wall_frame(&self, now: UnixMillis) -> Option<screen::wall::Panes> {
        let shown = match (self.stack.top(), self.stack.under()) {
            (Screen::Help(_), Some(under)) => under,
            (top, _) => top,
        };
        match shown {
            Screen::Wall(wall) => Some(wall.panes(
                &self.model,
                now,
                Size {
                    width: self.size.width,
                    height: self.size.height.saturating_sub(4),
                },
            )),
            Screen::Session(_) | Screen::Resume(_) | Screen::Help(_) => None,
        }
    }

    /// The completions offered, by label.
    #[cfg(test)]
    pub(crate) fn completions(&self) -> Vec<String> {
        self.message
            .popup(&screen::slash::Known::of(&self.model))
            .into_iter()
            .map(|(suggestion, _)| suggestion.label)
            .collect()
    }

    /// The session watched on top, if one is.
    #[cfg(test)]
    pub(crate) fn watching(&self) -> Option<ren_recorder::id::SessionId> {
        match self.stack.top() {
            Screen::Session(screen) => screen.watched(),
            Screen::Wall(_) | Screen::Resume(_) | Screen::Help(_) => None,
        }
    }

    /// Whether the help is open.
    #[cfg(test)]
    pub(crate) fn help_shown(&self) -> bool {
        matches!(self.stack.top(), Screen::Help(_))
    }
}
