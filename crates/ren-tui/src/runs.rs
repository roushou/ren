//! Agent runs on the wall: which session each live run drives, what its
//! pane shows of it, and what the keys ask of it. The view reads runs as
//! the control API lists them; the shell sends what the keys ask.

use ren_harness::{
    Harness,
    drive::{ApprovalAsk, ApprovalId, Decision, Prompt},
};
use ren_recorder::id::RunId;
use ren_store::view::SessionView;
use ren_supervisor::{Folder, RunSnapshot, RunState};

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
    /// Start this client in this folder on the daemon's host.
    Start {
        harness: Harness,
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

/// A prompt being typed into the status line, for a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) run: RunId,
    pub(crate) text: String,
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

/// How a [`RunControl`] was answered when the daemon took it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    /// A command reached the run.
    Sent,
    /// A new run started.
    Started(RunId),
}

/// The folders a run can start in, as the daemon lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Folders {
    Reading,
    Read(Vec<Folder>),
    /// The list could not be read, and why.
    Failed(String),
}

/// Choosing where a new run starts, and which client: typing narrows the
/// daemon's folders, or names any absolute path on its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Picker {
    harness: Harness,
    filter: String,
    folders: Folders,
    /// Which matching folder is chosen.
    chosen: usize,
}

/// What a key did to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Picked {
    Open,
    Cancelled,
    Start {
        harness: Harness,
        dir: String,
    },
    /// Enter with nothing to start: why.
    Nothing(&'static str),
}

/// What the wall shows of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerView {
    /// The client a run would start, as its name reads.
    pub client: &'static str,
    pub filter: String,
    /// Matching folders, `(name, path)`, the chosen one marked.
    pub rows: Vec<PickerRow>,
    /// Why there is nothing to list, while there is not.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub name: String,
    pub path: String,
    pub chosen: bool,
}

impl Picker {
    pub(crate) fn new() -> Self {
        Self {
            harness: Harness::Codex,
            filter: String::new(),
            folders: Folders::Reading,
            chosen: 0,
        }
    }

    pub(crate) fn read(&mut self, folders: Folders) {
        self.folders = folders;
        self.chosen = 0;
    }

    /// The folders whose name or path holds the filter, ignoring case.
    fn matching(&self) -> Vec<&Folder> {
        let filter = self.filter.to_lowercase();
        match &self.folders {
            Folders::Read(folders) => folders
                .iter()
                .filter(|folder| {
                    folder.name.to_lowercase().contains(&filter)
                        || folder.path.to_lowercase().contains(&filter)
                })
                .collect(),
            Folders::Reading | Folders::Failed(_) => Vec::new(),
        }
    }

    pub(crate) fn press(&mut self, key: crate::Key) -> Picked {
        use crate::Key;
        match key {
            Key::Char(ch) => {
                self.filter.push(ch);
                self.chosen = 0;
            }
            Key::Backspace => {
                self.filter.pop();
                self.chosen = 0;
            }
            Key::Tab | Key::BackTab => {
                self.harness = match self.harness {
                    Harness::Codex => Harness::ClaudeCode,
                    Harness::ClaudeCode | Harness::Pi | Harness::OpenCode => Harness::Codex,
                };
            }
            Key::Up => self.chosen = self.chosen.saturating_sub(1),
            Key::Down => {
                let last = self.matching().len().saturating_sub(1);
                self.chosen = (self.chosen + 1).min(last);
            }
            Key::Left | Key::Right => {}
            Key::Esc | Key::Interrupt => return Picked::Cancelled,
            Key::Enter => {
                let harness = self.harness;
                if let Some(folder) = self.matching().get(self.chosen) {
                    return Picked::Start {
                        harness,
                        dir: folder.path.clone(),
                    };
                }
                let typed = self.filter.trim();
                if typed.starts_with('/') {
                    return Picked::Start {
                        harness,
                        dir: typed.to_owned(),
                    };
                }
                return Picked::Nothing(
                    "no folder matches; type part of a name, or a full path on the daemon's host",
                );
            }
        }
        Picked::Open
    }

    pub(crate) fn view(&self) -> PickerView {
        let matching = self.matching();
        let note = match &self.folders {
            Folders::Reading => Some("reading the daemon's folders…".to_owned()),
            Folders::Failed(why) => Some(format!("cannot list folders: {why}")),
            Folders::Read(_) if matching.is_empty() && self.filter.starts_with('/') => {
                Some(format!("enter starts in {}", self.filter.trim()))
            }
            Folders::Read(_) if matching.is_empty() => Some("no folder matches".to_owned()),
            Folders::Read(_) => None,
        };
        PickerView {
            client: match self.harness {
                Harness::ClaudeCode => "Claude Code",
                Harness::Codex | Harness::Pi | Harness::OpenCode => "Codex",
            },
            filter: self.filter.clone(),
            rows: matching
                .iter()
                .enumerate()
                .map(|(index, folder)| PickerRow {
                    name: folder.name.clone(),
                    path: folder.path.clone(),
                    chosen: index == self.chosen,
                })
                .collect(),
            note,
        }
    }
}

/// Why a run command was not carried out: the daemon's error code, read
/// once, or the shell's own reason when the daemon was not reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    RunsDisabled,
    InvalidDirectory,
    DirectoryNotFound,
    NotDrivable,
    SpawnFailed,
    Unrecorded,
    Busy,
    Starting,
    NoTurn,
    NoSuchApproval,
    Ended,
    NotFound,
    Denied,
    /// A code this view does not know, or the shell's reason, as given.
    Other(String),
}

impl Refusal {
    /// The daemon's error code, or any other text, as a refusal.
    pub fn parse(code: &str) -> Self {
        match code {
            "runs_disabled" => Self::RunsDisabled,
            "invalid_directory" => Self::InvalidDirectory,
            "directory_not_found" => Self::DirectoryNotFound,
            "not_drivable" => Self::NotDrivable,
            "spawn_failed" => Self::SpawnFailed,
            "unrecorded" => Self::Unrecorded,
            "busy" => Self::Busy,
            "starting" => Self::Starting,
            "not_working" | "turn_not_started" => Self::NoTurn,
            "no_such_approval" => Self::NoSuchApproval,
            "stopping" | "ended" => Self::Ended,
            "not_found" => Self::NotFound,
            "unauthorized" | "forbidden" => Self::Denied,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RunsDisabled => "runs are off on the daemon: add a folder to [runs] roots",
            Self::InvalidDirectory => {
                "that folder is not a full path under one of the daemon's run roots"
            }
            Self::DirectoryNotFound => "that folder does not exist on the daemon's host",
            Self::NotDrivable => "Ren cannot drive that client yet",
            Self::SpawnFailed => "the client did not start: is it installed on the daemon's host?",
            Self::Unrecorded => "the recorder has no room to record a new run right now",
            Self::Busy => "a turn is already running",
            Self::Starting => "the run is still opening its conversation",
            Self::NoTurn => "no turn is running",
            Self::NoSuchApproval => "that approval no longer waits",
            Self::Ended => "the run is stopping or has ended",
            Self::NotFound => "the daemon does not know that run",
            Self::Denied => "the daemon refused this terminal's control access",
            Self::Other(text) => text,
        })
    }
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
        EntryLine {
            label: "prompt",
            text: self.text.clone(),
        }
    }
}
