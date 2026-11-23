//! The previews in their slots, and the pure steps that move them on.

use super::slot::Slot;
use ren_github::PullNumber;
use ren_release::{Commit, PreviewCapacity};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// The previews the factory runs, each in its slot. Kept by the factory as
/// a file in production's data folder ([`Previews::FILE`]), which the
/// gateway reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Previews {
    /// By slot, each slot at most once.
    previews: Vec<Preview>,
}

/// One preview: what it is of, the commit it runs, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub slot: Slot,
    pub key: PreviewKey,
    pub commit: Commit,
    pub state: PreviewState,
}

/// What a preview is of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKey {
    /// An open pull request, at its head.
    Pull(PullNumber),
    /// A commit the operator asked for.
    Commit(Commit),
}

/// Where a preview stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PreviewState {
    /// Its commit is being built and started.
    Building,
    /// It answers on its slot's ports, reached as `reach` says.
    Running { reach: Reach },
    /// Building or starting it failed; not tried again for this commit.
    Failed { detail: String },
}

/// Where a running preview is reached from.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Reach {
    /// The host's loopback ports alone.
    Loopback,
    /// The tailnet as well, on its slot's published ports.
    Tailnet,
}

/// A preview that should run: the commit for its key, in priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub key: PreviewKey,
    pub commit: Commit,
}

/// What the factory does next, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Stop whatever runs in the slot.
    Stop(Slot),
    /// Build the commit and start it in the slot, in place of whatever
    /// runs there; report back with [`Previews::started`] or
    /// [`Previews::failed`].
    Start {
        slot: Slot,
        key: PreviewKey,
        commit: Commit,
    },
}

/// A report about a slot that is not building that commit: a pass raced
/// another, or reported twice.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("slot {} is not building {commit}", slot.index())]
pub struct StaleReport {
    pub slot: Slot,
    pub commit: Commit,
}

impl Previews {
    /// Its file in production's data folder.
    pub const FILE: &str = "previews.json";

    pub fn previews(&self) -> &[Preview] {
        &self.previews
    }

    /// The preview of `key`, if one holds a slot.
    pub fn of(&self, key: &PreviewKey) -> Option<&Preview> {
        self.previews.iter().find(|preview| &preview.key == key)
    }

    /// The previews that should run now that `wanted` (in priority order)
    /// is wanted, and what to do to get there. The first wants, as many as
    /// `capacity` allows, hold slots; a key wanted twice counts once. A
    /// preview whose key is still wanted keeps its slot: unchanged when it
    /// runs or failed at the same commit, started again when its build was
    /// left unfinished or its key wants another commit. One no longer
    /// wanted is stopped; a new one takes the lowest free slot.
    #[must_use]
    pub fn want(self, wanted: &[Wanted], capacity: PreviewCapacity) -> (Self, Vec<Action>) {
        let mut chosen: Vec<&Wanted> = Vec::new();
        for want in wanted {
            if chosen.len() == capacity.get() {
                break;
            }
            if !chosen.iter().any(|kept| kept.key == want.key) {
                chosen.push(want);
            }
        }
        let slots: Vec<Slot> = Slot::first(capacity.get()).collect();
        let mut stops = Vec::new();
        let mut starts = Vec::new();
        let mut kept: Vec<Preview> = Vec::new();
        for preview in self.previews {
            let still = chosen.iter().find(|want| want.key == preview.key);
            match still {
                Some(want) if slots.contains(&preview.slot) => {
                    let current = want.commit == preview.commit;
                    match (&preview.state, current) {
                        (PreviewState::Running { .. } | PreviewState::Failed { .. }, true) => {
                            kept.push(preview);
                        }
                        (PreviewState::Building, true)
                        | (
                            PreviewState::Running { .. }
                            | PreviewState::Building
                            | PreviewState::Failed { .. },
                            false,
                        ) => {
                            starts.push(Action::Start {
                                slot: preview.slot,
                                key: want.key.clone(),
                                commit: want.commit.clone(),
                            });
                            kept.push(Preview {
                                commit: want.commit.clone(),
                                state: PreviewState::Building,
                                ..preview
                            });
                        }
                    }
                }
                Some(_) | None => stops.push(Action::Stop(preview.slot)),
            }
        }
        for want in chosen {
            if kept.iter().any(|preview| preview.key == want.key) {
                continue;
            }
            let Some(slot) = slots
                .iter()
                .copied()
                .find(|slot| kept.iter().all(|preview| preview.slot != *slot))
            else {
                break;
            };
            // A stopped slot taken again is replaced by its start.
            stops.retain(|stop| stop != &Action::Stop(slot));
            starts.push(Action::Start {
                slot,
                key: want.key.clone(),
                commit: want.commit.clone(),
            });
            kept.push(Preview {
                slot,
                key: want.key.clone(),
                commit: want.commit.clone(),
                state: PreviewState::Building,
            });
        }
        kept.sort_by_key(|preview| preview.slot);
        stops.extend(starts);
        (Self { previews: kept }, stops)
    }

    /// After the factory started `commit` in `slot`, reached as `reach`.
    pub fn started(
        &mut self,
        slot: Slot,
        commit: &Commit,
        reach: Reach,
    ) -> Result<(), StaleReport> {
        *self.building(slot, commit)? = PreviewState::Running { reach };
        Ok(())
    }

    /// After building or starting `commit` in `slot` failed.
    pub fn failed(
        &mut self,
        slot: Slot,
        commit: &Commit,
        detail: String,
    ) -> Result<(), StaleReport> {
        *self.building(slot, commit)? = PreviewState::Failed { detail };
        Ok(())
    }

    fn building(&mut self, slot: Slot, commit: &Commit) -> Result<&mut PreviewState, StaleReport> {
        self.previews
            .iter_mut()
            .find(|preview| {
                preview.slot == slot
                    && &preview.commit == commit
                    && preview.state == PreviewState::Building
            })
            .map(|preview| &mut preview.state)
            .ok_or_else(|| StaleReport {
                slot,
                commit: commit.clone(),
            })
    }
}
