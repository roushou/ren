//! The screens. There is one, `main`: the session list beside the selected
//! session. The new-run form, the action menu, and the help open over it
//! and close back to it. [`Screen`] is an enum because the set is ours and fixed.
//!
//! Every screen has the same shape: it reads the model through a [`Look`]
//! to say what to draw (`view`), takes keys through a [`Ctx`] (`press`),
//! notices when the model changed (`changed`), and answers with
//! [`Action`]s. Screens hold only their own state; everything read lives
//! in the model.

mod actions;
mod composer;
mod help;
pub(crate) mod main;
mod message;
mod new_run;
#[cfg(test)]
mod tests;

pub(crate) use actions::Actions;
pub(crate) use help::Help;
pub(crate) use main::Main;
pub(crate) use message::MessageBox;
pub(crate) use new_run::NewRun;

use crate::keys::HelpRow;
use crate::model::{Command, Model, Read};
use crate::ui::View;
use crate::{Key, Size};
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;

/// One screen.
#[derive(Debug)]
pub(crate) enum Screen {
    /// The session list beside the selected session: always at the bottom.
    Main(Main),
    /// Starting a run, over the main screen.
    NewRun(NewRun),
    /// What can be done with the selection, over the main screen.
    Actions(Actions),
    /// The keys, over the screen under it.
    Help(Help),
}

/// What a screen asks for.
#[derive(Debug)]
pub(crate) enum Action {
    Read(Read),
    Send(Command),
    /// Open a screen over this one.
    Push(Screen),
    /// Close this screen, back to the one under it.
    Pop,
    /// Show the keys of this screen over it.
    Help,
    /// Do this to the main screen's selection.
    Verb(main::Verb),
    /// Say something in the key bar until the next key.
    Say(Notice),
    /// Preview the next theme; the shell owns the local preference.
    CycleTheme,
    Quit,
}

/// A line in the key bar until the next key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub text: String,
    pub tone: NoticeTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeTone {
    Info,
    Refusal,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: NoticeTone::Info,
        }
    }

    pub fn refusal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: NoticeTone::Refusal,
        }
    }
}

/// What a screen may change while it takes a key or a change.
pub(crate) struct Ctx<'a> {
    pub model: &'a mut Model,
    /// The message box for writing to runs.
    pub message: &'a mut MessageBox,
    pub size: Size,
}

/// What a screen reads to say what to draw.
pub(crate) struct Look<'a> {
    pub model: &'a Model,
    pub message: &'a MessageBox,
    pub notice: Option<&'a Notice>,
    pub size: Size,
    pub now: UnixMillis,
}

/// What happened in the model that a screen may act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum News {
    /// A run asked for here started.
    Started(RunId),
}

impl Screen {
    /// Just shown, on top: what it reads first.
    pub fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match self {
            Self::Main(screen) => screen.shown(ctx),
            Self::NewRun(screen) => screen.shown(),
            Self::Actions(_) | Self::Help(_) => Vec::new(),
        }
    }

    /// The model changed while it is on top.
    pub fn changed(&mut self, ctx: &mut Ctx<'_>, news: &[News]) -> Vec<Action> {
        match self {
            Self::Main(screen) => screen.changed(ctx, news),
            Self::NewRun(_) | Self::Actions(_) | Self::Help(_) => Vec::new(),
        }
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match self {
            Self::Main(screen) => screen.press(key, ctx),
            Self::NewRun(screen) => screen.press(key, ctx),
            Self::Actions(screen) => screen.press(key),
            Self::Help(screen) => screen.press(key),
        }
    }

    /// What to draw, over `under` for a screen that opens over another.
    pub fn view(&self, look: &Look<'_>, under: Option<View>) -> View {
        match self {
            Self::Main(screen) => screen.view(look),
            Self::NewRun(screen) => screen.view(under.unwrap_or(View::Empty), look),
            Self::Actions(screen) => screen.over(under.unwrap_or(View::Empty)),
            Self::Help(screen) => screen.over(under.unwrap_or(View::Empty)),
        }
    }

    /// Its keys, as the help lists them.
    pub fn keys(&self) -> Vec<HelpRow> {
        match self {
            Self::Main(_) => Main::keys(),
            Self::NewRun(_) | Self::Actions(_) | Self::Help(_) => Vec::new(),
        }
    }
}
