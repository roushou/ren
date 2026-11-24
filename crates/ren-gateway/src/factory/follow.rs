//! What the GitHub watch's sightings ask of the factory
//! (docs/plans/instances.md, phases 4 and 6): previews follow their pull
//! requests, and the canary follows `main`. The daemon tells the
//! [`Follower`] each repository it sees, and the follower answers the
//! requests that sighting makes:
//!
//! - [`FactoryRequest::Previews`] when what its pull requests want moved
//!   ([`Wanted::pull`]): one opened, closed, became ready or a draft, or its
//!   head moved; also on the first sighting, which a pass of previews
//!   takes up harmlessly;
//! - [`FactoryRequest::Canary`], for each project watched on it, when its
//!   default branch's head moved from one seen before. Not on the first
//!   sighting: a canary pass is a build and its checks, and a daemon that
//!   restarts (as a promotion restarts it) would otherwise start one.
//!
//! A read that failed, or none yet, asks nothing: the last answer is never
//! taken as current.

use super::FactoryRequest;
use crate::instance::Wanted;
use ren_github::{Observation, Oid, RepositorySlug};
use ren_recorder::project::ProjectName;
use std::collections::BTreeMap;

/// Each watched repository's projects, and what was last seen of it.
#[derive(Debug, Default)]
pub struct Follower {
    projects: BTreeMap<RepositorySlug, Vec<ProjectName>>,
    seen: BTreeMap<RepositorySlug, Seen>,
}

/// What a sighting showed that the factory follows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    wanted: Vec<Wanted>,
    main: Option<Oid>,
}

impl Follower {
    /// Following each project's repository, nothing seen yet.
    pub fn watching(projects: impl IntoIterator<Item = (ProjectName, RepositorySlug)>) -> Self {
        let mut watched: BTreeMap<RepositorySlug, Vec<ProjectName>> = BTreeMap::new();
        for (project, repository) in projects {
            watched.entry(repository).or_default().push(project);
        }
        Self {
            projects: watched,
            seen: BTreeMap::new(),
        }
    }

    /// Takes what was seen of `repository`, and answers what it asks of
    /// the factory.
    pub fn see(
        &mut self,
        repository: &RepositorySlug,
        observation: &Observation,
    ) -> Vec<FactoryRequest> {
        let Some((sighting, true)) = observation.sighting() else {
            return Vec::new();
        };
        let snapshot = &sighting.snapshot;
        let now = Seen {
            wanted: snapshot
                .pull_requests
                .iter()
                .filter_map(|pull| {
                    // GitHub's object id is a full commit id.
                    let head = pull.head.as_str().parse().ok()?;
                    Wanted::pull(pull.number, head, pull.readiness)
                })
                .collect(),
            main: snapshot
                .default_branch
                .as_ref()
                .map(|(_, head)| head.clone()),
        };
        let before = self.seen.insert(repository.clone(), now.clone());
        let mut asks = Vec::new();
        if before
            .as_ref()
            .is_none_or(|before| before.wanted != now.wanted)
        {
            asks.push(FactoryRequest::Previews);
        }
        if before.is_some_and(|before| before.main != now.main) {
            asks.extend(
                self.projects
                    .get(repository)
                    .into_iter()
                    .flatten()
                    .map(|project| FactoryRequest::Canary(project.clone())),
            );
        }
        asks
    }
}
