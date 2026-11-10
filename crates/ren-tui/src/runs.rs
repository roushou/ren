//! Agent runs on the wall: which session each live run drives, what its
//! pane shows of it, and what the keys ask of it. The view reads runs as
//! the control API lists them; the shell sends what the keys ask.

use ren_harness::drive::{ApprovalAsk, ApprovalId, Decision, Prompt};
use ren_recorder::id::RunId;
use ren_store::view::SessionView;
use ren_supervisor::{RunSnapshot, RunState};

/// The gateway's runs, as last read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Runs {
    /// Not read yet.
    Unread,
    Read(Vec<RunSnapshot>),
    /// Runs cannot be read or driven from here, and why: no control
    /// access, or runs are off on the daemon.
    Unavailable(String),
}

/// What a key asked of a run; the shell sends it to the control API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunControl {
    /// Start Codex in this folder on the daemon's host.
    Start {
        dir: String,
    },
    Prompt {
        run: RunId,
        prompt: Prompt,
    },
    Answer {
        run: RunId,
        approval: ApprovalId,
        decision: Decision,
    },
    Interrupt(RunId),
    Stop(RunId),
}

/// A live run's mark on its session's pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunMark {
    pub run: RunId,
    pub phase: RunPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunPhase {
    /// Waiting for a prompt.
    Idle,
    /// A turn is running.
    Working,
    /// The agent waits for the operator: the oldest ask, and how many more.
    Waiting { ask: ApprovalAsk, more: usize },
}

/// Text being typed into the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// A prompt for a run.
    Prompt { run: RunId, text: String },
    /// The folder a new run starts in.
    Folder { text: String },
}

/// What the status line shows of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
    pub label: &'static str,
    pub text: String,
}

/// Live runs, and how many wait for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCounts {
    pub live: usize,
    pub waiting: usize,
}

impl Runs {
    /// Runs that have not ended.
    fn live(&self) -> impl Iterator<Item = &RunSnapshot> {
        let runs: &[RunSnapshot] = match self {
            Self::Read(runs) => runs,
            Self::Unread | Self::Unavailable(_) => &[],
        };
        runs.iter().filter(|run| !run.state.is_ended())
    }

    /// The live run driving `session`: the client names its conversation
    /// with the run's thread id, and the session is keyed by it.
    pub(crate) fn of(&self, session: &SessionView) -> Option<&RunSnapshot> {
        self.live().find(|run| match &run.state {
            RunState::Idle { thread } | RunState::Working { thread, .. } => {
                thread.to_string() == session.native_id
            }
            RunState::Starting | RunState::Stopping { .. } | RunState::Ended { .. } => false,
        })
    }

    /// What `run`'s pane shows of it, while it can take commands.
    pub(crate) fn mark(run: &RunSnapshot) -> Option<RunMark> {
        let phase = match &run.state {
            RunState::Idle { .. } => RunPhase::Idle,
            RunState::Working { approvals, .. } => match approvals.first() {
                Some(waiting) => RunPhase::Waiting {
                    ask: waiting.ask.clone(),
                    more: approvals.len() - 1,
                },
                None => RunPhase::Working,
            },
            RunState::Starting | RunState::Stopping { .. } | RunState::Ended { .. } => {
                return None;
            }
        };
        Some(RunMark {
            run: run.run,
            phase,
        })
    }

    /// Counts for the status line, once runs were read.
    pub(crate) fn counts(&self) -> Option<RunCounts> {
        match self {
            Self::Read(_) => Some(RunCounts {
                live: self.live().count(),
                waiting: self
                    .live()
                    .filter(|run| match &run.state {
                        RunState::Working { approvals, .. } => !approvals.is_empty(),
                        RunState::Starting
                        | RunState::Idle { .. }
                        | RunState::Stopping { .. }
                        | RunState::Ended { .. } => false,
                    })
                    .count(),
            }),
            Self::Unread | Self::Unavailable(_) => None,
        }
    }

    /// The native ids of sessions that live runs drive, which keep a pane.
    pub(crate) fn threads(&self) -> Vec<String> {
        self.live()
            .filter_map(|run| match &run.state {
                RunState::Idle { thread } | RunState::Working { thread, .. } => {
                    Some(thread.to_string())
                }
                RunState::Starting | RunState::Stopping { .. } | RunState::Ended { .. } => None,
            })
            .collect()
    }
}

impl Entry {
    pub(crate) fn line(&self) -> EntryLine {
        match self {
            Self::Prompt { text, .. } => EntryLine {
                label: "prompt",
                text: text.clone(),
            },
            Self::Folder { text } => EntryLine {
                label: "start codex in",
                text: text.clone(),
            },
        }
    }

    pub(crate) fn text(&mut self) -> &mut String {
        match self {
            Self::Prompt { text, .. } | Self::Folder { text } => text,
        }
    }
}
