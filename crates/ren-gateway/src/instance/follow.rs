//! Previews that follow their pull requests (docs/plans/instances.md,
//! phase 4): the daemon's GitHub watch tells the [`PullFollower`] each
//! repository it sees, and the follower says whether what its pull
//! requests want ([`Wanted::pull`]) moved: one opened, closed, left or
//! became a draft, or its head moved. A move is when the factory is asked
//! for a pass, rather than at its timer's next. A read that failed, or none
//! yet, moves nothing: the last answer is never taken as current.

use super::instances::Wanted;
use ren_github::{Observation, RepositorySlug};
use std::collections::BTreeMap;

/// What each repository's pull requests last wanted, in its order.
#[derive(Debug, Default)]
pub struct PullFollower {
    wanted: BTreeMap<RepositorySlug, Vec<Wanted>>,
}

/// Whether what a repository's pull requests want moved.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// It moved, or is seen for the first time: a pass is due.
    Moved,
    /// Nothing a preview follows changed.
    Still,
}

impl PullFollower {
    /// Takes what was seen of `repository`, and says whether it moved.
    pub fn see(&mut self, repository: &RepositorySlug, observation: &Observation) -> Followed {
        let Some((sighting, true)) = observation.sighting() else {
            return Followed::Still;
        };
        let wanted: Vec<Wanted> = sighting
            .snapshot
            .pull_requests
            .iter()
            .filter_map(|pull| {
                // GitHub's object id is a full commit id.
                let head = pull.head.as_str().parse().ok()?;
                Wanted::pull(pull.number, head, pull.readiness)
            })
            .collect();
        match self.wanted.insert(repository.clone(), wanted.clone()) {
            Some(before) if before == wanted => Followed::Still,
            Some(_) | None => Followed::Moved,
        }
    }
}
