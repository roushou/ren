//! Ren in the terminal: one app over one gateway for watching and driving
//! agent sessions.
//!
//! [`Tui`] is sans-IO. It holds the model (everything read from the
//! gateway: sessions and their calls, runs and their conversations,
//! projects) and its screens (`screen`): the session list beside the
//! selected session, with the new-run form and the help opening over it.
//! Keys go to the screen on top; what was read goes to the model, then to
//! that screen. Both answer with [`Out`]s: the reads to make and the
//! commands to send through the control API. [`Tui::view`] is what to
//! draw, built from the drawing primitives in [`ui`]; [`Terminal`] is the
//! only part that touches the terminal.

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
use ren_recorder::time::UnixMillis;
use screen::{Action, Ctx, Help, Look, Main, MessageBox, News, Notice, Screen};
use std::collections::VecDeque;
use ui::View;

/// The app over one gateway.
#[derive(Debug)]
pub struct Tui {
    model: Model,
    /// The main screen.
    main: Screen,
    /// Screens open over it, the newest last.
    over: Vec<Screen>,
    /// The message box for writing to runs.
    message: MessageBox,
    /// Shown in the key bar until the next key.
    notice: Option<Notice>,
    size: Size,
}

/// What the shell does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Read(Read),
    /// A command for the control API.
    Send(Command),
    /// Preview the next theme; the shell owns the local preference.
    CycleTheme,
    /// Open this text in the operator's editor, and hand what it saves to
    /// [`Tui::edited`].
    Edit(String),
    Quit,
}

impl Tui {
    pub fn new(size: Size) -> Self {
        Self {
            model: Model::new(),
            main: Screen::Main(Main::default()),
            over: Vec::new(),
            message: MessageBox::default(),
            notice: None,
            size,
        }
    }

    /// The reads for a snapshot at `now`.
    pub fn snapshot_query(now: UnixMillis) -> SnapshotQuery {
        Model::snapshot_query(now)
    }

    /// The reads to make first: everything the app shows.
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

    /// Text pasted (bracketed paste): one block, its line breaks kept.
    pub fn paste(&mut self, text: &str) -> Vec<Out> {
        self.notice = None;
        let actions = self.with_top(|screen, ctx| screen.paste(text, ctx));
        let outs = self.act(actions);
        Self::once(outs)
    }

    /// What the operator's editor gave back, after [`Out::Edit`]: the text,
    /// or why it could not be edited (the text stays as it was).
    pub fn edited(&mut self, edited: Result<&str, &str>) -> Vec<Out> {
        match edited {
            Ok(text) => {
                let actions = self.with_top(|screen, ctx| screen.edited(text, ctx));
                let outs = self.act(actions);
                Self::once(outs)
            }
            Err(why) => {
                self.notice = Some(Notice::refusal(format!("not edited: {why}")));
                Vec::new()
            }
        }
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
        // The main screen follows the model even under the form or help.
        let mut ctx = Ctx {
            model: &mut self.model,
            message: &mut self.message,
            size: self.size,
        };
        let actions = self.main.changed(&mut ctx, &news);
        outs.extend(self.act(actions));
        Self::once(outs)
    }

    /// What to draw at `now`.
    pub fn view(&self, now: UnixMillis) -> View {
        let look = Look {
            model: &self.model,
            message: &self.message,
            notice: self.notice.as_ref(),
            size: self.size,
            now,
        };
        let mut view = self.main.view(&look, None);
        for screen in &self.over {
            view = screen.view(&look, Some(view));
        }
        view
    }

    /// Each read asked for once: the model and a screen may both ask for
    /// the same one.
    fn once(outs: Vec<Out>) -> Vec<Out> {
        let mut kept: Vec<Out> = Vec::with_capacity(outs.len());
        for out in outs {
            let repeated = match &out {
                Out::Read(_) => kept.contains(&out),
                Out::Send(_) | Out::CycleTheme | Out::Edit(_) | Out::Quit => false,
            };
            if !repeated {
                kept.push(out);
            }
        }
        kept
    }

    fn top(&self) -> &Screen {
        self.over.last().unwrap_or(&self.main)
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
        act(self.over.last_mut().unwrap_or(&mut self.main), &mut ctx)
    }

    /// Carries out what a screen asked for, and what a screen it opens
    /// asks for when shown.
    fn act(&mut self, actions: Vec<Action>) -> Vec<Out> {
        let mut outs = Vec::new();
        let mut pending: VecDeque<Action> = actions.into();
        while let Some(action) = pending.pop_front() {
            match action {
                Action::Read(read) => outs.push(Out::Read(read)),
                Action::Send(command) => outs.push(Out::Send(command)),
                Action::CycleTheme => outs.push(Out::CycleTheme),
                Action::Edit(text) => outs.push(Out::Edit(text)),
                Action::Quit => outs.push(Out::Quit),
                Action::Say(notice) => self.notice = Some(notice),
                Action::Push(screen) => {
                    self.over.push(screen);
                    pending.extend(self.with_top(Screen::shown));
                }
                Action::Pop => {
                    self.over.pop();
                }
                Action::Help => {
                    let rows = self.top().keys();
                    self.over.push(Screen::Help(Help::new(rows)));
                }
                Action::Verb(verb) => {
                    let mut ctx = Ctx {
                        model: &mut self.model,
                        message: &mut self.message,
                        size: self.size,
                    };
                    if let Screen::Main(main) = &mut self.main {
                        pending.extend(main.verb(verb, &mut ctx));
                    }
                }
            }
        }
        outs
    }

    #[cfg(test)]
    fn main(&self) -> &Main {
        match &self.main {
            Screen::Main(main) => main,
            Screen::NewRun(_) | Screen::Actions(_) | Screen::Help(_) => {
                unreachable!("the main screen is Main")
            }
        }
    }

    /// The entry the list has selected.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<screen::main::Item> {
        self.main().selected()
    }

    /// The run the list has selected.
    #[cfg(test)]
    pub(crate) fn shown_run(&self) -> Option<ren_recorder::id::RunId> {
        self.main().run()
    }

    #[cfg(test)]
    pub(crate) fn mode(&self) -> screen::main::Mode {
        self.main().mode()
    }

    /// The action menu, when it is on top.
    #[cfg(test)]
    pub(crate) fn top_actions(&self) -> Option<&screen::Actions> {
        match self.over.last() {
            Some(Screen::Actions(actions)) => Some(actions),
            Some(Screen::Main(_) | Screen::NewRun(_) | Screen::Help(_)) | None => None,
        }
    }

    /// Whether a screen is open over the main one: the form or the help.
    #[cfg(test)]
    pub(crate) fn overlaid(&self) -> bool {
        !self.over.is_empty()
    }
}
