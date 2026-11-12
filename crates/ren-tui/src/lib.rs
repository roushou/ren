//! Ren in the terminal: one program with two routes over one gateway. The
//! wall ([`wall`]) watches every session's traffic, one pane per session;
//! the console ([`console`]) works with the daemon's agent runs, one
//! conversation at a time.
//!
//! [`Tui`] is sans-IO. It holds both routes, hands keys to the one shown
//! and what was read to the one it is for, and answers with [`Out`]s: the
//! reads each route needs and the commands to send through the control
//! API. [`Tui::frame`] is what to draw; [`Screen`] is the only part that
//! touches the terminal.

pub mod console;
mod event;
#[cfg(test)]
mod purity;
mod screen;
#[cfg(test)]
mod tests;
mod theme;
pub mod wall;

pub use event::{Key, Size};
pub use screen::{Drawn, Screen, ScreenError, ScreenEvent, ScreenEvents};
pub use theme::{Ink, Theme, ThemeName};

use console::Console;
use ren_recorder::time::UnixMillis;
use ren_supervisor::RunSnapshot;
use wall::{Pressed, Runs, Wall};

/// The terminal's views of one gateway, one of them shown.
#[derive(Debug)]
pub struct Tui {
    route: Route,
    wall: Wall,
    console: Console,
}

/// Which view fills the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Every session's traffic, watched.
    Wall,
    /// The agent runs, worked with.
    Console,
}

/// What was read, or what happened, and which route it is for.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Wall(wall::Input),
    Console(console::Input),
    /// The daemon's live runs, which both routes show, or why they cannot
    /// be read.
    Runs(Result<Vec<RunSnapshot>, String>),
    Resize(Size),
}

/// What the shell does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Wall(wall::Read),
    Console(console::Read),
    /// A command for the control API.
    Send(console::Command),
    /// Preview the next theme; the shell owns the local preference.
    CycleTheme,
    Quit,
}

/// What to draw: the shown route's frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Wall(wall::Frame),
    Console(console::Frame),
}

impl Tui {
    pub fn new(size: Size, route: Route) -> Self {
        Self {
            route,
            wall: Wall::new(size),
            console: Console::new(size),
        }
    }

    pub fn route(&self) -> Route {
        self.route
    }

    /// The reads to make first: both routes' own, so each is ready when
    /// shown.
    pub fn start(&mut self) -> Vec<Out> {
        let mut outs = vec![Out::Wall(wall::Read::Snapshot), Out::Wall(wall::Read::Runs)];
        let console = self.console.start();
        outs.extend(self.console_outs(console));
        outs
    }

    /// Takes one key press, for the route shown.
    pub fn press(&mut self, key: Key) -> Vec<Out> {
        match self.route {
            Route::Wall => match self.wall.press(key) {
                Pressed::Continue(reads) => reads.into_iter().map(Out::Wall).collect(),
                Pressed::CycleTheme => vec![Out::CycleTheme],
                Pressed::Runs(run) => {
                    self.route = Route::Console;
                    let outs = self.console.show(run);
                    self.console_outs(outs)
                }
                Pressed::Quit => vec![Out::Quit],
            },
            Route::Console => {
                let outs = self.console.press(key);
                self.console_outs(outs)
            }
        }
    }

    /// Takes one input; returns what it calls for.
    pub fn apply(&mut self, input: Input) -> Vec<Out> {
        match input {
            Input::Wall(input) => self.wall.apply(input).into_iter().map(Out::Wall).collect(),
            Input::Console(input) => {
                let outs = self.console.apply(input);
                self.console_outs(outs)
            }
            Input::Runs(runs) => {
                let listed = match &runs {
                    Ok(runs) => Runs::Read(runs.clone()),
                    Err(why) => Runs::Unavailable(why.clone()),
                };
                let mut outs: Vec<Out> = self
                    .wall
                    .apply(wall::Input::Runs(listed))
                    .into_iter()
                    .map(Out::Wall)
                    .collect();
                let console = self.console.apply(console::Input::Runs(runs));
                outs.extend(self.console_outs(console));
                outs
            }
            Input::Resize(size) => {
                let mut outs: Vec<Out> = self
                    .wall
                    .apply(wall::Input::Resize(size))
                    .into_iter()
                    .map(Out::Wall)
                    .collect();
                let console = self.console.apply(console::Input::Resize(size));
                outs.extend(self.console_outs(console));
                outs
            }
        }
    }

    /// What to draw at `now`.
    pub fn frame(&self, now: UnixMillis) -> Frame {
        match self.route {
            Route::Wall => Frame::Wall(self.wall.frame(now)),
            Route::Console => Frame::Console(self.console.frame()),
        }
    }

    /// The console's outs as the shell's; asking for the wall shows it,
    /// with a fresh snapshot.
    fn console_outs(&mut self, outs: Vec<console::Out>) -> Vec<Out> {
        outs.into_iter()
            .map(|out| match out {
                console::Out::Read(read) => Out::Console(read),
                console::Out::Send(command) => Out::Send(command),
                console::Out::Wall => {
                    self.route = Route::Wall;
                    Out::Wall(wall::Read::Snapshot)
                }
                console::Out::Quit => Out::Quit,
            })
            .collect()
    }
}
