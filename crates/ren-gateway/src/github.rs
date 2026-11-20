//! What Ren last saw of each project's repository on GitHub
//! (docs/plans/self-build.md, phase 8): a [`PullBoard`] the daemon's
//! poller records each repository's `ren_github::Observation` on, and the
//! read API reads by project.

use ren_github::{Observation, RepositorySlug};
use ren_recorder::project::ProjectName;
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

impl PullBoard {
    /// A board watching each project's repository, nothing seen yet.
    pub fn watching(projects: impl IntoIterator<Item = (ProjectName, RepositorySlug)>) -> Self {
        let watched = projects
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

    /// The repositories watched, each once.
    pub fn repositories(&self) -> Vec<RepositorySlug> {
        let mut repositories: Vec<RepositorySlug> = self
            .read()
            .values()
            .map(|watched| watched.repository.clone())
            .collect();
        repositories.sort();
        repositories.dedup();
        repositories
    }

    /// What was seen of `project`'s repository; none when it is not watched.
    pub fn watched(&self, project: &ProjectName) -> Option<Watched> {
        self.read().get(project).cloned()
    }

    /// Records what was seen of `repository`, for every project it is.
    pub fn record(&self, repository: &RepositorySlug, observation: &Observation) {
        // A poisoned lock still holds whole values: each change replaces one.
        let mut watched = self.watched.write().unwrap_or_else(PoisonError::into_inner);
        for entry in watched.values_mut() {
            if &entry.repository == repository {
                entry.observation = observation.clone();
            }
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<ProjectName, Watched>> {
        self.watched.read().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::PullBoard;
    use ren_github::Observation;

    #[test]
    fn a_repository_is_recorded_for_each_project_it_is() {
        let board = PullBoard::watching([
            ("ren".parse().unwrap(), "acme/ren".parse().unwrap()),
            ("web".parse().unwrap(), "acme/web".parse().unwrap()),
        ]);
        let failed =
            Observation::Waiting.failed("refused".into(), std::time::SystemTime::UNIX_EPOCH);
        board.record(&"acme/ren".parse().unwrap(), &failed);
        assert_eq!(
            board.watched(&"ren".parse().unwrap()).unwrap().observation,
            failed
        );
        assert_eq!(
            board.watched(&"web".parse().unwrap()).unwrap().observation,
            Observation::Waiting
        );
        assert_eq!(board.watched(&"other".parse().unwrap()), None);
        assert_eq!(board.repositories().len(), 2);
    }
}
