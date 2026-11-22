//! A project's canary and promotion, as Ren's own (`cargo xtask canary`)
//! is for Ren (docs/plans/self-build.md, phase 6).
//!
//! A pass fetches the project's `main`, checks its tip out, detached, in a
//! worktree the factory keeps for the project (a folder in Ren's
//! worktrees folder, so its slot's ports are its own), and runs its
//! contract there: the build steps, the serve step until it answers its
//! health path, then each declared check, against it. The verdict, the build
//! production runs (the last one deployed), and what the operator's policy
//! decides make the project's release, kept in `<data>/projects/<name>`.
//! When the policy promotes, the contract's deploy step runs in the checked
//! build; a deploy that fails is marked and not tried again on its own.
//! A package the contract names that cannot be had fails the build check.
//! Each pass, and each promotion, closes the project's work items as Ren's
//! canary closes Ren's: landed from `main`'s trailers, shipped by the build
//! production runs.

use super::{ProjectCmd, Steps};
use crate::commands::{control::ControlTarget, history::TargetArgs};
use crate::context::Context;
use anyhow::{Context as _, Result, anyhow, bail};
use ren_daemon::host::SystemClock;
use ren_gateway::api::release::{ReleaseBody, ReleaseView};
use ren_gateway::api::{
    control::request::{ControlRequest, LandWorkRequest, ShipWorkRequest, WorkChange},
    read::ReadBody,
};
use ren_git::{Branch, CommitId, Git};
use ren_project::{Place, ProjectContract};
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{Landing, WorkStateKind},
};
use ren_recorder::{project::Factory, time::Clock as _};
use ren_release::{
    Assessment, Attempt, Canary, Check, CheckKind, Commit, Decision, Lineage, MinuteOfDay, Outcome,
    Release, ReleaseOptions, Revision, Tree, Verdict,
};
use ren_store::view::ProjectView;
use ren_store::{
    query::{PageLimit, Shelf},
    read::ReadRequest,
};
use ren_supervisor::{Slot, Worktree};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, process::ExitCode};

/// One project's factory: its repository, its canary worktree, and the
/// folder its release is kept in.
pub(super) struct ProjectFactory {
    /// The repository's folder name, which names the project.
    name: String,
    repo: PathBuf,
    /// Ren's worktrees folder, where the canary's worktree is made.
    worktrees: PathBuf,
    /// `<data>/projects/<name>`.
    state: PathBuf,
    /// `~/.config/ren/release.toml`.
    options: PathBuf,
    git: Git,
}

/// Which worktree is the project's canary, kept between passes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanaryRecord {
    /// Read under its old name `workspace` too, for records written before
    /// the rename.
    #[serde(alias = "workspace")]
    worktree: String,
}

impl Steps {
    /// One step's outcome: a step that cannot expand or start fails, as one
    /// that exits unsuccessfully does.
    fn outcome(&self, step: &ren_project::Step, place: &Place) -> Outcome {
        let ran = step
            .expand(place)
            .map_err(anyhow::Error::from)
            .and_then(|step| {
                println!("==> {}", step.argv.join(" "));
                self.run(&step, &place.worktree)
                    .map(|succeeded| (succeeded, step))
            });
        match ran {
            Ok((true, _)) => Outcome::Passed,
            Ok((false, step)) => Outcome::Failed {
                detail: format!("`{}` failed", step.argv.join(" ")),
            },
            Err(error) => Outcome::Failed {
                detail: format!("{error:#}"),
            },
        }
    }
}

impl ProjectFactory {
    const RELEASE: &str = "release.json";
    const CANARY: &str = "canary.json";

    /// The factory of `project`, a registered one built by its contract.
    pub(super) fn new(ctx: &Context, project: &ProjectView) -> Result<Self> {
        match project.factory {
            Stored::Known(Factory::Contract) => {}
            Stored::Known(Factory::Ren) => bail!(
                "{} is Ren itself: its canary is `cargo xtask canary`, and `install-vps` promotes it",
                project.name
            ),
            Stored::Unknown(ref label) => bail!(
                "{} is built by {}, which this ren does not know",
                project.name,
                label.as_str()
            ),
        }
        let repo = project.repository.as_path().to_path_buf();
        let name = project.name.to_string();
        let config = ctx.config()?;
        let data = ctx.data_dir(&config)?;
        let home = ctx.env.absolute_path("HOME").context("HOME is not set")?;
        Ok(Self {
            worktrees: data.join("worktrees"),
            state: data.join(Release::PROJECTS).join(&name),
            options: home.join(".config/ren/release.toml"),
            git: Git::inherited(),
            name,
            repo,
        })
    }

    /// One pass: fetch, check out, build, serve, check, decide, and deploy
    /// when the policy says.
    pub(super) async fn pass(&self, ctx: &Context, remote: &str, branch: &str) -> Result<ExitCode> {
        let branch: Branch = branch.parse().map_err(anyhow::Error::msg)?;
        println!("==> Fetching {branch} from {remote}");
        let tip = self.git.fetch(&self.repo, remote, &branch)?;
        let worktree = self.worktree()?;
        self.git
            .check_out_detached(&self.repo, &worktree.folder(), &tip)?;
        let place = Place {
            worktree: worktree.folder(),
            data: worktree.data(),
            ports: Some(worktree.ports()),
        };
        let contract = ProjectCmd::contract(&place.worktree)?;
        let steps = Steps::of(ctx, &contract);
        let verdict = self.check(&steps, &contract, &place, &worktree).await?;
        let commit = Self::release_commit(&tip)?;
        let canary = Canary::new(commit.clone(), verdict);
        let deployed = self.read()?.map(|release| release.production().clone());
        let release = self.assess(&tip, Some(canary), deployed)?;
        self.write(&release)?;
        if release.decision() == &Stored::Known(Decision::Promote) {
            // Only a build that passed is promoted, so its steps were had.
            let steps = steps.map_err(|error| anyhow!("{error:#}"))?;
            self.deploy(&steps, &contract, &place, &commit)?;
        }
        self.close_work(ctx, &tip).await?;
        Ok(ExitCode::SUCCESS)
    }

    /// `ren project promote`: the canary's checked build, now.
    pub(super) async fn promote(&self, ctx: &Context) -> Result<ExitCode> {
        let release = self
            .read()?
            .context("no canary pass has run for this project")?;
        let canary = release.canary().context("no build was checked")?;
        if !canary.verdict().passed() {
            bail!("the canary's build failed its checks");
        }
        let commit = canary.commit().clone();
        let worktree = self.worktree()?;
        let place = Place {
            worktree: worktree.folder(),
            data: worktree.data(),
            ports: Some(worktree.ports()),
        };
        let contract = ProjectCmd::contract(&place.worktree)?;
        let steps = Steps::of(ctx, &contract)?;
        let _ = fs::remove_file(self.failed(&commit));
        self.deploy(&steps, &contract, &place, &commit)?;
        let tip: CommitId = release.main().to_string().parse()?;
        self.close_work(ctx, &tip).await?;
        Ok(ExitCode::SUCCESS)
    }

    /// `ren project release`: where it stands.
    pub(super) fn show(&self) -> Result<ExitCode> {
        let body = ReleaseBody {
            release: self.read()?.as_ref().map(ReleaseView::from),
            shots: Vec::new(),
            promotion_requested: None,
        };
        print!("{}", crate::commands::release::ReleaseCmd::render(&body));
        Ok(ExitCode::SUCCESS)
    }

    /// The project's canary worktree: the one kept, else a new one in the
    /// lowest free slot.
    fn worktree(&self) -> Result<Worktree> {
        let record = self.state.join(Self::CANARY);
        if let Some(kept) = fs::read(&record)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CanaryRecord>(&bytes).ok())
            && let Some(worktree) =
                Worktree::of(&self.worktrees, &self.worktrees.join(&kept.worktree))
        {
            return Ok(worktree);
        }
        fs::create_dir_all(&self.worktrees)?;
        let names: Vec<String> = fs::read_dir(&self.worktrees)?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        let slot = Slot::free(&Worktree::taken(names.iter().map(String::as_str)))
            .context("every worktree slot is taken")?;
        let worktree = Worktree::new(&self.worktrees, &self.repo, slot)
            .context("the project's folder has no name")?;
        fs::create_dir_all(&self.state)?;
        let name = worktree
            .folder()
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .context("the canary's folder has no name")?;
        fs::write(
            &record,
            serde_json::to_vec(&CanaryRecord { worktree: name })?,
        )?;
        Ok(worktree)
    }

    /// The project's checks of the checked-out build: its build steps, its
    /// serve step up and answering, then each declared check against it.
    /// A check that cannot run fails; steps that cannot be had (a package
    /// the contract names is lacking) fail the build.
    async fn check(
        &self,
        steps: &Result<Steps>,
        contract: &ProjectContract,
        place: &Place,
        worktree: &Worktree,
    ) -> Result<Verdict> {
        let declared: Vec<String> = contract
            .checks()
            .iter()
            .map(|check| check.name().to_owned())
            .collect();
        let mut checks = Vec::new();
        let built = match steps {
            Err(error) => Outcome::Failed {
                detail: format!("{error:#}"),
            },
            Ok(steps) => {
                let mut built = Outcome::Passed;
                for step in contract.build() {
                    built = steps.outcome(step, place);
                    if !matches!(built, Outcome::Passed) {
                        break;
                    }
                }
                built
            }
        };
        let built_ok = matches!(built, Outcome::Passed);
        checks.push(Check::new(CheckKind::Build, built));
        let serving = match (steps, built_ok, contract.serve()) {
            (Err(_), _, _) | (_, false, _) => Outcome::Failed {
                detail: "not run: the build failed".to_owned(),
            },
            (Ok(_), true, None) => Outcome::Failed {
                detail: format!("{} declares no [serve] step", ProjectContract::FILE),
            },
            (Ok(steps), true, Some(serve)) => Self::serve(steps, serve, place, worktree)
                .await
                .map_or_else(
                    |error| Outcome::Failed {
                        detail: format!("{error:#}"),
                    },
                    |()| Outcome::Passed,
                ),
        };
        let served = matches!(serving, Outcome::Passed);
        checks.push(Check::new(CheckKind::Serve, serving));
        for check in contract.checks() {
            let outcome = match (steps, served) {
                (Ok(steps), true) => {
                    println!("==> {}", check.name());
                    steps.outcome(check.step(), place)
                }
                (Err(_), _) | (_, false) => Outcome::Failed {
                    detail: "not run: the project did not serve".to_owned(),
                },
            };
            checks.push(Check::declared(check.name(), outcome));
        }
        for check in &checks {
            let name = check
                .name()
                .map_or_else(|| check.check().to_string(), str::to_owned);
            match check.outcome() {
                Outcome::Passed => println!("{name}: passed"),
                Outcome::Failed { detail } => println!("{name}: failed: {detail}"),
            }
        }
        Ok(Verdict::project(declared, checks))
    }

    /// Starts the serve step in the canary's worktree and waits for it.
    async fn serve(
        steps: &Steps,
        serve: &ren_project::Serve,
        place: &Place,
        worktree: &Worktree,
    ) -> Result<()> {
        let unit = ProjectCmd::unit(worktree);
        Steps::stop(&unit)?;
        let step = serve.step().expand(place)?;
        steps.start(&unit, &step, &place.worktree)?;
        ProjectCmd::healthy(&serve.open(place)?, serve.health()).await
    }

    /// Where the release stands with `main` at `tip` and production at
    /// `deployed` (the build last deployed, if any), and what the policy
    /// decides now.
    fn assess(
        &self,
        tip: &CommitId,
        canary: Option<Canary>,
        deployed: Option<Revision>,
    ) -> Result<Release> {
        let running = match &deployed {
            Some(Revision::Commit {
                commit,
                tree: Tree::Clean,
            }) => Some(commit.clone()),
            Some(Revision::Commit {
                tree: Tree::Modified,
                ..
            })
            | Some(Revision::Unknown)
            | None => None,
        };
        let running_id: Option<CommitId> = running
            .as_ref()
            .and_then(|commit| commit.to_string().parse().ok());
        let between =
            |from: &CommitId, to: &CommitId| self.git.commits_between(&self.repo, from, to).ok();
        let behind = running_id
            .as_ref()
            .and_then(|running| between(running, tip));
        let canary = canary.map(|canary| {
            let candidate: Option<CommitId> = canary.commit().to_string().parse().ok();
            let lacks = match (&candidate, &running_id) {
                (Some(candidate), Some(running)) => between(candidate, running),
                (None, _) | (_, None) => None,
            };
            let lineage = Lineage::read(running.as_ref(), canary.commit(), lacks);
            let attempt = if self.failed(canary.commit()).exists() {
                Attempt::Failed
            } else {
                Attempt::Untried
            };
            (canary, lineage, attempt)
        });
        let policy = self.options()?.project(&self.name);
        let now = SystemClock::start().now().wall.as_millis();
        Ok(Release::assess(
            Assessment {
                main: Self::release_commit(tip)?,
                production: deployed.unwrap_or(Revision::Unknown),
                behind,
                canary,
            },
            policy,
            MinuteOfDay::of_unix_ms(now),
        ))
    }

    /// Runs the deploy step in the checked build; production is then that
    /// commit. A failed deploy is marked and not tried again on its own.
    fn deploy(
        &self,
        steps: &Steps,
        contract: &ProjectContract,
        place: &Place,
        commit: &Commit,
    ) -> Result<()> {
        let Some(deploy) = contract.deploy() else {
            println!(
                "{} declares no [deploy] step: nothing is promoted.",
                ProjectContract::FILE
            );
            return Ok(());
        };
        let step = deploy.expand(place)?;
        println!("==> Deploying {}: {}", commit.short(), step.argv.join(" "));
        let deployed = steps.run(&step, &place.worktree)?;
        let release = self.read()?.context("no release to promote")?;
        let tip: CommitId = release.main().to_string().parse()?;
        let canary = release.canary().cloned();
        let production = if deployed {
            Revision::Commit {
                commit: commit.clone(),
                tree: Tree::Clean,
            }
        } else {
            fs::create_dir_all(&self.state)?;
            fs::write(self.failed(commit), b"")?;
            release.production().clone()
        };
        self.write(&self.assess(&tip, canary, Some(production))?)?;
        if deployed {
            println!("Deployed {}.", commit.short());
            Ok(())
        } else {
            bail!("the deploy step failed; production keeps what it ran")
        }
    }

    /// Closes the project's work items from what the canary sees, as Ren's
    /// canary does for Ren: a sent item lands when a commit on `main` (from
    /// `tip`, the newest 500) names it (`Work-Item: <id>`), and a landed item
    /// ships once production runs a clean build whose history holds its
    /// commit. An item landed in this pass ships in the next, once the daemon
    /// has stored it. Each change goes through this host's daemon, which
    /// checks it against the item's state.
    async fn close_work(&self, ctx: &Context, tip: &CommitId) -> Result<()> {
        const HISTORY: u32 = 500;
        let host = TargetArgs::this_host();
        let ReadBody::WorkItems(listed) = host
            .gateway(ctx)?
            .serve(&ReadRequest::WorkItems {
                limit: PageLimit::LARGEST,
                shelf: Shelf::Current,
                project: Some(self.name.parse().map_err(anyhow::Error::msg)?),
            })
            .await?
        else {
            bail!("the daemon answered a work listing with another read");
        };
        let sent: Vec<WorkItemId> = listed
            .work
            .iter()
            .filter(|item| item.state == Stored::Known(WorkStateKind::Sent))
            .map(|item| item.id)
            .collect();
        let landed: Vec<(WorkItemId, CommitId)> = listed
            .work
            .iter()
            .filter(|item| item.state == Stored::Known(WorkStateKind::Landed))
            .filter_map(|item| Some((item.id, item.landed.as_ref()?.to_string().parse().ok()?)))
            .collect();
        let control = ControlTarget::select(&host, ctx)?;
        let send = async |item: WorkItemId, change: WorkChange| -> Result<()> {
            let answer = control.send(&ControlRequest::Work { item, change }).await?;
            if !answer.status.is_success() {
                bail!("moving work item {item} on answered {}", answer.status);
            }
            Ok(())
        };
        if !sent.is_empty() {
            let mut log = self.git.log(&self.repo, tip, HISTORY)?;
            // Oldest first: an item named twice lands as the first commit.
            log.reverse();
            let mut done = Vec::new();
            for (commit, message) in log {
                let release_commit = Self::release_commit(&commit)?;
                for item in Landing::read(release_commit.clone(), &message).items {
                    if sent.contains(&item) && !done.contains(&item) {
                        send(
                            item,
                            WorkChange::Land(LandWorkRequest {
                                commit: release_commit.clone(),
                            }),
                        )
                        .await?;
                        println!("Work item {item} landed as {}", release_commit.short());
                        done.push(item);
                    }
                }
            }
        }
        let production = match self.read()?.map(|release| release.production().clone()) {
            Some(Revision::Commit {
                commit,
                tree: Tree::Clean,
            }) => commit,
            Some(Revision::Commit {
                tree: Tree::Modified,
                ..
            })
            | Some(Revision::Unknown)
            | None => return Ok(()),
        };
        let running: CommitId = production.to_string().parse()?;
        for (item, commit) in &landed {
            if self.git.contains(&self.repo, &running, commit)? {
                send(
                    *item,
                    WorkChange::Ship(ShipWorkRequest {
                        revision: Revision::Commit {
                            commit: production.clone(),
                            tree: Tree::Clean,
                        },
                    }),
                )
                .await?;
                println!("Work item {item} shipped in {}", production.short());
            }
        }
        Ok(())
    }

    fn failed(&self, commit: &Commit) -> PathBuf {
        self.state.join(format!("deploy-failed-{commit}"))
    }

    fn options(&self) -> Result<ReleaseOptions> {
        match fs::read_to_string(&self.options) {
            Ok(text) => Ok(toml::from_str(&text)
                .with_context(|| format!("{} does not read", self.options.display()))?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ReleaseOptions::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn read(&self) -> Result<Option<Release>> {
        match fs::read(self.state.join(Self::RELEASE)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write(&self, release: &Release) -> Result<()> {
        fs::create_dir_all(&self.state)?;
        let path = self.state.join(Self::RELEASE);
        fs::write(&path, serde_json::to_vec_pretty(release)?)?;
        println!("Release: {}", release.decision());
        Ok(())
    }

    fn release_commit(id: &CommitId) -> Result<Commit> {
        id.as_str()
            .parse()
            .with_context(|| format!("{id} is not a commit"))
    }
}

#[cfg(test)]
mod tests {
    use super::CanaryRecord;

    #[test]
    fn a_record_written_before_the_rename_reads() {
        for record in [r#"{"workspace":"acme-2"}"#, r#"{"worktree":"acme-2"}"#] {
            let read: CanaryRecord = serde_json::from_str(record).unwrap();
            assert_eq!(read.worktree, "acme-2", "{record}");
        }
        assert_eq!(
            serde_json::to_string(&CanaryRecord {
                worktree: "acme-2".to_owned()
            })
            .unwrap(),
            r#"{"worktree":"acme-2"}"#
        );
        assert!(serde_json::from_str::<CanaryRecord>(r#"{"worktree":"a","other":1}"#).is_err());
    }
}
