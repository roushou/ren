//! The screens, one type each. A [`Screen`] is one of a fixed set, so it is
//! an enum: adding one is a file here and a variant, and the compiler lists
//! every place that has to say what it does.
//!
//! Every screen has the same shape. It reads the model through a [`Look`]
//! to say what to draw (`view`), takes keys through a [`Ctx`] (`press`),
//! notices when the model changed (`changed`), and answers with
//! [`Action`]s: reads, commands for the control API, and moves between
//! screens. Screens hold only their own state (a choice, a scroll, a
//! search); everything read lives in the model.

mod chrome;
mod composer;
mod help;
mod home;
mod message;
mod resume;
mod run;
mod slash;
#[cfg(test)]
mod tests;
pub(crate) mod wall;

pub use slash::Suggestion;

pub(crate) use help::Help;
pub(crate) use home::Home;
pub(crate) use message::MessageBox;
pub(crate) use resume::Resume;
pub(crate) use run::RunScreen;
pub(crate) use wall::Wall;

use crate::keys::HelpRow;
use crate::model::{Command, Model, Read};
use crate::ui::View;
use crate::{Key, Size};
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;

/// One screen.
#[derive(Debug)]
pub(crate) enum Screen {
    /// Every session's traffic.
    Wall(Wall),
    /// The live runs, when none is open.
    Home(Home),
    /// One run's conversation.
    Run(RunScreen),
    /// Every run, to pick one, the chosen one previewed.
    Resume(Resume),
    /// The keys of the screen under it.
    Help(Help),
}

/// What a screen asks for.
#[derive(Debug)]
pub(crate) enum Action {
    Read(Read),
    Send(Command),
    /// Open a screen over this one.
    Push(Screen),
    /// Put a screen in this one's place.
    Replace(Screen),
    /// Close this screen, back to the one under it.
    Pop,
    /// Show the wall: the one under this screen, or a new one.
    Wall,
    /// Show this screen's keys over it.
    Help,
    /// Say something under the message box until the next key.
    Say(Notice),
    /// Preview the next theme; the shell owns the local preference.
    CycleTheme,
    Quit,
}

/// A line under the message box until the next key.
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
    /// The message box, shared by the screens that type into it.
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
    /// The console's way in: `run` if one is named, else the live runs.
    pub fn runs(run: Option<RunId>) -> Self {
        match run {
            Some(run) => Self::Run(RunScreen::new(run)),
            None => Self::Home(Home::default()),
        }
    }

    /// Just shown, on top: what it reads first.
    pub fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match self {
            Self::Wall(screen) => screen.shown(ctx),
            Self::Home(screen) => screen.changed(ctx, &[]),
            Self::Run(screen) => screen.shown(ctx),
            Self::Resume(screen) => screen.shown(ctx),
            Self::Help(_) => Vec::new(),
        }
    }

    /// The model changed while it is on top.
    pub fn changed(&mut self, ctx: &mut Ctx<'_>, news: &[News]) -> Vec<Action> {
        match self {
            Self::Wall(screen) => screen.changed(ctx),
            Self::Home(screen) => screen.changed(ctx, news),
            Self::Run(screen) => screen.changed(ctx, news),
            Self::Resume(screen) => screen.changed(ctx),
            Self::Help(_) => Vec::new(),
        }
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match self {
            Self::Wall(screen) => screen.press(key, ctx),
            Self::Home(screen) => screen.press(key, ctx),
            Self::Run(screen) => screen.press(key, ctx),
            Self::Resume(screen) => screen.press(key, ctx),
            Self::Help(screen) => screen.press(key),
        }
    }

    pub fn view(&self, look: &Look<'_>) -> View {
        match self {
            Self::Wall(screen) => screen.view(look),
            Self::Home(screen) => screen.view(look),
            Self::Run(screen) => screen.view(look),
            Self::Resume(screen) => screen.view(look),
            // It is laid over the screen under it.
            Self::Help(_) => View::Empty,
        }
    }

    /// Its keys, as the help lists them.
    pub fn keys(&self) -> Vec<HelpRow> {
        match self {
            Self::Wall(screen) => screen.keys(),
            Self::Home(_) | Self::Run(_) | Self::Resume(_) => RunScreen::help(),
            Self::Help(_) => Vec::new(),
        }
    }

    /// The run it shows, if any.
    #[cfg(test)]
    pub fn run(&self) -> Option<RunId> {
        match self {
            Self::Run(screen) => Some(screen.run()),
            Self::Wall(_) | Self::Home(_) | Self::Resume(_) | Self::Help(_) => None,
        }
    }
}
