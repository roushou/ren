//! A project's canary and promotion, as Ren's own (`cargo xtask canary`)
//! is for Ren (docs/plans/self-build.md, phase 6).
//!
//! A pass fetches the project's `main`, checks its tip out, detached, in a
//! workspace the factory keeps for the project (a folder in Ren's
//! worktrees folder, so its slot's ports are its own), and runs its
//! contract there: the build steps, the serve step until it answers its
//! health path, then each declared check, against it. The verdict, the build
//! production runs (the last one deployed), and what the operator's policy
//! decides make the project's release, kept in `<data>/projects/<name>`.
//! When the policy promotes, the contract's deploy step runs in the checked
//! build; a deploy that fails is marked and not tried again on its own.
//! A tool the contract names that cannot be had fails the build check.

use super::{ProjectCmd, Steps};
use crate::context::Context;
use anyhow::{Context as _, Result, anyhow, bail};
use ren_daemon::host::SystemClock;
use ren_gateway::api::release::{ReleaseBody, ReleaseView};
use ren_git::{Branch, CommitId, Git};
use ren_project::{Place, ProjectContract};
use ren_protocol::label::Stored;
use ren_recorder::time::Clock as _;
use ren_release::{
    Assessment, Attempt, Canary, Check, CheckKind, Commit, Decision, Lineage, MinuteOfDay, Outcome,
    Release, ReleaseOptions, Revision, Tree, Verdict,
};
use ren_supervisor::{Slot, Workspace};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, process::ExitCode};

/// One project's factory: its repository, its canary workspace, and the
/// folder its release is kept in.
pub(super) struct ProjectFactory {
    /// The repository's folder name, which names the project.
    name: String,
    repo: PathBuf,
    /// Ren's worktrees folder, where the canary's workspace is made.
    worktrees: PathBuf,
    /// `<data>/projects/<name>`.
    state: PathBuf,
    /// `~/.config/ren/release.toml`.
    options: PathBuf,
    git: Git,
}

/// Which workspace is the project's canary, kept between passes.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanaryRecord {
    workspace: String,
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
                self.run(&step, &place.workspace)
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

    pub(super) fn new(ctx: &Context, dir: Option<PathBuf>) -> Result<Self> {
        let repo = match dir {
            Some(dir) => dir,
            None => std::env::current_dir()?,
        };
        let repo =
            fs::canonicalize(&repo).with_context(|| format!("no folder {}", repo.display()))?;
        let name = repo
            .file_name()
            .and_then(|name| name.to_str())
            .context("the project's folder has no name")?
            .to_owned();
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
        let workspace = self.workspace()?;
        self.git
            .check_out_detached(&self.repo, &workspace.folder(), &tip)?;
        let place = Place {
            workspace: workspace.folder(),
            data: workspace.data(),
            ports: Some(workspace.ports()),
        };
        let contract = ProjectCmd::contract(&place.workspace)?;
        let steps = Steps::of(ctx, &contract);
        let verdict = self.check(&steps, &contract, &place, &workspace).await?;
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
        Ok(ExitCode::SUCCESS)
    }

    /// `ren project promote`: the canary's checked build, now.
    pub(super) fn promote(&self, ctx: &Context) -> Result<ExitCode> {
        let release = self
            .read()?
            .context("no canary pass has run for this project")?;
        let canary = release.canary().context("no build was checked")?;
        if !canary.verdict().passed() {
            bail!("the canary's build failed its checks");
        }
        let commit = canary.commit().clone();
        let workspace = self.workspace()?;
        let place = Place {
            workspace: workspace.folder(),
            data: workspace.data(),
            ports: Some(workspace.ports()),
        };
        let contract = ProjectCmd::contract(&place.workspace)?;
        let steps = Steps::of(ctx, &contract)?;
        let _ = fs::remove_file(self.failed(&commit));
        self.deploy(&steps, &contract, &place, &commit)?;
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

    /// The project's canary workspace: the one kept, else a new one in the
    /// lowest free slot.
    fn workspace(&self) -> Result<Workspace> {
        let record = self.state.join(Self::CANARY);
        if let Some(kept) = fs::read(&record)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CanaryRecord>(&bytes).ok())
            && let Some(workspace) =
                Workspace::of(&self.worktrees, &self.worktrees.join(&kept.workspace))
        {
            return Ok(workspace);
        }
        fs::create_dir_all(&self.worktrees)?;
        let names: Vec<String> = fs::read_dir(&self.worktrees)?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        let slot = Slot::free(&Workspace::taken(names.iter().map(String::as_str)))
            .context("every workspace slot is taken")?;
        let workspace = Workspace::new(&self.worktrees, &self.repo, slot)
            .context("the project's folder has no name")?;
        fs::create_dir_all(&self.state)?;
        let name = workspace
            .folder()
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .context("the canary's folder has no name")?;
        fs::write(
            &record,
            serde_json::to_vec(&CanaryRecord { workspace: name })?,
        )?;
        Ok(workspace)
    }

    /// The project's checks of the checked-out build: its build steps, its
    /// serve step up and answering, then each declared check against it.
    /// A check that cannot run fails; steps that cannot be had (a tool
    /// the contract names is lacking) fail the build.
    async fn check(
        &self,
        steps: &Result<Steps>,
        contract: &ProjectContract,
        place: &Place,
        workspace: &Workspace,
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
            (Ok(steps), true, Some(serve)) => Self::serve(steps, serve, place, workspace)
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

    /// Starts the serve step in the canary's workspace and waits for it.
    async fn serve(
        steps: &Steps,
        serve: &ren_project::Serve,
        place: &Place,
        workspace: &Workspace,
    ) -> Result<()> {
        let unit = ProjectCmd::unit(workspace);
        Steps::stop(&unit)?;
        let step = serve.step().expand(place)?;
        steps.start(&unit, &step, &place.workspace)?;
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
        let deployed = steps.run(&step, &place.workspace)?;
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
