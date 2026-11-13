//! What the installed `ren` reports about its daemon, read only as far as an
//! upgrade needs: `ren control state` (running calls and admission),
//! `ren runs ls` (the agent runs it drives), and `ren doctor --json` (its
//! checks).
use ren_harness::Harness;
use ren_protocol::label::Stored;
use ren_recorder::id::RunId;
use serde::Deserialize;
use serde_json::Value;
use std::fmt;

/// The daemon's operator state and running calls.
#[derive(Deserialize)]
pub struct ControlState {
    state: OperatorState,
    running: Vec<Value>,
}

#[derive(Deserialize)]
struct OperatorState {
    pause: PauseState,
}

#[derive(Deserialize)]
struct PauseState {
    state: Admission,
}

/// Whether the daemon admits new calls. Text this task does not know is kept,
/// and treated as an operator's choice to leave alone.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum Admission {
    Running,
    Paused,
    Other(String),
}

impl From<String> for Admission {
    fn from(text: String) -> Self {
        match text.as_str() {
            "running" => Self::Running,
            "paused" => Self::Paused,
            _ => Self::Other(text),
        }
    }
}

/// What an upgrade does next, from the daemon's state.
#[derive(Debug, PartialEq, Eq)]
pub enum Quiesce {
    /// Nothing runs and calls are admitted: pause admission.
    Pause,
    /// Nothing runs, and an operator already paused it (or set a state this
    /// task does not know): proceed and leave admission as it is.
    LeaveAlone,
    /// Calls or runs a restart would cut off: wait for them.
    Wait(Busy),
}

/// Whether the pause just made left the daemon quiet.
#[derive(Debug, PartialEq, Eq)]
pub enum AfterPause {
    Quiet,
    /// A call started, or a run was prompted, between the check and the
    /// pause: resume and wait.
    SlippedIn(Busy),
}

/// What a restart would cut off now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    calls: usize,
    runs: Vec<LiveRun>,
}

/// Whether runs that are mid-way hold up a restart (`--drain-runs` lets it
/// go ahead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPolicy {
    /// Wait until no run is starting, working, or stopping.
    Wait,
    /// Restart anyway: working runs lose their turn and end with the daemon.
    Drain,
}

impl ControlState {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The next step, from this state and the daemon's `runs`.
    pub fn quiesce(&self, runs: &Runs, policy: RunPolicy) -> Quiesce {
        let busy = self.busy(runs, policy);
        match (self.admission(), busy.is_quiet()) {
            (Admission::Running, true) => Quiesce::Pause,
            (Admission::Paused | Admission::Other(_), true) => Quiesce::LeaveAlone,
            (Admission::Running | Admission::Paused | Admission::Other(_), false) => {
                Quiesce::Wait(busy)
            }
        }
    }

    pub fn after_pause(&self, runs: &Runs, policy: RunPolicy) -> AfterPause {
        let busy = self.busy(runs, policy);
        if busy.is_quiet() {
            AfterPause::Quiet
        } else {
            AfterPause::SlippedIn(busy)
        }
    }

    fn busy(&self, runs: &Runs, policy: RunPolicy) -> Busy {
        Busy {
            calls: self.running(),
            runs: runs.holding(policy),
        }
    }

    /// How many calls (HTTP attempts or WebSocket channels) are in flight.
    pub fn running(&self) -> usize {
        self.running.len()
    }

    pub fn admission(&self) -> &Admission {
        &self.state.pause.state
    }
}

impl Busy {
    /// Only runs: the dev slot restarts its daemon under constant fake
    /// traffic, so it waits for runs and not for calls.
    pub fn of_runs(runs: &Runs, policy: RunPolicy) -> Self {
        Self {
            calls: 0,
            runs: runs.holding(policy),
        }
    }

    pub fn is_quiet(&self) -> bool {
        self.calls == 0 && self.runs.is_empty()
    }

    /// Whether runs, and not only calls, hold up the restart.
    pub fn has_runs(&self) -> bool {
        !self.runs.is_empty()
    }

    /// One phrase: `2 running calls and 1 run`.
    pub fn summary(&self) -> String {
        let count = |n: usize, one: &str, many: &str| match n {
            1 => format!("1 {one}"),
            n => format!("{n} {many}"),
        };
        match (self.calls, self.runs.len()) {
            (0, runs) => count(runs, "run", "runs"),
            (calls, 0) => count(calls, "running call", "running calls"),
            (calls, runs) => format!(
                "{} and {}",
                count(calls, "running call", "running calls"),
                count(runs, "run", "runs")
            ),
        }
    }

    /// What to do when the wait ran out.
    pub fn advice(&self) -> &'static str {
        if self.has_runs() {
            "Run again once the runs are idle, raise --idle-timeout, or pass --drain-runs to restart anyway (their turns are lost; each conversation can be continued)"
        } else {
            "Run again between sessions, or raise --idle-timeout"
        }
    }

    /// The runs waited for, one line each.
    pub fn run_lines(&self) -> Vec<String> {
        self.runs.iter().map(ToString::to_string).collect()
    }
}

/// `ren runs ls`: the runs the daemon drives now. A daemon with runs off
/// drives none.
#[derive(Debug, Default)]
pub struct Runs(Vec<LiveRun>);

/// What `ren runs ls` printed: the runs, or the control API's refusal.
#[derive(Deserialize)]
#[serde(untagged)]
enum RunsAnswer {
    Listed { runs: Vec<LiveRun> },
    Refused { error: String },
}

/// Why `ren runs ls` gave no list.
#[derive(Debug, thiserror::Error)]
pub enum RunsError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the control API refused: {0}")]
    Refused(String),
}

/// One run, as far as a restart concerns it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LiveRun {
    run: RunId,
    /// The session it serves; an older daemon names none.
    #[serde(default)]
    session: Option<String>,
    harness: Stored<Harness>,
    workdir: String,
    state: LiveState,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LiveState {
    phase: Phase,
    /// The client's conversation, once it is open.
    #[serde(default)]
    thread: Option<String>,
    /// Approvals waiting for the operator, while working.
    #[serde(default)]
    approvals: Vec<Value>,
}

/// Where a run is in its life (`state.phase`); unknown text is kept.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum Phase {
    Starting,
    Idle,
    Working,
    Stopping,
    Ended,
    Other(String),
}

impl From<String> for Phase {
    fn from(text: String) -> Self {
        match text.as_str() {
            "starting" => Self::Starting,
            "idle" => Self::Idle,
            "working" => Self::Working,
            "stopping" => Self::Stopping,
            "ended" => Self::Ended,
            _ => Self::Other(text),
        }
    }
}

/// What restarting the daemon does to a run, by its phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restart {
    /// Its process is killed mid-way: a turn (or the conversation's opening)
    /// is lost, or an operator's stop is not recorded as one.
    CutsOff,
    /// It waits for a prompt: its process ends with the daemon, and its
    /// conversation can be continued.
    Ends,
    /// It has already ended.
    Spares,
}

impl Phase {
    /// A phase this task does not know is treated as mid-way, never as idle.
    fn restart(&self) -> Restart {
        match self {
            Self::Starting | Self::Working | Self::Stopping | Self::Other(_) => Restart::CutsOff,
            Self::Idle => Restart::Ends,
            Self::Ended => Restart::Spares,
        }
    }
}

impl Runs {
    pub fn parse(json: &str) -> Result<Self, RunsError> {
        match serde_json::from_str(json)? {
            RunsAnswer::Listed { runs } => Ok(Self(runs)),
            RunsAnswer::Refused { error } => match error.as_str() {
                "runs_disabled" => Ok(Self::default()),
                _ => Err(RunsError::Refused(error)),
            },
        }
    }

    /// The runs that hold up a restart under `policy`.
    fn holding(&self, policy: RunPolicy) -> Vec<LiveRun> {
        match policy {
            RunPolicy::Drain => Vec::new(),
            RunPolicy::Wait => self
                .0
                .iter()
                .filter(|run| run.state.phase.restart() == Restart::CutsOff)
                .cloned()
                .collect(),
        }
    }

    /// The runs whose process ends with the daemon: every one not ended.
    pub fn ending(&self) -> Vec<LiveRun> {
        self.0
            .iter()
            .filter(|run| run.state.phase.restart() != Restart::Spares)
            .cloned()
            .collect()
    }
}

impl LiveRun {
    /// How to continue its conversation after the restart: by writing to
    /// its session, which starts the client again and resumes it; from an
    /// older daemon, which names no session, by resuming its thread.
    pub fn continuation(&self) -> String {
        let client = match self.harness.known() {
            Some(Harness::Codex) => Some("codex"),
            Some(Harness::ClaudeCode) => Some("claude"),
            Some(Harness::Pi) => Some("pi"),
            Some(Harness::OpenCode) | None => None,
        };
        match (&self.session, client, &self.state.thread) {
            (Some(session), _, _) => format!(
                "write to its session to go on: from the Sessions page or `ren runs say {session} <prompt>`"
            ),
            (None, Some(client), Some(thread)) => format!(
                "continue it from the Sessions page or `ren runs start {client} {} --resume {thread}`",
                self.workdir
            ),
            (None, None, Some(_)) => "continue it from the Sessions page".to_owned(),
            (None, Some(_) | None, None) => "its conversation had not opened".to_owned(),
        }
    }
}

impl fmt::Display for LiveRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} run {} in {}: ", self.harness, self.run, self.workdir)?;
        match (&self.state.phase, self.state.approvals.len()) {
            (Phase::Working, 0) => f.write_str("working"),
            (Phase::Working, 1) => f.write_str("working, waiting for an approval"),
            (Phase::Working, n) => write!(f, "working, waiting for {n} approvals"),
            (Phase::Starting, _) => f.write_str("starting"),
            (Phase::Idle, _) => f.write_str("idle"),
            (Phase::Stopping, _) => f.write_str("stopping"),
            (Phase::Ended, _) => f.write_str("ended"),
            (Phase::Other(text), _) => f.write_str(text),
        }
    }
}

/// `ren doctor --json`.
#[derive(Deserialize)]
pub struct DoctorReport {
    checks: Vec<DoctorCheck>,
}

#[derive(Deserialize)]
pub struct DoctorCheck {
    name: String,
    status: CheckStatus,
    message: String,
}

/// A check's outcome; unknown text is kept.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum CheckStatus {
    Ok,
    Warning,
    Failed,
    Skipped,
    Other(String),
}

impl From<String> for CheckStatus {
    fn from(text: String) -> Self {
        match text.as_str() {
            "ok" => Self::Ok,
            "warning" => Self::Warning,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => Self::Other(text),
        }
    }
}

impl DoctorReport {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The check comparing the daemon's build with the CLI's.
    pub fn build(&self) -> Option<&DoctorCheck> {
        self.checks.iter().find(|check| check.name == "build")
    }
}

impl DoctorCheck {
    pub fn status(&self) -> &CheckStatus {
        &self.status
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_running_calls_and_admission() {
        let idle = ControlState::parse(
            r#"{"api_version":1,"state":{"revision":0,"pause":{"state":"running"},"blocked_sessions":{}},"running":[]}"#,
        )
        .unwrap();
        assert_eq!(idle.running(), 0);
        assert_eq!(idle.admission(), &Admission::Running);

        let busy = ControlState::parse(
            r#"{"api_version":1,"state":{"revision":3,"pause":{"state":"paused","retry_after":30},"blocked_sessions":{}},"running":[{"call":"0193e0f1-0000-7000-8000-000000000001","transport":"http"},{"call":"0193e0f1-0000-7000-8000-000000000002","transport":"websocket"}]}"#,
        )
        .unwrap();
        assert_eq!(busy.running(), 2);
        assert_eq!(busy.admission(), &Admission::Paused);
    }

    #[test]
    fn keeps_an_unknown_admission() {
        let state = ControlState::parse(r#"{"state":{"pause":{"state":"draining"}},"running":[]}"#)
            .unwrap();
        assert_eq!(state.admission(), &Admission::Other("draining".to_owned()));
    }

    #[test]
    fn finds_the_build_check() {
        let report = DoctorReport::parse(
            r#"{"ok":true,"checks":[{"name":"gateway","status":"ok","message":"reachable"},{"name":"build","status":"warning","message":"daemon runs 0.1.0 (a) but this CLI is 0.1.0 (b)"}]}"#,
        )
        .unwrap();
        let build = report.build().unwrap();
        assert_eq!(build.status(), &CheckStatus::Warning);
        assert!(build.message().starts_with("daemon runs"));
        assert!(
            DoctorReport::parse(r#"{"ok":true,"checks":[]}"#)
                .unwrap()
                .build()
                .is_none()
        );
    }

    fn state(admission: &str, running: usize) -> ControlState {
        let calls: Vec<_> = (0..running)
            .map(|n| serde_json::json!({ "call": format!("0193e0f1-0000-7000-8000-{n:012}"), "transport": "http" }))
            .collect();
        ControlState::parse(
            &serde_json::json!({ "state": { "pause": { "state": admission } }, "running": calls })
                .to_string(),
        )
        .unwrap()
    }

    /// A run in `phase` as `ren runs ls` lists it, numbered `n`.
    fn run(n: u32, harness: &str, phase: &str) -> serde_json::Value {
        let mut state = serde_json::json!({ "phase": phase });
        match phase {
            "idle" | "working" => {
                state["thread"] = format!("0193e0f1-0000-7000-9000-{n:012}").into()
            }
            _ => {}
        }
        if phase == "working" {
            state["turn"] = serde_json::Value::Null;
            state["approvals"] = serde_json::json!([]);
        }
        if phase == "ended" {
            state["end"] = "exited".into();
        }
        serde_json::json!({
            "run": format!("0193e0f1-0000-7000-8000-{n:012}"),
            "harness": harness,
            "workdir": format!("/home/op/acme-{n}"),
            "state": state,
            "settings": {},
        })
    }

    fn runs(list: &[serde_json::Value]) -> Runs {
        Runs::parse(&serde_json::json!({ "api_version": 1, "runs": list }).to_string()).unwrap()
    }

    #[test]
    fn pauses_only_an_idle_daemon_that_admits_calls() {
        let none = Runs::default();
        assert_eq!(
            state("running", 0).quiesce(&none, RunPolicy::Wait),
            Quiesce::Pause
        );
        assert_eq!(
            state("paused", 0).quiesce(&none, RunPolicy::Wait),
            Quiesce::LeaveAlone
        );
        assert_eq!(
            state("draining", 0).quiesce(&none, RunPolicy::Wait),
            Quiesce::LeaveAlone
        );
        for admission in ["running", "paused", "draining"] {
            let Quiesce::Wait(busy) = state(admission, 2).quiesce(&none, RunPolicy::Wait) else {
                panic!("{admission} with calls running must wait");
            };
            assert_eq!(busy.summary(), "2 running calls");
            assert!(!busy.has_runs());
        }
    }

    #[test]
    fn a_call_that_slipped_in_before_the_pause_is_waited_for() {
        let none = Runs::default();
        assert_eq!(
            state("paused", 0).after_pause(&none, RunPolicy::Wait),
            AfterPause::Quiet
        );
        let AfterPause::SlippedIn(busy) = state("paused", 1).after_pause(&none, RunPolicy::Wait)
        else {
            panic!("a running call is not quiet");
        };
        assert_eq!(busy.summary(), "1 running call");
    }

    #[test]
    fn waits_for_runs_that_a_restart_would_cut_off() {
        for phase in ["starting", "working", "stopping", "resuming"] {
            let listed = runs(&[run(1, "codex", phase), run(2, "pi", "idle")]);
            let Quiesce::Wait(busy) = state("running", 0).quiesce(&listed, RunPolicy::Wait) else {
                panic!("a {phase} run must be waited for");
            };
            assert!(busy.has_runs());
            assert_eq!(busy.summary(), "1 run");
            assert_eq!(
                busy.run_lines(),
                vec![format!(
                    "codex run 0193e0f1-0000-7000-8000-000000000001 in /home/op/acme-1: {phase}"
                )]
            );
            // The same daemon a moment later, paused: a run prompted between
            // the check and the pause is waited for too.
            assert!(matches!(
                state("paused", 0).after_pause(&listed, RunPolicy::Wait),
                AfterPause::SlippedIn(_)
            ));
        }
        let mixed = runs(&[run(1, "claude_code", "working"), run(2, "codex", "working")]);
        let Quiesce::Wait(busy) = state("running", 3).quiesce(&mixed, RunPolicy::Wait) else {
            panic!("busy");
        };
        assert_eq!(busy.summary(), "3 running calls and 2 runs");
    }

    #[test]
    fn idle_and_ended_runs_do_not_hold_up_a_restart() {
        let listed = runs(&[run(1, "codex", "idle"), run(2, "claude_code", "ended")]);
        assert_eq!(
            state("running", 0).quiesce(&listed, RunPolicy::Wait),
            Quiesce::Pause
        );
        assert_eq!(
            state("paused", 0).after_pause(&listed, RunPolicy::Wait),
            AfterPause::Quiet
        );
    }

    #[test]
    fn draining_restarts_with_working_runs_but_still_waits_for_calls() {
        let listed = runs(&[run(1, "codex", "working")]);
        assert_eq!(
            state("running", 0).quiesce(&listed, RunPolicy::Drain),
            Quiesce::Pause
        );
        assert!(matches!(
            state("running", 1).quiesce(&listed, RunPolicy::Drain),
            Quiesce::Wait(_)
        ));
        assert!(Busy::of_runs(&listed, RunPolicy::Drain).is_quiet());
        let busy = Busy::of_runs(&listed, RunPolicy::Wait);
        assert!(!busy.is_quiet());
        assert_eq!(busy.summary(), "1 run");
    }

    #[test]
    fn every_run_not_ended_ends_with_the_daemon_and_says_how_to_continue() {
        let listed = runs(&[
            run(1, "codex", "idle"),
            run(2, "claude_code", "idle"),
            run(3, "opencode", "idle"),
            run(4, "codex", "starting"),
            run(5, "codex", "ended"),
            // A daemon with sessions names each run's.
            {
                let mut served = run(6, "pi", "idle");
                served["session"] = "0193e0f1-0000-7000-a000-000000000006".into();
                served
            },
        ]);
        let ending: Vec<_> = listed
            .ending()
            .iter()
            .map(|run| format!("{run}; {}", run.continuation()))
            .collect();
        assert_eq!(
            ending,
            [
                "codex run 0193e0f1-0000-7000-8000-000000000001 in /home/op/acme-1: idle; continue it from the Sessions page or `ren runs start codex /home/op/acme-1 --resume 0193e0f1-0000-7000-9000-000000000001`",
                "claude_code run 0193e0f1-0000-7000-8000-000000000002 in /home/op/acme-2: idle; continue it from the Sessions page or `ren runs start claude /home/op/acme-2 --resume 0193e0f1-0000-7000-9000-000000000002`",
                "opencode run 0193e0f1-0000-7000-8000-000000000003 in /home/op/acme-3: idle; continue it from the Sessions page",
                "codex run 0193e0f1-0000-7000-8000-000000000004 in /home/op/acme-4: starting; its conversation had not opened",
                "pi run 0193e0f1-0000-7000-8000-000000000006 in /home/op/acme-6: idle; write to its session to go on: from the Sessions page or `ren runs say 0193e0f1-0000-7000-a000-000000000006 <prompt>`",
            ]
        );
    }

    #[test]
    fn reads_runs_and_the_control_apis_refusals() {
        let mut working = run(1, "codex", "working");
        working["state"]["approvals"] = serde_json::json!([
            { "approval": "a1", "ask": { "kind": "command" } },
            { "approval": "a2", "ask": { "kind": "file_change" } },
        ]);
        let listed = runs(&[working, run(2, "opencode_next", "idle")]);
        let Quiesce::Wait(busy) = state("running", 0).quiesce(&listed, RunPolicy::Wait) else {
            panic!("busy");
        };
        assert!(busy.run_lines()[0].ends_with("working, waiting for 2 approvals"));
        // A client this task does not know is kept by its label.
        assert!(
            listed.ending()[1]
                .to_string()
                .starts_with("opencode_next run")
        );
        // A daemon with runs off drives none.
        assert!(
            Runs::parse(r#"{"api_version":1,"error":"runs_disabled"}"#)
                .unwrap()
                .ending()
                .is_empty()
        );
        assert!(matches!(
            Runs::parse(r#"{"api_version":1,"error":"unauthorized"}"#),
            Err(RunsError::Refused(error)) if error == "unauthorized"
        ));
        assert!(matches!(Runs::parse("not json"), Err(RunsError::Json(_))));
    }
}
