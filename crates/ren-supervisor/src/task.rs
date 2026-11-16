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
//! [`Supervisor`]: crate::Supervisor

use crate::attachments::{AttachmentFolders, AttachmentUse};
use crate::host::Scratch;
use crate::login::{SignInError, SignIns, StatusCheck};
use crate::models::{Listings, ModelProbe, ModelsError, ModelsReply};
use crate::process::{Process, ProcessEffect, Rest, Serving};
use crate::{
    Create, IdleStop, Places, Reply, Request, RunSnapshot, RunUpdate, SessionError, SessionRequest,
    SessionSeed, SessionSnapshot, SignInRequest, StartError, StartPlace, Warmup,
    client::{Client, ClientStatus},
    files::{FileListing, FileMatch, FileQuery, FoundPath},
    host::{AgentHost, AgentProcess, KillSwitch},
    project::Search,
    run::{Delivery, ProcessExit, RunCommand, RunState},
    session::{Launch, Room, Session, SessionEffect, Shelving, TurnsEnded},
    workdir::{Bounds, RequestedDir, Workdir},
};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, BoxStream, FuturesUnordered, SelectAll},
};
use ren_harness::sign_in::Console;
use ren_harness::{
    Harness,
    attachment::{Attachment, Limits, NotAccepted, Offer},
    drive::{Driver, Opening, Prompt, PromptImage},
    endpoint::GatewayEndpoint,
    lines::{Framed, Line, LineLimit, Lines},
    models::{DefaultChoice, ModelId, ModelListing},
    settings::{ApprovalPolicy, Midway, RunSettings},
};
use ren_protocol::label::Labeled;
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    Recorder, RunSession, RunTap, SessionSlot,
    batch::{Actor, HistoryChange, HistorySubject, SessionFact},
    id::{AttachmentId, IdSource, RunId, SessionId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    /// Unused drafts Ren discarded: the store may still hold one until the
    /// recorder reaches its delete, and it is not served again from there.
    discarded: BTreeSet<SessionId>,
    /// Each client's models, listed or being listed.
    listings: Listings,
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
                    served.listings.finish(harness, outcome);
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
                    self.ended(run, &mut served).await;
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

    /// Search the session's folder as last walked, answering now, or the
    /// walk off the task that answers once [`Self::walked`] takes it.
    /// Bounded by `FileListing::LIMITS`.
    #[must_use = "a walk answers its search only once it is taken"]
    fn search_files(
        &self,
        session: SessionId,
        query: FileQuery,
        limit: usize,
        reply: Reply<Vec<FileMatch>>,
        served: &Served,
    ) -> Option<BoxFuture<'static, Walked>> {
        let Some(served_session) = served.sessions.get(&session) else {
            let _ = reply.send(Err(SessionError::NoSuchSession(session)));
            return None;
        };
        if let Some(files) = served_session.files() {
            let _ = reply.send(Ok(files.search(&query, limit)));
            return None;
        }
        let (workdir, turns_ended) = served_session.walk();
        let dir = workdir.as_path().to_path_buf();
        let walk = self.off_task(move |host| host.files(&dir, FileListing::LIMITS));
        Some(
            async move {
                Walked {
                    session,
                    turns_ended,
                    found: walk.await,
                    query,
                    limit,
                    reply,
                }
            }
            .boxed(),
        )
    }

    /// A walk finished: kept for later searches, unless a turn ended since
    /// it began, and the search that asked answered.
    fn walked(walked: Walked, served: &mut Served) {
        let Walked {
            session,
            turns_ended,
            found,
            query,
            limit,
            reply,
        } = walked;
        let answer = found.map_err(SessionError::Unreadable).map(|found| {
            let listing = FileListing::new(found);
            let matches = listing.search(&query, limit);
            if let Some(served_session) = served.sessions.get_mut(&session) {
                served_session.walked(listing, turns_ended);
            }
            matches
        });
        let _ = reply.send(answer);
    }

    /// An attachment's bytes, read off the task.
    fn read_attachment(
        &self,
        session: SessionId,
        attachment: AttachmentId,
        served: &Served,
    ) -> Result<Reading, SessionError> {
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let held = served
            .sessions
            .get(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        let read = held
            .attached()
            .get(attachment)
            .ok_or(SessionError::NoSuchAttachment(attachment))?
            .clone();
        let path = folders.file(session, attachment, read.name());
        let reading = self.off_task(move |host| host.read_file(&path));
        Ok(async move {
            reading
                .await
                .map(|bytes| (read, bytes))
                .map_err(SessionError::AttachmentStorage)
        }
        .boxed())
    }

    /// The folder a session works in: a folder inside the bounds, or a new
    /// scratch folder of its own.
    async fn place(&self, place: StartPlace) -> Result<Workdir, StartError> {
        let requested = match place {
            StartPlace::Folder(dir) => RequestedDir::parse(&dir).map_err(StartError::Workdir)?,
            StartPlace::Scratch => {
                let scratch = self.bounds.scratch().ok_or(StartError::NoScratch)?;
                // Its last eight hex digits: random for real ids, and
                // distinct for sequential ones.
                let id = self.ids.next_uuid().simple().to_string();
                let folder = scratch.join(format!(
                    "scratch-{}",
                    id.get(id.len().saturating_sub(8)..).unwrap_or(&id)
                ));
                let making = folder.clone();
                let made = self
                    .off_task(move |host| host.make_scratch(&making, &host.tools()))
                    .await
                    .map_err(StartError::Scratch)?;
                match made {
                    Scratch::Repository => {}
                    Scratch::Untracked(why) => {
                        tracing::warn!(?folder, ?why, "a scratch folder is not a git repository");
                    }
                }
                RequestedDir::parse(folder.to_str().ok_or(StartError::NoScratch)?)
                    .map_err(StartError::Workdir)?
            }
        };
        self.confirm(&requested)
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
        let mut live = Live {
            process,
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

    /// Carry out `serving`'s effects, with the slot its fact records in
    /// (each change records at most one), and serve it again unless it was
    /// discarded. A process it started, or why it did not start.
    async fn carry_out_session(
        &self,
        session: SessionId,
        mut serving: Session,
        effects: Vec<SessionEffect>,
        mut slot: Option<SessionSlot>,
        served: &mut Served,
    ) -> Option<Result<RunId, StartError>> {
        let mut started = None;
        let mut discard = false;
        for effect in effects {
            match effect {
                SessionEffect::Record(fact) => match slot.take() {
                    Some(slot) => slot.record(session, fact),
                    None => tracing::warn!(%session, "a session fact had no slot"),
                },
                SessionEffect::StopProcess => {
                    let run = served.serving(&serving);
                    self.stop_process(session, run, served);
                }
                SessionEffect::StartProcess { first } => {
                    started = Some(
                        self.spawn(
                            session,
                            &mut serving,
                            RunSession::Existing(session),
                            first,
                            served,
                        )
                        .await,
                    );
                }
                SessionEffect::Discard => discard = true,
            }
        }
        let kept = if discard {
            self.discard(session, serving, served)
        } else {
            Some(serving)
        };
        if let Some(serving) = kept {
            self.publish_session(session, &serving);
            served.sessions.insert(session, serving);
        }
        started
    }

    /// Answer a session request, each in its own reply. A folder walk and
    /// an attachment read answer later, off the task.
    async fn session(&self, request: SessionRequest, served: &mut Served) {
        match request {
            SessionRequest::Create { create, reply } => {
                let _ = reply.send(self.create(create, served).await);
            }
            SessionRequest::Adopt { seed, reply } => {
                let _ = reply.send(self.adopt(seed, served));
            }
            SessionRequest::Send {
                session,
                command,
                reply,
            } => {
                let _ = reply.send(self.send(session, command, served).await);
            }
            SessionRequest::ChooseClient {
                session,
                harness,
                reply,
            } => {
                let _ = reply.send(self.choose_client(session, harness, served).await);
            }
            SessionRequest::Shelve {
                session,
                shelving,
                reply,
            } => {
                let _ = reply.send(self.shelve(session, shelving, served).await);
            }
            SessionRequest::Forget { session, reply } => {
                let _ = reply.send(self.forget(session, served).await);
            }
            SessionRequest::Attach {
                session,
                name,
                bytes,
                reply,
            } => {
                let _ = reply.send(self.attach(session, &name, bytes, served).await);
            }
            SessionRequest::Attachments { session, reply } => {
                let _ = reply.send(
                    served
                        .sessions
                        .get(&session)
                        .ok_or(SessionError::NoSuchSession(session))
                        .map(|serving| serving.attached().list()),
                );
            }
            SessionRequest::Prompt {
                session,
                text,
                attachments,
                delivery,
                reply,
            } => {
                let prompted = self
                    .prompt_with(session, text, attachments, delivery, served)
                    .await;
                let _ = reply.send(prompted);
            }
            SessionRequest::ReadAttachment {
                session,
                attachment,
                reply,
            } => match self.read_attachment(session, attachment, served) {
                Ok(reading) => served.replies.push(
                    async move {
                        let _ = reply.send(reading.await);
                    }
                    .boxed(),
                ),
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            SessionRequest::Detach {
                session,
                attachment,
                reply,
            } => {
                let _ = reply.send(self.detach(session, attachment, served).await);
            }
            SessionRequest::List { reply } => {
                let _ = reply.send(Ok(served
                    .sessions
                    .iter()
                    .map(|(session, serving)| serving.snapshot(*session))
                    .collect()));
            }
            SessionRequest::Commands { session, reply } => {
                let _ = reply.send(if served.sessions.contains_key(&session) {
                    Ok(served
                        .process(session)
                        .and_then(|run| served.runs.get(&run))
                        .map(|live| live.process.commands().to_vec())
                        .unwrap_or_default())
                } else {
                    Err(SessionError::NoSuchSession(session))
                });
            }
            SessionRequest::Files {
                session,
                query,
                limit,
                reply,
            } => {
                if let Some(walk) = self.search_files(session, query, limit, reply, served) {
                    served.walks.push(walk);
                }
            }
        }
    }

    /// Stop serving `session`, which is being deleted, with its attachments
    /// and ended runs; refused while a process serves it.
    async fn forget(&self, session: SessionId, served: &mut Served) -> Result<(), SessionError> {
        if served.process(session).is_some() {
            return Err(SessionError::Running);
        }
        // Its attachments go with it, served or not.
        if let Some(folders) = &self.attachments {
            let folder = folders.session(session);
            self.off_task(move |host| host.remove_tree(&folder))
                .await
                .map_err(SessionError::AttachmentStorage)?;
        }
        if served.sessions.remove(&session).is_none() {
            return Err(SessionError::NoSuchSession(session));
        }
        served
            .runs
            .retain(|_, live| live.process.session() != session);
        Ok(())
    }

    /// Prompt `session` with `text` and its `attachments`; sent, they stay.
    async fn prompt_with(
        &self,
        session: SessionId,
        text: String,
        attachments: Vec<AttachmentId>,
        delivery: Delivery,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let composing = self.compose(session, text, &attachments, served)?;
        let prompt = self.prompt(composing).await?;
        self.send(session, delivery.command(prompt), served).await?;
        if !attachments.is_empty() {
            self.record(
                session,
                SessionFact::AttachmentsSent {
                    attachments: attachments.clone(),
                },
            )?;
            if let Some(serving) = served.sessions.get_mut(&session) {
                serving.attached_mut().sent(&attachments);
            }
        }
        Ok(())
    }

    /// Remove one of `session`'s unsent attachments, its record, then its
    /// file.
    async fn detach(
        &self,
        session: SessionId,
        attachment: AttachmentId,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let serving = served
            .sessions
            .get_mut(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        match serving.attached().used(attachment) {
            None => return Err(SessionError::NoSuchAttachment(attachment)),
            Some(AttachmentUse::Sent) => {
                return Err(SessionError::AttachmentSent(attachment));
            }
            Some(AttachmentUse::Unsent) => {}
        }
        self.record(session, SessionFact::Detached { attachment })?;
        serving.attached_mut().remove(attachment);
        // Detached as recorded; a file left behind is only space.
        let folder = folders.attachment(session, attachment);
        if let Err(error) = self.off_task(move |host| host.remove_tree(&folder)).await {
            tracing::warn!(%session, %attachment, %error, "a detached attachment's file stays");
        }
        Ok(())
    }

    async fn create(
        &self,
        Create {
            harness,
            place,
            settings,
            resume,
        }: Create,
        served: &mut Served,
    ) -> Result<SessionId, SessionError> {
        let workdir = self.place(place).await.map_err(SessionError::Start)?;
        Self::drivable(harness, &settings).map_err(SessionError::Start)?;
        let session = SessionId::generate(self.ids.as_ref());
        self.record(
            session,
            SessionFact::Created {
                harness,
                workdir: workdir.as_str().to_owned(),
                settings: settings.clone(),
            },
        )?;
        // Its client starts now, so its commands are there before the first
        // prompt. One that cannot start is kept, ended, as its process.
        let new = Session::new(harness, workdir, settings, resume);
        let start = vec![SessionEffect::StartProcess { first: None }];
        if let Some(Err(error)) = self
            .carry_out_session(session, new, start, None, served)
            .await
        {
            tracing::debug!(%session, %error, "a new session's client did not start");
        }
        Ok(session)
    }

    /// The prompt `text` and `attachments` make for `session`: each one the
    /// session's, together within the limits, of a kind its client takes,
    /// and read from its file.
    fn compose(
        &self,
        session: SessionId,
        text: String,
        attachments: &[AttachmentId],
        served: &Served,
    ) -> Result<Composing, SessionError> {
        let held = served
            .sessions
            .get(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        if attachments.is_empty() {
            return Prompt::parse(text)
                .map(Composing::Ready)
                .map_err(|_| SessionError::BlankPrompt);
        }
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let reads = attachments
            .iter()
            .map(|attachment| {
                held.attached()
                    .get(*attachment)
                    .cloned()
                    .ok_or(SessionError::NoSuchAttachment(*attachment))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.attachment_limits
            .check_prompt(&reads)
            .map_err(SessionError::PromptAttachments)?;
        // What the running client said; with none running, the next one says
        // as it opens, and its codec refuses what it was not offered.
        let offer = served
            .process(session)
            .and_then(|run| served.runs.get(&run))
            .map_or(Offer::Unknown, |live| live.process.image_offer());
        let mut files = Vec::with_capacity(reads.len());
        for (attachment, read) in attachments.iter().zip(reads) {
            match held.harness().acceptance().of(read.kind()).resolve(offer) {
                Ok(_) | Err(NotAccepted::Unknown) => {}
                Err(refused @ (NotAccepted::Never | NotAccepted::NotOffered)) => {
                    return Err(SessionError::ImagesNotTaken(refused));
                }
            }
            let path = folders.file(session, *attachment, read.name());
            files.push((read, path));
        }
        Ok(Composing::Images { text, files })
    }

    /// The prompt, its images read off the task.
    async fn prompt(&self, composing: Composing) -> Result<Prompt, SessionError> {
        let (text, files) = match composing {
            Composing::Ready(prompt) => return Ok(prompt),
            Composing::Images { text, files } => (text, files),
        };
        let paths: Vec<_> = files.iter().map(|(_, path)| path.clone()).collect();
        let bytes = self
            .off_task(move |host| {
                paths
                    .iter()
                    .map(|path| host.read_file(path))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(SessionError::AttachmentStorage)?;
        let images = files
            .into_iter()
            .zip(bytes)
            .map(|((read, path), bytes)| PromptImage::new(read, path, bytes.into()))
            .collect::<Vec<_>>();
        Prompt::with_images(text, images).map_err(|_| SessionError::BlankPrompt)
    }

    /// Read `bytes`, uploaded as `name`, as an attachment of `session`,
    /// write it to the session's folder, and record it.
    async fn attach(
        &self,
        session: SessionId,
        name: &str,
        bytes: Vec<u8>,
        served: &mut Served,
    ) -> Result<(AttachmentId, Attachment), SessionError> {
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let held = served
            .sessions
            .get_mut(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        let read = Attachment::parse(name, &bytes, &self.attachment_limits)
            .map_err(SessionError::Attachment)?;
        let attachment = AttachmentId::generate(self.ids.as_ref());
        let file = folders.file(session, attachment, read.name());
        let incoming = folders.incoming(attachment);
        let placing = file.clone();
        self.off_task(move |host| host.place_file(&incoming, &placing, &bytes))
            .await
            .map_err(SessionError::AttachmentStorage)?;
        // One that cannot be recorded is not kept.
        if let Err(error) = self.record(
            session,
            SessionFact::Attached {
                attachment,
                read: read.clone(),
            },
        ) {
            let folder = folders.attachment(session, attachment);
            let _ = self.off_task(move |host| host.remove_tree(&folder)).await;
            return Err(error);
        }
        held.attached_mut().insert(attachment, read.clone());
        Ok((attachment, read))
    }

    fn adopt(&self, seed: SessionSeed, served: &mut Served) -> Result<(), SessionError> {
        if served.sessions.contains_key(&seed.session) {
            return Ok(());
        }
        if served.discarded.contains(&seed.session) {
            return Err(SessionError::NoSuchSession(seed.session));
        }
        let requested = RequestedDir::parse(&seed.workdir)
            .map_err(|error| SessionError::Start(StartError::Workdir(error)))?;
        let workdir = self.confirm(&requested).map_err(SessionError::Start)?;
        let session = seed.session;
        let restored = Session::restore(seed, workdir);
        self.publish_session(session, &restored);
        served.sessions.insert(session, restored);
        Ok(())
    }

    async fn send(
        &self,
        session: SessionId,
        command: RunCommand,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let prompt = match &command {
            RunCommand::Prompt(prompt) | RunCommand::Queue(prompt) | RunCommand::Steer(prompt) => {
                Some(prompt.clone())
            }
            RunCommand::Unqueue
            | RunCommand::Configure(_)
            | RunCommand::Answer { .. }
            | RunCommand::Interrupt
            | RunCommand::Stop => None,
        };
        let (slot, room) = self.room();
        let outcome = match served.serving(&serving) {
            Some(run) => match served.runs.get_mut(&run) {
                None => Err(SessionError::NoProcess),
                Some(live) => match live.process.command(command) {
                    Err(error) => Err(SessionError::Command(error)),
                    Ok(effects) => {
                        self.carry_out(live, effects, &mut served.idles);
                        serving.took(live.process.settings().clone());
                        match prompt {
                            Some(_) => serving.prompted(room),
                            None => Ok(Vec::new()),
                        }
                    }
                },
            },
            // No process: settings wait for the next one, and a prompt
            // starts one that resumes the conversation with it.
            None => match (command, prompt) {
                (RunCommand::Configure(settings), _) => serving.configure_unserved(settings, room),
                (_, Some(prompt)) => serving.prompt_unserved(prompt, room),
                (
                    RunCommand::Prompt(_)
                    | RunCommand::Queue(_)
                    | RunCommand::Steer(_)
                    | RunCommand::Unqueue
                    | RunCommand::Answer { .. }
                    | RunCommand::Interrupt
                    | RunCommand::Stop,
                    None,
                ) => Err(SessionError::NoProcess),
            },
        };
        match self
            .settle_session(session, serving, outcome, slot, served)
            .await?
        {
            Some(Err(error)) => Err(SessionError::Start(error)),
            Some(Ok(_)) | None => Ok(()),
        }
    }

    async fn choose_client(
        &self,
        session: SessionId,
        harness: Harness,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        if !served.sessions.contains_key(&session) {
            return Err(SessionError::NoSuchSession(session));
        }
        Self::drivable(harness, &RunSettings::default()).map_err(SessionError::Start)?;
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let (slot, room) = self.room();
        let outcome = serving.choose_client(harness, room);
        self.settle_session(session, serving, outcome, slot, served)
            .await
            .map(|started| {
                if let Some(Err(error)) = started {
                    tracing::debug!(%session, %error, "the chosen client did not start");
                }
            })
    }

    async fn shelve(
        &self,
        session: SessionId,
        shelving: Shelving,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let (slot, room) = self.room();
        let outcome = serving.shelve(shelving, room);
        self.settle_session(session, serving, outcome, slot, served)
            .await
            .map(|_| ())
    }

    /// Carry out a session change's effects, or serve it again unchanged
    /// when it was refused.
    async fn settle_session(
        &self,
        session: SessionId,
        serving: Session,
        outcome: Result<Vec<SessionEffect>, SessionError>,
        slot: Option<SessionSlot>,
        served: &mut Served,
    ) -> Result<Option<Result<RunId, StartError>>, SessionError> {
        match outcome {
            Ok(effects) => Ok(self
                .carry_out_session(session, serving, effects, slot, served)
                .await),
            Err(refused) => {
                served.sessions.insert(session, serving);
                Err(refused)
            }
        }
    }

    /// After `run` ended, its session takes that in: a draft never asked
    /// anything and holding nothing, whose newest process stopped after
    /// resting idle, is discarded.
    async fn ended(&self, run: RunId, served: &mut Served) {
        let Some(live) = served.runs.get(&run) else {
            return;
        };
        let RunState::Ended { end, .. } = *live.process.state() else {
            return;
        };
        let session = live.process.session();
        let Some(serving) = served.sessions.get_mut(&session) else {
            return;
        };
        let effects = serving.process_ended(run, end);
        if effects.is_empty() {
            return;
        }
        if let Some(serving) = served.sessions.remove(&session) {
            let _ = self
                .carry_out_session(session, serving, effects, None, served)
                .await;
        }
    }

    /// Delete `session`, a draft never asked anything, as the operator's
    /// delete would: it stops being served, and the store deletes it and its
    /// ended runs when the recorder reaches the edit, after the run's end.
    /// Without room to record that, it is kept and served as it was.
    fn discard(
        &self,
        session: SessionId,
        serving: Session,
        served: &mut Served,
    ) -> Option<Session> {
        let Some(slot) = self.recorder.history_slot() else {
            tracing::debug!(%session, "no room to record an unused draft's discard; it is kept");
            return Some(serving);
        };
        if let Some(folders) = &self.attachments {
            let folder = folders.session(session);
            let removing = self.off_task(move |host| host.remove_tree(&folder));
            served.replies.push(
                async move {
                    if let Err(error) = removing.await {
                        tracing::warn!(%session, %error, "cannot remove an unused draft's attachments folder");
                    }
                }
                .boxed(),
            );
        }
        served
            .runs
            .retain(|_, live| live.process.session() != session);
        served.discarded.insert(session);
        slot.record(
            Actor::Ren,
            HistorySubject::OperatorSession(session),
            HistoryChange::Delete,
        );
        tracing::debug!(%session, "an unused draft was discarded");
        None
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

    /// Whether `harness`'s program is on this host's path.
    fn installed(&self, harness: Harness) -> bool {
        self.host.finds(harness.driven_program())
    }

    /// Every client as this host has it, with its sign-in as last read.
    fn clients(&self, sign_ins: &SignIns) -> Vec<Client> {
        Harness::ALL
            .iter()
            .map(|harness| Client {
                harness: *harness,
                status: match self.installed(*harness) {
                    true => ClientStatus::Ready,
                    false => ClientStatus::NotInstalled,
                },
                approvals: ApprovalPolicy::offered(*harness).to_vec(),
                midway: Midway::offered(*harness).to_vec(),
                sign_in: sign_ins.state(*harness),
                sign_in_methods: harness.sign_in_methods().to_vec(),
            })
            .collect()
    }

    /// Check the sign-in of each installed client that offers a status
    /// command and has not been checked.
    fn check_unchecked(&self, sign_ins: &mut SignIns) {
        for harness in Harness::ALL {
            let unchecked = matches!(
                sign_ins.state(*harness),
                crate::sign_in::SignInState::NotChecked
            );
            if unchecked && !harness.sign_in_status().is_empty() && self.installed(*harness) {
                sign_ins.check(*harness, None, || self.status_check(*harness));
            }
        }
    }

    /// The first root, where a client's own short commands run.
    fn root_workdir(&self) -> Result<Workdir, SignInError> {
        let root = self
            .bounds
            .roots()
            .iter()
            .next()
            .ok_or(SignInError::NoRoot)?;
        Workdir::confirm(&self.bounds, root.to_path_buf()).map_err(|_| SignInError::NoRoot)
    }

    /// Start `harness`'s status commands, read when they end.
    fn status_check(
        &self,
        harness: Harness,
    ) -> Result<BoxFuture<'static, crate::sign_in::SignInState>, SignInError> {
        let plans = harness.sign_in_status();
        if !self.installed(harness) {
            return Err(SignInError::NotInstalled(harness));
        }
        let workdir = self.root_workdir()?;
        let tools = self.host.tools();
        let processes = plans
            .iter()
            .map(|plan| self.host.spawn_with_stderr(plan, &workdir, &tools))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                tracing::debug!(%harness, %error, "cannot start a client's status command");
                SignInError::Spawn
            })?;
        Ok(StatusCheck::run(harness, processes).boxed())
    }

    /// Carry out a sign-in request.
    fn sign_in(&self, request: SignInRequest, sign_ins: &mut SignIns) {
        match request {
            SignInRequest::Check { harness, reply } => {
                sign_ins.check(harness, Some(reply), || self.status_check(harness));
            }
            SignInRequest::Start {
                harness,
                method,
                reply,
            } => {
                let started = harness
                    .sign_in(method)
                    .map_err(SignInError::from)
                    .and_then(|plan| {
                        if !self.installed(harness) {
                            return Err(SignInError::NotInstalled(harness));
                        }
                        let workdir = self.root_workdir()?;
                        let tools = self.host.tools();
                        let process = match plan.console {
                            Console::Pipes => {
                                self.host.spawn_with_stderr(&plan.launch, &workdir, &tools)
                            }
                            Console::Terminal => {
                                self.host.spawn_in_terminal(&plan.launch, &workdir, &tools)
                            }
                        };
                        process
                            .map(|process| (process, plan.console, plan.flow))
                            .map_err(|error| {
                                tracing::debug!(%harness, %error, "cannot start a client's login");
                                SignInError::Spawn
                            })
                    })
                    .map(|(process, console, flow)| {
                        sign_ins.login(harness, process, console, flow, self.line_limit)
                    });
                let _ = reply.send(started);
            }
            SignInRequest::Answer {
                harness,
                answer,
                reply,
            } => sign_ins.answer(
                harness,
                answer,
                |address| self.host.open_local(address),
                reply,
            ),
            SignInRequest::Cancel { harness, reply } => {
                let _ = reply.send(sign_ins.cancel(harness));
            }
            SignInRequest::Wait { harness, reply } => {
                let _ = reply.send(sign_ins.wait(harness));
            }
        }
    }

    /// List the models of every client installed here, with no one waiting.
    fn warm(&self, listings: &mut Listings) {
        for harness in Harness::ALL {
            if !self.installed(*harness) {
                continue;
            }
            match self.probe(*harness) {
                Ok(listing) => listings.warm(*harness, listing),
                Err(error) => tracing::debug!(%harness, %error, "cannot list a client's models"),
            }
        }
    }

    /// Answer from the listings, or start the client briefly in the first
    /// root to list its models.
    fn models(&self, harness: Harness, reply: ModelsReply, listings: &mut Listings) {
        let Some(reply) = listings.ask(harness, reply) else {
            return;
        };
        match self.probe(harness) {
            Ok(listing) => listings.start(harness, reply, listing),
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    fn probe(
        &self,
        harness: Harness,
    ) -> Result<BoxFuture<'static, Result<Vec<ren_harness::models::Model>, ModelsError>>, ModelsError>
    {
        let plan = harness.list_models(&self.gateway);
        let root = self
            .bounds
            .roots()
            .iter()
            .next()
            .ok_or(ModelsError::NoRoot)?;
        let workdir =
            Workdir::confirm(&self.bounds, root.to_path_buf()).map_err(|_| ModelsError::NoRoot)?;
        let (listing, first) =
            ModelListing::open(harness, &self.version, &workdir.as_path().to_string_lossy());
        let process = self
            .host
            .spawn(&plan, &workdir, &self.host.tools())
            .map_err(|error| {
                tracing::debug!(%harness, %error, "cannot start a client to list its models");
                ModelsError::Spawn
            })?;
        Ok(ModelProbe::run(listing, first, process, self.line_limit).boxed())
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
