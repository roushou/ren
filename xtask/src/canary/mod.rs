//! `cargo xtask canary`: the factory's canary and promotion
//! (docs/plans/self-build.md, phase 4). One pass:
//!
//! 1. fetches `main` and checks its tip out in the factory's own worktree
//!    (`~/.local/share/ren/factory`), then goes on with that checkout's
//!    xtask, so a commit is checked by its own checks;
//! 2. builds the commit once (`crate::builds`);
//! 3. starts it as the canary on a fresh backup of production's history;
//! 4. checks it (`checks`): doctor, every read, every page;
//! 5. writes where the release stands to production's data folder, where
//!    `/api/release`, `ren release`, and the UI read it; and
//! 6. promotes the build under the operator's policy
//!    (`~/.config/ren/release.toml`), with `install-vps --build`'s steps.
//!
//! `enable` runs a pass on a systemd user timer.

mod checks;
mod work;

use self::{checks::Checks, work::WorkLedger};
use crate::{
    builds::{Builds, BuildsError, StoredBuild},
    dev::DevError,
    file::{FileError, FileMode},
    install::{InstallError, InstallVps, publish::Publish},
    invocation::{Invocation, InvocationError},
    progress::Progress,
    replica::{Production, ProductionPlace, Replica, ReplicaError},
    tailscale::{TailscaleCli, TailscaleError},
    ui::UiBuildError,
};
use clap::{Args, Subcommand, ValueEnum};
use ren_daemon::host::SystemClock;
use ren_git::{Branch, CommitId, Git, GitError, Tree};
use ren_harness::host::HostEnv;
use ren_protocol::label::Stored;
use ren_recorder::time::Clock as _;
use ren_release::{
    Assessment, Canary as CanaryBuild, Commit, Decision, Lineage, MinuteOfDay, Policy, Release,
    ReleaseOptions, Revision, Tree as BuildTree,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Build main's tip once, check it as the canary on a copy of production's
/// history, and promote it as the operator's policy says.
#[derive(Args)]
pub struct Canary {
    #[command(subcommand)]
    action: CanaryAction,
}

#[derive(Subcommand)]
enum CanaryAction {
    /// One pass: fetch main, build it, check it as the canary, write where
    /// the release stands, and promote it if the policy says so.
    Run(RunArgs),
    /// Install the canary's build into production now, whatever the policy
    /// says, once it has passed.
    Promote(TargetArgs),
    /// Close work items: land those main's commits name, and ship those
    /// production's build holds. Every pass does this too.
    Work(WorkArgs),
    /// Stop the canary and remove its routes.
    Down(PublishArgs),
    /// Run a pass on a systemd user timer.
    Enable(EnableArgs),
    /// Stop the timer and remove its units.
    Disable,
}

#[derive(Args)]
struct RunArgs {
    /// What to check: main's tip, fetched from `origin`; or this checkout's
    /// commit, as it is (clean).
    #[arg(long, value_enum, default_value_t = Source::Main)]
    source: Source,
    #[command(flatten)]
    target: TargetArgs,
    #[command(flatten)]
    publish: PublishArgs,
}

#[derive(Args, Clone, Copy)]
struct TargetArgs {
    /// The instance the canary copies and promotes into.
    #[arg(long, value_enum, default_value_t = Production::Installed)]
    of: Production,
}

#[derive(Args)]
struct WorkArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// The commit main is at; by default, this checkout's.
    #[arg(long, value_name = "COMMIT")]
    main: Option<CommitId>,
}

#[derive(Args, Clone, Copy)]
struct PublishArgs {
    /// Publish the canary on the tailnet (gateway 9445, UI 9446), with the
    /// install's saved Tailscale choice.
    #[arg(long)]
    publish: bool,
}

#[derive(Args)]
struct EnableArgs {
    /// Minutes between passes.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..))]
    every: u16,
    #[command(flatten)]
    publish: PublishArgs,
}

/// What a pass checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Source {
    Main,
    Checkout,
}

#[derive(Debug, thiserror::Error)]
pub enum CanaryError {
    #[error("{0} is not set")]
    MissingEnv(&'static str),
    #[error("tracked files differ from the commit; the canary checks only a clean commit")]
    Dirty,
    #[error("{0} is not a commit the release reads")]
    Commit(String),
    #[error("the canary has no build that passed to promote ({0})")]
    NothingToPromote(&'static str),
    #[error("the dev slot is never promoted into")]
    DevPromotion,
    #[error(transparent)]
    Replica(#[from] ReplicaError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Builds(#[from] BuildsError),
    #[error(transparent)]
    Install(#[from] Box<InstallError>),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Tailscale(#[from] TailscaleError),
    #[error(transparent)]
    Ui(#[from] UiBuildError),
    #[error(transparent)]
    Dev(#[from] Box<DevError>),
    #[error(transparent)]
    Publish(#[from] crate::install::publish::PublishError),
}

impl From<DevError> for CanaryError {
    fn from(error: DevError) -> Self {
        Self::Dev(Box::new(error))
    }
}

impl From<InstallError> for CanaryError {
    fn from(error: InstallError) -> Self {
        Self::Install(Box::new(error))
    }
}

/// The factory: the checkout it runs from, its own worktree of `main`, its
/// builds, and the programs it runs.
struct Factory {
    home: PathBuf,
    /// The checkout this xtask was built from.
    repo: PathBuf,
    /// Ren's own worktree, at the commit being checked.
    worktree: PathBuf,
    cargo: PathBuf,
    builds: Builds,
    git: Git,
}

impl Factory {
    const REMOTE: &str = "origin";
    const MAIN: &str = "main";
    const TIMER: &str = "ren-canary";
    /// The canary's tailnet HTTPS ports, with `--publish`, beside the
    /// installed service's and the dev slot's (9443 and 9444).
    const GATEWAY_PORT: u16 = 9445;
    const UI_PORT: u16 = 9446;

    fn new(env: &HostEnv) -> Result<Self, CanaryError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(CanaryError::MissingEnv("HOME"))?;
        Ok(Self {
            worktree: home.join(".local/share/ren/factory"),
            builds: Builds::new(&home),
            // xtask/ sits at the root of the checkout.
            repo: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            cargo: env
                .absolute_path("CARGO")
                .unwrap_or_else(|| PathBuf::from("cargo")),
            git: Git::inherited(),
            home,
        })
    }

    /// A commit as the release names it.
    fn commit(id: &CommitId) -> Result<Commit, CanaryError> {
        id.as_str()
            .parse()
            .map_err(|_| CanaryError::Commit(id.to_string()))
    }

    /// A commit as git names it.
    fn id(commit: &Commit) -> Result<CommitId, CanaryError> {
        commit
            .to_string()
            .parse()
            .map_err(|_| CanaryError::Commit(commit.to_string()))
    }

    /// The operator's release settings; the defaults when there are none.
    fn options(&self) -> Result<ReleaseOptions, CanaryError> {
        let path = self.home.join(".config/ren/release.toml");
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|error| {
                CanaryError::File(FileError::at(&path)(std::io::Error::other(error)))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ReleaseOptions::default())
            }
            Err(error) => Err(FileError::at(&path)(error).into()),
        }
    }

    /// The saved Tailscale choice, for publishing the canary.
    fn tailscale(&self, publish: PublishArgs) -> Result<Option<TailscaleCli>, CanaryError> {
        if !publish.publish {
            return Ok(None);
        }
        let saved = Publish::default().resolve(&Publish::path(&self.home))?;
        Ok(Some(if saved.sudo_tailscale {
            TailscaleCli::Sudo
        } else {
            TailscaleCli::Direct
        }))
    }

    /// Fetches main, checks its tip out in the factory's worktree, and goes
    /// on there with that commit's own xtask.
    fn delegate(&self, args: &RunArgs) -> Result<(), CanaryError> {
        let main: Branch = Self::MAIN
            .parse()
            .map_err(|_| CanaryError::Commit(Self::MAIN.to_owned()))?;
        Progress(&format!("Fetching {} from {}", Self::MAIN, Self::REMOTE)).show();
        let tip = self.git.fetch(&self.repo, Self::REMOTE, &main)?;
        self.git
            .check_out_detached(&self.repo, &self.worktree, &tip)?;
        Progress(&format!(
            "Checking {} with its own checks, in {}",
            tip.abbreviated(12),
            self.worktree.display()
        ))
        .show();
        let mut pass = Invocation::new(&self.cargo)
            .current_dir(&self.worktree)
            .args(["xtask", "canary", "run", "--source", "checkout", "--of"])
            .arg(
                args.target
                    .of
                    .to_possible_value()
                    .map_or_else(String::new, |value| value.get_name().to_owned()),
            );
        if args.publish.publish {
            pass = pass.arg("--publish");
        }
        Ok(pass.run()?)
    }

    /// One pass over this checkout's commit.
    fn pass(&self, env: &HostEnv, args: &RunArgs) -> Result<(), CanaryError> {
        match self.git.tree(&self.repo)? {
            Tree::Clean => {}
            Tree::Modified => return Err(CanaryError::Dirty),
        }
        let id = self.git.head_commit(&self.repo)?;
        let commit = Self::commit(&id)?;
        let production = ProductionPlace::new(env, args.target.of)?;
        let canary = Replica::canary(env)?;
        let build = self.builds.make(env, &self.cargo, &self.repo, &commit)?;
        let backup = production.backup()?;
        canary.replace_data(&backup)?;
        let keys = canary.configure(&production)?;
        let publish = self
            .tailscale(args.publish)?
            .map(|tailscale| (tailscale, Self::GATEWAY_PORT, Self::UI_PORT));
        let reached = canary.start(&self.repo, &build.ren(), &build.ui(), &keys, publish)?;
        println!("\nCanary: {}\n", reached.ui);
        let verdict = Checks {
            env,
            canary: &canary,
            build: &build,
            shots: self.home.join(".local/share/ren-canary/shots"),
        }
        .run();
        production.keep_shots(&self.home.join(".local/share/ren-canary/shots"))?;
        let checked = CanaryBuild::new(commit.clone(), verdict);
        let release = self.assess(&production, &commit, Some((checked, &build)))?;
        production.write_release(&release)?;
        Self::report(&release);
        if release.decision() == &Stored::Known(Decision::Promote) {
            self.promote(env, &production, &commit, &build)?;
        }
        self.requested(env, &production)?;
        WorkLedger::new(&production).close(self, &id)?;
        let keep: Vec<Commit> = [Some(commit), Self::running(&production)]
            .into_iter()
            .flatten()
            .collect();
        self.builds.prune(&keep.iter().collect::<Vec<_>>())?;
        Ok(())
    }

    /// The commit production runs, when it is a clean one.
    fn running(production: &ProductionPlace) -> Option<Commit> {
        match production.revision().ok()? {
            Revision::Commit {
                commit,
                tree: BuildTree::Clean,
            } => Some(commit),
            Revision::Commit {
                tree: BuildTree::Modified,
                ..
            }
            | Revision::Unknown => None,
        }
    }

    /// Where the release stands with `main` at `main`, and what the policy
    /// decides now for the canary's build.
    fn assess(
        &self,
        production: &ProductionPlace,
        main: &Commit,
        canary: Option<(CanaryBuild, &StoredBuild)>,
    ) -> Result<Release, CanaryError> {
        let revision = production.revision()?;
        let running = match &revision {
            Revision::Commit {
                commit,
                tree: BuildTree::Clean,
            } => Self::id(commit).ok(),
            Revision::Commit {
                tree: BuildTree::Modified,
                ..
            }
            | Revision::Unknown => None,
        };
        let main_id = Self::id(main)?;
        // A commit the factory's repository does not have counts as none.
        let between =
            |from: &CommitId, to: &CommitId| self.git.commits_between(&self.repo, from, to).ok();
        let behind = running
            .as_ref()
            .and_then(|running| between(running, &main_id));
        let canary = match canary {
            Some((checked, build)) => {
                let candidate = Self::id(checked.commit())?;
                let running_commit = running.as_ref().and_then(|id| Self::commit(id).ok());
                let lacks = running
                    .as_ref()
                    .and_then(|running| between(&candidate, running));
                let lineage = Lineage::read(running_commit.as_ref(), checked.commit(), lacks);
                Some((checked, lineage, build.attempt()))
            }
            None => None,
        };
        let policy = match production.kind {
            Production::Installed => self.options()?.promote,
            // The dev slot is never promoted into: only the operator's word.
            Production::Dev => Policy::Manual,
        };
        let now = SystemClock::start().now().wall.as_millis();
        Ok(Release::assess(
            Assessment {
                main: main.clone(),
                production: revision,
                behind,
                canary,
            },
            policy,
            MinuteOfDay::of_unix_ms(now),
        ))
    }

    /// Installs `build` into production, and writes where the release
    /// stands after. A failed install is rolled back by the install itself
    /// and marked on the build, so it is not tried again on its own.
    fn promote(
        &self,
        env: &HostEnv,
        production: &ProductionPlace,
        commit: &Commit,
        build: &StoredBuild,
    ) -> Result<(), CanaryError> {
        match production.kind {
            Production::Installed => {}
            Production::Dev => return Err(CanaryError::DevPromotion),
        }
        Progress(&format!("Promoting {} into production", commit.short())).show();
        let installed = InstallVps::promotion(commit.clone()).run(env);
        if installed.is_err() {
            build.failed_promotion()?;
        }
        let canary = production
            .release()
            .and_then(|release| release.canary().cloned());
        let main = production
            .release()
            .map_or_else(|| commit.clone(), |release| release.main().clone());
        let release = self.assess(production, &main, canary.map(|canary| (canary, build)))?;
        production.write_release(&release)?;
        Self::report(&release);
        Ok(installed?)
    }

    /// Carries out the operator's promotion request from the Release view,
    /// if one waits: only for the canary's build that passed, which
    /// production does not run yet; any other is dropped, with why.
    fn requested(&self, env: &HostEnv, production: &ProductionPlace) -> Result<(), CanaryError> {
        let Some(request) = production.take_request() else {
            return Ok(());
        };
        let release = production.release();
        let canary = release.as_ref().and_then(Release::canary);
        let current = release
            .as_ref()
            .is_some_and(|release| release.decision() == &Stored::Known(Decision::Current));
        match (production.kind, canary) {
            (Production::Dev, _) => {
                Progress(
                    "A promotion was asked of the dev slot, which is never promoted into; dropped",
                )
                .show();
                Ok(())
            }
            (Production::Installed, Some(canary))
                if *canary.commit() == request.commit && canary.verdict().passed() && !current =>
            {
                Progress(&format!(
                    "Promoting {}, as the operator asked in the Release view",
                    request.commit.short()
                ))
                .show();
                self.promote_now(
                    env,
                    TargetArgs {
                        of: Production::Installed,
                    },
                )
            }
            (Production::Installed, _) => {
                Progress(&format!(
                    "A promotion of {} was asked, but it is not the canary's passed build, or production runs it; dropped",
                    request.commit.short()
                ))
                .show();
                Ok(())
            }
        }
    }

    /// `canary promote`: the canary's build, if it passed, now.
    fn promote_now(&self, env: &HostEnv, target: TargetArgs) -> Result<(), CanaryError> {
        let production = ProductionPlace::new(env, target.of)?;
        let release = production
            .release()
            .ok_or(CanaryError::NothingToPromote("no pass has run"))?;
        let canary = release
            .canary()
            .ok_or(CanaryError::NothingToPromote("no build was checked"))?;
        if !canary.verdict().passed() {
            return Err(CanaryError::NothingToPromote("its build failed its checks"));
        }
        let build = self
            .builds
            .get(canary.commit())
            .ok_or_else(|| BuildsError::NotBuilt(canary.commit().clone()))?;
        build.retry_promotion()?;
        self.promote(env, &production, canary.commit(), &build)
    }

    fn report(release: &Release) {
        Progress(&format!("Release: {}", release.decision())).show();
        println!("Run `ren release` to see where it stands.");
    }

    /// Writes and starts the timer that runs a pass every `every` minutes,
    /// from this checkout.
    fn enable(&self, env: &HostEnv, args: &EnableArgs) -> Result<(), CanaryError> {
        let units = self.home.join(".config/systemd/user");
        fs::create_dir_all(&units).map_err(FileError::at(&units))?;
        let mut environment = String::new();
        for name in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "BUN_INSTALL"] {
            if let Some(value) = env.get(name) {
                environment.push_str(&format!("Environment=\"{name}={value}\"\n"));
            }
        }
        let publish = if args.publish.publish {
            " --publish"
        } else {
            ""
        };
        let service = format!(
            "[Unit]\nDescription=Ren canary: build main's tip, check it, and promote it by policy\n\n[Service]\nType=oneshot\nWorkingDirectory={}\n{environment}ExecStart={} xtask canary run{publish}\n",
            self.repo.display(),
            self.cargo.display(),
        );
        let timer = format!(
            "[Unit]\nDescription=Ren canary, every {} minutes\n\n[Timer]\nOnBootSec=5min\nOnUnitInactiveSec={}min\n\n[Install]\nWantedBy=timers.target\n",
            args.every, args.every
        );
        FileMode::Readable.write(
            &units.join(format!("{}.service", Self::TIMER)),
            service.as_bytes(),
        )?;
        FileMode::Readable.write(
            &units.join(format!("{}.timer", Self::TIMER)),
            timer.as_bytes(),
        )?;
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now"])
            .arg(format!("{}.timer", Self::TIMER))
            .run()?;
        Progress(&format!(
            "A pass runs every {} minutes; follow it with `journalctl --user -u {}`",
            args.every,
            Self::TIMER
        ))
        .show();
        Ok(())
    }

    fn disable(&self) -> Result<(), CanaryError> {
        let units = self.home.join(".config/systemd/user");
        Invocation::new("systemctl")
            .args(["--user", "disable", "--now"])
            .arg(format!("{}.timer", Self::TIMER))
            .succeeds(crate::invocation::Echo::Show)?;
        for unit in ["service", "timer"] {
            let path = units.join(format!("{}.{unit}", Self::TIMER));
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(FileError::at(&path)(error).into()),
            }
        }
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Ok(())
    }
}

impl Canary {
    pub fn run(self, env: &HostEnv) -> Result<(), CanaryError> {
        let factory = Factory::new(env)?;
        match self.action {
            CanaryAction::Run(args) => match args.source {
                Source::Main => factory.delegate(&args),
                Source::Checkout => factory.pass(env, &args),
            },
            CanaryAction::Promote(target) => factory.promote_now(env, target),
            CanaryAction::Work(args) => {
                let production = ProductionPlace::new(env, args.target.of)?;
                let main = match args.main {
                    Some(main) => main,
                    None => factory.git.head_commit(&factory.repo)?,
                };
                let closed = WorkLedger::new(&production).close(&factory, &main)?;
                println!("{} landed, {} shipped", closed.landed, closed.shipped);
                Ok(())
            }
            CanaryAction::Down(publish) => {
                let canary = Replica::canary(env)?;
                canary.stop()?;
                if let Some(tailscale) = factory.tailscale(publish)? {
                    canary.unpublish(tailscale, (Factory::GATEWAY_PORT, Factory::UI_PORT))?;
                }
                Ok(())
            }
            CanaryAction::Enable(args) => factory.enable(env, &args),
            CanaryAction::Disable => factory.disable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Factory;
    use ren_git::CommitId;

    #[test]
    fn a_commit_reads_the_same_to_git_and_to_the_release() {
        let id: CommitId = "62e9e9e0000000000000000000000000000000ff".parse().unwrap();
        let commit = Factory::commit(&id).unwrap();
        assert_eq!(Factory::id(&commit).unwrap(), id);
    }
}
