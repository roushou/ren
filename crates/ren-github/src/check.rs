//! A pull request's checks: each check run or commit status on its head,
//! and how GitHub sums them up.

use crate::reported::Reported;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Where one check stands. A check run's status while it has not
/// completed, its conclusion once it has; a commit status's state.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CheckState {
    /// Requested, queued, or waiting, not started.
    Queued,
    /// Running now.
    Running,
    /// A commit status GitHub expects and no one has posted yet.
    Expected,
    Passed,
    Failed,
    /// A commit status that could not run.
    Errored,
    Cancelled,
    TimedOut,
    Skipped,
    Neutral,
    /// Completed, but waits for someone to act on it.
    ActionRequired,
    /// Completed so long ago that GitHub marked it stale.
    Stale,
    /// The check could not start.
    StartupFailed,
}

/// How GitHub sums up every check on a commit.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ChecksState {
    /// Every check passed.
    Passing,
    /// One failed.
    Failing,
    /// One could not run.
    Errored,
    /// One is still to finish.
    Pending,
    /// One is expected and not yet posted.
    Expected,
    /// The commit has no checks.
    None,
}

/// One check on a pull request's head: its name and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub state: Reported<CheckState>,
}

impl CheckState {
    /// A check run's state: its conclusion once `status` is `COMPLETED`,
    /// otherwise its status. Unknown text is kept, joined as GitHub sent it.
    pub(crate) fn of_run(status: &str, conclusion: Option<&str>) -> Reported<Self> {
        let known = match (status, conclusion) {
            ("COMPLETED", Some(conclusion)) => match conclusion {
                "SUCCESS" => Some(Self::Passed),
                "FAILURE" => Some(Self::Failed),
                "CANCELLED" => Some(Self::Cancelled),
                "TIMED_OUT" => Some(Self::TimedOut),
                "SKIPPED" => Some(Self::Skipped),
                "NEUTRAL" => Some(Self::Neutral),
                "ACTION_REQUIRED" => Some(Self::ActionRequired),
                "STALE" => Some(Self::Stale),
                "STARTUP_FAILURE" => Some(Self::StartupFailed),
                _ => None,
            },
            ("QUEUED" | "WAITING" | "PENDING" | "REQUESTED", _) => Some(Self::Queued),
            ("IN_PROGRESS", _) => Some(Self::Running),
            _ => None,
        };
        known.map_or_else(
            || Reported::Unknown(format!("{status}/{}", conclusion.unwrap_or("-"))),
            Reported::Known,
        )
    }

    /// A commit status's state.
    pub(crate) fn of_status(state: &str) -> Reported<Self> {
        match state {
            "SUCCESS" => Reported::Known(Self::Passed),
            "FAILURE" => Reported::Known(Self::Failed),
            "ERROR" => Reported::Known(Self::Errored),
            "PENDING" => Reported::Known(Self::Running),
            "EXPECTED" => Reported::Known(Self::Expected),
            other => Reported::Unknown(other.to_owned()),
        }
    }
}

impl ChecksState {
    /// GitHub's rollup state; no rollup at all is [`ChecksState::None`].
    pub(crate) fn of_rollup(state: Option<&str>) -> Reported<Self> {
        match state {
            None => Reported::Known(Self::None),
            Some("SUCCESS") => Reported::Known(Self::Passing),
            Some("FAILURE") => Reported::Known(Self::Failing),
            Some("ERROR") => Reported::Known(Self::Errored),
            Some("PENDING") => Reported::Known(Self::Pending),
            Some("EXPECTED") => Reported::Known(Self::Expected),
            Some(other) => Reported::Unknown(other.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckState, ChecksState};
    use crate::reported::Reported;

    #[test]
    fn a_run_reads_its_conclusion_once_completed() {
        assert_eq!(
            CheckState::of_run("COMPLETED", Some("FAILURE")),
            Reported::Known(CheckState::Failed)
        );
        assert_eq!(
            CheckState::of_run("IN_PROGRESS", None),
            Reported::Known(CheckState::Running)
        );
        assert_eq!(
            CheckState::of_run("QUEUED", None),
            Reported::Known(CheckState::Queued)
        );
    }

    #[test]
    fn unknown_states_are_kept_not_rounded() {
        let state = CheckState::of_run("COMPLETED", Some("SOMETHING_NEW"));
        assert_eq!(state.label(), "COMPLETED/SOMETHING_NEW");
        assert_eq!(state.known(), None);
        // Completed without a conclusion is not a pass.
        assert_eq!(CheckState::of_run("COMPLETED", None).known(), None);
        assert_eq!(CheckState::of_status("SOMETHING_NEW").known(), None);
        assert_eq!(ChecksState::of_rollup(Some("NEW")).label(), "NEW");
    }

    #[test]
    fn no_rollup_is_no_checks() {
        assert_eq!(
            ChecksState::of_rollup(None),
            Reported::Known(ChecksState::None)
        );
    }
}
