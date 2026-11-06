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

impl ControlState {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
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
}
