//! The deployer's previews (docs/plans/self-build.md, phase 9): one pass
//! asks `ren_gateway::deployment::Deployments` which previews should run, from the
//! commits the operator asked for and Ren's open pull requests as
//! production last saw them on GitHub, and carries out what it says: each
//! start fetches the commit, builds it once (`crate::builds`) in the
//! deployer's previews worktree, and starts it on a fresh copy of
//! production's history in its slot's pod; each stop removes a slot's pod.
//! What happened is written to production's data folder after every step,
//! where the gateway serves it.

use super::{CanaryError, Deployer, PublishArgs};
use crate::{
    progress::Progress,
    replica::{ListedPull, ProductionPlace, Replica},
};
use ren_gateway::api::control::request::{PreviewWorkRequest, WorkChange};
use ren_gateway::api::read::work::WorkPreviewBody;
use ren_gateway::deployment::{
    Action, Deployment, Deployments, Event, Reach, Seed, Slot, Trigger, Wanted,
};
use ren_git::{Commit, SourceRevision, Tree};
use ren_harness::host::HostEnv;
use ren_protocol::label::Stored;
use ren_recorder::id::WorkItemId;
use std::path::PathBuf;

/// One previews pass, against `production`.
pub(super) struct PreviewPass<'a> {
    pub deployer: &'a Deployer,
    pub production: &'a ProductionPlace,
    pub publish: PublishArgs,
    /// The project Ren is registered as, whose pull requests are previewed.
    pub project: &'a str,
}

impl PreviewPass<'_> {
    /// Runs the pass; returns the commits its previews run, so their builds
    /// are kept.
    pub fn run(&self, env: &HostEnv) -> Result<Vec<Commit>, CanaryError> {
        self.run_planned(env, || {})
    }

    /// Runs the pass, calling `planned` once it has written which previews
    /// run, before it builds and starts any.
    pub fn run_planned(
        &self,
        env: &HostEnv,
        planned: impl FnOnce(),
    ) -> Result<Vec<Commit>, CanaryError> {
        let capacity = self.deployer.options()?.previews;
        let before = self.production.previews();
        let wanted = self.wanted(&before)?;
        let (mut instances, actions) = before
            .clone()
            .step(Event::Wanted { wanted, capacity })
            .map_err(|stale| CanaryError::Preview(stale.to_string()))?;
        self.production.write_previews(&instances)?;
        planned();
        // A work item no deployment previews any longer has its record
        // taken off: its deployment stopped, or was chosen without it.
        for gone in before.deployments() {
            let kept = instances
                .of(&gone.trigger)
                .map_or(&[][..], |now| now.work.as_slice());
            let left: Vec<WorkItemId> = gone
                .work
                .iter()
                .filter(|item| !kept.contains(item))
                .copied()
                .collect();
            self.record(&left, None);
        }
        let tailscale = self.deployer.tailscale(self.publish)?;
        let mut backup: Option<PathBuf> = None;
        for action in actions {
            match action {
                Action::Stop(slot) => {
                    Progress(&format!("Stopping the preview in slot {}", slot.index())).show();
                    self.stop(env, slot, tailscale)?;
                }
                Action::Start {
                    slot,
                    trigger,
                    commit,
                    seed,
                } => {
                    Progress(&format!(
                        "Previewing {} ({}, on {seed}) in slot {}",
                        commit.short(),
                        Self::describe(&trigger),
                        slot.index()
                    ))
                    .show();
                    let data = match seed {
                        Seed::Copy => Data::Copy(match &backup {
                            Some(path) => path.clone(),
                            None => backup.insert(self.production.backup()?).clone(),
                        }),
                        Seed::Invented(number) => Data::Invented(number),
                    };
                    let event = match self.start(env, slot, &commit, &data, tailscale) {
                        Ok(ui) => {
                            let work = instances
                                .deployments()
                                .iter()
                                .find(|d| d.slot == slot)
                                .map(|d: &Deployment| d.work.clone())
                                .unwrap_or_default();
                            self.record(&work, Some((&ui, &commit)));
                            Event::Started {
                                slot,
                                commit,
                                reach: if tailscale.is_some() {
                                    Reach::Tailnet
                                } else {
                                    Reach::Loopback
                                },
                            }
                        }
                        Err(error) => {
                            println!("warning: the preview of {} failed: {error}", commit.short());
                            // What ran in the slot before is not this
                            // preview: it never outlives the failure.
                            self.stop(env, slot, tailscale)?;
                            Event::Failed {
                                slot,
                                commit,
                                detail: error.to_string(),
                            }
                        }
                    };
                    // A start reports no further actions.
                    let (after, _) = instances
                        .step(event)
                        .map_err(|stale| CanaryError::Preview(stale.to_string()))?;
                    instances = after;
                    self.production.write_previews(&instances)?;
                }
            }
        }
        Ok(instances
            .deployments()
            .iter()
            .map(|instance| instance.commit.clone())
            .collect())
    }

    /// What should run, first first: the operator's asks, newest first,
    /// then each open pull request that is not a draft, most recently
    /// updated first. While production has not seen the pull requests, the
    /// pull requests previewed now are kept as they are, never taken as
    /// closed.
    fn wanted(&self, current: &Deployments) -> Result<Vec<Wanted>, CanaryError> {
        let asked = self
            .production
            .preview_requests()
            .chosen()
            .iter()
            .map(|chosen| Wanted {
                trigger: Trigger::Chosen(chosen.commit.clone()),
                commit: chosen.commit.clone(),
                seed: chosen.seed,
                work: chosen.work.clone(),
            })
            .collect::<Vec<_>>();
        let pulls: Vec<Wanted> = match self.production.pulls(self.project)? {
            Some(pulls) => pulls.into_iter().filter_map(Self::pull).collect(),
            None => current
                .deployments()
                .iter()
                .filter(|instance| matches!(instance.trigger, Trigger::FollowsPull(_)))
                .map(|instance| Wanted {
                    trigger: instance.trigger.clone(),
                    commit: instance.commit.clone(),
                    seed: instance.seed,
                    work: instance.work.clone(),
                })
                .collect(),
        };
        Ok(asked.into_iter().chain(pulls).collect())
    }

    /// What a pull request wants ([`Wanted::pull`]); one whose readiness
    /// this build does not know wants nothing.
    fn pull(pull: ListedPull) -> Option<Wanted> {
        match pull.readiness {
            Stored::Known(readiness) => {
                Wanted::pull(pull.number.to_string().parse().ok()?, pull.head, readiness)
            }
            Stored::Unknown(_) => None,
        }
    }

    fn describe(trigger: &Trigger) -> String {
        match trigger {
            Trigger::FollowsPull(number) => format!("pull request #{number}"),
            Trigger::Chosen(_) => "asked for".to_owned(),
        }
    }

    /// Stops whatever runs in `slot`, and removes its routes.
    fn stop(
        &self,
        env: &HostEnv,
        slot: Slot,
        tailscale: Option<crate::tailscale::Tailnet>,
    ) -> Result<(), CanaryError> {
        let replica = Replica::preview_slot(env, slot)?;
        replica.stop()?;
        if let Some(tailscale) = tailscale {
            replica.unpublish(tailscale, slot.published())?;
        }
        Ok(())
    }

    /// Records the preview at `ui` of `commit` on each of `work`, or that
    /// there is none any longer. A record that fails is a warning: the
    /// deployment stands either way.
    fn record(&self, work: &[WorkItemId], preview: Option<(&str, &Commit)>) {
        let preview = match preview {
            None => None,
            Some((ui, commit)) => match ui.parse() {
                Ok(ui) => Some(WorkPreviewBody {
                    ui,
                    revision: SourceRevision::Commit {
                        commit: commit.clone(),
                        tree: Tree::Clean,
                    },
                }),
                Err(_) => {
                    println!("warning: {ui} is not an address a work item keeps");
                    return;
                }
            },
        };
        for item in work {
            let change = WorkChange::Preview(PreviewWorkRequest {
                preview: preview.clone(),
            });
            if let Err(error) = self.production.move_work(*item, change) {
                println!("warning: the preview was not recorded on work item {item}: {error}");
            }
        }
    }

    /// Fetches, builds, and starts `commit` in `slot` on `data`; answers
    /// its UI's address.
    fn start(
        &self,
        env: &HostEnv,
        slot: Slot,
        commit: &Commit,
        data: &Data,
        tailscale: Option<crate::tailscale::Tailnet>,
    ) -> Result<String, CanaryError> {
        let deployer = self.deployer;
        // A commit this host already holds (an agent's, in a worktree of
        // the same repository) needs no fetch, and may be on no remote.
        if !deployer.git.has_commit(&deployer.repo, commit)? {
            deployer
                .git
                .fetch_commit(&deployer.repo, Deployer::REMOTE, commit)?;
        }
        let worktree = deployer.home.join(".local/share/ren/preview-factory");
        deployer
            .git
            .check_out_detached(&deployer.repo, &worktree, commit)?;
        let build = deployer.builds.make(env, &worktree, commit)?;
        let replica = Replica::preview_slot(env, slot)?;
        match data {
            Data::Copy(backup) => replica.replace_data(backup)?,
            Data::Invented(_) => replica.empty_data()?,
        }
        let keys = replica.configure(self.production)?;
        if let Data::Invented(number) = data {
            replica.seed(&build.ren(), *number)?;
        }
        let publish = tailscale.map(|tailscale| {
            let (gateway, ui) = slot.published();
            (tailscale, gateway, ui)
        });
        let reached = replica.start(&deployer.repo, &build.ren(), &build.ui(), &keys, publish)?;
        println!("Deployment of {}: {}", commit.short(), reached.ui);
        Ok(reached.ui)
    }
}

/// What a deployment's data starts as, ready to write: a backup of
/// production's, or the number a history is invented from.
enum Data {
    Copy(PathBuf),
    Invented(ren_gateway::deployment::SeedNumber),
}
