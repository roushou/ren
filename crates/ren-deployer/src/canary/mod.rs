//! `ren deployer canary`: the deployer's canary and promotion
//! (docs/plans/self-build.md, phase 4). One pass:
//!
//! 1. fetches `main`, checks its tip out in the deployer's own worktree
//!    (`~/.local/share/ren/factory`), and builds it once (`crate::builds`);
//! 2. goes on with that build's own `ren deployer`, so a commit is checked
//!    by its own checks;
//! 3. starts it as the canary on a fresh backup of production's history;
//! 4. checks it (`checks`): doctor, every read, every page;
//! 5. writes where the release stands to production's data folder, where
//!    `/api/release`, `ren release`, and the UI read it; and
//! 6. promotes the build under the operator's policy
//!    (`~/.config/ren/release.toml`), with `install-vps --build`'s steps.
//!
//! `enable` runs a pass on a systemd user timer.

mod checks;
mod previews;
mod serve;
mod work;

use self::{checks::Checks, previews::PreviewPass, serve::Service, work::WorkLedger};
use crate::{
    builds::{Builds, BuildsError, StoredBuild},
    file::{FileError, FileMode},
    install::{InstallError, InstallVps, publish::Publish},
    invocation::{Invocation, InvocationError},
    progress::Progress,
    replica::{ProductionPlace, Replica, ReplicaError},
    tailscale::{Tailnet, TailscaleError},
    ui::UiBuildError,
};
use clap::{Args, Subcommand, ValueEnum};
use ren_daemon::host::SystemClock;
use ren_gateway::deployer::DeployerRequest;
use ren_gateway::release::{
    Assessment, Canary as CanaryBuild, Decision, Lineage, MinuteOfDay, Release, ReleaseOptions,
};
use ren_git::SourceRevision;
use ren_git::{Branch, Commit, Git, GitError, Tree};
use ren_harness::host::HostEnv;
use ren_network::tailscale::Cli;
use ren_protocol::label::Stored;
use ren_recorder::time::Clock as _;
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
    Promote,
    /// Close work items: land those main's commits name, and ship those
    /// production's build holds. Every pass does this too.
    Work(WorkArgs),
    /// Bring the previews up to date: one for each commit asked for and each
    /// open pull request that is not a draft, as many as release.toml's
    /// `previews` allows. Every pass does this too.
    Previews(PreviewsArgs),
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
    publish: PublishArgs,
}

/// Listen on a socket and take up each ask at once: the deployer the
/// daemon asks (`[deployer] socket`), so a preview asked for starts now, the
/// canary follows `main`, and a promotion asked for is carried out. The
/// timer's passes then leave these to it.
#[derive(Args)]
pub struct ServeArgs {
    #[command(flatten)]
    previews: PreviewsArgs,
    /// Where to listen; `$XDG_RUNTIME_DIR/ren-deployer/deployer.sock` by
    /// default.
    #[arg(long)]
    socket: Option<PathBuf>,
}

#[derive(Args)]
struct PreviewsArgs {
    #[command(flatten)]
    publish: PublishArgs,
    /// The project whose pull requests are previewed: Ren itself, as
    /// production registers it.
    #[arg(long, default_value = "ren")]
    project: String,
}

#[derive(Args)]
struct WorkArgs {
    /// The commit main is at; by default, this checkout's.
    #[arg(long, value_name = "COMMIT")]
    main: Option<Commit>,
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
    #[error("the deployer could not listen: {0}")]
    Serve(#[source] std::io::Error),
    #[error("tracked files differ from the commit; the canary checks only a clean commit")]
    Dirty,
    #[error("{0} is not a commit the release reads")]
    Commit(String),
    #[error("the canary has no build that passed to promote ({0})")]
    NothingToPromote(&'static str),
    #[error(transparent)]
    Replica(#[from] ReplicaError),
    #[error("the previews: {0}")]
    Preview(String),
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
    Publish(#[from] crate::install::publish::PublishError),
}

impl From<InstallError> for CanaryError {
    fn from(error: InstallError) -> Self {
        Self::Install(Box::new(error))
    }
}

/// The deployer: the checkout it runs from, its own worktree of `main`, its
/// builds, and the programs it runs.
pub struct Deployer {
    home: PathBuf,
    /// The checkout it builds from: the operator's clone, or, for a pass
    /// handed off, the deployer's worktree at the commit being checked.
    repo: PathBuf,
    /// Ren's own worktree, at the commit being checked.
    worktree: PathBuf,
    builds: Builds,
    git: Git,
}

impl Deployer {
    const REMOTE: &str = "origin";
    /// The project Ren itself is registered as.
    const REN: &str = "ren";
    const MAIN: &str = "main";
    const TIMER: &str = "ren-canary";
    /// The deployer service's unit (`ren deployer serve`), beside the timer.
    const SERVICE: &str = "ren-deployer";
    /// The canary's tailnet HTTPS ports, with `--publish`, beside the
    /// installed service's (9443).
    const GATEWAY_PORT: u16 = 9445;
    const UI_PORT: u16 = 9446;

    /// The deployer building from `checkout`.
    pub fn new(env: &HostEnv, checkout: &Path) -> Result<Self, CanaryError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(CanaryError::MissingEnv("HOME"))?;
        Ok(Self {
            worktree: home.join(".local/share/ren/factory"),
            builds: Builds::new(&home),
            repo: checkout.to_path_buf(),
            git: Git::inherited(),
            home,
        })
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
    fn tailscale(&self, publish: PublishArgs) -> Result<Option<Tailnet>, CanaryError> {
        if !publish.publish {
            return Ok(None);
        }
        let saved = Publish::default().resolve(&Publish::path(&self.home))?;
        Ok(Some(if saved.sudo_tailscale {
            Tailnet::new(Cli::Sudo)
        } else {
            Tailnet::new(Cli::Direct)
        }))
    }

    /// A pass over main's tip: handed to the deployer service when it
    /// listens for production (it runs one pass at a time, and the timer's
    /// would race it), else run here.
    fn follow(&self, env: &HostEnv, args: &RunArgs) -> Result<(), CanaryError> {
        let ren = Self::REN
            .parse()
            .map_err(|_| CanaryError::Commit(Self::REN.to_owned()))?;
        let served = Service::socket(env)
            .is_some_and(|socket| Service::nudge(&socket, &DeployerRequest::Canary(ren)));
        if served {
            Progress("The deployer service runs the canary pass").show();
            return Ok(());
        }
        self.delegate(env, args)
    }

    /// Fetches main, checks its tip out in the deployer's worktree, builds
    /// it, and goes on there with that build's own `ren deployer`, so a
    /// commit is checked by its own checks. One at a time: a second waits
    /// for the first.
    fn delegate(&self, env: &HostEnv, args: &RunArgs) -> Result<(), CanaryError> {
        let lock = self.lock()?;
        let main: Branch = Self::MAIN
            .parse()
            .map_err(|_| CanaryError::Commit(Self::MAIN.to_owned()))?;
        Progress(&format!("Fetching {} from {}", Self::MAIN, Self::REMOTE)).show();
        let tip = self.git.fetch(&self.repo, Self::REMOTE, &main)?;
        self.git
            .check_out_detached(&self.repo, &self.worktree, &tip)?;
        let build = self.builds.make(env, &self.worktree, &tip)?;
        Progress(&format!(
            "Checking {} with its own checks, in {}",
            tip.abbreviated(12),
            self.worktree.display()
        ))
        .show();
        let mut pass = Invocation::new(build.ren())
            .current_dir(&self.worktree)
            .args(["deployer", "--checkout"])
            .arg(&self.worktree)
            .args(["canary", "run", "--source", "checkout"]);
        if args.publish.publish {
            pass = pass.arg("--publish");
        }
        let ran = pass.run();
        drop(lock);
        Ok(ran?)
    }

    /// The canary's lock, held while a pass runs over main.
    fn lock(&self) -> Result<fs::File, CanaryError> {
        let dir = self.home.join(".local/share/ren");
        fs::create_dir_all(&dir).map_err(FileError::at(&dir))?;
        let path = dir.join("canary.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(FileError::at(&path))?;
        file.lock().map_err(FileError::at(&path))?;
        Ok(file)
    }

    /// One pass over this checkout's commit.
    fn pass(&self, env: &HostEnv, args: &RunArgs) -> Result<(), CanaryError> {
        match self.git.tree(&self.repo)? {
            Tree::Clean => {}
            Tree::Modified => return Err(CanaryError::Dirty),
        }
        let id = self.git.head_commit(&self.repo)?;
        let commit = id.clone();
        let production = ProductionPlace::new(env)?;
        let canary = Replica::canary(env)?;
        let build = self.builds.make(env, &self.repo, &commit)?;
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
        // The deployer service, when it listens, runs the previews: two
        // passes at once would race over them. Its previews are kept.
        let served = Service::socket(env)
            .is_some_and(|socket| Service::nudge(&socket, &DeployerRequest::Previews));
        let running = || -> Vec<Commit> {
            production
                .previews()
                .deployments()
                .iter()
                .map(|instance| instance.commit.clone())
                .collect()
        };
        let previewed = if served {
            Progress("The deployer service runs the previews").show();
            running()
        } else {
            // A previews pass that fails leaves the canary's verdict standing.
            PreviewPass {
                deployer: self,
                production: &production,
                publish: args.publish,
                project: Self::REN,
            }
            .run(env)
            .unwrap_or_else(|error| {
                println!("warning: the previews pass failed: {error}");
                running()
            })
        };
        let keep: Vec<Commit> = [Some(commit), Self::running(&production)]
            .into_iter()
            .flatten()
            .chain(previewed)
            .collect();
        self.builds.prune(&keep.iter().collect::<Vec<_>>())?;
        Ok(())
    }

    /// The commit production runs, when it is a clean one.
    fn running(production: &ProductionPlace) -> Option<Commit> {
        match production.revision().ok()? {
            SourceRevision::Commit {
                commit,
                tree: Tree::Clean,
            } => Some(commit),
            SourceRevision::Commit {
                tree: Tree::Modified,
                ..
            }
            | SourceRevision::Unknown => None,
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
            SourceRevision::Commit {
                commit,
                tree: Tree::Clean,
            } => Some(commit.clone()),
            SourceRevision::Commit {
                tree: Tree::Modified,
                ..
            }
            | SourceRevision::Unknown => None,
        };
        let main_id = main.clone();
        // A commit the deployer's repository does not have counts as none.
        let between =
            |from: &Commit, to: &Commit| self.git.commits_between(&self.repo, from, to).ok();
        let behind = running
            .as_ref()
            .and_then(|running| between(running, &main_id));
        let canary = match canary {
            Some((checked, build)) => {
                let candidate = checked.commit().clone();
                let running_commit = running.clone();
                let lacks = running
                    .as_ref()
                    .and_then(|running| between(&candidate, running));
                let lineage = Lineage::read(running_commit.as_ref(), checked.commit(), lacks);
                Some((checked, lineage, build.attempt()))
            }
            None => None,
        };
        let policy = self.options()?.promote;
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
        Progress(&format!("Promoting {} into production", commit.short())).show();
        let installed = InstallVps::promotion(commit.clone()).run(env, &self.repo);
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
        match canary {
            Some(canary)
                if *canary.commit() == request.commit && canary.verdict().passed() && !current =>
            {
                Progress(&format!(
                    "Promoting {}, as the operator asked in the Release view",
                    request.commit.short()
                ))
                .show();
                self.promote_now(env)
            }
            Some(_) | None => {
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
    fn promote_now(&self, env: &HostEnv) -> Result<(), CanaryError> {
        let production = ProductionPlace::new(env)?;
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
        // The installed `ren` runs them: a promotion upgrades the deployer
        // with production.
        let ren = self.home.join(".local/bin/ren");
        let service = format!(
            "[Unit]\nDescription=Ren canary: build main's tip, check it, and promote it by policy\n\n[Service]\nType=oneshot\nWorkingDirectory={repo}\n{environment}ExecStart={ren} deployer --checkout {repo} canary run{publish}\n",
            repo = self.repo.display(),
            ren = ren.display(),
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
        // The deployer service: previews asked for start as they are asked,
        // with the timer's environment and publishing.
        let deployer = format!(
            "[Unit]\nDescription=Ren deployer: previews started as they are asked for\n\n[Service]\nType=simple\nWorkingDirectory={repo}\n{environment}ExecStart={ren} deployer --checkout {repo} serve{publish}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            repo = self.repo.display(),
            ren = ren.display(),
        );
        FileMode::Readable.write(
            &units.join(format!("{}.service", Self::SERVICE)),
            deployer.as_bytes(),
        )?;
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now"])
            .arg(format!("{}.timer", Self::TIMER))
            .run()?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now"])
            .arg(format!("{}.service", Self::SERVICE))
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
        Invocation::new("systemctl")
            .args(["--user", "disable", "--now"])
            .arg(format!("{}.service", Self::SERVICE))
            .succeeds(crate::invocation::Echo::Show)?;
        for unit in [
            format!("{}.service", Self::TIMER),
            format!("{}.timer", Self::TIMER),
            format!("{}.service", Self::SERVICE),
        ] {
            let path = units.join(unit);
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

impl ServeArgs {
    /// Serves until stopped, building from `checkout`.
    pub fn run(self, env: &HostEnv, checkout: &Path) -> Result<(), CanaryError> {
        let deployer = Deployer::new(env, checkout)?;
        let production = ProductionPlace::new(env)?;
        let socket = self
            .socket
            .or_else(|| Service::socket(env))
            .ok_or(CanaryError::MissingEnv("XDG_RUNTIME_DIR"))?;
        Service {
            deployer: &deployer,
            production: &production,
            publish: self.previews.publish,
            project: &self.previews.project,
        }
        .serve(env, &socket)
    }
}

impl Canary {
    /// Runs the command, building from `checkout`.
    pub fn run(self, env: &HostEnv, checkout: &Path) -> Result<(), CanaryError> {
        let deployer = Deployer::new(env, checkout)?;
        match self.action {
            CanaryAction::Run(args) => match args.source {
                Source::Main => deployer.follow(env, &args),
                Source::Checkout => deployer.pass(env, &args),
            },
            CanaryAction::Promote => deployer.promote_now(env),
            CanaryAction::Work(args) => {
                let production = ProductionPlace::new(env)?;
                let main = match args.main {
                    Some(main) => main,
                    None => deployer.git.head_commit(&deployer.repo)?,
                };
                let closed = WorkLedger::new(&production).close(&deployer, &main)?;
                println!("{} landed, {} shipped", closed.landed, closed.shipped);
                Ok(())
            }
            CanaryAction::Previews(args) => {
                let production = ProductionPlace::new(env)?;
                PreviewPass {
                    deployer: &deployer,
                    production: &production,
                    publish: args.publish,
                    project: &args.project,
                }
                .run(env)
                .map(drop)
            }
            CanaryAction::Down(publish) => {
                let canary = Replica::canary(env)?;
                canary.stop()?;
                if let Some(tailscale) = deployer.tailscale(publish)? {
                    canary.unpublish(tailscale, (Deployer::GATEWAY_PORT, Deployer::UI_PORT))?;
                }
                Ok(())
            }
            CanaryAction::Enable(args) => deployer.enable(env, &args),
            CanaryAction::Disable => deployer.disable(),
        }
    }
}
