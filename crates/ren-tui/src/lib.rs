//! Ren in the terminal: one program over one gateway, made of screens.
//!
//! [`Tui`] is sans-IO. It holds the model (everything read from the
//! gateway: sessions and their calls, runs and their conversations,
//! projects) and a stack of screens (`screen`): the wall of every
//! session's traffic, the console's live runs, one run, `/resume`'s list,
//! and the help. Keys go to the screen on top; what was read goes to the
//! model, then to that screen. Both answer with [`Out`]s: the reads to
//! make and the commands to send through the control API. [`Tui::view`]
//! is what to draw, built from the drawing primitives in [`ui`];
//! [`Terminal`] is the only part that touches the terminal.

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
use ren_recorder::time::UnixMillis;
use screen::{Action, Ctx, Help, Look, MessageBox, News, Notice, Screen, Wall};
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
}

/// Which screen the terminal opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// Every session's traffic (`ren tui`).
    Wall,
    /// The agent runs (`ren runs`).
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

/// Screens, one on another: never empty.
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

    /// Closes the screens over the wall, if one is open: whether it was.
    fn back_to_wall(&mut self) -> bool {
        if let Some(at) = self
            .above
            .iter()
            .rposition(|screen| matches!(screen, Screen::Wall(_)))
        {
            self.above.truncate(at + 1);
            return true;
        }
        if let Screen::Wall(_) = self.bottom {
            self.above.clear();
            return true;
        }
        false
    }
}

impl Tui {
    pub fn new(size: Size, opening: Opening) -> Self {
        let bottom = match opening {
            Opening::Wall => Screen::Wall(Wall::new(size)),
            Opening::Runs => Screen::runs(None),
        };
        Self {
            model: Model::new(),
            stack: Stack {
                bottom,
                above: Vec::new(),
            },
            message: MessageBox::default(),
            notice: None,
            size,
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
        let actions = self.with_top(|screen, ctx| screen.changed(ctx, &news));
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
                    if !self.stack.back_to_wall() {
                        self.stack.above.push(Screen::Wall(Wall::new(self.size)));
                    }
                    true
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

    /// The run the screen on top shows.
    #[cfg(test)]
    pub(crate) fn shown_run(&self) -> Option<ren_recorder::id::RunId> {
        self.stack.top().run()
    }

    /// The wall's frame, when the wall is shown: on top, or under the help.
    #[cfg(test)]
    pub(crate) fn wall_frame(&self, now: UnixMillis) -> Option<screen::wall::Frame> {
        let shown = match (self.stack.top(), self.stack.under()) {
            (Screen::Help(_), Some(under)) => under,
            (top, _) => top,
        };
        match shown {
            Screen::Wall(wall) => Some(wall.frame(&self.look(now))),
            Screen::Home(_) | Screen::Run(_) | Screen::Resume(_) | Screen::Help(_) => None,
        }
    }

    /// The completions offered, by label.
    #[cfg(test)]
    pub(crate) fn completions(&self) -> Vec<String> {
        self.message
            .popup(self.model.projects())
            .into_iter()
            .map(|(suggestion, _)| suggestion.label)
            .collect()
    }

    /// Whether the help is open.
    #[cfg(test)]
    pub(crate) fn help_shown(&self) -> bool {
        matches!(self.stack.top(), Screen::Help(_))
    }
}
