//! The instances the factory runs in their slots, and the one pure step that
//! moves them on: `(Instances, Event) -> (Instances, Vec<Action>)`.

use super::slot::Slot;
use ren_git::Commit;
use ren_github::{PullNumber, Readiness};
use ren_release::PreviewCapacity;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// The instances the factory runs, each in its slot: today, the previews
/// of Ren. Kept by the factory as a file in production's data folder
/// ([`Instances::FILE`]), which production's gateway reads. The factory and
/// that gateway are different builds (the canary's and the promoted one),
/// so the file keeps its shape: its keys are the ones previews had.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instances {
    /// By slot, each slot at most once.
    #[serde(rename = "previews")]
    instances: Vec<Instance>,
}

/// One instance: what it is of, the commit it runs, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instance {
    pub slot: Slot,
    #[serde(rename = "key")]
    pub source: Source,
    pub commit: Commit,
    pub state: InstanceState,
}

/// What an instance is of, and so which commit it runs: an open pull
/// request's head, followed while it is open, or a commit the operator
/// asked for, until the ask is withdrawn. Each is run on a copy of
/// production's history, every operator key a placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Pull(PullNumber),
    Commit(Commit),
}

/// Where an instance stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InstanceState {
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

/// Where a running instance is reached from.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Reach {
    /// The host's loopback ports alone.
    Loopback,
    /// The tailnet as well, on its slot's published ports.
    Tailnet,
}

/// An instance that should run: the commit for its source, in priority
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub source: Source,
    pub commit: Commit,
}

/// What moves the instances on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A pass found these wanted, first first, with room for `capacity`.
    Wanted {
        wanted: Vec<Wanted>,
        capacity: PreviewCapacity,
    },
    /// The factory started `commit` in `slot`, reached as `reach`.
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

/// What the factory does next, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Stop whatever runs in the slot.
    Stop(Slot),
    /// Build the commit and start it in the slot, in place of whatever
    /// runs there; report back with [`Event::Started`] or
    /// [`Event::Failed`].
    Start {
        slot: Slot,
        source: Source,
        commit: Commit,
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
                source: Source::Pull(number),
                commit: head,
            }),
            Readiness::Draft => None,
        }
    }
}

impl Instances {
    /// Its file in production's data folder: still named for previews,
    /// which production's gateway reads it as.
    pub const FILE: &str = "previews.json";

    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// The instance of `source`, if one holds a slot.
    pub fn of(&self, source: &Source) -> Option<&Instance> {
        self.instances
            .iter()
            .find(|instance| &instance.source == source)
    }

    /// The instances after `event`, and what to do to get there. A report
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
                .settle(slot, commit, InstanceState::Running { reach })
                .map(|instances| (instances, Vec::new())),
            Event::Failed {
                slot,
                commit,
                detail,
            } => self
                .settle(slot, commit, InstanceState::Failed { detail })
                .map(|instances| (instances, Vec::new())),
        }
    }

    /// The instances that should run now that `wanted` (in priority order)
    /// is wanted, and what to do to get there. The first wants, as many as
    /// `capacity` allows, hold slots; a source wanted twice counts once. An
    /// instance whose source is still wanted keeps its slot: unchanged when
    /// it runs or failed at the same commit, started again when its
    /// outcome is unknown (left building) or its source wants another
    /// commit. One no longer wanted is stopped; a new one takes the lowest
    /// free slot.
    fn want(self, wanted: &[Wanted], capacity: PreviewCapacity) -> (Self, Vec<Action>) {
        let mut chosen: Vec<&Wanted> = Vec::new();
        for want in wanted {
            if chosen.len() == capacity.get() {
                break;
            }
            if !chosen.iter().any(|kept| kept.source == want.source) {
                chosen.push(want);
            }
        }
        let slots: Vec<Slot> = Slot::first(capacity.get()).collect();
        let mut stops = Vec::new();
        let mut starts = Vec::new();
        let mut kept: Vec<Instance> = Vec::new();
        for instance in self.instances {
            let still = chosen.iter().find(|want| want.source == instance.source);
            match still {
                Some(want) if slots.contains(&instance.slot) => {
                    let current = want.commit == instance.commit;
                    match (&instance.state, current) {
                        (InstanceState::Running { .. } | InstanceState::Failed { .. }, true) => {
                            kept.push(instance);
                        }
                        (InstanceState::Building, true)
                        | (
                            InstanceState::Running { .. }
                            | InstanceState::Building
                            | InstanceState::Failed { .. },
                            false,
                        ) => {
                            starts.push(Action::Start {
                                slot: instance.slot,
                                source: want.source.clone(),
                                commit: want.commit.clone(),
                            });
                            kept.push(Instance {
                                commit: want.commit.clone(),
                                state: InstanceState::Building,
                                ..instance
                            });
                        }
                    }
                }
                Some(_) | None => stops.push(Action::Stop(instance.slot)),
            }
        }
        for want in chosen {
            if kept.iter().any(|instance| instance.source == want.source) {
                continue;
            }
            let Some(slot) = slots
                .iter()
                .copied()
                .find(|slot| kept.iter().all(|instance| instance.slot != *slot))
            else {
                break;
            };
            // A stopped slot taken again is replaced by its start.
            stops.retain(|stop| stop != &Action::Stop(slot));
            starts.push(Action::Start {
                slot,
                source: want.source.clone(),
                commit: want.commit.clone(),
            });
            kept.push(Instance {
                slot,
                source: want.source.clone(),
                commit: want.commit.clone(),
                state: InstanceState::Building,
            });
        }
        kept.sort_by_key(|instance| instance.slot);
        stops.extend(starts);
        (Self { instances: kept }, stops)
    }

    /// `slot`'s build of `commit`, settled as `state`.
    fn settle(
        mut self,
        slot: Slot,
        commit: Commit,
        state: InstanceState,
    ) -> Result<Self, StaleReport> {
        let building = self.instances.iter_mut().find(|instance| {
            instance.slot == slot
                && instance.commit == commit
                && instance.state == InstanceState::Building
        });
        match building {
            Some(instance) => {
                instance.state = state;
                Ok(self)
            }
            None => Err(StaleReport { slot, commit }),
        }
    }
}
