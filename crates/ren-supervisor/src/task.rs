//! The shell around the pure run machine: one task owns every run, reads each
//! agent's stdout, writes its stdin, and waits for its exit. Components do not
//! spawn, so the daemon runs this task; it ends once every [`Supervisor`]
//! handle is gone and every run has ended.
//!
//! Writes to an agent's stdin are awaited in the task. Ren's lines are small,
//! so this waits only if an agent stops reading its stdin altogether.
//!
//! [`Supervisor`]: crate::Supervisor

use crate::{
    CommandError, Conversation, Request, RunSnapshot, RunUpdate, StartError,
    host::{AgentHost, AgentProcess, KillSwitch},
    run::{ProcessExit, RunAction, RunCommand, RunEvent, RunState},
    workdir::{RequestedDir, Roots, Workdir},
};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, BoxStream, FuturesUnordered, SelectAll},
};
use ren_harness::{
    Harness,
    drive::{DriveEvent, Driver, Framed, Line, LineLimit, Lines, Opening},
    endpoint::GatewayEndpoint,
};
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
    pub(crate) roots: Roots,
    pub(crate) gateway: GatewayEndpoint,
    pub(crate) version: String,
    pub(crate) line_limit: LineLimit,
}

/// One run the task knows of.
struct Live {
    harness: Harness,
    workdir: Workdir,
    state: RunState,
    codec: Option<Driver>,
    stdin: Option<Box<dyn AsyncWrite + Send + Unpin>>,
    kill: Option<KillSwitch>,
    /// Records the run's opening and end; taken when the end is recorded.
    tap: Option<RunTap>,
}

type Outputs = SelectAll<BoxStream<'static, (RunId, Vec<Framed>)>>;
type Exits = FuturesUnordered<BoxFuture<'static, (RunId, ProcessExit)>>;

impl<H: AgentHost> SupervisorTask<H> {
    pub async fn run(mut self) {
        let mut runs: BTreeMap<RunId, Live> = BTreeMap::new();
        let mut outputs: Outputs = SelectAll::new();
        let mut exits: Exits = FuturesUnordered::new();
        let mut accepting = true;
        loop {
            tokio::select! {
                request = self.requests.recv(), if accepting => match request {
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
            Request::Start {
                harness,
                dir,
                conversation,
                reply,
            } => {
                let started = self
                    .start(harness, &dir, conversation, runs, outputs, exits)
                    .await;
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
            Request::List { reply } => {
                let _ = reply.send(
                    runs.iter()
                        .map(|(run, live)| RunSnapshot {
                            run: *run,
                            harness: live.harness,
                            workdir: live.workdir.as_path().to_path_buf(),
                            state: live.state.clone(),
                        })
                        .collect(),
                );
            }
        }
    }

    async fn start(
        &self,
        harness: Harness,
        dir: &str,
        conversation: Conversation,
        runs: &mut BTreeMap<RunId, Live>,
        outputs: &mut Outputs,
        exits: &mut Exits,
    ) -> Result<RunId, StartError> {
        let requested = RequestedDir::parse(dir).map_err(StartError::Workdir)?;
        let resolved = self
            .host
            .resolve(requested.as_path())
            .map_err(StartError::Resolve)?;
        let workdir = Workdir::confirm(&self.roots, resolved).map_err(StartError::Workdir)?;
        let opening = match conversation {
            Conversation::New => Opening::New {
                cwd: workdir.as_str().to_owned(),
                id: NativeThreadId::new(self.ids.next_uuid()).ok_or(StartError::NilId)?,
            },
            Conversation::Resume(thread) => Opening::Resume {
                thread,
                cwd: workdir.as_str().to_owned(),
            },
        };
        let plan = harness
            .drive(&self.gateway, &opening)
            .map_err(StartError::NotDrivable)?;
        let (codec, first) =
            Driver::open(harness, opening, &self.version).map_err(StartError::NotDrivable)?;
        let run = RunId::generate(self.ids.as_ref());
        // Every run is recorded: without room, it does not start.
        let tap = self
            .recorder
            .run(run, harness, workdir.as_str().to_owned())
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
                        state,
                        codec: None,
                        stdin: None,
                        kill: None,
                        tap: None,
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
            state: RunState::Starting,
            codec: Some(codec),
            stdin: Some(stdin),
            kill: Some(kill),
            tap: Some(tap),
        };
        self.publish(RunUpdate::State {
            run,
            state: RunState::Starting,
        });
        for line in &first {
            Self::write(run, &mut live, line).await;
        }
        outputs.push(Self::output(run, stdout, self.line_limit));
        exits.push(exit.map(move |exit| (run, exit)).boxed());
        runs.insert(run, live);
        Ok(run)
    }

    async fn command(
        &self,
        run: RunId,
        live: &mut Live,
        command: RunCommand,
    ) -> Result<(), CommandError> {
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
                    lines.push(codec.command(command).map_err(CommandError::Protocol)?);
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
            if let (DriveEvent::Ready { thread }, Some(tap)) = (&event, live.tap.as_ref()) {
                tap.opened(*thread);
            }
            self.publish(RunUpdate::Event {
                run,
                event: event.clone(),
            });
            self.apply(run, live, RunEvent::Agent(event)).await;
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
                    if let Some(line) = live
                        .codec
                        .as_mut()
                        .and_then(|codec| codec.command(command).ok())
                    {
                        Self::write(run, live, &line).await;
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
