//! What was seen of a repository over time: a small state machine a
//! watcher keeps per repository.

use crate::watch::Snapshot;
use std::time::SystemTime;

/// What was seen of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// Not answered yet.
    Waiting,
    /// The newest read was answered.
    Seen(Sighting),
    /// The newest read failed; the last answer, if any, is kept apart and
    /// never passed off as current.
    Failed {
        error: String,
        at: SystemTime,
        last: Option<Sighting>,
    },
}

/// One answer, and when it came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    pub snapshot: Snapshot,
    pub at: SystemTime,
}

impl Observation {
    /// After an answer.
    #[must_use]
    pub fn seen(self, sighting: Sighting) -> Self {
        Self::Seen(sighting)
    }

    /// After a read that failed: the last answer is kept, never as current.
    #[must_use]
    pub fn failed(self, error: String, at: SystemTime) -> Self {
        let last = match self {
            Self::Waiting => None,
            Self::Seen(sighting) => Some(sighting),
            Self::Failed { last, .. } => last,
        };
        Self::Failed { error, at, last }
    }

    /// The newest answer, current or not, and whether it is current.
    pub fn sighting(&self) -> Option<(&Sighting, bool)> {
        match self {
            Self::Waiting => None,
            Self::Seen(sighting) => Some((sighting, true)),
            Self::Failed { last, .. } => last.as_ref().map(|sighting| (sighting, false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Observation, Sighting};
    use crate::watch::{Listing, Snapshot};
    use std::time::{Duration, SystemTime};

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn sighting(seconds: u64) -> Sighting {
        Sighting {
            snapshot: Snapshot {
                default_branch: None,
                pull_requests: Vec::new(),
                listing: Listing::Complete,
                manifest: None,
            },
            at: at(seconds),
        }
    }

    #[test]
    fn a_failure_keeps_the_last_answer_apart() {
        let failed = Observation::Waiting
            .seen(sighting(1))
            .failed("timeout".into(), at(2));
        assert_eq!(failed.sighting(), Some((&sighting(1), false)));
        let again = failed.failed("refused".into(), at(3));
        assert_eq!(again.sighting(), Some((&sighting(1), false)));
        assert_eq!(
            Observation::Waiting.failed("x".into(), at(1)),
            Observation::Failed {
                error: "x".into(),
                at: at(1),
                last: None,
            }
        );
        assert_eq!(
            Observation::Waiting.seen(sighting(4)).sighting(),
            Some((&sighting(4), true))
        );
    }
}
