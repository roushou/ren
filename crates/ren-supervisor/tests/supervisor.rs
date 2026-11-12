//! The supervisor with an in-memory host: each "process" is a scripted agent
//! that speaks Codex's app-server protocol, or Claude Code's stream-json, over
//! in-memory pipes. No real client, network, or credentials.
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
use ren_protocol::label::Stored;
use ren_recorder::{
    CaptureOptions, Recorder, RecorderSetup,
    capture::RunEnd,
    id::SequentialIds,
    retention::RetentionPolicy,
    secrets::SecretScanner,
    time::{ManualClock, UnixMillis},
};
use ren_store::{Reader, Store, query::PageLimit, view::RunView};
use ren_supervisor::{
    AgentHost, AgentProcess, CommandError, Conversation, KillSignal, KillSwitch, Look, ProcessExit,
    RunCommand, RunRefused, RunState, RunUpdate, StartError, Supervisor, SupervisorOptions,
    Workdir,
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
    /// A well-behaved Claude Code in stream-json print mode: echoes each
    /// prompt, asks to run one command, and ends the turn once answered.
    Claude,
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
    fn finds(&self, program: &str) -> bool {
        // Codex is installed here; Claude Code is not.
        program == "codex"
    }

    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        match path.to_str() {
            // A link that leads out of the roots.
            Some("/work/escape") => Ok(PathBuf::from("/etc")),
            Some(path) if path.starts_with("/work") || path == "/elsewhere" => Ok(path.into()),
            Some(_) | None => Err(io::Error::from(io::ErrorKind::NotFound)),
        }
    }

    fn look(&self, dir: &Path) -> io::Result<Look> {
        Ok(match dir.to_str() {
            Some("/work") => Look {
                folders: vec!["acme".into(), "notes".into()],
                git: None,
            },
            Some("/work/acme") => Look {
                folders: vec!["src".into()],
                git: Some(Some("ref: refs/heads/main".into())),
            },
            Some(_) | None => Look::default(),
        })
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
        match self.script {
            Script::Claude => tokio::spawn(claude(agent_in, agent_out, signal, exit_tx)),
            Script::Codex | Script::RefusesOpening | Script::ExitsEarly => {
                tokio::spawn(agent(self.script, agent_in, agent_out, signal, exit_tx))
            }
        };
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
                (Some("thread/resume"), _) => {
                    let thread = message["params"]["threadId"].clone();
                    send(
                        &mut out,
                        json!({ "id": id, "result": { "thread": { "id": thread } } }),
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

/// A supervisor recording into a real store, with its recorder running.
/// A scripted Claude Code in stream-json print mode.
async fn claude(
    input: DuplexStream,
    mut out: DuplexStream,
    kill: KillSignal,
    exit: oneshot::Sender<ProcessExit>,
) {
    let mut lines = BufReader::new(input).lines();
    let serve = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let message: Value = serde_json::from_str(&line).unwrap();
            match message["type"].as_str() {
                Some("control_request") => {
                    let id = message["request_id"].clone();
                    send(
                        &mut out,
                        json!({ "type": "control_response", "response": {
                            "subtype": "success", "request_id": id, "response": {} } }),
                    )
                    .await;
                }
                Some("user") => {
                    let mut echo = message.clone();
                    echo["uuid"] = TURN.into();
                    echo["isReplay"] = true.into();
                    send(&mut out, echo).await;
                    send(
                        &mut out,
                        json!({ "type": "control_request", "request_id": "perm-1", "request": {
                            "subtype": "can_use_tool", "tool_name": "Bash",
                            "input": { "command": "cargo test" }, "description": "Run tests" } }),
                    )
                    .await;
                }
                Some("control_response") => {
                    assert_eq!(message["response"]["request_id"], "perm-1");
                    assert_eq!(message["response"]["response"]["behavior"], "allow");
                    send(
                        &mut out,
                        json!({ "type": "result", "subtype": "success", "is_error": false,
                                "result": "Tests pass." }),
                    )
                    .await;
                }
                Some(_) | None => {}
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

struct Supervised {
    supervisor: Supervisor,
    task: tokio::task::JoinHandle<()>,
    recorder: tokio::task::JoinHandle<()>,
    dir: tempfile::TempDir,
}

async fn supervise(host: FakeHost) -> Supervised {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::at(UnixMillis::from_millis(1_000)));
    let ids = Arc::new(SequentialIds::default());
    let (store, recovery) = Store::open(dir.path(), clock.as_ref(), ids.clone())
        .await
        .unwrap();
    let setup = RecorderSetup {
        options: CaptureOptions::default(),
        retention: RetentionPolicy::default(),
        max_calls: 16,
        live_buffer: 64,
        scanner: Arc::new(SecretScanner::new().unwrap()),
    };
    let (recorder, recording) =
        Recorder::new(store, recovery.calls, setup, clock, ids.clone()).unwrap();
    let (supervisor, task) = Supervisor::new(options(), host, ids, recorder).unwrap();
    Supervised {
        supervisor,
        task: tokio::spawn(task.run()),
        recorder: tokio::spawn(async move { recording.run().await.unwrap() }),
        dir,
    }
}

impl Supervised {
    /// Drop the supervisor, let its runs drain and the recorder commit, and
    /// read what was stored.
    async fn stored(self) -> (Vec<RunView>, tempfile::TempDir) {
        drop(self.supervisor);
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .expect("the supervisor drains")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.recorder)
            .await
            .expect("the recorder drains")
            .unwrap();
        let reader = Reader::open(self.dir.path(), 1).await.unwrap();
        let runs = reader.runs(PageLimit::DEFAULT).await.unwrap();
        reader.close().await;
        (runs, self.dir)
    }
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
    let supervised = supervise(host).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();

    let run = supervisor
        .start(Harness::Codex, "/work/acme", Conversation::New, None)
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
    let waiting = approvals.first().unwrap();
    assert_eq!(
        waiting.ask,
        ren_harness::drive::ApprovalAsk::FileChange {
            reason: Some("edit src".into())
        }
    );
    let approval: ApprovalId = waiting.approval.clone();
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

    let (stored, _dir) = supervised.stored().await;
    let [stored] = stored.as_slice() else {
        panic!("one run is stored: {stored:?}");
    };
    assert_eq!(stored.id, run);
    assert_eq!(stored.harness, Stored::Known(Harness::Codex));
    assert_eq!(stored.workdir, "/work/acme");
    assert_eq!(stored.thread.as_deref(), Some(THREAD));
    assert_eq!(stored.end, Some(Stored::Known(RunEnd::Stopped)));
    assert_eq!(stored.exit, Some(ProcessExit::Signal(9)));
}

#[tokio::test]
async fn refuses_directories_outside_the_roots_and_undrivable_clients() {
    let supervised = supervise(FakeHost::new(Script::Codex)).await;
    let supervisor = &supervised.supervisor;
    let refused = [
        ("relative/path", "Workdir(Relative)"),
        ("/work/../etc", "Workdir(Unnormalized)"),
        ("/elsewhere", "Workdir(OutsideRoots)"),
        ("/work/escape", "Workdir(OutsideRoots)"),
        ("/missing", "Resolve"),
    ];
    for (dir, expected) in refused {
        let error = supervisor
            .start(Harness::Codex, dir, Conversation::New, None)
            .await
            .unwrap_err();
        assert!(
            format!("{error:?}").starts_with(expected),
            "{dir}: {error:?}"
        );
    }
    assert!(matches!(
        supervisor
            .start(Harness::Pi, "/work/acme", Conversation::New, None)
            .await,
        Err(StartError::NotDrivable(_))
    ));
    assert!(supervisor.runs().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_spawn_is_reported_and_kept() {
    let mut host = FakeHost::new(Script::Codex);
    host.spawn_fails = true;
    let supervised = supervise(host).await;
    let supervisor = &supervised.supervisor;
    let Err(StartError::Spawn { run, .. }) = supervisor
        .start(Harness::Codex, "/work/acme", Conversation::New, None)
        .await
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
    let (stored, _dir) = supervised.stored().await;
    assert_eq!(stored[0].end, Some(Stored::Known(RunEnd::SpawnFailed)));
    assert_eq!(stored[0].exit, None);
}

#[tokio::test]
async fn a_refused_opening_stops_the_run() {
    let supervised = supervise(FakeHost::new(Script::RefusesOpening)).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();
    supervisor
        .start(Harness::Codex, "/work/acme", Conversation::New, None)
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
    let supervised = supervise(FakeHost::new(Script::ExitsEarly)).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();
    supervisor
        .start(Harness::Codex, "/work/acme", Conversation::New, None)
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
    let supervised = supervise(FakeHost::new(Script::Codex)).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();
    supervisor
        .start(Harness::Codex, "/work/a", Conversation::New, None)
        .await
        .unwrap();
    supervisor
        .start(Harness::Codex, "/work/b", Conversation::New, None)
        .await
        .unwrap();
    state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await;
    let (stored, _dir) = supervised.stored().await;
    assert_eq!(stored.len(), 2);
    for run in &stored {
        assert_eq!(run.end, Some(Stored::Known(RunEnd::Drained)));
    }
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

#[tokio::test]
async fn a_run_can_resume_the_clients_thread() {
    let supervised = supervise(FakeHost::new(Script::Codex)).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();
    let earlier = "0190f3a0-0000-7000-8000-0000000000ee";
    let thread =
        ren_protocol::native::NativeThreadId::new(uuid::Uuid::parse_str(earlier).unwrap()).unwrap();
    supervisor
        .start(
            Harness::Codex,
            "/work/acme",
            Conversation::Resume(thread),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await,
        RunState::Idle { thread }
    );
    let (stored, _dir) = supervised.stored().await;
    assert_eq!(stored[0].thread.as_deref(), Some(earlier));
}

#[tokio::test]
async fn drives_claude_code_through_a_turn_in_the_conversation_ren_names() {
    let host = FakeHost::new(Script::Claude);
    let spawned = host.spawned.clone();
    let supervised = supervise(host).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();

    let run = supervisor
        .start(Harness::ClaudeCode, "/work/acme", Conversation::New, None)
        .await
        .unwrap();
    let (arguments, _) = spawned.lock().unwrap()[0].clone();
    assert_eq!(arguments[0], "-p");
    let named = arguments
        .iter()
        .position(|argument| argument == "--session-id")
        .map(|at| arguments[at + 1].clone())
        .expect("Ren names the conversation");
    let RunState::Idle { thread } =
        state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(thread.to_string(), named);

    supervisor.command(run, prompt()).await.unwrap();
    let waiting = state(
        &mut updates,
        |s| matches!(s, RunState::Working { turn: Some(_), approvals, .. } if !approvals.is_empty()),
    )
    .await;
    let RunState::Working { approvals, .. } = waiting else {
        unreachable!()
    };
    let waiting = approvals.first().unwrap();
    assert_eq!(
        waiting.ask,
        ren_harness::drive::ApprovalAsk::Command {
            command: Some("cargo test".into()),
            cwd: None,
            reason: Some("Run tests".into()),
        }
    );
    supervisor
        .command(
            run,
            RunCommand::Answer {
                approval: waiting.approval.clone(),
                decision: Decision::Accept,
            },
        )
        .await
        .unwrap();
    state(&mut updates, |s| matches!(s, RunState::Idle { .. })).await;
    supervisor.command(run, RunCommand::Stop).await.unwrap();
    state(&mut updates, RunState::is_ended).await;

    let (stored, _dir) = supervised.stored().await;
    let [stored] = stored.as_slice() else {
        panic!("one run is stored: {stored:?}");
    };
    assert_eq!(stored.harness, Stored::Known(Harness::ClaudeCode));
    assert_eq!(stored.thread.as_deref(), Some(named.as_str()));
}

#[tokio::test]
async fn a_first_prompt_goes_out_once_the_conversation_opens() {
    let supervised = supervise(FakeHost::new(Script::Claude)).await;
    let supervisor = &supervised.supervisor;
    let mut updates = supervisor.subscribe();
    supervisor
        .start(
            Harness::ClaudeCode,
            "/work/acme",
            Conversation::New,
            Some(prompt_text("run the tests")),
        )
        .await
        .unwrap();
    // No prompt command: the run asks its first approval on its own.
    state(
        &mut updates,
        |s| matches!(s, RunState::Working { approvals, .. } if !approvals.is_empty()),
    )
    .await;
}

#[tokio::test]
async fn projects_are_the_repositories_and_plain_folders_under_the_roots() {
    let supervised = supervise(FakeHost::new(Script::Codex)).await;
    let projects = supervised.supervisor.projects().await.unwrap();
    let listed: Vec<(&str, &str)> = projects
        .iter()
        .map(|project| (project.name.as_str(), project.path.as_str()))
        .collect();
    assert_eq!(listed, [("acme", "/work/acme"), ("notes", "/work/notes")]);
}

#[tokio::test]
async fn clients_say_whether_a_run_can_start_with_them_here() {
    use ren_supervisor::{Client, ClientStatus};
    let supervised = supervise(FakeHost::new(Script::Codex)).await;
    let clients = supervised.supervisor.clients().await.unwrap();
    let status = |harness: Harness| {
        clients
            .iter()
            .find(|client| client.harness == harness)
            .map(|client: &Client| client.status)
    };
    assert_eq!(status(Harness::Codex), Some(ClientStatus::Ready));
    assert_eq!(
        status(Harness::ClaudeCode),
        Some(ClientStatus::NotInstalled)
    );
    assert_eq!(status(Harness::Pi), Some(ClientStatus::NotDrivable));
    assert_eq!(status(Harness::OpenCode), Some(ClientStatus::NotDrivable));
}

fn prompt_text(text: &str) -> Prompt {
    Prompt::parse(text.into()).unwrap()
}
