//! The factory's previews (docs/plans/self-build.md, phase 9): one pass
//! asks `ren_gateway::preview::Previews` which previews should run, from the
//! commits the operator asked for and Ren's open pull requests as
//! production last saw them on GitHub, and carries out what it says: each
//! start fetches the commit, builds it once (`crate::builds`) in the
//! factory's previews worktree, and starts it on a fresh copy of
//! production's history in its slot's pod; each stop removes a slot's pod.
//! What happened is written to production's data folder after every step,
//! where the gateway serves it.

use super::{CanaryError, Factory, PublishArgs};
use crate::{
    progress::Progress,
    replica::{ListedPull, ProductionPlace, Replica},
};
use ren_gateway::preview::{Action, PreviewKey, Previews, Reach, Slot, Wanted};
use ren_git::Commit;
use ren_github::Readiness;
use ren_harness::host::HostEnv;
use ren_protocol::label::Stored;
use std::path::PathBuf;

/// One previews pass, against `production`.
pub(super) struct PreviewPass<'a> {
    pub factory: &'a Factory,
    pub production: &'a ProductionPlace,
    pub publish: PublishArgs,
    /// The project Ren is registered as, whose pull requests are previewed.
    pub project: &'a str,
}

impl PreviewPass<'_> {
    /// Runs the pass; returns the commits its previews run, so their builds
    /// are kept.
    pub fn run(&self, env: &HostEnv) -> Result<Vec<Commit>, CanaryError> {
        let capacity = self.factory.options()?.previews;
        let before = self.production.previews();
        let wanted = self.wanted(&before)?;
        let (mut previews, actions) = before.want(&wanted, capacity);
        self.production.write_previews(&previews)?;
        let tailscale = self.factory.tailscale(self.publish)?;
        let mut backup: Option<PathBuf> = None;
        for action in actions {
            match action {
                Action::Stop(slot) => {
                    Progress(&format!("Stopping the preview in slot {}", slot.index())).show();
                    self.stop(env, slot, tailscale)?;
                }
                Action::Start { slot, key, commit } => {
                    Progress(&format!(
                        "Previewing {} ({}) in slot {}",
                        commit.short(),
                        Self::describe(&key),
                        slot.index()
                    ))
                    .show();
                    let backup = match &backup {
                        Some(path) => path.clone(),
                        None => backup.insert(self.production.backup()?).clone(),
                    };
                    let started = self.start(env, slot, &commit, &backup, tailscale);
                    let report = match started {
                        Ok(()) => previews.started(
                            slot,
                            &commit,
                            if tailscale.is_some() {
                                Reach::Tailnet
                            } else {
                                Reach::Loopback
                            },
                        ),
                        Err(error) => {
                            println!("warning: the preview of {} failed: {error}", commit.short());
                            // What ran in the slot before is not this
                            // preview: it never outlives the failure.
                            self.stop(env, slot, tailscale)?;
                            previews.failed(slot, &commit, error.to_string())
                        }
                    };
                    report.map_err(|stale| CanaryError::Preview(stale.to_string()))?;
                    self.production.write_previews(&previews)?;
                }
            }
        }
        Ok(previews
            .previews()
            .iter()
            .map(|preview| preview.commit.clone())
            .collect())
    }

    /// What should run, first first: the operator's asks, newest first,
    /// then each open pull request that is not a draft, most recently
    /// updated first. While production has not seen the pull requests, the
    /// pull requests previewed now are kept as they are, never taken as
    /// closed.
    fn wanted(&self, current: &Previews) -> Result<Vec<Wanted>, CanaryError> {
        let asked = self
            .production
            .preview_requests()
            .commits()
            .iter()
            .map(|commit| Wanted {
                key: PreviewKey::Commit(commit.clone()),
                commit: commit.clone(),
            })
            .collect::<Vec<_>>();
        let pulls: Vec<Wanted> = match self.production.pulls(self.project)? {
            Some(pulls) => pulls.into_iter().filter_map(Self::pull).collect(),
            None => current
                .previews()
                .iter()
                .filter(|preview| matches!(preview.key, PreviewKey::Pull(_)))
                .map(|preview| Wanted {
                    key: preview.key.clone(),
                    commit: preview.commit.clone(),
                })
                .collect(),
        };
        Ok(asked.into_iter().chain(pulls).collect())
    }

    /// A pull request that wants a preview: one that is not a draft.
    fn pull(pull: ListedPull) -> Option<Wanted> {
        match pull.readiness {
            Stored::Known(Readiness::Ready) => Some(Wanted {
                key: PreviewKey::Pull(pull.number.to_string().parse().ok()?),
                commit: pull.head,
            }),
            Stored::Known(Readiness::Draft) | Stored::Unknown(_) => None,
        }
    }

    fn describe(key: &PreviewKey) -> String {
        match key {
            PreviewKey::Pull(number) => format!("pull request #{number}"),
            PreviewKey::Commit(_) => "asked for".to_owned(),
        }
    }

    /// Stops whatever runs in `slot`, and removes its routes.
    fn stop(
        &self,
        env: &HostEnv,
        slot: Slot,
        tailscale: Option<crate::tailscale::TailscaleCli>,
    ) -> Result<(), CanaryError> {
        let replica = Replica::preview_slot(env, slot)?;
        replica.stop()?;
        if let Some(tailscale) = tailscale {
            replica.unpublish(tailscale, slot.published())?;
        }
        Ok(())
    }

    /// Fetches, builds, and starts `commit` in `slot` on `backup`'s history.
    fn start(
        &self,
        env: &HostEnv,
        slot: Slot,
        commit: &Commit,
        backup: &std::path::Path,
        tailscale: Option<crate::tailscale::TailscaleCli>,
    ) -> Result<(), CanaryError> {
        let factory = self.factory;
        factory
            .git
            .fetch_commit(&factory.repo, Factory::REMOTE, commit)?;
        let worktree = factory.home.join(".local/share/ren/preview-factory");
        factory
            .git
            .check_out_detached(&factory.repo, &worktree, commit)?;
        let build = factory
            .builds
            .make(env, &factory.cargo, &worktree, commit)?;
        let replica = Replica::preview_slot(env, slot)?;
        replica.replace_data(backup)?;
        let keys = replica.configure(self.production)?;
        let publish = tailscale.map(|tailscale| {
            let (gateway, ui) = slot.published();
            (tailscale, gateway, ui)
        });
        let reached = replica.start(&factory.repo, &build.ren(), &build.ui(), &keys, publish)?;
        println!("Preview of {}: {}", commit.short(), reached.ui);
        Ok(())
    }
}
