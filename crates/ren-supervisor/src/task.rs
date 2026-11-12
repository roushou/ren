//! The shell around the pure run machine: one task owns every run, reads each
//! agent's stdout, writes its stdin, and waits for its exit. Components do not
//! spawn, so the daemon runs this task; it ends once every [`Supervisor`]
//! handle is gone and every run has ended.
//!
//! Writes to an agent's stdin are awaited in the task. Ren's lines are small,
//! so this waits only if an agent stops reading its stdin altogether.
//!
//! [`Supervisor`]: crate::Supervisor

use crate::entries::Entries;
use crate::models::{Listings, ModelProbe, ModelsError, ModelsReply};
use crate::{
    CommandError, ComposeError, Conversation, Places, Request, RunSnapshot, RunUpdate, Start,
    StartError, StartPlace, Warmup,
    client::{Client, ClientStatus},
    files::FileListing,
    host::{AgentHost, AgentProcess, KillSwitch},
    project::Search,
    run::{ProcessExit, RunAction, RunCommand, RunEvent, RunState},
    workdir::{Bounds, RequestedDir, Workdir},
};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, BoxStream, FuturesUnordered, SelectAll},
};
use ren_harness::{
    Harness,
    drive::{DriveEvent, Driver, Opening, Prompt},
    endpoint::GatewayEndpoint,
    lines::{Framed, Line, LineLimit, Lines},
    models::ModelListing,
    settings::{ApprovalPolicy, Midway, RunSettings},
};
use ren_protocol::label::Labeled;
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    Recorder, RunTap,
    capture::RunEnd,
    id::{IdSource, RunId},
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{broadcast, mpsc},
};

/// Runs every run's I/O. Returned by [`Supervisor::new`](crate::Supervisor::new).
#[must_use = "the supervisor does nothing until its task runs"]
pub struct SupervisorTask<H> {
    pub(crate) requests: mpsc::Receiver<Request>,
    pub(crate) updates: broadcast::Sender<RunUpdate>,
    pub(crate) host: H,
    pub(crate) ids: Arc<dyn IdSource>,
    pub(crate) recorder: Recorder,
    pub(crate) bounds: Bounds,
    pub(crate) gateway: GatewayEndpoint,
    pub(crate) warmup: Warmup,
    pub(crate) version: String,
    pub(crate) line_limit: LineLimit,
}

/// One run the task knows of.
struct Live {
    harness: Harness,
    workdir: Workdir,
    settings: RunSettings,
    state: RunState,
    codec: Option<Driver>,
    stdin: Option<Box<dyn AsyncWrite + Send + Unpin>>,
    kill: Option<KillSwitch>,
    /// Records the run's opening and end; taken when the end is recorded.
    tap: Option<RunTap>,
    /// The first prompt, sent once the conversation opens.
    first: Option<Prompt>,
    /// What the run's conversation keeps of its client's events.
    entries: Entries,
    /// The run's folder as walked for the last file search, until a turn
    /// ends: turns change files.
    files: Option<FileListing>,
}

type Outputs = SelectAll<BoxStream<'static, (RunId, Vec<Framed>)>>;
type Exits = FuturesUnordered<BoxFuture<'static, (RunId, ProcessExit)>>;

impl<H: AgentHost> SupervisorTask<H> {
    pub async fn run(mut self) {
        let mut runs: BTreeMap<RunId, Live> = BTreeMap::new();
        let mut outputs: Outputs = SelectAll::new();
        let mut exits: Exits = FuturesUnordered::new();
        let mut listings = Listings::default();
        match self.warmup {
            Warmup::AtStart => self.warm(&mut listings),
            Warmup::WhenAsked => {}
        }
        let mut accepting = true;
        loop {
            tokio::select! {
                request = self.requests.recv(), if accepting => match request {
                    Some(Request::Models { harness, reply }) => self.models(harness, reply, &mut listings),
                    Some(request) => self.request(request, &mut runs, &mut outputs, &mut exits).await,
                    None => {
                        accepting = false;
                        for (run, live) in &mut runs {
                            self.apply(*run, live, RunEvent::Shutdown).await;
                        }
                    }
                },
                Some((run, frames)) = outputs.next(), if !outputs.is_empty() => {
                    if let Some(live) = runs.get_mut(&run) {
                        for frame in frames {
                            self.frame(run, live, frame).await;
                        }
                    }
                }
                Some((harness, outcome)) = listings.running.next(), if !listings.running.is_empty() => {
                    listings.finish(harness, outcome);
                }
                Some((run, exit)) = exits.next(), if !exits.is_empty() => {
                    if let Some(live) = runs.get_mut(&run) {
                        live.stdin = None;
                        live.kill = None;
                        self.apply(run, live, RunEvent::Exited(exit)).await;
                    }
                }
                else => break,
            }
            if !accepting && exits.is_empty() {
                break;
            }
        }
    }

    async fn request(
        &self,
        request: Request,
        runs: &mut BTreeMap<RunId, Live>,
        outputs: &mut Outputs,
        exits: &mut Exits,
    ) {
        match request {
            Request::Start { start, reply } => {
                let started = self.start(start, runs, outputs, exits).await;
                let _ = reply.send(started);
            }
            Request::Command {
                run,
                command,
                reply,
            } => {
                let result = match runs.get_mut(&run) {
                    Some(live) => self.command(run, live, command).await,
                    None => Err(CommandError::NoSuchRun(run)),
                };
                let _ = reply.send(result);
            }
            Request::Projects { reply } => {
                // Bounded by `Search`; the host reads only folder listings
                // and `HEAD` files.
                let projects = self.bounds.places(Search::DEFAULT, |folder| {
                    self.host
                        .look(folder)
                        .inspect_err(|error| tracing::debug!(%error, "cannot look in a folder"))
                        .ok()
                });
                let _ = reply.send(Places {
                    projects,
                    scratch: self
                        .bounds
                        .scratch()
                        .and_then(|scratch| scratch.to_str())
                        .map(str::to_owned),
                });
            }
            Request::Clients { reply } => {
                let clients = Harness::ALL
                    .iter()
                    .map(|harness| Client {
                        harness: *harness,
                        status: match harness.driven_program() {
                            Ok(program) if self.host.finds(program) => ClientStatus::Ready,
                            Ok(_) => ClientStatus::NotInstalled,
                            Err(_) => ClientStatus::NotDrivable,
                        },
                        approvals: ApprovalPolicy::offered(*harness).to_vec(),
                        midway: Midway::offered(*harness).to_vec(),
                    })
                    .collect();
                let _ = reply.send(clients);
            }
            Request::Commands { run, reply } => {
                let _ = reply.send(match runs.get(&run) {
                    Some(live) => Ok(live
                        .codec
                        .as_ref()
                        .map(|codec| codec.commands().to_vec())
                        .unwrap_or_default()),
                    None => Err(ComposeError::NoSuchRun(run)),
                });
            }
            Request::Files {
                run,
                query,
                limit,
                reply,
            } => {
                let _ = reply.send(match runs.get_mut(&run) {
                    Some(live) => self.files(live).map(|files| files.search(&query, limit)),
                    None => Err(ComposeError::NoSuchRun(run)),
                });
            }
            Request::Models { harness, reply } => {
                // Answered by the loop, which holds the listings.
                let _ = reply.send(Err(ModelsError::Gone));
                tracing::debug!(%harness, "a models request reached the run handler");
            }
            Request::List { reply } => {
                let _ = reply.send(
                    runs.iter()
                        .map(|(run, live)| RunSnapshot {
                            run: *run,
                            harness: live.harness,
                            workdir: live.workdir.as_path().to_path_buf(),
                            state: live.state.clone(),
                            settings: live.settings.clone(),
                        })
                        .collect(),
                );
            }
        }
    }

    /// The run's folder as last walked, walked now when it is not kept.
    /// Bounded by `FileListing::LIMITS`, so a walk is short.
    fn files<'a>(&self, live: &'a mut Live) -> Result<&'a FileListing, ComposeError> {
        if live.files.is_none() {
            let found = self
                .host
                .files(live.workdir.as_path(), FileListing::LIMITS)
                .map_err(ComposeError::Unreadable)?;
            live.files = Some(FileListing::new(found));
        }
        live.files.as_ref().ok_or(ComposeError::Gone)
    }

    async fn start(
        &self,
        Start {
            harness,
            place,
            conversation,
            settings,
            first,
        }: Start,
        runs: &mut BTreeMap<RunId, Live>,
        outputs: &mut Outputs,
        exits: &mut Exits,
    ) -> Result<RunId, StartError> {
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
                self.host
                    .make_scratch(&folder)
                    .map_err(StartError::Scratch)?;
                RequestedDir::parse(folder.to_str().ok_or(StartError::NoScratch)?)
                    .map_err(StartError::Workdir)?
            }
        };
        let resolved = self
            .host
            .resolve(requested.as_path())
            .map_err(StartError::Resolve)?;
        let workdir = Workdir::confirm(&self.bounds, resolved).map_err(StartError::Workdir)?;
        if let Some(policy) = settings.approvals
            && !ApprovalPolicy::offered(harness).contains(&policy)
        {
            return Err(StartError::NoApprovals(harness));
        }
        let opening = match conversation {
            Conversation::New => Opening::New {
                cwd: workdir.as_str().to_owned(),
                id: NativeThreadId::new(self.ids.next_uuid()).ok_or(StartError::NilId)?,
                settings: settings.clone(),
            },
            Conversation::Resume(thread) => Opening::Resume {
                thread,
                cwd: workdir.as_str().to_owned(),
                settings: settings.clone(),
            },
        };
        let plan = harness
            .drive(&self.gateway, &opening)
            .map_err(StartError::NotDrivable)?;
        let (codec, opening_lines) =
            Driver::open(harness, opening, &self.version, self.ids.next_uuid())
                .map_err(StartError::NotDrivable)?;
        let run = RunId::generate(self.ids.as_ref());
        // Every run is recorded: without room, it does not start.
        let tap = self
            .recorder
            .run(run, harness, workdir.as_str().to_owned(), settings.clone())
            .map_err(StartError::Unrecorded)?;
        let process = match self.host.spawn(&plan, &workdir) {
            Ok(process) => process,
            Err(source) => {
                tap.end(RunEnd::SpawnFailed, None);
                let state = RunState::Ended {
                    end: RunEnd::SpawnFailed,
                    exit: None,
                };
                self.publish(RunUpdate::State {
                    run,
                    state: state.clone(),
                });
                runs.insert(
                    run,
                    Live {
                        harness,
                        workdir,
                        settings,
                        state,
                        codec: None,
                        stdin: None,
                        kill: None,
                        tap: None,
                        first: None,
                        entries: Entries::default(),
                        files: None,
                    },
                );
                return Err(StartError::Spawn { run, source });
            }
        };
        let AgentProcess {
            stdin,
            stdout,
            exit,
            kill,
        } = process;
        let mut live = Live {
            harness,
            workdir,
            settings,
            state: RunState::Starting,
            codec: Some(codec),
            stdin: Some(stdin),
            kill: Some(kill),
            tap: Some(tap),
            first,
            entries: Entries::default(),
            files: None,
        };
        self.publish(RunUpdate::State {
            run,
            state: RunState::Starting,
        });
        for line in &opening_lines {
            Self::write(run, &mut live, line).await;
        }
        outputs.push(Self::output(run, stdout, self.line_limit));
        exits.push(exit.map(move |exit| (run, exit)).boxed());
        runs.insert(run, live);
        Ok(run)
    }

    /// List the models of every client installed here, with no one waiting.
    fn warm(&self, listings: &mut Listings) {
        for harness in Harness::ALL {
            let installed = harness
                .driven_program()
                .is_ok_and(|program| self.host.finds(program));
            if !installed {
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
        let plan = harness
            .list_models(&self.gateway)
            .map_err(ModelsError::NotDrivable)?;
        let (listing, first) =
            ModelListing::open(harness, &self.version).map_err(ModelsError::NotDrivable)?;
        let root = self
            .bounds
            .roots()
            .iter()
            .next()
            .ok_or(ModelsError::NoRoot)?;
        let workdir =
            Workdir::confirm(&self.bounds, root.to_path_buf()).map_err(|_| ModelsError::NoRoot)?;
        let process = self.host.spawn(&plan, &workdir).map_err(|error| {
            tracing::debug!(%harness, %error, "cannot start a client to list its models");
            ModelsError::Spawn
        })?;
        Ok(ModelProbe::run(listing, first, process, self.line_limit).boxed())
    }

    async fn command(
        &self,
        run: RunId,
        live: &mut Live,
        command: RunCommand,
    ) -> Result<(), CommandError> {
        // Settings the run moves to, once the client takes them.
        let configuring = if let RunCommand::Configure(settings) = &command {
            Some(settings.clone())
        } else {
            None
        };
        let transition = live
            .state
            .apply(RunEvent::Operator(command))
            .map_err(CommandError::Refused)?;
        // Encode before committing, so a command the protocol refuses leaves
        // the run as it was.
        let mut lines = Vec::new();
        let mut kill = false;
        for action in transition.actions {
            match action {
                RunAction::Send(command) => {
                    let codec = live.codec.as_mut().ok_or(CommandError::NoSuchRun(run))?;
                    lines.extend(codec.command(command).map_err(CommandError::Protocol)?);
                }
                RunAction::Kill => kill = true,
            }
        }
        for line in &lines {
            Self::write(run, live, line).await;
        }
        if kill && let Some(switch) = live.kill.take() {
            switch.fire();
        }
        if let Some(settings) = configuring {
            if let Some(tap) = live.tap.as_ref() {
                tap.configured(settings.clone());
            }
            live.settings = settings;
            // The state may not change, but the run did: say so.
            self.publish(RunUpdate::State {
                run,
                state: transition.state.clone(),
            });
        }
        self.settle(run, live, transition.state);
        Ok(())
    }

    async fn frame(&self, run: RunId, live: &mut Live, frame: Framed) {
        let Some(codec) = live.codec.as_mut() else {
            return;
        };
        let received = codec.receive(frame);
        for reply in &received.replies {
            Self::write(run, live, reply).await;
        }
        for event in received.events {
            if let DriveEvent::TurnEnded { .. } = event {
                live.files = None;
            }
            if let (DriveEvent::Ready { thread }, Some(tap)) = (&event, live.tap.as_ref()) {
                tap.opened(*thread);
            }
            if let Some(tap) = live.tap.as_ref() {
                for (turn, entry) in live.entries.take(&event) {
                    tap.entry(turn, entry);
                }
            }
            self.publish(RunUpdate::Event {
                run,
                event: event.clone(),
            });
            self.apply(run, live, RunEvent::Agent(event)).await;
            // Open now: the first prompt goes out.
            if let RunState::Idle { .. } = live.state
                && let Some(prompt) = live.first.take()
                && let Err(error) = self.command(run, live, RunCommand::Prompt(prompt)).await
            {
                tracing::debug!(%run, %error, "the first prompt was refused");
            }
        }
    }

    /// Apply an event that is not the operator's. Refusals here mean the run
    /// already ended; they change nothing.
    async fn apply(&self, run: RunId, live: &mut Live, event: RunEvent) {
        let transition = match live.state.apply(event) {
            Ok(transition) => transition,
            Err(refused) => {
                tracing::debug!(%run, %refused, "event after the run's end");
                return;
            }
        };
        for action in transition.actions {
            match action {
                RunAction::Send(command) => {
                    let lines = match live.codec.as_mut().map(|codec| codec.command(command)) {
                        Some(Ok(lines)) => lines,
                        Some(Err(refused)) => {
                            // A queued prompt the client would not take.
                            tracing::debug!(%run, %refused, "the client refused a command");
                            Vec::new()
                        }
                        None => Vec::new(),
                    };
                    for line in &lines {
                        Self::write(run, live, line).await;
                    }
                }
                RunAction::Kill => {
                    if let Some(switch) = live.kill.take() {
                        switch.fire();
                    }
                }
            }
        }
        self.settle(run, live, transition.state);
    }

    fn settle(&self, run: RunId, live: &mut Live, state: RunState) {
        if let RunState::Ended { end, exit } = &state
            && let Some(tap) = live.tap.take()
        {
            for (turn, entry) in live.entries.finish() {
                tap.entry(turn, entry);
            }
            tap.end(*end, *exit);
        }
        if state != live.state {
            live.state = state.clone();
            self.publish(RunUpdate::State { run, state });
        }
    }

    fn publish(&self, update: RunUpdate) {
        // No subscriber is not an error.
        let _ = self.updates.send(update);
    }

    async fn write(run: RunId, live: &mut Live, line: &Line) {
        let Some(stdin) = live.stdin.as_mut() else {
            return;
        };
        let written = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        }
        .await;
        if let Err(error) = written {
            // The process is ending; its exit will say how.
            tracing::debug!(%run, %error, "agent stdin closed");
            live.stdin = None;
        }
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
