//! The supervisor with an in-memory host: each "process" is a scripted agent
//! that speaks Codex's app-server protocol over in-memory pipes. No real
//! client, network, or credentials.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and scripted agents are spawned tasks"
)]

use ren_harness::{
    Harness,
    drive::{ApprovalId, Decision, LineLimit, Prompt},
    endpoint::GatewayEndpoint,
    launch::LaunchPlan,
};
use ren_recorder::{capture::RunEnd, id::SequentialIds};
use ren_supervisor::{
    AgentHost, AgentProcess, CommandError, KillSignal, KillSwitch, ProcessExit, RunCommand,
    RunRefused, RunState, RunUpdate, StartError, Supervisor, SupervisorOptions, Workdir,
};
use serde_json::{Value, json};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream},
    sync::{broadcast, oneshot},
};

const THREAD: &str = "0190f3a0-0000-7000-8000-000000000001";
const TURN: &str = "0190f3a0-0000-7000-8000-000000000002";

/// How the scripted agent behaves.
#[derive(Clone, Copy)]
enum Script {
    /// A well-behaved Codex: opens a thread, runs turns, asks one approval.
    Codex,
    /// Refuses `thread/start`.
    RefusesOpening,
    /// Exits with code 3 right after `initialize`.
    ExitsEarly,
}

/// The arguments and working directory of one spawned agent.
type Spawned = (Vec<String>, PathBuf);

#[derive(Clone)]
struct FakeHost {
    script: Script,
    spawn_fails: bool,
    spawned: Arc<Mutex<Vec<Spawned>>>,
}

impl FakeHost {
    fn new(script: Script) -> Self {
        Self {
            script,
            spawn_fails: false,
            spawned: Arc::default(),
        }
    }
}

impl AgentHost for FakeHost {
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        match path.to_str() {
            // A link that leads out of the roots.
            Some("/work/escape") => Ok(PathBuf::from("/etc")),
            Some(path) if path.starts_with("/work") || path == "/elsewhere" => Ok(path.into()),
            Some(_) | None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }

    fn spawn(&self, plan: &LaunchPlan, workdir: &Workdir) -> io::Result<AgentProcess> {
        if self.spawn_fails {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let command = plan.command(None, &[]);
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        self.spawned
            .lock()
            .unwrap()
            .push((arguments, workdir.as_path().to_path_buf()));
        let (stdin, agent_in) = tokio::io::duplex(64 * 1024);
        let (agent_out, stdout) = tokio::io::duplex(64 * 1024);
        let (exit_tx, exit_rx) = oneshot::channel();
        let (kill, signal) = KillSwitch::pair();
        tokio::spawn(agent(self.script, agent_in, agent_out, signal, exit_tx));
        Ok(AgentProcess {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            exit: Box::pin(async move { exit_rx.await.unwrap_or(ProcessExit::Unknown) }),
            kill,
        })
    }
}

async fn send(out: &mut DuplexStream, message: Value) {
    let mut line = message.to_string().into_bytes();
    line.push(b'\n');
    out.write_all(&line).await.unwrap();
}

/// A scripted Codex app server.
async fn agent(
    script: Script,
    input: DuplexStream,
    mut out: DuplexStream,
    kill: KillSignal,
    exit: oneshot::Sender<ProcessExit>,
) {
    let mut lines = BufReader::new(input).lines();
    let serve = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let message: Value = serde_json::from_str(&line).unwrap();
            let id = message["id"].clone();
            match (message["method"].as_str(), script) {
                (Some("initialize"), Script::ExitsEarly) => return ProcessExit::Code(3),
                (Some("initialize"), _) => send(&mut out, json!({ "id": id, "result": {} })).await,
                (Some("thread/start"), Script::RefusesOpening) => {
                    send(
                        &mut out,
                        json!({ "id": id, "error": { "code": -1, "message": "no" } }),
                    )
                    .await;
                }
                (Some("thread/start"), _) => {
                    send(
                        &mut out,
                        json!({ "id": id, "result": { "thread": { "id": THREAD } } }),
                    )
                    .await;
                }
                (Some("turn/start"), _) => {
                    let turn = json!({ "id": TURN, "status": "inProgress" });
                    send(&mut out, json!({ "id": id, "result": { "turn": turn } })).await;
                    send(
                        &mut out,
                        json!({ "id": "ap-1", "method": "item/fileChange/requestApproval",
                            "params": { "threadId": THREAD, "turnId": TURN, "itemId": "fc",
                                        "startedAtMs": 1, "reason": "edit src" } }),
                    )
                    .await;
                }
                (Some("turn/interrupt"), _) => {
                    send(&mut out, json!({ "id": id, "result": {} })).await;
                    send(
                        &mut out,
                        json!({ "method": "turn/completed", "params": { "threadId": THREAD,
                            "turn": { "id": TURN, "status": "interrupted" } } }),
                    )
                    .await;
                }
                (None, _) if id == "ap-1" => {
                    assert_eq!(message["result"]["decision"], "accept");
                    send(
                        &mut out,
                        json!({ "method": "item/agentMessage/delta", "params": { "threadId": THREAD,
                            "turnId": TURN, "itemId": "msg", "delta": "Edited." } }),
                    )
                    .await;
                    send(
                        &mut out,
                        json!({ "method": "turn/completed", "params": { "threadId": THREAD,
                            "turn": { "id": TURN, "status": "completed" } } }),
                    )
                    .await;
                }
                (Some(_) | None, _) => {}
            }
        }
        ProcessExit::Code(0)
    };
    let status = tokio::select! {
        status = serve => status,
        () = kill.fired() => ProcessExit::Signal(9),
    };
    let _ = exit.send(status);
}

fn options() -> SupervisorOptions {
    SupervisorOptions {
        roots: vec![PathBuf::from("/work")],
        gateway: GatewayEndpoint::parse("http://127.0.0.1:8787").unwrap(),
        version: "0.1.0".into(),
        line_limit: LineLimit::DEFAULT,
    }
}

fn supervise(host: FakeHost) -> (Supervisor, tokio::task::JoinHandle<()>) {
    let (supervisor, task) =
        Supervisor::new(options(), host, Arc::new(SequentialIds::default())).unwrap();
    (supervisor, tokio::spawn(task.run()))
}

/// The next state update matching `wanted`, skipping others.
async fn state(
    updates: &mut broadcast::Receiver<RunUpdate>,
    wanted: fn(&RunState) -> bool,
) -> RunState {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let RunUpdate::State { state, .. } = updates.recv().await.unwrap()
                && wanted(&state)
            {
                return state;
            }
        }
    })
    .await
    .expect("state update")
}

fn prompt() -> RunCommand {
    RunCommand::Prompt(Prompt::parse("edit src".into()).unwrap())
}

#[tokio::test]
async fn runs_a_turn_through_an_approval_and_stops() {
    let host = FakeHost::new(Script::Codex);
    let spawned = host.spawned.clone();
    let (supervisor, _task) = supervise(host);
    let mut updates = supervisor.subscribe();

    let run = supervisor
        .start(Harness::Codex, "/work/acme")
        .await
        .unwrap();
    let (arguments, workdir) = spawned.lock().unwrap()[0].clone();
    assert_eq!(arguments[..3], ["app-server", "--listen", "stdio://"]);
    assert_eq!(workdir, PathBuf::from("/work/acme"));
    state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await;

    supervisor.command(run, prompt()).await.unwrap();
    assert!(matches!(
        supervisor.command(run, prompt()).await,
        Err(CommandError::Refused(RunRefused::Busy))
    ));
    let waiting = state(
        &mut updates,
        |s| matches!(s, RunState::Working { approvals, .. } if !approvals.is_empty()),
    )
    .await;
    let RunState::Working { approvals, .. } = waiting else {
        unreachable!()
    };
    let approval: ApprovalId = approvals.into_iter().next().unwrap();
    assert_eq!(approval.to_string(), "ap-1");
    supervisor
        .command(
            run,
            RunCommand::Answer {
                approval,
                decision: Decision::Accept,
            },
        )
        .await
        .unwrap();
    state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await;

    supervisor.command(run, RunCommand::Stop).await.unwrap();
    let ended = state(&mut updates, RunState::is_ended).await;
    assert_eq!(
        ended,
        RunState::Ended {
            end: RunEnd::Stopped,
            exit: Some(ProcessExit::Signal(9))
        }
    );
    assert!(matches!(
        supervisor.command(run, prompt()).await,
        Err(CommandError::Refused(RunRefused::Ended))
    ));
    let runs = supervisor.runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, ended);
}

#[tokio::test]
async fn refuses_directories_outside_the_roots_and_undrivable_clients() {
    let (supervisor, _task) = supervise(FakeHost::new(Script::Codex));
    let refused = [
        ("relative/path", "Workdir(Relative)"),
        ("/work/../etc", "Workdir(Unnormalized)"),
        ("/elsewhere", "Workdir(OutsideRoots)"),
        ("/work/escape", "Workdir(OutsideRoots)"),
        ("/missing", "Resolve"),
    ];
    for (dir, expected) in refused {
        let error = supervisor.start(Harness::Codex, dir).await.unwrap_err();
        assert!(
            format!("{error:?}").starts_with(expected),
            "{dir}: {error:?}"
        );
    }
    assert!(matches!(
        supervisor.start(Harness::ClaudeCode, "/work/acme").await,
        Err(StartError::NotDrivable(_))
    ));
    assert!(supervisor.runs().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_spawn_is_reported_and_kept() {
    let mut host = FakeHost::new(Script::Codex);
    host.spawn_fails = true;
    let (supervisor, _task) = supervise(host);
    let Err(StartError::Spawn { run, .. }) = supervisor.start(Harness::Codex, "/work/acme").await
    else {
        panic!("expected a spawn failure");
    };
    let runs = supervisor.runs().await.unwrap();
    assert_eq!(runs[0].run, run);
    assert_eq!(
        runs[0].state,
        RunState::Ended {
            end: RunEnd::SpawnFailed,
            exit: None
        }
    );
}

#[tokio::test]
async fn a_refused_opening_stops_the_run() {
    let (supervisor, _task) = supervise(FakeHost::new(Script::RefusesOpening));
    let mut updates = supervisor.subscribe();
    supervisor
        .start(Harness::Codex, "/work/acme")
        .await
        .unwrap();
    assert_eq!(
        state(&mut updates, RunState::is_ended).await,
        RunState::Ended {
            end: RunEnd::OpenRefused,
            exit: Some(ProcessExit::Signal(9))
        }
    );
}

#[tokio::test]
async fn an_agent_that_exits_on_its_own_ends_the_run() {
    let (supervisor, _task) = supervise(FakeHost::new(Script::ExitsEarly));
    let mut updates = supervisor.subscribe();
    supervisor
        .start(Harness::Codex, "/work/acme")
        .await
        .unwrap();
    assert_eq!(
        state(&mut updates, RunState::is_ended).await,
        RunState::Ended {
            end: RunEnd::Exited,
            exit: Some(ProcessExit::Code(3))
        }
    );
}

#[tokio::test]
async fn dropping_every_handle_drains_the_runs_and_ends_the_task() {
    let (supervisor, task) = supervise(FakeHost::new(Script::Codex));
    let mut updates = supervisor.subscribe();
    supervisor.start(Harness::Codex, "/work/a").await.unwrap();
    supervisor.start(Harness::Codex, "/work/b").await.unwrap();
    state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await;
    drop(supervisor);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the task ends")
        .unwrap();
    let mut drained = 0;
    while let Ok(update) = updates.try_recv() {
        if let RunUpdate::State {
            state: RunState::Ended { end, .. },
            ..
        } = update
        {
            assert_eq!(end, RunEnd::Drained);
            drained += 1;
        }
    }
    assert_eq!(drained, 2);
}
