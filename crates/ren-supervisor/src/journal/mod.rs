//! A process's journal, pure: everything the daemon gave a client process,
//! in the order it gave it, so a daemon that restarts while the process
//! goes on can rebuild it exactly. The process's output is not in the
//! journal: its run host keeps every byte of it (the spool), and the
//! journal says how far the daemon had read it before each command. A
//! [`Replay`] feeds the journal and the spool through the same `Process`
//! machine the daemon ran, discarding what it would have done (its lines
//! were sent, its entries recorded), and ends where the daemon left off:
//! the codec's requests, approvals, and turn as they were.
//!
//! Each entry is one line of JSON.

use crate::process::{Process, Rest, Serving};
use crate::run::RunCommand;
use crate::workdir::Workdir;
use ren_harness::Harness;
use ren_harness::drive::{ApprovalId, Decision, Driver, Opening};
use ren_harness::lines::{LineLimit, Lines};
use ren_harness::prompt::{KeptPrompt, KeptPromptError};
use ren_harness::question::Replies;
use ren_harness::settings::RunSettings;
use ren_recorder::id::{RunId, SessionId};
use serde::{Deserialize, Serialize};

/// One entry of a journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub(crate) enum Entry {
    /// The process started: always the first entry.
    Started(Box<Start>),
    /// The daemon read the process's output through this byte offset of the
    /// spool, and did all it said.
    Read { upto: u64 },
    /// The operator's command, given after everything before it.
    Command(Command),
    /// The idle stop armed for this rest ran out.
    RestOver { rest: Rest },
}

/// How a process was started: what the daemon opened its codec with and
/// who it serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Start {
    pub run: RunId,
    pub session: SessionId,
    pub harness: Harness,
    pub workdir: String,
    pub settings: RunSettings,
    pub opening: Opening,
    /// Ren's version, as the codec tells its client.
    pub version: String,
    /// The seed the codec mints its own ids from.
    pub seed: uuid::Uuid,
    /// The first prompt, held until the conversation opens.
    pub first: Option<KeptPrompt>,
}

/// An operator's command as the journal keeps it: prompts whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum Command {
    Prompt {
        prompt: KeptPrompt,
    },
    Queue {
        prompt: KeptPrompt,
    },
    Unqueue,
    Steer {
        prompt: KeptPrompt,
    },
    Configure {
        settings: RunSettings,
    },
    Answer {
        approval: ApprovalId,
        decision: Decision,
    },
    /// Replies to questions; a secret reply's text is written empty.
    Reply {
        approval: ApprovalId,
        replies: Replies,
    },
    Interrupt,
    Stop,
}

/// Why a journal could not be replayed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ReplayError {
    #[error("journal line {line} is not an entry: {reason}")]
    Unreadable { line: usize, reason: String },
    #[error("the journal does not begin with the process's start")]
    NoStart,
    #[error("the journal starts the process twice")]
    StartedTwice,
    #[error("the journal reads the output back to {upto} after reading to {read}")]
    ReadBack { read: u64, upto: u64 },
    #[error("the journal read the output to {upto}, but the spool holds only {read} bytes")]
    SpoolShort { read: u64, upto: u64 },
    #[error("the journal's folder {0:?} is not absolute")]
    Folder(String),
    #[error(transparent)]
    Prompt(#[from] KeptPromptError),
}

impl Entry {
    /// The entry as one line of JSON, ending in a newline.
    pub(crate) fn line(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut line = serde_json::to_vec(self)?;
        line.push(b'\n');
        Ok(line)
    }

    /// The entries of a journal, one a line. A last line cut short (a
    /// daemon stopped while writing it) is not an entry yet.
    pub(crate) fn read(journal: &[u8]) -> Result<Vec<Self>, ReplayError> {
        let whole = match journal.iter().rposition(|byte| *byte == b'\n') {
            Some(end) => journal.get(..=end).unwrap_or_default(),
            None => &[],
        };
        whole
            .split(|byte| *byte == b'\n')
            .enumerate()
            .filter(|(_, line)| !line.is_empty())
            .map(|(index, line)| {
                serde_json::from_slice(line).map_err(|error| ReplayError::Unreadable {
                    line: index + 1,
                    reason: error.to_string(),
                })
            })
            .collect()
    }
}

impl From<&RunCommand> for Command {
    fn from(command: &RunCommand) -> Self {
        match command {
            RunCommand::Prompt(prompt) => Self::Prompt {
                prompt: prompt.into(),
            },
            RunCommand::Queue(prompt) => Self::Queue {
                prompt: prompt.into(),
            },
            RunCommand::Unqueue => Self::Unqueue,
            RunCommand::Steer(prompt) => Self::Steer {
                prompt: prompt.into(),
            },
            RunCommand::Configure(settings) => Self::Configure {
                settings: settings.clone(),
            },
            RunCommand::Answer { approval, decision } => Self::Answer {
                approval: approval.clone(),
                decision: *decision,
            },
            RunCommand::Reply { approval, replies } => Self::Reply {
                approval: approval.clone(),
                replies: replies.clone(),
            },
            RunCommand::Interrupt => Self::Interrupt,
            RunCommand::Stop => Self::Stop,
        }
    }
}

impl Command {
    /// The command as it was given.
    pub(crate) fn command(&self) -> Result<RunCommand, ReplayError> {
        Ok(match self {
            Self::Prompt { prompt } => RunCommand::Prompt(prompt.prompt()?),
            Self::Queue { prompt } => RunCommand::Queue(prompt.prompt()?),
            Self::Unqueue => RunCommand::Unqueue,
            Self::Steer { prompt } => RunCommand::Steer(prompt.prompt()?),
            Self::Configure { settings } => RunCommand::Configure(settings.clone()),
            Self::Answer { approval, decision } => RunCommand::Answer {
                approval: approval.clone(),
                decision: *decision,
            },
            Self::Reply { approval, replies } => RunCommand::Reply {
                approval: approval.clone(),
                replies: replies.clone(),
            },
            Self::Interrupt => RunCommand::Interrupt,
            Self::Stop => RunCommand::Stop,
        })
    }
}

/// A process being rebuilt from its journal and its spool.
#[derive(Debug)]
pub(crate) struct Replay {
    process: Process,
    lines: Lines,
    read: u64,
}

/// A process rebuilt: where the daemon left it, its output framed as far as
/// the daemon had read it, and that offset, where reading goes on.
#[derive(Debug)]
pub(crate) struct Replayed {
    pub process: Process,
    pub lines: Lines,
    pub read: u64,
}

impl Replay {
    /// A process as `start` opened it.
    pub(crate) fn begin(start: &Start, limit: LineLimit) -> Result<Self, ReplayError> {
        let workdir = Workdir::journaled(&start.workdir)
            .ok_or_else(|| ReplayError::Folder(start.workdir.clone()))?;
        let first = start.first.as_ref().map(KeptPrompt::prompt).transpose()?;
        let (codec, opening) = Driver::open(
            start.harness,
            start.opening.clone(),
            &start.version,
            start.seed,
        );
        let serving = Serving {
            run: start.run,
            session: start.session,
            harness: start.harness,
            workdir,
            settings: start.settings.clone(),
        };
        let (process, _sent) = Process::opening(serving, codec, opening, first);
        Ok(Self {
            process,
            lines: Lines::new(limit),
            read: 0,
        })
    }

    /// How far into the spool it has read.
    pub(crate) fn read(&self) -> u64 {
        self.read
    }

    /// The spool's next bytes, as the daemon read them.
    pub(crate) fn output(&mut self, bytes: &[u8]) {
        for frame in self.lines.push(bytes) {
            let _done = self.process.receive(frame);
        }
        self.read += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    }

    /// An entry after the start. For [`Entry::Read`], the spool's bytes
    /// from [`Replay::read`] to its offset must be given to
    /// [`Replay::output`] first; this checks that they were.
    pub(crate) fn entry(&mut self, entry: &Entry) -> Result<(), ReplayError> {
        match entry {
            Entry::Started(_) => return Err(ReplayError::StartedTwice),
            Entry::Read { upto } => {
                let (read, upto) = (self.read, *upto);
                if upto < read {
                    return Err(ReplayError::ReadBack { read, upto });
                }
                if upto > read {
                    return Err(ReplayError::SpoolShort { read, upto });
                }
            }
            // A command the run refused then is refused again, and changes
            // nothing again.
            Entry::Command(command) => {
                let _done = self.process.command(command.command()?);
            }
            Entry::RestOver { rest } => {
                let _done = self.process.rest_over(*rest);
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Replayed {
        Replayed {
            process: self.process,
            lines: self.lines,
            read: self.read,
        }
    }
}

#[cfg(test)]
mod tests;
