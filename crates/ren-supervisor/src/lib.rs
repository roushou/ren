//! Runs: agent processes Ren starts in a folder and drives through their
//! client's structured mode. The primary type is [`Supervisor`], the handle
//! the gateway's routes use; [`SupervisorTask`] runs every run's I/O and is
//! returned to the caller to run, because components do not spawn.
//!
//! A run's lifecycle is the pure [`RunState`] machine (`run`); where a run may
//! work is [`Workdir`] (`workdir`); starting processes is the [`AgentHost`]
//! port (`host`), which the daemon implements. Clients keep their own sign-in:
//! the supervisor starts the official program and never handles credentials.

pub mod client;
mod entries;
pub mod host;
mod models;
pub mod project;
#[cfg(test)]
mod purity;
pub mod run;
mod task;
pub mod workdir;

pub use client::{Client, ClientStatus};
pub use host::{AgentHost, AgentProcess, KillSignal, KillSwitch};
pub use models::ModelsError;
pub use project::{Look, Project, ProjectGit, Search};
pub use run::{Approvals, ProcessExit, RunCommand, RunRefused, RunState, Waiting};
pub use task::SupervisorTask;
pub use workdir::{Roots, Workdir, WorkdirError};

use ren_harness::{
    Harness,
    drive::{CommandRefused, DriveEvent, LineLimit, Model, ModelId, Prompt},
    endpoint::GatewayEndpoint,
    launch::NotDrivable,
};
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    Recorder, RunUnrecorded,
    id::{IdSource, RunId},
};
use std::{io, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, oneshot};

/// What a supervisor is set up with.
pub struct SupervisorOptions {
    /// The folders runs may work in, as configured; the host resolves them.
    pub roots: Vec<PathBuf>,
    /// Where driven clients send their model traffic.
    pub gateway: GatewayEndpoint,
    /// Ren's version, reported to clients as theirs.
    pub version: String,
    pub line_limit: LineLimit,
}

/// The handle to every run: start one, command it, list them, and watch them.
/// Clones share the same runs.
#[derive(Clone)]
pub struct Supervisor {
    requests: mpsc::Sender<Request>,
    updates: broadcast::Sender<RunUpdate>,
}

/// A change to a run, as it happens.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunUpdate {
    State { run: RunId, state: RunState },
    Event { run: RunId, event: DriveEvent },
}

/// Which conversation a new run opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversation {
    New,
    /// Continue the client's own conversation, by its thread id: a run that
    /// ended, or was recovered after a restart, picks up where it left off.
    Resume(NativeThreadId),
}

/// One run as the supervisor holds it now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunSnapshot {
    pub run: RunId,
    pub harness: Harness,
    pub workdir: PathBuf,
    pub state: RunState,
}

/// Why the supervisor could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("cannot resolve a run root")]
    Root(#[source] io::Error),
    #[error(transparent)]
    Roots(WorkdirError),
}

/// Why a run did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Workdir(WorkdirError),
    #[error("cannot resolve the directory")]
    Resolve(#[source] io::Error),
    #[error(transparent)]
    NotDrivable(NotDrivable),
    #[error("the recorder has no room to record the run")]
    Unrecorded(RunUnrecorded),
    #[error("cannot start the client; check it is installed on PATH")]
    Spawn {
        run: RunId,
        #[source]
        source: io::Error,
    },
    #[error("the supervisor has stopped")]
    Gone,
    /// The ID source gave the nil UUID, which names no conversation.
    #[error("cannot name the new conversation: the ID source gave a nil UUID")]
    NilId,
}

/// Why a command was not carried out.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("no run {0}")]
    NoSuchRun(RunId),
    #[error(transparent)]
    Refused(RunRefused),
    #[error(transparent)]
    Protocol(CommandRefused),
    #[error("the supervisor has stopped")]
    Gone,
}

/// The supervisor has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the supervisor has stopped")]
pub struct Gone;

/// What a new run starts: which client, where, which conversation, on which
/// model, and its first prompt.
pub struct Start {
    pub harness: Harness,
    /// The folder, which must lie inside a root.
    pub dir: String,
    pub conversation: Conversation,
    /// With none, the client's own default.
    pub model: Option<ModelId>,
    /// Sent once the conversation is open.
    pub first: Option<Prompt>,
}

pub(crate) enum Request {
    Start {
        start: Start,
        reply: oneshot::Sender<Result<RunId, StartError>>,
    },
    Command {
        run: RunId,
        command: RunCommand,
        reply: oneshot::Sender<Result<(), CommandError>>,
    },
    List {
        reply: oneshot::Sender<Vec<RunSnapshot>>,
    },
    Projects {
        reply: oneshot::Sender<Vec<Project>>,
    },
    Clients {
        reply: oneshot::Sender<Vec<Client>>,
    },
    Models {
        harness: Harness,
        reply: models::ModelsReply,
    },
}

impl Supervisor {
    /// Requests that wait for the task beyond this are refused at once.
    const QUEUE: usize = 64;
    /// Updates a slow subscriber may fall behind by before it lags.
    const UPDATES: usize = 1024;

    /// A supervisor and the task that runs it. Roots are resolved by `host`
    /// now, so a link cannot change what they cover later. Every run is
    /// recorded through `recorder`.
    pub fn new<H: AgentHost>(
        options: SupervisorOptions,
        host: H,
        ids: Arc<dyn IdSource>,
        recorder: Recorder,
    ) -> Result<(Self, SupervisorTask<H>), SupervisorError> {
        let resolved = options
            .roots
            .iter()
            .map(|root| host.resolve(root))
            .collect::<Result<Vec<_>, _>>()
            .map_err(SupervisorError::Root)?;
        let roots = Roots::new(resolved).map_err(SupervisorError::Roots)?;
        let (requests, receiver) = mpsc::channel(Self::QUEUE);
        let (updates, _) = broadcast::channel(Self::UPDATES);
        let task = SupervisorTask {
            requests: receiver,
            updates: updates.clone(),
            host,
            ids,
            recorder,
            roots,
            gateway: options.gateway,
            version: options.version,
            line_limit: options.line_limit,
        };
        Ok((Self { requests, updates }, task))
    }

    /// Start a run.
    pub async fn start(&self, start: Start) -> Result<RunId, StartError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Start { start, reply })
            .await
            .map_err(|_| StartError::Gone)?;
        answer.await.map_err(|_| StartError::Gone)?
    }

    pub async fn command(&self, run: RunId, command: RunCommand) -> Result<(), CommandError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Command {
                run,
                command,
                reply,
            })
            .await
            .map_err(|_| CommandError::Gone)?;
        answer.await.map_err(|_| CommandError::Gone)?
    }

    /// Every run the supervisor knows of, ended ones included.
    pub async fn runs(&self) -> Result<Vec<RunSnapshot>, Gone> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::List { reply })
            .await
            .map_err(|_| Gone)?;
        answer.await.map_err(|_| Gone)
    }

    /// The projects a run can start in: the git repositories under the
    /// roots, and each root's folders with none below them.
    pub async fn projects(&self) -> Result<Vec<Project>, Gone> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Projects { reply })
            .await
            .map_err(|_| Gone)?;
        answer.await.map_err(|_| Gone)
    }

    /// Every client, and whether a run can start with it here.
    pub async fn clients(&self) -> Result<Vec<Client>, Gone> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Clients { reply })
            .await
            .map_err(|_| Gone)?;
        answer.await.map_err(|_| Gone)
    }

    /// The models `harness` offers, as it lists them itself. The first
    /// non-empty answer is kept for as long as the supervisor runs.
    pub async fn models(&self, harness: Harness) -> Result<Vec<Model>, ModelsError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Models { harness, reply })
            .await
            .map_err(|_| ModelsError::Gone)?;
        answer.await.map_err(|_| ModelsError::Gone)?
    }

    /// Updates from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<RunUpdate> {
        self.updates.subscribe()
    }
}
