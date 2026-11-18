//! The shell around the pure session and run machines: one task owns every
//! session and every run (a session's client process), reads each agent's
//! stdout, writes its stdin, and waits for its exit. Components do not
//! spawn, so the daemon runs this task; it ends once every [`Supervisor`]
//! handle is gone and every run has ended.
//!
//! Each agent's stdin is a bounded queue, written in order by a writer the
//! task polls beside the run's output, so an agent that stops reading its
//! stdin holds up only its own lines. One whose queue fills has its stdin
//! closed: it is not reading what it is sent.
//!
//! What the task does for a session's requests is in `sessions`, and for
//! the clients themselves (sign-ins, listings) in `clients`, with their
//! shells `login` and `listing`.
//!
//! [`Supervisor`]: crate::Supervisor

mod clients;
pub(crate) mod listing;
pub(crate) mod login;
mod sessions;

use crate::attachments::AttachmentFolders;
use crate::process::{Process, ProcessEffect, Rest, Serving};
use crate::{
    CommandsError, IdleStop, Places, Reply, Request, RunSnapshot, RunUpdate, SessionError,
    SessionSnapshot, StartError, Warmup,
    client::Client,
    files::{FileMatch, FileQuery, FoundPath},
    host::{AgentHost, AgentProcess, KillSwitch},
    project::Search,
    run::{ProcessExit, RunCommand, RunState},
    session::{Launch, Room, Session, TurnsEnded},
    workdir::{Bounds, RequestedDir, Workdir},
};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, BoxStream, FuturesUnordered, SelectAll},
};
use listing::{CommandKey, Listings};
use login::SignIns;
use ren_harness::{
    Harness,
    attachment::{Attachment, Limits},
    commands::ClientCommand,
    drive::{Driver, Opening},
    endpoint::GatewayEndpoint,
    lines::{Framed, Line, LineLimit, Lines},
    models::{DefaultChoice, Model, ModelId},
    prompt::Prompt,
    settings::{ApprovalPolicy, RunSettings},
};
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    Recorder, RunSession, RunTap, SessionSlot,
    batch::SessionFact,
    id::{IdSource, RunId, SessionId},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{broadcast, mpsc, oneshot},
};

/// Runs every run's I/O. Returned by [`Supervisor::new`](crate::Supervisor::new).
#[must_use = "the supervisor does nothing until its task runs"]
pub struct SupervisorTask<H> {
    pub(crate) requests: mpsc::Receiver<Request>,
    pub(crate) updates: broadcast::Sender<RunUpdate>,
    pub(crate) sessions: broadcast::Sender<SessionSnapshot>,
    pub(crate) host: Arc<H>,
    pub(crate) ids: Arc<dyn IdSource>,
    pub(crate) recorder: Recorder,
    pub(crate) bounds: Bounds,
    pub(crate) gateway: GatewayEndpoint,
    pub(crate) warmup: Warmup,
    pub(crate) version: String,
    pub(crate) line_limit: LineLimit,
    pub(crate) idle: IdleStop,
    /// Where sessions' attachments are kept; `None`: they take none.
    pub(crate) attachments: Option<AttachmentFolders>,
    pub(crate) attachment_limits: Limits,
}

/// One process the task drives: its pure lifecycle, and the handles that
/// carry out what it says.
struct Live {
    process: Process,
    /// Where what its client lists is kept: the installed client and the
    /// folder it runs in; `None` when the client was not found.
    book: Option<CommandKey>,
    /// The commands last kept from it, so each list is kept once.
    booked: Vec<ClientCommand>,
    /// Lines queued for its stdin; `None` once its stdin is closed.
    stdin: Option<mpsc::Sender<Line>>,
    kill: Option<KillSwitch>,
    /// Records the run's opening and end; taken when the end is recorded.
    tap: Option<RunTap>,
}

/// What a process's effects mean for the session it serves.
enum Turned {
    Started {
        conversation: Option<NativeThreadId>,
    },
    Ended {
        failed: bool,
    },
}

/// An attachment being read off the task: its read, and its bytes.
type Reading = BoxFuture<'static, Result<(Attachment, Vec<u8>), SessionError>>;

type Outputs = SelectAll<BoxStream<'static, (RunId, Vec<Framed>)>>;
type Exits = FuturesUnordered<BoxFuture<'static, (RunId, ProcessExit)>>;

/// A prompt checked against its session: whole, or with its images still
/// to be read.
enum Composing {
    Ready(Prompt),
    Images {
        text: String,
        files: Vec<(Attachment, std::path::PathBuf)>,
    },
}

/// A session's folder walked off the task, for the search that asked.
struct Walked {
    session: SessionId,
    /// The session's `turns_ended` when the walk began.
    turns_ended: TurnsEnded,
    found: std::io::Result<Vec<FoundPath>>,
    query: FileQuery,
    limit: usize,
    reply: Reply<Vec<FileMatch>>,
}

/// Everything the task serves: sessions, their runs, and the runs' output
/// and exits it waits on.
#[derive(Default)]
struct Served {
    sessions: BTreeMap<SessionId, Session>,
    runs: BTreeMap<RunId, Live>,
    outputs: Outputs,
    /// Answers being worked out off the task, each sending its own reply.
    replies: FuturesUnordered<BoxFuture<'static, ()>>,
    /// Folder walks for file searches, cached as they finish.
    walks: FuturesUnordered<BoxFuture<'static, Walked>>,
    /// Each run's stdin writer: how it ended.
    writers: FuturesUnordered<BoxFuture<'static, (RunId, std::io::Result<()>)>>,
    exits: Exits,
    /// Idle stops: the run, and the rest each was armed for.
    idles: FuturesUnordered<BoxFuture<'static, (RunId, Rest)>>,
    /// The default model each client listed, for a client that is started
    /// on it by name when none is chosen.
    defaults: HashMap<Harness, ModelId>,
    /// Each client's models, listed or being listed.
    listings: Listings<Harness, Vec<Model>>,
    /// What each installed client offers in each folder: kept from its
    /// runs, or listed by starting it briefly there.
    commands: Listings<CommandKey, Vec<ClientCommand>, CommandsError>,
    /// Each client's sign-in, and the checks and logins running.
    sign_ins: SignIns,
    /// Who asked for the clients while sign-in checks were running.
    clients_waiting: Vec<oneshot::Sender<Vec<Client>>>,
}

impl Served {
    /// The run serving `session` now, if one runs.
    fn process(&self, session: SessionId) -> Option<RunId> {
        self.serving(self.sessions.get(&session)?)
    }

    /// The run serving `session` now, if one runs.
    fn serving(&self, session: &Session) -> Option<RunId> {
        let run = session.process()?;
        match self.runs.get(&run)?.process.state() {
            RunState::Ended { .. } => None,
            RunState::Starting
            | RunState::Idle { .. }
            | RunState::Working { .. }
            | RunState::Stopping { .. } => Some(run),
        }
    }
}

impl<H: AgentHost> SupervisorTask<H> {
    pub async fn run(mut self) {
        let mut served = Served::default();
        match self.warmup {
            Warmup::AtStart => {
                self.warm(&mut served.listings);
                self.check_unchecked(&mut served.sign_ins);
            }
            Warmup::WhenAsked => {}
        }
        let mut accepting = true;
        loop {
            tokio::select! {
                request = self.requests.recv(), if accepting => match request {
                    Some(request) => self.request(request, &mut served).await,
                    None => {
                        accepting = false;
                        for live in served.runs.values_mut() {
                            let effects = live.process.shut_down();
                            self.carry_out(live, effects, &mut served.idles);
                        }
                    }
                },
                Some((run, frames)) = served.outputs.next(), if !served.outputs.is_empty() => {
                    if let Some(live) = served.runs.get_mut(&run) {
                        let mut turned = Vec::new();
                        for frame in frames {
                            let effects = live.process.receive(frame);
                            turned.extend(self.carry_out(live, effects, &mut served.idles));
                        }
                        Self::book(live, &mut served.commands);
                        let (session, harness) = (live.process.session(), live.process.harness());
                        let failed = Self::turned(served.sessions.get_mut(&session), turned);
                        // A failed turn may be a sign-in that ran out:
                        // read the client's sign-in again.
                        if failed {
                            served.sign_ins.check(harness, None, || self.status_check(harness));
                        }
                    }
                }
                Some((harness, outcome)) = served.listings.running.next(), if !served.listings.running.is_empty() => {
                    if let Ok(models) = &outcome
                        && let Some(listed) = models
                            .iter()
                            .find(|model| model.choice == DefaultChoice::Default)
                    {
                        served.defaults.insert(harness, listed.id.clone());
                    }
                    // A client that lists no models (not signed in yet) is
                    // asked again next time.
                    served.listings.finish(harness, outcome, |models| !models.is_empty());
                }
                Some((key, outcome)) = served.commands.running.next(), if !served.commands.running.is_empty() => {
                    // A folder whose client lists no commands is kept too.
                    served.commands.finish(key, outcome, |_| true);
                }
                Some((harness, state)) = served.sign_ins.checks.next(), if !served.sign_ins.checks.is_empty() => {
                    served.sign_ins.checked(harness, state);
                    if !served.sign_ins.checking() {
                        let clients = self.clients(&served.sign_ins);
                        for reply in served.clients_waiting.drain(..) {
                            let _ = reply.send(clients.clone());
                        }
                    }
                }
                Some((harness, attempt, end)) = served.sign_ins.talks.next(), if !served.sign_ins.talks.is_empty() => {
                    tracing::debug!(%harness, "a client's sign-in ended");
                    if served.sign_ins.talked(harness, attempt, end) {
                        let mut unstarted = None;
                        served.sign_ins.check(harness, None, || {
                            self.status_check(harness).inspect_err(|error| unstarted = Some(error.clone()))
                        });
                        if let Some(error) = unstarted {
                            served.sign_ins.unchecked(harness, &error);
                        }
                    }
                }
                Some(()) = served.replies.next(), if !served.replies.is_empty() => {}
                Some(walked) = served.walks.next(), if !served.walks.is_empty() => {
                    Self::walked(walked, &mut served);
                }
                Some((run, written)) = served.writers.next(), if !served.writers.is_empty() => {
                    if let Err(error) = written {
                        // The process is ending; its exit will say how.
                        tracing::debug!(%run, %error, "agent stdin closed");
                        if let Some(live) = served.runs.get_mut(&run) {
                            live.stdin = None;
                        }
                    }
                }
                Some((run, rest)) = served.idles.next(), if !served.idles.is_empty() => {
                    if let Some(live) = served.runs.get_mut(&run) {
                        let effects = live.process.rest_over(rest);
                        self.carry_out(live, effects, &mut served.idles);
                    }
                }
                Some((run, exit)) = served.exits.next(), if !served.exits.is_empty() => {
                    if let Some(live) = served.runs.get_mut(&run) {
                        live.stdin = None;
                        live.kill = None;
                        let effects = live.process.exited(exit);
                        self.carry_out(live, effects, &mut served.idles);
                    }
                }
                else => break,
            }
            if !accepting && served.exits.is_empty() {
                break;
            }
        }
    }

    async fn request(&self, request: Request, served: &mut Served) {
        match request {
            Request::Session(request) => self.session(request, served).await,
            Request::Projects { reply } => {
                // Bounded by `Search`; the host reads only folder listings
                // and `HEAD` files, off the task.
                let bounds = self.bounds.clone();
                let scratch = self
                    .bounds
                    .scratch()
                    .and_then(|scratch| scratch.to_str())
                    .map(str::to_owned);
                let looked = self.off_task(move |host| {
                    Ok(bounds.places(Search::DEFAULT, |folder| {
                        host.look(folder)
                            .inspect_err(|error| tracing::debug!(%error, "cannot look in a folder"))
                            .ok()
                    }))
                });
                served.replies.push(
                    async move {
                        let projects = looked.await.unwrap_or_default();
                        let _ = reply.send(Places { projects, scratch });
                    }
                    .boxed(),
                );
            }
            Request::Clients { reply } => {
                self.check_unchecked(&mut served.sign_ins);
                if served.sign_ins.checking() {
                    served.clients_waiting.push(reply);
                } else {
                    let _ = reply.send(self.clients(&served.sign_ins));
                }
            }
            Request::SignIn(request) => self.sign_in(request, &mut served.sign_ins),
            Request::Models { harness, reply } => {
                self.models(harness, reply, &mut served.listings);
            }
            Request::Commands { asked, reply } => self.commands(asked, reply, served),
            Request::List { reply } => {
                let _ = reply.send(
                    served
                        .runs
                        .iter()
                        .map(|(run, live)| RunSnapshot {
                            run: *run,
                            session: live.process.session(),
                            harness: live.process.harness(),
                            workdir: live.process.workdir().as_path().to_path_buf(),
                            state: live.process.state().clone(),
                            settings: live.process.settings().clone(),
                        })
                        .collect(),
                );
            }
        }
    }

    /// Do `work` with the host off the task: see [`AgentHost::blocking`].
    fn off_task<T, W>(&self, work: W) -> BoxFuture<'static, std::io::Result<T>>
    where
        T: Send + 'static,
        W: FnOnce(&H) -> std::io::Result<T> + Send + 'static,
    {
        let host = Arc::clone(&self.host);
        self.host.blocking(move || work(&host))
    }

    fn confirm(&self, requested: &RequestedDir) -> Result<Workdir, StartError> {
        let resolved = self
            .host
            .resolve(requested.as_path())
            .map_err(StartError::Resolve)?;
        Workdir::confirm(&self.bounds, resolved).map_err(StartError::Workdir)
    }

    /// Whether `harness` runs on `settings`: an approval policy it offers.
    fn drivable(harness: Harness, settings: &RunSettings) -> Result<(), StartError> {
        match settings.approvals {
            Some(policy) if !ApprovalPolicy::offered(harness).contains(&policy) => {
                Err(StartError::NoApprovals(harness))
            }
            Some(_) | None => Ok(()),
        }
    }

    /// A new process for `session`, recorded as `recorded`: it resumes the
    /// session's conversation when one was opened, and is sent `first` once
    /// open. A process that cannot be spawned is kept, ended, as its run.
    async fn spawn(
        &self,
        session: SessionId,
        serving: &mut Session,
        recorded: RunSession,
        first: Option<Prompt>,
        served: &mut Served,
    ) -> Result<RunId, StartError> {
        let Launch {
            harness,
            workdir,
            settings,
            resume,
        } = serving.launch(served.defaults.get(&serving.harness()));
        let opening = match resume {
            None => Opening::New {
                cwd: workdir.as_str().to_owned(),
                id: NativeThreadId::new(self.ids.next_uuid()).ok_or(StartError::NilId)?,
                settings: settings.clone(),
            },
            Some(thread) => Opening::Resume {
                thread,
                cwd: workdir.as_str().to_owned(),
                settings: settings.clone(),
            },
        };
        let plan = harness.drive(&self.gateway, &opening);
        let (codec, opening_lines) =
            Driver::open(harness, opening, &self.version, self.ids.next_uuid());
        let run = RunId::generate(self.ids.as_ref());
        let tools = self.host.tools();
        // Every run is recorded: without room, it does not start.
        let tap = self
            .recorder
            .run(
                run,
                recorded,
                harness,
                workdir.as_str().to_owned(),
                settings.clone(),
                tools.generation(),
            )
            .map_err(StartError::Unrecorded)?;
        serving.started(run);
        let serving = Serving {
            run,
            session,
            harness,
            workdir: workdir.clone(),
            settings,
        };
        let process = match self.host.spawn(&plan, &workdir, &tools) {
            Ok(process) => process,
            Err(source) => {
                let (process, effects) = Process::unstarted(serving);
                let mut live = Live {
                    process,
                    book: None,
                    booked: Vec::new(),
                    stdin: None,
                    kill: None,
                    tap: Some(tap),
                };
                self.carry_out(&mut live, effects, &mut served.idles);
                served.runs.insert(run, live);
                return Err(StartError::Spawn { run, source });
            }
        };
        let AgentProcess {
            stdin,
            stdout,
            exit,
            kill,
        } = process;
        let (queue, lines) = mpsc::channel(Self::STDIN_LINES);
        served.writers.push(
            Self::writer(stdin, lines)
                .map(move |written| (run, written))
                .boxed(),
        );
        let (process, effects) = Process::opening(serving, codec, opening_lines, first);
        let book = self
            .host
            .client_stamp(harness.profile().program, &tools)
            .map(|stamp| CommandKey {
                harness,
                stamp,
                folder: workdir.as_path().to_path_buf(),
            });
        let mut live = Live {
            process,
            book,
            booked: Vec::new(),
            stdin: Some(queue),
            kill: Some(kill),
            tap: Some(tap),
        };
        self.carry_out(&mut live, effects, &mut served.idles);
        served
            .outputs
            .push(Self::output(run, stdout, self.line_limit));
        served.exits.push(exit.map(move |exit| (run, exit)).boxed());
        served.runs.insert(run, live);
        Ok(run)
    }

    /// Whether the recorder has room for a session's fact, with the slot
    /// that records it.
    fn room(&self) -> (Option<SessionSlot>, Room) {
        let slot = self.recorder.session_slot();
        let room = match slot {
            Some(_) => Room::Recordable,
            None => Room::Full,
        };
        (slot, room)
    }

    /// Record `fact` about `session`; without room it is counted lost by
    /// the caller's error.
    fn record(&self, session: SessionId, fact: SessionFact) -> Result<(), SessionError> {
        self.recorder
            .session_slot()
            .ok_or(SessionError::Unrecorded)?
            .record(session, fact);
        Ok(())
    }

    fn publish_session(&self, session: SessionId, serving: &Session) {
        // No subscriber is not an error.
        let _ = self.sessions.send(serving.snapshot(session));
    }

    /// Stop `run`, the process serving `session`, if one runs.
    fn stop_process(&self, session: SessionId, run: Option<RunId>, served: &mut Served) {
        let Some(live) = run.and_then(|run| served.runs.get_mut(&run)) else {
            return;
        };
        match live.process.command(RunCommand::Stop) {
            Ok(effects) => {
                self.carry_out(live, effects, &mut served.idles);
            }
            Err(error) => {
                tracing::debug!(%session, %error, "the session's process would not stop");
            }
        }
    }

    /// Carry out what `live`'s process asked for, with its handles. What
    /// its turns mean for its session is returned for the caller to apply.
    fn carry_out(
        &self,
        live: &mut Live,
        effects: Vec<ProcessEffect>,
        idles: &mut FuturesUnordered<BoxFuture<'static, (RunId, Rest)>>,
    ) -> Vec<Turned> {
        let run = live.process.run();
        let mut turned = Vec::new();
        for effect in effects {
            match effect {
                ProcessEffect::Write(line) => Self::write(run, live, &line),
                ProcessEffect::Kill => {
                    if let Some(switch) = live.kill.take() {
                        switch.fire();
                    }
                }
                ProcessEffect::Rested(rest) => match self.idle {
                    IdleStop::After(after) => {
                        idles.push(tokio::time::sleep(after).map(move |()| (run, rest)).boxed())
                    }
                    IdleStop::Never => {}
                },
                ProcessEffect::Publish(update) => self.publish(update),
                ProcessEffect::Opened(thread) => {
                    if let Some(tap) = live.tap.as_ref() {
                        tap.opened(thread);
                    }
                }
                ProcessEffect::Configured(settings) => {
                    if let Some(tap) = live.tap.as_ref() {
                        tap.configured(settings);
                    }
                }
                ProcessEffect::Entry(turn, entry) => {
                    if let Some(tap) = live.tap.as_ref() {
                        tap.entry(turn, entry);
                    }
                }
                ProcessEffect::Ended { end, exit } => {
                    if let Some(tap) = live.tap.take() {
                        tap.end(end, exit);
                    }
                }
                ProcessEffect::TurnStarted { conversation } => {
                    turned.push(Turned::Started { conversation });
                }
                ProcessEffect::TurnEnded { failed } => turned.push(Turned::Ended { failed }),
                ProcessEffect::FirstRefused(error) => {
                    tracing::debug!(%run, %error, "the first prompt was refused");
                }
            }
        }
        turned
    }

    /// Apply what a process's turns mean to the session it serves. Whether
    /// a turn failed.
    fn turned(mut serving: Option<&mut Session>, turned: Vec<Turned>) -> bool {
        let mut failed = false;
        for turn in turned {
            match turn {
                Turned::Started { conversation } => {
                    if let Some(serving) = serving.as_deref_mut() {
                        serving.turn_started(conversation);
                    }
                }
                Turned::Ended {
                    failed: ended_failed,
                } => {
                    failed |= ended_failed;
                    if let Some(serving) = serving.as_deref_mut() {
                        serving.turn_ended();
                    }
                }
            }
        }
        failed
    }

    fn publish(&self, update: RunUpdate) {
        // No subscriber is not an error.
        let _ = self.updates.send(update);
    }

    /// Lines an agent's stdin queue holds before it counts as not reading.
    const STDIN_LINES: usize = 256;

    /// Queue `line` for the run's stdin. A queue that is full belongs to an
    /// agent that stopped reading: its stdin is closed rather than the task
    /// waiting on it.
    fn write(run: RunId, live: &mut Live, line: &Line) {
        let Some(stdin) = live.stdin.as_ref() else {
            return;
        };
        match stdin.try_send(line.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // The process is ending; its exit will say how.
                tracing::debug!(%run, "agent stdin closed");
                live.stdin = None;
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(%run, "an agent stopped reading its stdin; closing it");
                live.stdin = None;
            }
        }
    }

    /// Write each queued line to `stdin`, in order, until the queue closes
    /// or a write fails.
    async fn writer(
        mut stdin: Box<dyn AsyncWrite + Send + Unpin>,
        mut lines: mpsc::Receiver<Line>,
    ) -> std::io::Result<()> {
        while let Some(line) = lines.recv().await {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }
        Ok(())
    }

    fn output(
        run: RunId,
        stdout: Box<dyn AsyncRead + Send + Unpin>,
        limit: LineLimit,
    ) -> BoxStream<'static, (RunId, Vec<Framed>)> {
        const CHUNK: usize = 64 * 1024;
        stream::unfold(
            Some((stdout, Lines::new(limit), vec![0; CHUNK])),
            move |reading| async move {
                let (mut stdout, mut lines, mut buffer) = reading?;
                match stdout.read(&mut buffer).await {
                    Ok(0) | Err(_) => Some(((run, lines.finish().into_iter().collect()), None)),
                    Ok(read) => {
                        let frames = lines.push(buffer.get(..read).unwrap_or_default());
                        Some(((run, frames), Some((stdout, lines, buffer))))
                    }
                }
            },
        )
        .boxed()
    }
}
