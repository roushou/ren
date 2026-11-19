//! Runs: agent processes Ren starts in a folder and drives through their
//! client's structured mode. The primary type is [`Supervisor`], the handle
//! the gateway's routes use; [`SupervisorTask`] runs every run's I/O and is
//! returned to the caller to run, because components do not spawn.
//!
//! A run's lifecycle is the pure [`RunState`] machine (`run`); where a run may
//! work is [`Workdir`] (`workdir`); starting processes is the [`AgentHost`]
//! port (`host`), which the daemon implements. Clients keep their own sign-in:
//! the supervisor starts the official program and never handles credentials.
//! It can run a client's own sign-in commands (`sign_in`, run by `login`):
//! its status, and its login, whose link and code it relays.

mod attachments;
pub use attachments::{AttachmentUse, KeptAttachment};
mod client;
mod entries;
mod files;
mod host;
mod journal;
mod process;
mod project;
#[cfg(test)]
mod purity;
mod run;
mod session;
mod sign_in;
mod task;
mod workdir;
mod workspace;

pub use client::{Client, ClientStatus};
pub use files::{FileListing, FileMatch, FileQuery, FoundPath, PathKind, WalkLimits};
pub use host::{
    AgentHost, AgentProcess, ClientStamp, HeldRun, KillSignal, KillSwitch, RunTools, Scratch,
    Tenure, Untracked,
};
pub use project::{Look, PlaceKind, Project, ProjectGit, Search};
pub use run::{
    Approvals, Delivery, ProcessExit, RunAction, RunCommand, RunEvent, RunPhase, RunRefused,
    RunState, RunTenure, StopCause, Transition, Waiting,
};
pub use session::{
    SessionAction, SessionInput, SessionPhase, SessionRefused, SessionState, Shelving,
};
pub use sign_in::{
    Awaiting, Login, LoginEnd, LoginInput, LoginOutcome, LoginRefused, SignInState, Waited,
};
pub use task::SupervisorTask;
pub use task::listing::ListingError;
pub use task::login::SignInError;
use task::{listing, login};
pub use workdir::{Bounds, Roots, Workdir, WorkdirError};

use attachments::AttachmentFolders;
use ren_harness::{
    Harness,
    attachment::{Attachment, AttachmentError, Limits, NotAccepted, PromptLimitError, Takes},
    commands::ClientCommand,
    drive::{CommandRefused, DriveEvent},
    endpoint::GatewayEndpoint,
    lines::LineLimit,
    models::Model,
    settings::RunSettings,
    sign_in::{SignInAnswer, SignInMethod},
};
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    Recorder, RunUnrecorded,
    id::{AttachmentId, IdSource, RunId, SessionId},
};
use std::{io, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, mpsc, oneshot};

/// What a supervisor is set up with.
pub struct SupervisorOptions {
    /// The folders runs may work in, as configured; the host resolves them.
    pub roots: Vec<PathBuf>,
    /// Other places runs may start in (`~/.config`), offered only when
    /// asked for: as configured, resolved by the host.
    pub places: Vec<PathBuf>,
    /// Where scratch runs each get a new folder; created when missing.
    /// None: runs cannot start in scratch.
    pub scratch: Option<PathBuf>,
    /// Where workspaces are made: a worktree of a repository per session
    /// that asks for one; created when missing. None: no workspaces.
    pub worktrees: Option<PathBuf>,
    /// Where each session's attachments are kept, a folder per session;
    /// Ren's own, made when missing. None: sessions take no attachments.
    pub attachments: Option<PathBuf>,
    /// How much an attachment, and a prompt's attachments, may hold.
    pub attachment_limits: Limits,
    /// The operator's home, for showing paths as `~/…`.
    pub home: Option<PathBuf>,
    /// When each client's models are first listed.
    pub warmup: Warmup,
    /// Where driven clients send their model traffic.
    pub gateway: GatewayEndpoint,
    /// Ren's version, reported to clients as theirs.
    pub version: String,
    pub line_limit: LineLimit,
    /// When a client sitting idle between turns is stopped.
    pub idle: IdleStop,
}

/// When a client sitting idle between turns is stopped. Its session goes
/// on: the next prompt starts a client that resumes the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleStop {
    Never,
    After(std::time::Duration),
}

impl IdleStop {
    /// Ten minutes: a client held for a turn soon after, and given back
    /// when the operator has moved on.
    pub const DEFAULT: Self = Self::After(std::time::Duration::from_secs(600));
}

/// The handle to every run: start one, command it, list them, and watch them.
/// Clones share the same runs.
#[derive(Clone)]
pub struct Supervisor {
    requests: mpsc::Sender<Request>,
    updates: broadcast::Sender<RunUpdate>,
    sessions: broadcast::Sender<SessionSnapshot>,
    /// The attachment limits; `None` when sessions take no attachments.
    attachment_limits: Option<Limits>,
}

/// A session as the supervisor serves it: what it is, where it stands,
/// and its newest process, whose state the run feed carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub session: SessionId,
    pub harness: Harness,
    pub workdir: PathBuf,
    /// What its next turn runs on; each left out is the client's own.
    pub settings: RunSettings,
    pub state: SessionState,
    pub process: Option<RunId>,
    /// Whether its client takes images with a prompt.
    pub images: ImageTaking,
}

/// Whether a session's client takes images with a prompt: always, when the
/// running client says it does (OpenCode, in its answer to `initialize`;
/// a prompt it has not said yes to is refused when sent), or never.
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde_with::SerializeDisplay, serde_with::DeserializeFromStr,
)]
pub enum ImageTaking {
    Always,
    IfOffered,
    Never,
}

impl ImageTaking {
    /// What `harness` declares of images.
    pub fn of(harness: Harness) -> Self {
        match harness.acceptance().images() {
            Takes::Always(_) => Self::Always,
            Takes::IfOffered(_) => Self::IfOffered,
            Takes::Never => Self::Never,
        }
    }
}

/// A new session: its client, where it works, and its settings. Its client
/// starts at once, so its commands are there before the first prompt.
pub struct Create {
    pub harness: Harness,
    pub place: StartPlace,
    pub settings: RunSettings,
    /// A client conversation to go on with (one begun elsewhere), instead
    /// of a new one.
    pub resume: Option<NativeThreadId>,
}

/// A session the store holds that this supervisor has not served since it
/// started, to serve it again.
pub struct SessionSeed {
    pub session: SessionId,
    pub harness: Harness,
    pub workdir: String,
    pub settings: RunSettings,
    /// The client's conversation, when one was opened: its next process
    /// resumes it.
    pub thread: Option<NativeThreadId>,
    pub state: SessionState,
    /// The attachments uploaded to it, as stored.
    pub attachments: Vec<KeptAttachment>,
}

/// Why a session request was not carried out.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("no session {0}")]
    NoSuchSession(SessionId),
    #[error(transparent)]
    Refused(SessionRefused),
    #[error("no process serves the session")]
    NoProcess,
    #[error("a process still serves the session")]
    Running,
    #[error("cannot read the session's folder: {0}")]
    Unreadable(io::Error),
    #[error("the recorder has no room to record the session")]
    Unrecorded,
    #[error(transparent)]
    Start(StartError),
    #[error(transparent)]
    Command(CommandError),
    #[error(transparent)]
    Attachment(AttachmentError),
    #[error("sessions take no attachments here")]
    NoAttachments,
    #[error("no attachment {0} in the session")]
    NoSuchAttachment(AttachmentId),
    #[error("cannot keep or read the session's attachment: {0}")]
    AttachmentStorage(io::Error),
    #[error("attachment {0} went out with a prompt and stays")]
    AttachmentSent(AttachmentId),
    #[error("a prompt needs text or an image")]
    BlankPrompt,
    #[error(transparent)]
    PromptAttachments(PromptLimitError),
    #[error(transparent)]
    ImagesNotTaken(NotAccepted),
    #[error("the supervisor has stopped")]
    Gone,
}

/// A change to a run, as it happens.
#[derive(Debug, Clone, PartialEq)]
pub enum RunUpdate {
    State { run: RunId, state: RunState },
    Event { run: RunId, event: DriveEvent },
}

/// When each client's models are first listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warmup {
    /// As the task starts, for every client installed here, so the first
    /// form that offers them has them at once.
    AtStart,
    /// The first time they are asked for.
    WhenAsked,
}

/// Where a new run works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartPlace {
    /// A new folder of its own in the scratch folder.
    Scratch,
    /// This folder, which must lie inside a root or another place.
    Folder(String),
    /// A workspace of its own: a new worktree of the repository in this
    /// folder, on a branch of its own, with ports and a data folder no other
    /// workspace shares.
    Worktree(String),
}

/// Where runs can start: every place, and the folder scratch runs get
/// theirs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub projects: Vec<Project>,
    /// None when runs cannot start in scratch here.
    pub scratch: Option<String>,
}

/// One run as the supervisor holds it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run: RunId,
    /// The session the run serves.
    pub session: SessionId,
    pub harness: Harness,
    pub workdir: PathBuf,
    pub state: RunState,
    /// What it was started with; each left out is the client's own.
    pub settings: RunSettings,
    /// Whether it outlives the daemon.
    pub tenure: RunTenure,
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
    #[error("runs cannot start in scratch here: its folder could not be set up")]
    NoScratch,
    #[error("cannot create the scratch run's folder")]
    Scratch(#[source] io::Error),
    #[error("workspaces cannot be made here: their folder could not be set up")]
    NoWorktrees,
    #[error("every workspace slot is taken: remove a session's workspace first")]
    NoSlot,
    #[error("cannot make the workspace")]
    Worktree(#[source] io::Error),
    #[error("{0} does not take that approval policy")]
    NoApprovals(Harness),
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

/// Where a session request's answer goes: its own type, or why not.
pub(crate) type Reply<T> = oneshot::Sender<Result<T, SessionError>>;

/// What is asked of the sessions, each with the reply its answer goes to.
pub(crate) enum SessionRequest {
    Create {
        create: Create,
        reply: Reply<SessionId>,
    },
    Adopt {
        seed: SessionSeed,
        reply: Reply<()>,
    },
    Send {
        session: SessionId,
        command: RunCommand,
        reply: Reply<()>,
    },
    ChooseClient {
        session: SessionId,
        harness: Harness,
        reply: Reply<()>,
    },
    ChooseFolder {
        session: SessionId,
        place: StartPlace,
        reply: Reply<()>,
    },
    Shelve {
        session: SessionId,
        shelving: Shelving,
        reply: Reply<()>,
    },
    /// Stop serving a session that is being deleted.
    Forget {
        session: SessionId,
        reply: Reply<()>,
    },
    List {
        reply: Reply<Vec<SessionSnapshot>>,
    },
    Files {
        session: SessionId,
        query: FileQuery,
        limit: usize,
        reply: Reply<Vec<FileMatch>>,
    },
    Attach {
        session: SessionId,
        name: String,
        bytes: Vec<u8>,
        reply: Reply<(AttachmentId, Attachment)>,
    },
    ReadAttachment {
        session: SessionId,
        attachment: AttachmentId,
        reply: Reply<(Attachment, Vec<u8>)>,
    },
    Detach {
        session: SessionId,
        attachment: AttachmentId,
        reply: Reply<()>,
    },
    Attachments {
        session: SessionId,
        reply: Reply<Vec<KeptAttachment>>,
    },
    Prompt {
        session: SessionId,
        text: String,
        attachments: Vec<AttachmentId>,
        delivery: Delivery,
        reply: Reply<()>,
    },
}

/// The supervisor has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the supervisor has stopped")]
pub struct Gone;

pub(crate) enum Request {
    List {
        reply: oneshot::Sender<Vec<RunSnapshot>>,
    },
    Projects {
        reply: oneshot::Sender<Places>,
    },
    Clients {
        reply: oneshot::Sender<Vec<Client>>,
    },
    Models {
        harness: Harness,
        reply: listing::ListingReply<Vec<Model>>,
    },
    Commands {
        asked: CommandsAsked,
        reply: listing::ListingReply<Vec<ClientCommand>, CommandsError>,
    },
    Session(SessionRequest),
    SignIn(SignInRequest),
}

/// Whose commands are asked for.
pub(crate) enum CommandsAsked {
    Session(SessionId),
    Client { harness: Harness, place: StartPlace },
}

/// Why a client's commands could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CommandsError {
    #[error("no session {0}")]
    NoSuchSession(SessionId),
    #[error(transparent)]
    Place(StartError),
    #[error("{0} is not installed here")]
    NotInstalled(Harness),
    #[error(transparent)]
    Listing(#[from] ListingError),
    #[error("the supervisor has stopped")]
    Gone,
}

/// A request about a client's sign-in, answered by the task's loop.
pub(crate) enum SignInRequest {
    Check {
        harness: Harness,
        reply: login::CheckReply,
    },
    Start {
        harness: Harness,
        method: SignInMethod,
        reply: oneshot::Sender<Result<tokio::sync::watch::Receiver<Login>, SignInError>>,
    },
    Answer {
        harness: Harness,
        answer: SignInAnswer,
        reply: login::OutcomeReply,
    },
    Cancel {
        harness: Harness,
        reply: oneshot::Sender<Result<(), SignInError>>,
    },
    Wait {
        harness: Harness,
        reply: oneshot::Sender<Result<login::WaitOn, SignInError>>,
    },
}

impl Supervisor {
    /// Requests that wait for the task beyond this are refused at once.
    const QUEUE: usize = 64;
    /// Updates a slow subscriber may fall behind by before it lags.
    const UPDATES: usize = 1024;
    /// How long starting a sign-in waits for the client's link: under the
    /// UI proxy's 30 seconds.
    pub const LINK_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

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
        // Other places that cannot be resolved are left out, not fatal.
        let places = options
            .places
            .iter()
            .filter_map(|place| {
                host.resolve(place)
                    .inspect_err(
                        |error| tracing::warn!(?place, %error, "a run place is unreadable"),
                    )
                    .ok()
            })
            .collect();
        let scratch = options.scratch.as_deref().and_then(|scratch| {
            host.make_dir(scratch)
                .and_then(|()| host.resolve(scratch))
                .inspect_err(|error| tracing::warn!(?scratch, %error, "no scratch folder for runs"))
                .ok()
        });
        let worktrees = options.worktrees.as_deref().and_then(|worktrees| {
            host.make_dir(worktrees)
                .and_then(|()| host.resolve(worktrees))
                .inspect_err(|error| tracing::warn!(?worktrees, %error, "no folder for workspaces"))
                .ok()
        });
        let bounds = Bounds::new(roots, places, scratch, options.home)
            .map_err(SupervisorError::Roots)?
            .with_worktrees(worktrees);
        // An attachments folder that cannot be made leaves sessions without
        // attachments, not the supervisor without sessions.
        let attachments = options.attachments.and_then(|root| {
            host.make_dir(&root)
                .inspect_err(|error| tracing::warn!(?root, %error, "no attachments folder"))
                .ok()
                .map(|()| AttachmentFolders::new(root))
        });
        let attachment_limits = attachments.as_ref().map(|_| options.attachment_limits);
        let (requests, receiver) = mpsc::channel(Self::QUEUE);
        let (updates, _) = broadcast::channel(Self::UPDATES);
        let (sessions, _) = broadcast::channel(Self::UPDATES);
        let task = SupervisorTask {
            requests: receiver,
            updates: updates.clone(),
            sessions: sessions.clone(),
            host: Arc::new(host),
            ids,
            recorder,
            bounds,
            gateway: options.gateway,
            warmup: options.warmup,
            version: options.version,
            line_limit: options.line_limit,
            idle: options.idle,
            attachments,
            attachment_limits: options.attachment_limits,
        };
        Ok((
            Self {
                requests,
                updates,
                sessions,
                attachment_limits,
            },
            task,
        ))
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
    pub async fn projects(&self) -> Result<Places, Gone> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Projects { reply })
            .await
            .map_err(|_| Gone)?;
        answer.await.map_err(|_| Gone)
    }

    /// Every client, whether a run can start with it here, and its sign-in.
    /// Waits for sign-in checks running now, each bounded.
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
    pub async fn models(&self, harness: Harness) -> Result<Vec<Model>, ListingError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Models { harness, reply })
            .await
            .map_err(|_| ListingError::Gone)?;
        answer.await.map_err(|_| ListingError::Gone)?
    }

    /// What `harness` offers a prompt in `place`, with no session: as the
    /// installed client last listed it there, or listed now by starting it
    /// briefly in that folder. Scratch is the scratch folder itself.
    pub async fn client_commands(
        &self,
        harness: Harness,
        place: StartPlace,
    ) -> Result<Vec<ClientCommand>, CommandsError> {
        self.commands(CommandsAsked::Client { harness, place })
            .await
    }

    async fn commands(&self, asked: CommandsAsked) -> Result<Vec<ClientCommand>, CommandsError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Commands { asked, reply })
            .await
            .map_err(|_| CommandsError::Gone)?;
        answer.await.map_err(|_| CommandsError::Gone)?
    }

    async fn sign_in_request<T>(
        &self,
        request: impl FnOnce(oneshot::Sender<Result<T, SignInError>>) -> SignInRequest,
    ) -> Result<T, SignInError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::SignIn(request(reply)))
            .await
            .map_err(|_| SignInError::Gone)?;
        answer.await.map_err(|_| SignInError::Gone)?
    }

    /// Check `harness`'s sign-in now with its own status command.
    pub async fn check_sign_in(&self, harness: Harness) -> Result<SignInState, SignInError> {
        self.sign_in_request(|reply| SignInRequest::Check { harness, reply })
            .await
    }

    /// Start `harness`'s own login with `method`, replacing any running
    /// one. Answers once it shows what the operator needs (its link, a
    /// device login's code, or its key prompt),
    /// or with how it stands after [`Self::LINK_WAIT`].
    pub async fn sign_in(
        &self,
        harness: Harness,
        method: SignInMethod,
    ) -> Result<Login, SignInError> {
        let mut login = self
            .sign_in_request(|reply| SignInRequest::Start {
                harness,
                method,
                reply,
            })
            .await?;
        let shown = tokio::time::timeout(Self::LINK_WAIT, login.wait_for(Login::shown))
            .await
            .map(|shown| shown.map(|state| state.clone()));
        Ok(match shown {
            Ok(Ok(state)) => state,
            // The login ended and its state went with it: it was replaced.
            Ok(Err(_)) => Login::Ended(LoginEnd::Cancelled),
            Err(_) => login.borrow().clone(),
        })
    }

    /// Hand `answer` to `harness`'s login: the code the page showed, an API
    /// key, or the address the browser could not open (opened on this host
    /// when it is the one the login waits on). Answers once the login has
    /// ended and the client's sign-in was checked.
    pub async fn answer_sign_in(
        &self,
        harness: Harness,
        answer: SignInAnswer,
    ) -> Result<LoginOutcome, SignInError> {
        self.sign_in_request(|reply| SignInRequest::Answer {
            harness,
            answer,
            reply,
        })
        .await
    }

    /// Wait for `harness`'s login to end and its sign-in to be checked, at
    /// most `within`: a device login ends by itself once approved. Answers
    /// with how it stands when that time is up.
    pub async fn wait_sign_in(
        &self,
        harness: Harness,
        within: std::time::Duration,
    ) -> Result<Waited, SignInError> {
        match self
            .sign_in_request(|reply| SignInRequest::Wait { harness, reply })
            .await?
        {
            login::WaitOn::Done(outcome) => Ok(Waited::Done { outcome }),
            login::WaitOn::Going { login, outcome } => {
                match tokio::time::timeout(within, outcome).await {
                    Ok(Ok(outcome)) => outcome.map(|outcome| Waited::Done { outcome }),
                    Ok(Err(_)) => Err(SignInError::Gone),
                    Err(_) => Ok(Waited::Going { login }),
                }
            }
        }
    }

    /// Cancel `harness`'s running login.
    pub async fn cancel_sign_in(&self, harness: Harness) -> Result<(), SignInError> {
        self.sign_in_request(|reply| SignInRequest::Cancel { harness, reply })
            .await
    }

    /// Ask the task, and wait for its answer.
    async fn ask<T>(
        &self,
        request: impl FnOnce(Reply<T>) -> SessionRequest,
    ) -> Result<T, SessionError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Session(request(reply)))
            .await
            .map_err(|_| SessionError::Gone)?;
        answer.await.map_err(|_| SessionError::Gone)?
    }

    /// A new session, a draft: its client starts with its first prompt.
    pub async fn create(&self, create: Create) -> Result<SessionId, SessionError> {
        self.ask(|reply| SessionRequest::Create { create, reply })
            .await
    }

    /// Serve a session the store holds; nothing when it is served already.
    pub async fn adopt(&self, seed: SessionSeed) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::Adopt { seed, reply })
            .await
    }

    /// Carry out `command` in `session`: through its process, or, for a
    /// prompt with none running, through a new one that resumes its
    /// conversation; settings with none running apply to the next.
    pub async fn send(&self, session: SessionId, command: RunCommand) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::Send {
            session,
            command,
            reply,
        })
        .await
    }

    /// Work with `harness` instead, while the session is a draft: any
    /// process of the old client goes, and its settings go back to the
    /// client's own.
    pub async fn choose_client(
        &self,
        session: SessionId,
        harness: Harness,
    ) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::ChooseClient {
            session,
            harness,
            reply,
        })
        .await
    }

    /// Work in `place` instead, while the session is a draft: any process
    /// goes, with its conversation; the next starts there.
    pub async fn choose_folder(
        &self,
        session: SessionId,
        place: StartPlace,
    ) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::ChooseFolder {
            session,
            place,
            reply,
        })
        .await
    }

    /// Archive a session (which stops its process) or unarchive it.
    pub async fn shelve(&self, session: SessionId, shelving: Shelving) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::Shelve {
            session,
            shelving,
            reply,
        })
        .await
    }

    /// Stop serving `session`, which is being deleted, with its ended runs:
    /// refused while a process serves it. Whether it was served.
    pub async fn forget(&self, session: SessionId) -> Result<bool, SessionError> {
        match self
            .ask(|reply| SessionRequest::Forget { session, reply })
            .await
        {
            Ok(()) => Ok(true),
            Err(SessionError::NoSuchSession(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Every session served since the supervisor started.
    pub async fn sessions(&self) -> Result<Vec<SessionSnapshot>, SessionError> {
        self.ask(|reply| SessionRequest::List { reply }).await
    }

    /// What the session's client offers a prompt: as its running client
    /// lists them, else as the installed client last listed them in its
    /// folder, or listed now by starting it briefly there.
    pub async fn session_commands(
        &self,
        session: SessionId,
    ) -> Result<Vec<ClientCommand>, CommandsError> {
        self.commands(CommandsAsked::Session(session)).await
    }

    /// The `limit` paths of the session's folder that best match `query`,
    /// with or without a process.
    pub async fn session_files(
        &self,
        session: SessionId,
        query: FileQuery,
        limit: usize,
    ) -> Result<Vec<FileMatch>, SessionError> {
        self.ask(|reply| SessionRequest::Files {
            session,
            query,
            limit,
            reply,
        })
        .await
    }

    /// How much an attachment may hold; `None` when sessions take no
    /// attachments here.
    pub fn attachment_limits(&self) -> Option<Limits> {
        self.attachment_limits
    }

    /// Keep `bytes`, uploaded as `name`, with `session`, to go with a
    /// prompt: read as an attachment within the limits, written to the
    /// session's folder, and recorded.
    pub async fn attach(
        &self,
        session: SessionId,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<(AttachmentId, Attachment), SessionError> {
        self.ask(|reply| SessionRequest::Attach {
            session,
            name,
            bytes,
            reply,
        })
        .await
    }

    /// One of `session`'s attachments, and its bytes.
    pub async fn attachment(
        &self,
        session: SessionId,
        attachment: AttachmentId,
    ) -> Result<(Attachment, Vec<u8>), SessionError> {
        self.ask(|reply| SessionRequest::ReadAttachment {
            session,
            attachment,
            reply,
        })
        .await
    }

    /// Every attachment of `session`, and whether each was sent.
    pub async fn attachments(
        &self,
        session: SessionId,
    ) -> Result<Vec<KeptAttachment>, SessionError> {
        self.ask(|reply| SessionRequest::Attachments { session, reply })
            .await
    }

    /// Prompt `session` with `text` and its `attachments`, delivered this
    /// way: each must be the session's, within the limits, and of a kind
    /// its client takes. Sent, they stay with the session.
    pub async fn prompt(
        &self,
        session: SessionId,
        text: String,
        attachments: Vec<AttachmentId>,
        delivery: Delivery,
    ) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::Prompt {
            session,
            text,
            attachments,
            delivery,
            reply,
        })
        .await
    }

    /// Remove one of `session`'s attachments, its file and its record.
    pub async fn detach(
        &self,
        session: SessionId,
        attachment: AttachmentId,
    ) -> Result<(), SessionError> {
        self.ask(|reply| SessionRequest::Detach {
            session,
            attachment,
            reply,
        })
        .await
    }

    /// Session changes from now on.
    pub fn subscribe_sessions(&self) -> broadcast::Receiver<SessionSnapshot> {
        self.sessions.subscribe()
    }

    /// Updates from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<RunUpdate> {
        self.updates.subscribe()
    }
}
