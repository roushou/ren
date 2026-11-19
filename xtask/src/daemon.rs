//! The daemon as a restart sees it, through the installed `ren`: what it
//! reports, read only as far as an upgrade needs (`ren control state`,
//! running calls and admission; `ren runs ls`, the agent runs it drives;
//! `ren doctor --json`, its checks), and what a restart tells the operator
//! while it waits and about the runs that end with it.
use crate::{
    invocation::{Invocation, InvocationError},
    progress::Progress,
};
use ren_gateway::ControlRefusal;
use ren_gateway::operator::PauseState;
use ren_harness::Harness;
use ren_protocol::label::Stored;
use ren_recorder::id::RunId;
use ren_supervisor::{RunPhase, RunTenure};
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
    /// The operator state's revision: each pause or resume makes a new one.
    #[serde(default)]
    revision: Option<u64>,
    pause: PauseRecord,
}

#[derive(Deserialize)]
struct PauseRecord {
    /// Whether the daemon admits new calls. Text this task does not know is
    /// kept, and treated as an operator's choice to leave alone.
    state: Stored<PauseState>,
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
        match (self.admission().known(), busy.is_quiet()) {
            (Some(PauseState::Running), true) => Quiesce::Pause,
            (Some(PauseState::Paused) | None, true) => Quiesce::LeaveAlone,
            (Some(PauseState::Running | PauseState::Paused) | None, false) => Quiesce::Wait(busy),
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

    pub fn admission(&self) -> &Stored<PauseState> {
        &self.state.pause.state
    }

    /// The operator state's revision, when the daemon reports one.
    pub fn revision(&self) -> Option<u64> {
        self.state.revision
    }

    /// Whether the daemon is paused by the install that marked `mark`, the
    /// revision its pause made: nothing has changed admission since, so the
    /// pause is that install's, stranded, not an operator's.
    pub fn paused_by(&self, mark: Option<u64>) -> bool {
        self.admission().known() == Some(PauseState::Paused)
            && mark.is_some()
            && self.revision() == mark
    }
}

impl Busy {
    /// Says what a restart waits for, when it changed since `reported`.
    pub fn report(&self, reported: &mut Option<Busy>, timeout: u64) {
        if reported.as_ref() == Some(self) {
            return;
        }
        Progress(&format!(
            "Waiting for {} to finish (up to {timeout} seconds)",
            self.summary()
        ))
        .show();
        for line in self.run_lines() {
            println!("    {line}");
        }
        *reported = Some(self.clone());
    }

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
    Refused { error: Stored<ControlRefusal> },
}

/// Why `ren runs ls` gave no list.
#[derive(Debug, thiserror::Error)]
pub enum RunsError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the control API refused: {0}")]
    Refused(Stored<ControlRefusal>),
}

/// One run, as far as a restart concerns it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LiveRun {
    run: RunId,
    /// The session it serves.
    session: String,
    harness: Stored<Harness>,
    workdir: String,
    state: LiveState,
    /// Whether it outlives the daemon; a daemon that does not say holds
    /// none, as before run hosts.
    #[serde(default)]
    tenure: Option<Stored<RunTenure>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LiveState {
    /// Where the run is; unknown text is kept.
    phase: Stored<RunPhase>,
    /// Approvals waiting for the operator, while working.
    #[serde(default)]
    approvals: Vec<Value>,
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
    /// A run host holds it: it goes on through the restart, mid-turn, and
    /// the next daemon takes it up.
    GoesOn,
}

impl LiveRun {
    /// What restarting the daemon does to it. A tenure this task does not
    /// know is treated as bound to the daemon.
    fn restart(&self) -> Restart {
        let held = self.tenure.as_ref().and_then(Stored::known) == Some(RunTenure::Held);
        match self.state.restart() {
            Restart::CutsOff | Restart::Ends if held => Restart::GoesOn,
            restart @ (Restart::CutsOff | Restart::Ends | Restart::Spares | Restart::GoesOn) => {
                restart
            }
        }
    }
}

impl LiveState {
    /// A phase this task does not know is treated as mid-way, never as idle.
    fn restart(&self) -> Restart {
        match self.phase.known() {
            Some(RunPhase::Starting | RunPhase::Working | RunPhase::Stopping) | None => {
                Restart::CutsOff
            }
            Some(RunPhase::Idle) => Restart::Ends,
            Some(RunPhase::Ended) => Restart::Spares,
        }
    }
}

impl Runs {
    pub fn parse(json: &str) -> Result<Self, RunsError> {
        match serde_json::from_str(json)? {
            RunsAnswer::Listed { runs } => Ok(Self(runs)),
            RunsAnswer::Refused { error } => match error.known() {
                Some(ControlRefusal::RunsDisabled) => Ok(Self::default()),
                Some(_) | None => Err(RunsError::Refused(error)),
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
                .filter(|run| run.restart() == Restart::CutsOff)
                .cloned()
                .collect(),
        }
    }

    /// The runs the daemon drives now, through `ren` (`ren runs ls`).
    pub fn read(ren: Invocation) -> Result<Self, InvocationError> {
        let text = ren.args(["runs", "ls"]).answer()?;
        Self::parse(&text).map_err(InvocationError::output("ren runs ls"))
    }

    /// The runs whose process ends with the daemon: every one not ended.
    pub fn ending(&self) -> Ending {
        Ending(
            self.0
                .iter()
                .filter(|run| matches!(run.restart(), Restart::CutsOff | Restart::Ends))
                .cloned()
                .collect(),
        )
    }
}

/// The runs whose process ends with a daemon about to stop.
#[derive(Debug, Default)]
pub struct Ending(Vec<LiveRun>);

impl Ending {
    #[cfg(test)]
    pub fn runs(&self) -> &[LiveRun] {
        &self.0
    }

    /// Names each, with how to continue its conversation.
    pub fn announce(&self) {
        if self.0.is_empty() {
            return;
        }
        Progress(&format!(
            "{} end with the daemon; each conversation can be continued:",
            match self.0.len() {
                1 => "1 run".to_owned(),
                n => format!("{n} runs"),
            }
        ))
        .show();
        for run in &self.0 {
            println!("    {run}\n      {}", run.continuation());
        }
    }

    /// After the restart, once more, so it is the last thing shown.
    pub fn remind(&self) {
        match self.0.len() {
            0 => {}
            1 => println!("1 run ended with the restart (listed above); it can be continued."),
            n => println!("{n} runs ended with the restart (listed above); each can be continued."),
        }
    }
}

/// A restart that gave up waiting, having changed nothing.
#[derive(Debug, thiserror::Error)]
#[error(
    "{} still running after {waited} seconds; nothing was changed. {}",
    .busy.summary(),
    .busy.advice()
)]
pub struct StillBusy {
    pub busy: Busy,
    pub waited: u64,
}

impl LiveRun {
    /// How to continue its conversation after the restart: by writing to
    /// its session, which starts the client again and resumes it.
    pub fn continuation(&self) -> String {
        format!(
            "write to its session to go on: from the Sessions page or `ren runs say {} <prompt>`",
            self.session
        )
    }
}

impl fmt::Display for LiveRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} run {} in {}: ", self.harness, self.run, self.workdir)?;
        match (self.state.phase.known(), self.state.approvals.len()) {
            (Some(RunPhase::Working), 1) => f.write_str("working, waiting for an approval"),
            (Some(RunPhase::Working), n) if n > 1 => {
                write!(f, "working, waiting for {n} approvals")
            }
            // The phase's label (or the unknown text) is what is printed.
            (
                Some(
                    RunPhase::Working
                    | RunPhase::Starting
                    | RunPhase::Idle
                    | RunPhase::Stopping
                    | RunPhase::Ended,
                )
                | None,
                _,
            ) => write!(f, "{}", self.state.phase),
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

    pub fn checks(&self) -> &[DoctorCheck] {
        &self.checks
    }

    /// The check comparing the daemon's build with the CLI's.
    pub fn build(&self) -> Option<&DoctorCheck> {
        self.checks.iter().find(|check| check.name == "build")
    }
}

impl DoctorCheck {
    pub fn name(&self) -> &str {
        &self.name
    }

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
        assert_eq!(idle.admission(), &Stored::Known(PauseState::Running));

        let busy = ControlState::parse(
            r#"{"api_version":1,"state":{"revision":3,"pause":{"state":"paused","retry_after":30},"blocked_sessions":{}},"running":[{"call":"0193e0f1-0000-7000-8000-000000000001","transport":"http"},{"call":"0193e0f1-0000-7000-8000-000000000002","transport":"websocket"}]}"#,
        )
        .unwrap();
        assert_eq!(busy.running(), 2);
        assert_eq!(busy.admission(), &Stored::Known(PauseState::Paused));
    }

    #[test]
    fn keeps_an_unknown_admission() {
        let state = ControlState::parse(r#"{"state":{"pause":{"state":"draining"}},"running":[]}"#)
            .unwrap();
        assert!(state.admission().known().is_none());
        assert_eq!(state.admission().to_string(), "draining");
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
            "session": format!("0193e0f1-0000-7000-a000-{n:012}"),
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
    fn a_pause_is_an_installs_own_only_at_the_revision_it_made() {
        let paused = |revision: u64| {
            ControlState::parse(
                &serde_json::json!({ "state": { "revision": revision,
                    "pause": { "state": "paused", "retry_after": 10 } }, "running": [] })
                .to_string(),
            )
            .unwrap()
        };
        assert!(paused(125).paused_by(Some(125)));
        // An operator paused or resumed since: theirs, left alone.
        assert!(!paused(127).paused_by(Some(125)));
        assert!(!paused(125).paused_by(None));
        let running = ControlState::parse(
            r#"{"state":{"revision":125,"pause":{"state":"running"}},"running":[]}"#,
        )
        .unwrap();
        assert!(!running.paused_by(Some(125)));
        // A daemon that reports no revision is never taken for one.
        assert!(!state("paused", 0).paused_by(Some(0)));
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
    fn held_runs_go_on_through_a_restart_and_hold_nothing_up() {
        let held = |mut run: serde_json::Value| {
            run["tenure"] = "held".into();
            run
        };
        let listed = runs(&[
            held(run(1, "codex", "working")),
            held(run(2, "claude_code", "idle")),
            held(run(3, "pi", "ended")),
            // A tenure this build does not know is bound to the daemon.
            {
                let mut run = run(4, "opencode", "idle");
                run["tenure"] = "orbiting".into();
                run
            },
        ]);
        assert_eq!(
            state("running", 0).quiesce(&listed, RunPolicy::Wait),
            Quiesce::Pause
        );
        let ending: Vec<_> = listed
            .ending()
            .runs()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            ending,
            ["opencode run 0193e0f1-0000-7000-8000-000000000004 in /home/op/acme-4: idle"]
        );
        // Bound, as a daemon before run hosts reports every run.
        let bound = runs(&[{
            let mut run = run(5, "codex", "working");
            run["tenure"] = "bound".into();
            run
        }]);
        assert!(matches!(
            state("running", 0).quiesce(&bound, RunPolicy::Wait),
            Quiesce::Wait(_)
        ));
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
        ]);
        let ending: Vec<_> = listed
            .ending()
            .runs()
            .iter()
            .map(|run| format!("{run}; {}", run.continuation()))
            .collect();
        assert_eq!(
            ending,
            [
                "codex run 0193e0f1-0000-7000-8000-000000000001 in /home/op/acme-1: idle; write to its session to go on: from the Sessions page or `ren runs say 0193e0f1-0000-7000-a000-000000000001 <prompt>`",
                "claude_code run 0193e0f1-0000-7000-8000-000000000002 in /home/op/acme-2: idle; write to its session to go on: from the Sessions page or `ren runs say 0193e0f1-0000-7000-a000-000000000002 <prompt>`",
                "opencode run 0193e0f1-0000-7000-8000-000000000003 in /home/op/acme-3: idle; write to its session to go on: from the Sessions page or `ren runs say 0193e0f1-0000-7000-a000-000000000003 <prompt>`",
                "codex run 0193e0f1-0000-7000-8000-000000000004 in /home/op/acme-4: starting; write to its session to go on: from the Sessions page or `ren runs say 0193e0f1-0000-7000-a000-000000000004 <prompt>`",
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
            listed.ending().runs()[1]
                .to_string()
                .starts_with("opencode_next run")
        );
        // A daemon with runs off drives none.
        assert!(
            Runs::parse(r#"{"api_version":1,"error":"runs_disabled"}"#)
                .unwrap()
                .ending()
                .runs()
                .is_empty()
        );
        assert!(matches!(
            Runs::parse(r#"{"api_version":1,"error":"unauthorized"}"#),
            Err(RunsError::Refused(Stored::Known(
                ControlRefusal::Unauthorized
            )))
        ));
        assert!(matches!(Runs::parse("not json"), Err(RunsError::Json(_))));
    }
}
