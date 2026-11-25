//! The deployments the deployer runs in their slots, and the one pure step that
//! moves them on: `(Deployments, Event) -> (Deployments, Vec<Action>)`.

use super::{Seed, slot::Slot};
use crate::release::PreviewCapacity;
use ren_git::Commit;
use ren_github::{PullNumber, Readiness};
use ren_recorder::id::WorkItemId;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// The deployments the deployer runs, each in its slot: today, the previews
/// of Ren. Kept by the deployer as a file in production's data folder
/// ([`Deployments::FILE`]), which production's gateway reads. The deployer and
/// that gateway are different builds (the canary's and the promoted one),
/// so the file keeps its shape: its keys are the ones previews had.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployments {
    /// By slot, each slot at most once.
    #[serde(rename = "previews")]
    deployments: Vec<Deployment>,
}

/// One deployment: what it is of, the commit it runs, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub slot: Slot,
    #[serde(rename = "key")]
    pub trigger: Trigger,
    pub commit: Commit,
    /// Left out when a copy of production, as files written before seeds
    /// hold every one, so an older reader still reads them.
    #[serde(default, skip_serializing_if = "Seed::is_copy")]
    pub seed: Seed,
    /// The work items whose change it previews, recorded on each once it
    /// runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub work: Vec<WorkItemId>,
    pub state: DeploymentState,
}

/// What picks a deployment's commit: an open pull request's head,
/// followed while it is ready, or a commit the operator chose, until the
/// choice is withdrawn. Each is run on a copy of production's history,
/// every operator key a placeholder. Written as previews wrote their key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    #[serde(rename = "pull")]
    FollowsPull(PullNumber),
    #[serde(rename = "commit")]
    Chosen(Commit),
}

/// Where an deployment stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeploymentState {
    /// A pass is building and starting its commit; or, as a later pass
    /// reads it, one was and stopped before it said how that ended. That
    /// outcome is unknown, never taken for running: the pass starts it
    /// again, in place of whatever runs in its slot.
    Building,
    /// It answers on its slot's ports, reached as `reach` says.
    Running { reach: Reach },
    /// Building or starting it failed; not tried again for this commit.
    Failed { detail: String },
}

/// Where a running deployment is reached from.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Reach {
    /// The host's loopback ports alone.
    Loopback,
    /// The tailnet as well, on its slot's published ports.
    Tailnet,
}

/// An deployment that should run: the commit for its trigger, in priority
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub trigger: Trigger,
    pub commit: Commit,
    pub seed: Seed,
    /// The work items whose change it previews.
    pub work: Vec<WorkItemId>,
}

/// What moves the deployments on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A pass found these wanted, first first, with room for `capacity`.
    Wanted {
        wanted: Vec<Wanted>,
        capacity: PreviewCapacity,
    },
    /// The deployer started `commit` in `slot`, reached as `reach`.
    Started {
        slot: Slot,
        commit: Commit,
        reach: Reach,
    },
    /// Building or starting `commit` in `slot` failed.
    Failed {
        slot: Slot,
        commit: Commit,
        detail: String,
    },
}

/// What the deployer does next, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Stop whatever runs in the slot.
    Stop(Slot),
    /// Build the commit and start it in the slot, in place of whatever
    /// runs there; report back with [`Event::Started`] or
    /// [`Event::Failed`].
    Start {
        slot: Slot,
        trigger: Trigger,
        commit: Commit,
        seed: Seed,
    },
}

/// A report about a slot that is not building that commit: a pass raced
/// another, or reported twice. Nothing changes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("slot {} is not building {commit}", slot.index())]
pub struct StaleReport {
    pub slot: Slot,
    pub commit: Commit,
}

impl Wanted {
    /// What an open pull request wants: a preview of its head, unless it
    /// is a draft.
    pub fn pull(number: PullNumber, head: Commit, readiness: Readiness) -> Option<Self> {
        match readiness {
            Readiness::Ready => Some(Self {
                trigger: Trigger::FollowsPull(number),
                commit: head,
                seed: Seed::Copy,
                work: Vec::new(),
            }),
            Readiness::Draft => None,
        }
    }
}

impl Deployments {
    /// Its file in production's data folder: still named for previews,
    /// which production's gateway reads it as.
    pub const FILE: &str = "previews.json";

    pub fn deployments(&self) -> &[Deployment] {
        &self.deployments
    }

    /// The deployment of `trigger`, if one holds a slot.
    pub fn of(&self, trigger: &Trigger) -> Option<&Deployment> {
        self.deployments
            .iter()
            .find(|deployment| &deployment.trigger == trigger)
    }

    /// The deployments after `event`, and what to do to get there. A report
    /// that does not fit (a slot not building that commit) is refused, and
    /// nothing changes.
    pub fn step(self, event: Event) -> Result<(Self, Vec<Action>), StaleReport> {
        match event {
            Event::Wanted { wanted, capacity } => Ok(self.want(&wanted, capacity)),
            Event::Started {
                slot,
                commit,
                reach,
            } => self
                .settle(slot, commit, DeploymentState::Running { reach })
                .map(|deployments| (deployments, Vec::new())),
            Event::Failed {
                slot,
                commit,
                detail,
            } => self
                .settle(slot, commit, DeploymentState::Failed { detail })
                .map(|deployments| (deployments, Vec::new())),
        }
    }

    /// The deployments that should run now that `wanted` (in priority order)
    /// is wanted, and what to do to get there. The first wants, as many as
    /// `capacity` allows, hold slots; a trigger wanted twice counts once. An
    /// deployment whose trigger is still wanted keeps its slot: unchanged when
    /// it runs or failed at the same commit, started again when its
    /// outcome is unknown (left building) or its trigger wants another
    /// commit. One no longer wanted is stopped; a new one takes the lowest
    /// free slot.
    fn want(self, wanted: &[Wanted], capacity: PreviewCapacity) -> (Self, Vec<Action>) {
        let mut chosen: Vec<&Wanted> = Vec::new();
        for want in wanted {
            if chosen.len() == capacity.get() {
                break;
            }
            if !chosen.iter().any(|kept| kept.trigger == want.trigger) {
                chosen.push(want);
            }
        }
        let slots: Vec<Slot> = Slot::first(capacity.get()).collect();
        let mut stops = Vec::new();
        let mut starts = Vec::new();
        let mut kept: Vec<Deployment> = Vec::new();
        for deployment in self.deployments {
            let still = chosen
                .iter()
                .find(|want| want.trigger == deployment.trigger);
            match still {
                Some(want) if slots.contains(&deployment.slot) => {
                    // Another commit or other data is another deployment;
                    // other work items are only recorded on.
                    let current = want.commit == deployment.commit && want.seed == deployment.seed;
                    match (&deployment.state, current) {
                        (
                            DeploymentState::Running { .. } | DeploymentState::Failed { .. },
                            true,
                        ) => {
                            kept.push(Deployment {
                                work: want.work.clone(),
                                ..deployment
                            });
                        }
                        (DeploymentState::Building, true)
                        | (
                            DeploymentState::Running { .. }
                            | DeploymentState::Building
                            | DeploymentState::Failed { .. },
                            false,
                        ) => {
                            starts.push(Action::Start {
                                slot: deployment.slot,
                                trigger: want.trigger.clone(),
                                commit: want.commit.clone(),
                                seed: want.seed,
                            });
                            kept.push(Deployment {
                                commit: want.commit.clone(),
                                seed: want.seed,
                                work: want.work.clone(),
                                state: DeploymentState::Building,
                                ..deployment
                            });
                        }
                    }
                }
                Some(_) | None => stops.push(Action::Stop(deployment.slot)),
            }
        }
        for want in chosen {
            if kept
                .iter()
                .any(|deployment| deployment.trigger == want.trigger)
            {
                continue;
            }
            let Some(slot) = slots
                .iter()
                .copied()
                .find(|slot| kept.iter().all(|deployment| deployment.slot != *slot))
            else {
                break;
            };
            // A stopped slot taken again is replaced by its start.
            stops.retain(|stop| stop != &Action::Stop(slot));
            starts.push(Action::Start {
                slot,
                trigger: want.trigger.clone(),
                commit: want.commit.clone(),
                seed: want.seed,
            });
            kept.push(Deployment {
                slot,
                trigger: want.trigger.clone(),
                commit: want.commit.clone(),
                seed: want.seed,
                work: want.work.clone(),
                state: DeploymentState::Building,
            });
        }
        kept.sort_by_key(|deployment| deployment.slot);
        stops.extend(starts);
        (Self { deployments: kept }, stops)
    }

    /// `slot`'s build of `commit`, settled as `state`.
    fn settle(
        mut self,
        slot: Slot,
        commit: Commit,
        state: DeploymentState,
    ) -> Result<Self, StaleReport> {
        let building = self.deployments.iter_mut().find(|deployment| {
            deployment.slot == slot
                && deployment.commit == commit
                && deployment.state == DeploymentState::Building
        });
        match building {
            Some(deployment) => {
                deployment.state = state;
                Ok(self)
            }
            None => Err(StaleReport { slot, commit }),
        }
    }
}
