//! What the installed `tgv` reports about its daemon, read only as far as an
//! upgrade needs: `tgv control state` (running calls and admission) and
//! `tgv doctor --json` (its checks).
use serde::Deserialize;
use serde_json::Value;

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
    /// Calls are running: wait for them.
    Wait { running: usize },
}

/// Whether the pause just made left the daemon quiet.
#[derive(Debug, PartialEq, Eq)]
pub enum AfterPause {
    Quiet,
    /// A call started between the check and the pause: resume and wait.
    SlippedIn {
        running: usize,
    },
}

impl ControlState {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn quiesce(&self) -> Quiesce {
        match (self.admission(), self.running()) {
            (Admission::Running, 0) => Quiesce::Pause,
            (Admission::Paused | Admission::Other(_), 0) => Quiesce::LeaveAlone,
            (Admission::Running | Admission::Paused | Admission::Other(_), running) => {
                Quiesce::Wait { running }
            }
        }
    }

    pub fn after_pause(&self) -> AfterPause {
        match self.running() {
            0 => AfterPause::Quiet,
            running => AfterPause::SlippedIn { running },
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

/// `tgv doctor --json`.
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

    #[test]
    fn pauses_only_an_idle_daemon_that_admits_calls() {
        assert_eq!(state("running", 0).quiesce(), Quiesce::Pause);
        assert_eq!(state("paused", 0).quiesce(), Quiesce::LeaveAlone);
        assert_eq!(state("draining", 0).quiesce(), Quiesce::LeaveAlone);
        for admission in ["running", "paused", "draining"] {
            assert_eq!(
                state(admission, 2).quiesce(),
                Quiesce::Wait { running: 2 },
                "{admission}"
            );
        }
    }

    #[test]
    fn a_call_that_slipped_in_before_the_pause_is_waited_for() {
        assert_eq!(state("paused", 0).after_pause(), AfterPause::Quiet);
        assert_eq!(
            state("paused", 1).after_pause(),
            AfterPause::SlippedIn { running: 1 }
        );
    }
}
