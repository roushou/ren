//! Closing work items from what the deployer sees: a sent item lands when a
//! commit on `main` names it (`Work-Item: <id>`, a [`Landing`]), and a
//! landed item ships when production runs a clean build whose history holds
//! that commit. Each change goes through production's own control API, with
//! its control token, as an operator's would; production's daemon checks it
//! against the item's state.

use super::{CanaryError, Deployer};
use crate::{
    progress::Progress,
    replica::{ProductionPlace, ReplicaError},
};
use ren_gateway::api::control::request::{LandWorkRequest, ShipWorkRequest, WorkChange};
use ren_git::Commit;
use ren_git::SourceRevision;
use ren_git::Tree;
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{Landing, WorkStateKind},
};
use std::{thread, time::Duration};

/// Production's work items, as far as closing them needs.
pub struct WorkLedger<'a> {
    production: &'a ProductionPlace,
}

/// What a pass did to the work items.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Closed {
    pub landed: usize,
    pub shipped: usize,
}

impl<'a> WorkLedger<'a> {
    /// How far back `main`'s history is read for trailers.
    const HISTORY: u32 = 500;
    /// How long, and how often, to wait for production to store a change.
    const POLL: Duration = Duration::from_millis(100);
    const POLLS: u32 = 100;

    pub fn new(production: &'a ProductionPlace) -> Self {
        Self { production }
    }

    /// Lands the sent items `main`'s recent commits name, then ships the
    /// landed items production's build holds. A production whose daemon
    /// keeps no work items yet is left alone.
    pub fn close(&self, deployer: &Deployer, main: &Commit) -> Result<Closed, CanaryError> {
        let Some(items) = self.production.work_items()? else {
            Progress("Production keeps no work items yet; none to close").show();
            return Ok(Closed::default());
        };
        let mut closed = Closed::default();
        let sent: Vec<WorkItemId> = items
            .iter()
            .filter(|item| item.state == Stored::Known(WorkStateKind::Sent))
            .map(|item| item.id)
            .collect();
        // Oldest first: an item named twice lands as the first commit.
        let mut landed: Vec<(WorkItemId, Commit)> = items
            .iter()
            .filter(|item| item.state == Stored::Known(WorkStateKind::Landed))
            .filter_map(|item| Some((item.id, item.landed.clone()?)))
            .collect();
        if !sent.is_empty() {
            let mut log = deployer.git.log(&deployer.repo, main, Self::HISTORY)?;
            log.reverse();
            for (commit, message) in log {
                for item in Landing::read(commit.clone(), &message).items {
                    if sent.contains(&item) && !landed.iter().any(|(id, _)| *id == item) {
                        self.production.move_work(
                            item,
                            WorkChange::Land(LandWorkRequest {
                                commit: commit.clone(),
                            }),
                        )?;
                        Progress(&format!(
                            "Work item {item} landed as {}",
                            commit.abbreviated(12)
                        ))
                        .show();
                        landed.push((item, commit.clone()));
                        closed.landed += 1;
                    }
                }
            }
        }
        // A change is stored a moment after it is accepted: ship only what
        // production says has landed.
        if closed.landed > 0 {
            self.settle(&landed)?;
        }
        let running = match self.production.revision()? {
            SourceRevision::Commit {
                commit,
                tree: Tree::Clean,
            } => Some(commit),
            SourceRevision::Commit {
                tree: Tree::Modified,
                ..
            }
            | SourceRevision::Unknown => None,
        };
        if let Some(running) = running {
            for (item, commit) in &landed {
                if deployer.git.contains(&deployer.repo, &running, commit)? {
                    self.production.move_work(
                        *item,
                        WorkChange::Ship(ShipWorkRequest {
                            revision: SourceRevision::Commit {
                                commit: running.clone(),
                                tree: Tree::Clean,
                            },
                        }),
                    )?;
                    Progress(&format!("Work item {item} shipped in {}", running.short())).show();
                    closed.shipped += 1;
                }
            }
        }
        Ok(closed)
    }

    /// Waits until production lists every item in `landed` as landed, so
    /// a ship is checked against the state it moves from. Polls are
    /// counted, not timed, so this reads no clock.
    fn settle(&self, landed: &[(WorkItemId, Commit)]) -> Result<(), CanaryError> {
        for _ in 0..Self::POLLS {
            let items = self.production.work_items()?.unwrap_or_default();
            let stored = landed.iter().all(|(id, _)| {
                items.iter().any(|item| {
                    item.id == *id && item.state == Stored::Known(WorkStateKind::Landed)
                })
            });
            if stored {
                return Ok(());
            }
            thread::sleep(Self::POLL);
        }
        Err(
            ReplicaError::WorkAnswer("production did not store the landings in time".to_owned())
                .into(),
        )
    }
}
