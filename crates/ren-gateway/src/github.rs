//! What Ren last saw of each watched GitHub repository
//! (docs/plans/self-build.md, phase 8): a [`PullBoard`] the daemon's poller
//! writes and the read API reads. Each project's [`Observation`] is a small
//! state machine: waiting for its first answer, seen, or failed, keeping
//! the last answer seen so a failed poll never hides what was known, and
//! never passes it off as current either.

use ren_github::{RepositorySlug, Snapshot};
use ren_recorder::{project::ProjectName, time::UnixMillis};
use std::{
    collections::BTreeMap,
    sync::{Arc, PoisonError, RwLock},
};

/// Each watched project's repository and what was last seen of it. Cheap
/// to clone; every clone is the same board.
#[derive(Debug, Clone, Default)]
pub struct PullBoard {
    watched: Arc<RwLock<BTreeMap<ProjectName, Watched>>>,
}

/// One project's repository on GitHub, and what was seen of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub repository: RepositorySlug,
    pub observation: Observation,
}

/// What was seen of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// Not answered yet.
    Waiting,
    /// The newest poll was answered.
    Seen(Sighting),
    /// The newest poll failed; the last answer, if any, is kept apart.
    Failed {
        error: String,
        at: UnixMillis,
        last: Option<Sighting>,
    },
}

/// One answer, and when it came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    pub snapshot: Snapshot,
    pub at: UnixMillis,
}

impl Observation {
    /// After an answer.
    #[must_use]
    pub fn seen(self, sighting: Sighting) -> Self {
        Self::Seen(sighting)
    }

    /// After a poll that failed: the last answer is kept, never as current.
    #[must_use]
    pub fn failed(self, error: String, at: UnixMillis) -> Self {
        let last = match self {
            Self::Waiting => None,
            Self::Seen(sighting) => Some(sighting),
            Self::Failed { last, .. } => last,
        };
        Self::Failed { error, at, last }
    }
}

impl PullBoard {
    /// A board watching each project's repository, nothing seen yet.
    pub fn watching(repositories: impl IntoIterator<Item = (ProjectName, RepositorySlug)>) -> Self {
        let watched = repositories
            .into_iter()
            .map(|(project, repository)| {
                (
                    project,
                    Watched {
                        repository,
                        observation: Observation::Waiting,
                    },
                )
            })
            .collect();
        Self {
            watched: Arc::new(RwLock::new(watched)),
        }
    }

    /// Each watched project and its repository.
    pub fn repositories(&self) -> Vec<(ProjectName, RepositorySlug)> {
        self.read()
            .iter()
            .map(|(project, watched)| (project.clone(), watched.repository.clone()))
            .collect()
    }

    /// What was seen of `project`'s repository; none when it is not watched.
    pub fn watched(&self, project: &ProjectName) -> Option<Watched> {
        self.read().get(project).cloned()
    }

    /// Records an answer for `project`.
    pub fn record_seen(&self, project: &ProjectName, sighting: Sighting) {
        self.change(project, |observation| observation.seen(sighting));
    }

    /// Records a failed poll for `project`.
    pub fn record_failed(&self, project: &ProjectName, error: String, at: UnixMillis) {
        self.change(project, |observation| observation.failed(error, at));
    }

    fn change(&self, project: &ProjectName, step: impl FnOnce(Observation) -> Observation) {
        // A poisoned lock still holds whole values: each change replaces one.
        let mut watched = self.watched.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = watched.get_mut(project) {
            let observation = std::mem::replace(&mut entry.observation, Observation::Waiting);
            entry.observation = step(observation);
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<ProjectName, Watched>> {
        self.watched.read().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{Observation, PullBoard, Sighting};
    use ren_github::{Listing, Snapshot};
    use ren_recorder::time::UnixMillis;

    fn sighting(at: u64) -> Sighting {
        Sighting {
            snapshot: Snapshot {
                default_branch: None,
                pull_requests: Vec::new(),
                listing: Listing::Complete,
            },
            at: UnixMillis::from_millis(at),
        }
    }

    #[test]
    fn a_failure_keeps_the_last_answer_apart() {
        let failed = Observation::Waiting
            .seen(sighting(1))
            .failed("timeout".into(), UnixMillis::from_millis(2));
        assert_eq!(
            failed,
            Observation::Failed {
                error: "timeout".into(),
                at: UnixMillis::from_millis(2),
                last: Some(sighting(1)),
            }
        );
        // A second failure keeps the same last answer.
        let again = failed.failed("refused".into(), UnixMillis::from_millis(3));
        let Observation::Failed { last, .. } = again else {
            panic!("still failed");
        };
        assert_eq!(last, Some(sighting(1)));
        assert_eq!(
            Observation::Waiting.failed("x".into(), UnixMillis::from_millis(1)),
            Observation::Failed {
                error: "x".into(),
                at: UnixMillis::from_millis(1),
                last: None,
            }
        );
    }

    #[test]
    fn the_board_records_only_watched_projects() {
        let ren = "ren".parse().unwrap();
        let board = PullBoard::watching([(ren, "acme/ren".parse().unwrap())]);
        let ren = "ren".parse().unwrap();
        let other = "other".parse().unwrap();
        board.record_seen(&ren, sighting(5));
        board.record_seen(&other, sighting(5));
        assert_eq!(
            board.watched(&ren).unwrap().observation,
            Observation::Seen(sighting(5))
        );
        assert_eq!(board.watched(&other), None);
    }
}
