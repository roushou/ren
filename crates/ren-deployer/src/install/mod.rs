//! `ren deployer install-vps`: install or upgrade Ren from this checkout as a
//! Linux systemd user service, optionally published with Tailscale Serve.
//!
//! An upgrade restarts the daemon only between calls and between runs' turns:
//! it waits until no call is running and no run is starting, working, or
//! stopping, pauses admission (clients get `503` with `retry-after` and
//! wait), replaces the binary, checks that the daemon runs the new build, and
//! resumes. Idle runs end with the old daemon; it names each, with how to
//! continue its conversation.
pub mod publish;
pub mod rollback;
mod ui;

use self::{
    publish::{Publish, PublishError},
    rollback::{Held, Part, Upgrade},
    ui::{UiDeploy, UiUrls},
};
use crate::{
    builds::{Builds, BuildsError, StoredBuild},
    daemon::{
        AfterPause, Busy, CheckStatus, ControlState, DoctorReport, Ending, Quiesce, RunPolicy,
        Runs, StillBusy,
    },
    daemon_container::{Contain, DaemonContainer, DaemonContainerError},
    file::{FileError, FileMode},
    invocation::{Echo, Invocation, InvocationError},
    pod::Pod,
    progress::Progress,
    tailscale::{Tailnet, TailscaleError},
    ui::UiBuildError,
};
use clap::Args;
use ren_daemon::config::{Config, ConfigError, RunRuntime};
use ren_git::Commit;
use ren_git::{Git, GitError, Published, Tree};
use ren_harness::host::HostEnv;
use ren_network::tailscale::Cli;
use ren_recorder::project::{Factory, ProjectName, RepositoryPath};
use serde::Deserialize;
use std::{
    fs,
    io::Write as _,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

/// Install or upgrade Ren from this checkout as a systemd user service.
///
/// Run on the host, as the user that owns the service, from the commit to
/// deploy. Each step checks the current state and changes only what differs.
/// Provider credentials and client configuration are never touched.
#[derive(Args)]
pub struct InstallVps {
    /// Publishing choices; with none given, the ones saved by the last
    /// successful run.
    #[command(flatten)]
    publish: Publish,
    /// Skip `ren control backup` before replacing a running daemon.
    #[arg(long)]
    skip_backup: bool,
    /// Install even though tracked files differ from the commit; the daemon
    /// then reports a `-dirty` revision.
    #[arg(long, conflicts_with = "build")]
    allow_dirty: bool,
    /// Install the build of this commit kept in `~/.local/share/ren/builds`
    /// (the one the canary checked), instead of building this checkout.
    #[arg(long, value_name = "COMMIT")]
    build: Option<Commit>,
    /// How long to wait for running calls and working runs to finish before
    /// giving up, without having changed anything.
    #[arg(long, value_name = "SECONDS", default_value_t = 900)]
    idle_timeout: u64,
    /// Restart even while runs are starting or working: their turns are
    /// lost, and each ends with the daemon (its conversation can be
    /// continued).
    /// Running calls are still waited for.
    #[arg(long)]
    drain_runs: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("install-vps requires Linux with a systemd user manager")]
    NotLinux,
    #[error(transparent)]
    Contain(#[from] DaemonContainerError),
    #[error(transparent)]
    Gvisor(#[from] crate::gvisor_service::GvisorServiceError),
    #[error(transparent)]
    Unit(#[from] crate::unit::NoExecStart),
    #[error("run as the regular user that owns the service, not root")]
    Root,
    #[error("{0} is not set")]
    MissingEnv(&'static str),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    Engine(#[from] ren_container::EngineError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error("invalid configuration {}: {source}", path.display())]
    Config {
        path: PathBuf,
        #[source]
        source: ConfigError,
    },
    #[error(
        "backup failed; fix it, or copy {} yourself and pass --skip-backup",
        data_dir.display()
    )]
    Backup { data_dir: PathBuf },
    #[error("doctor reported a failed check; see `journalctl --user -u ren`")]
    Doctor,
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(
        "tracked files differ from the commit; commit them, or pass --allow-dirty to deploy a -dirty build"
    )]
    Dirty,
    #[error(transparent)]
    Busy(#[from] StillBusy),
    #[error("the daemon does not run the build just installed: {0}")]
    BuildMismatch(String),
    #[error(transparent)]
    SavedOptions(#[from] PublishError),
    #[error(transparent)]
    UiBuild(#[from] UiBuildError),
    #[error("deploy/ren-ui.service no longer has the line {0:?} to render")]
    UiTemplate(String),
    #[error("the UI does not answer {url}; see `journalctl --user -u ren-ui`")]
    UiHealth { url: String },
    #[error(
        "{part} failed after the upgrade ({failure}); what ran before was put back and runs again"
    )]
    RolledBack {
        part: Part,
        failure: Box<InstallError>,
    },
    #[error(
        "{part} failed after the upgrade ({failure}), and putting back what ran before failed too ({rollback}); {}",
        part.advice()
    )]
    RollbackFailed {
        part: Part,
        failure: Box<InstallError>,
        rollback: Box<InstallError>,
    },
    #[error(transparent)]
    Tailscale(#[from] TailscaleError),
    #[error(transparent)]
    Builds(#[from] BuildsError),
}

/// Whether to back up history before replacing a running daemon.
#[derive(Clone, Copy)]
enum Backup {
    Run,
    Skip,
}

/// Whether the installed daemon runs the build just installed.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Confirmed,
    /// A `-dirty` build cannot be confirmed; warn and continue.
    Unconfirmed,
    Mismatch,
}

/// The quiet the upgrade made before replacing a running daemon: what it did
/// to admission, and the runs that end with the old daemon.
#[must_use]
struct Quiesced {
    admission: Quiet,
    ending: Ending,
}

/// What the upgrade did to admission before replacing a running daemon.
enum Quiet {
    /// No daemon was running.
    Stopped,
    /// This task paused admission, and resumes it when done.
    Paused,
    /// An operator had already paused it (or set a state this task does not
    /// know); it is left as it was.
    LeftAlone,
}

/// `loginctl show-user -p Linger --value`.
enum Linger {
    Enabled,
    Disabled,
}

impl Linger {
    fn parse(text: &str) -> Self {
        match text.trim() {
            "yes" => Self::Enabled,
            _ => Self::Disabled,
        }
    }
}

/// The registered projects, as `ren project list --json` prints them and the
/// install reads them: enough to tell whether one is Ren itself.
#[derive(Debug, Deserialize)]
struct Listed {
    projects: Vec<ListedProject>,
}

#[derive(Debug, Deserialize)]
struct ListedProject {
    name: ProjectName,
    repository: RepositoryPath,
    factory: Factory,
    archived_at: Option<u64>,
}

impl Listed {
    fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The project built as Ren itself, not archived, if one is.
    fn ren(&self) -> Option<&ListedProject> {
        self.projects
            .iter()
            .find(|project| project.archived_at.is_none() && project.factory == Factory::Ren)
    }
}

/// Where the checkout, binary, configuration, state, and unit live: the
/// standard paths of docs/deployment.md.
struct Layout {
    home: PathBuf,
    repo: PathBuf,
    cargo: PathBuf,
    user: String,
    bin_dir: PathBuf,
    binary: PathBuf,
    /// Where the installed binary waits while a new one is checked.
    rollback: PathBuf,
    config: PathBuf,
    data_dir: PathBuf,
    unit: PathBuf,
    /// The `ren` that talks to the running daemon (its state, pause,
    /// backup): the build being installed, which reads the configuration
    /// as the daemon about to run will, where the installed one may not.
    client: PathBuf,
}

const SERVICE: &str = "ren.service";
/// The transient unit an install started inside `ren.service` carries on in.
const RELAUNCH_UNIT: &str = "ren-install-vps";

/// Where this process runs: inside the service an install restarts, or not.
#[derive(Debug, PartialEq, Eq)]
enum Placement {
    InService,
    Elsewhere,
}

impl Placement {
    /// The environment a relaunched install keeps: the search path and the
    /// folders the build and the install read.
    const CARRIED: [&str; 8] = [
        "PATH",
        "HOME",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "BUN_INSTALL",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ];

    /// Read from `/proc/self/cgroup`: inside when a line's cgroup path has
    /// `service` as one of its parts.
    fn read(cgroup: &str, service: &str) -> Self {
        let inside = cgroup
            .lines()
            .filter_map(|line| line.splitn(3, ':').nth(2))
            .any(|path| path.split('/').any(|part| part == service));
        if inside {
            Self::InService
        } else {
            Self::Elsewhere
        }
    }
}

/// The file in the data folder that keeps the revision of the pause an
/// install made, until it resumes: a pause at that revision is the
/// install's own.
struct PauseMark {
    path: PathBuf,
}

impl PauseMark {
    const FILE: &str = "install-pause";

    fn of(layout: &Layout) -> Self {
        Self {
            path: layout.data_dir.join(Self::FILE),
        }
    }

    fn read(&self) -> Option<u64> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
    }

    fn write(&self, revision: Option<u64>) -> Result<(), InstallError> {
        match revision {
            Some(revision) => fs::write(&self.path, format!("{revision}\n"))
                .map_err(|error| FileError::at(&self.path)(error).into()),
            None => Ok(()),
        }
    }

    /// Gone once its pause is resumed or another's; a missing mark is fine.
    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Layout {
    fn new(env: &HostEnv, checkout: &Path) -> Result<Self, InstallError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(InstallError::MissingEnv("HOME"))?;
        let user = env
            .get("USER")
            .ok_or(InstallError::MissingEnv("USER"))?
            .to_owned();
        let bin_dir = home.join(".local/bin");
        Ok(Self {
            home: home.clone(),
            repo: checkout.to_path_buf(),
            cargo: env
                .absolute_path("CARGO")
                .unwrap_or_else(|| PathBuf::from("cargo")),
            user,
            binary: bin_dir.join("ren"),
            client: bin_dir.join("ren"),
            rollback: bin_dir.join("ren.rollback"),
            bin_dir,
            config: home.join(".config/ren/ren.toml"),
            data_dir: home.join(".local/state/ren"),
            unit: home.join(".config/systemd/user").join(SERVICE),
        })
    }

    /// `ren` with this layout's configuration and data directory.
    /// This layout, talking to the running daemon with `client`.
    fn talking_with(self, client: PathBuf) -> Self {
        Self { client, ..self }
    }

    fn ren(&self, binary: &Path) -> Invocation {
        Invocation::new(binary)
            .arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data_dir)
    }
}

impl InstallVps {
    /// The install a promotion makes: the kept build of `commit`, with the
    /// saved publishing choices, waiting for runs as an operator's install.
    pub fn promotion(commit: Commit) -> Self {
        Self {
            publish: Publish::default(),
            skip_backup: false,
            allow_dirty: false,
            build: Some(commit),
            idle_timeout: 900,
            drain_runs: false,
        }
    }

    /// Installs or upgrades from `checkout`.
    pub fn run(self, env: &HostEnv, checkout: &Path) -> Result<(), InstallError> {
        let backup = if self.skip_backup {
            Backup::Skip
        } else {
            Backup::Run
        };
        let policy = if self.drain_runs {
            RunPolicy::Drain
        } else {
            RunPolicy::Wait
        };
        Self::preflight()?;
        let layout = Layout::new(env, checkout)?;
        // Inside the service it restarts, the restart would stop this task
        // between its pause and its resume: carry on outside it.
        let cgroup = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
        match Placement::read(&cgroup, SERVICE) {
            Placement::InService => return Self::relaunch(env, &layout),
            Placement::Elsewhere => {}
        }
        let saved = Publish::path(&layout.home);
        let publish = self.publish.resolve(&saved)?;
        let tailscale = if publish.sudo_tailscale {
            Tailnet::new(Cli::Sudo)
        } else {
            Tailnet::new(Cli::Direct)
        };
        let ui = match publish.ui_https_port {
            Some(_) => Some(UiDeploy::new(env, &layout.home, &layout.repo)?),
            None => None,
        };
        let (tree, built) = match &self.build {
            Some(commit) => {
                let stored = Self::stored(&layout, commit)?;
                if let Some(ui) = &ui {
                    ui.stage(&stored.ui())?;
                }
                (Tree::Clean, stored.ren())
            }
            None => {
                let tree = Self::check_tree(&layout, self.allow_dirty)?;
                let built = Self::build(&layout)?;
                if let Some(ui) = &ui {
                    ui.build()?;
                }
                (tree, built)
            }
        };
        let layout = layout.talking_with(built.clone());
        Self::seed_config(&layout)?;
        let config = Self::load_config(&layout)?;
        // Made before anything stops: a configuration whose runs cannot be
        // held from a daemon in a container is refused here.
        let contained = match publish.contain {
            Contain::None | Contain::Runs => None,
            Contain::All => Some(Self::contain(&layout, &config, env)?),
        };
        let quiesced = Self::quiesce(&layout, self.idle_timeout, policy)?;
        quiesced.ending.announce();
        let replaced = Self::replace(&layout, &built, backup, tree, contained.as_ref());
        Self::release(&layout, quiesced.admission);
        replaced?;
        Self::register(&layout);
        let dns = match publish.https_port {
            Some(port) => {
                let dns = tailscale.publish(port, &format!("http://{}", config.listen))?;
                let url = Self::url(dns.as_deref(), port);
                println!(
                    "\nGateway root URL for clients: {url}\n  ren init --gateway {url}\n  ren run claude\n"
                );
                dns
            }
            None => None,
        };
        if let (Some(ui), Some(ui_port), Some(port)) =
            (&ui, publish.ui_https_port, publish.https_port)
        {
            let dns = dns.as_deref().ok_or(TailscaleError::NoDnsName)?;
            ui.install(
                &UiUrls {
                    gateway: format!("http://{}", config.listen),
                    public_gateway: format!("https://{dns}:{port}"),
                    origin: format!("https://{dns}:{ui_port}"),
                },
                publish.contain,
            )?;
            tailscale.publish(ui_port, UiDeploy::TARGET)?;
            println!("\nUI: {}\n", Self::url(Some(dns), ui_port));
        }
        publish.save(&saved)?;
        Progress("Done").show();
        quiesced.ending.remind();
        Ok(())
    }

    /// Runs this same command again as the transient user unit
    /// [`RELAUNCH_UNIT`], outside `ren.service`, and returns once it has
    /// started, without waiting for it. Inside the service, this process
    /// belongs to an agent a run drives, mid-turn: the install waits for
    /// every run's turn to end, so waiting here for the install would hold
    /// that turn open and the two would wait on each other until the
    /// install gave up. Its output goes to the journal.
    fn relaunch(env: &HostEnv, layout: &Layout) -> Result<(), InstallError> {
        let program = std::env::current_exe().map_err(FileError::at(Path::new("ren")))?;
        Progress(&format!(
            "This runs inside {SERVICE}, which the install restarts; continuing in the background as the unit {RELAUNCH_UNIT}. It restarts the daemon once every run's turn has ended, the one that started it included, so end that turn rather than wait for the install in it. Follow it with `journalctl --user -u {RELAUNCH_UNIT} -f`"
        )).show();
        let mut relaunch = Invocation::new("systemd-run")
            .args(["--user", "--unit", RELAUNCH_UNIT, "--collect", "--quiet"])
            .arg("--working-directory")
            .arg(&layout.repo);
        // What the build and the install read; never the operator's keys.
        for name in Placement::CARRIED {
            if let Some(value) = env.get(name) {
                relaunch = relaunch.arg(format!("--setenv={name}={value}"));
            }
        }
        relaunch
            .arg("--")
            .arg(program)
            .args(std::env::args_os().skip(1))
            .run()?;
        Ok(())
    }

    fn preflight() -> Result<(), InstallError> {
        if !cfg!(target_os = "linux") {
            return Err(InstallError::NotLinux);
        }
        let id = Invocation::new("id").arg("-u");
        let uid = id.capture()?;
        let uid: u32 = uid
            .trim()
            .parse()
            .map_err(|_| InvocationError::output("id -u")(uid.trim()))?;
        if uid == 0 {
            return Err(InstallError::Root);
        }
        Invocation::new("systemctl")
            .args(["--user", "show-environment"])
            .capture()?;
        Ok(())
    }

    /// Refuses modified tracked files unless allowed, and warns when the
    /// commit is on no remote branch, so the deployed source is saved
    /// somewhere other than this host.
    fn check_tree(layout: &Layout, allow_dirty: bool) -> Result<Tree, InstallError> {
        let (git, repo) = (Git::inherited(), layout.repo.as_path());
        let tree = match git.tree(repo)? {
            Tree::Clean => Tree::Clean,
            Tree::Modified if allow_dirty => {
                eprintln!(
                    "warning: tracked files differ from the commit; installing a -dirty build"
                );
                Tree::Modified
            }
            Tree::Modified => return Err(InstallError::Dirty),
        };
        let commit = git.head_commit(repo)?;
        let commit = commit.abbreviated(12);
        match git.published(repo)? {
            Published::OnRemote => {}
            Published::LocalOnly => eprintln!(
                "warning: commit {commit} is on no remote branch; push it so the deployed source is saved"
            ),
        }
        Progress(&format!("Installing commit {commit}")).show();
        Ok(tree)
    }

    /// The kept build of `commit`, which the canary made and checked.
    fn stored(layout: &Layout, commit: &Commit) -> Result<StoredBuild, InstallError> {
        let stored = Builds::new(&layout.home)
            .get(commit)
            .ok_or_else(|| BuildsError::NotBuilt(commit.clone()))?;
        Progress(&format!(
            "Installing the kept build of commit {}",
            commit.short()
        ))
        .show();
        Ok(stored)
    }

    /// The release binary: the installed `ren` is the operator's own, on the
    /// host they drive sessions from.
    fn build(layout: &Layout) -> Result<PathBuf, InstallError> {
        Progress("Building the release binary").show();
        Invocation::new(&layout.cargo)
            .args(["build", "--release", "--locked", "--package", "ren-rs"])
            .args(["--bin", "ren", "--manifest-path"])
            .arg(layout.repo.join("Cargo.toml"))
            .run()?;
        let built = layout.repo.join("target/release/ren");
        Ok(fs::canonicalize(&built).map_err(FileError::at(&built))?)
    }

    fn seed_config(layout: &Layout) -> Result<(), InstallError> {
        let unit_dir = layout.unit.parent().unwrap_or(Path::new("/"));
        let config_dir = layout.config.parent().unwrap_or(Path::new("/"));
        for dir in [config_dir, unit_dir, &layout.bin_dir, &layout.data_dir] {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(FileError::at(dir))?;
        }
        if layout.config.exists() {
            Progress(&format!(
                "Keeping existing configuration {}",
                layout.config.display()
            ))
            .show();
            return Ok(());
        }
        Progress(&format!(
            "Creating {} from ren.example.toml (review it)",
            layout.config.display()
        ))
        .show();
        let example = layout.repo.join("ren.example.toml");
        let contents = fs::read(&example).map_err(FileError::at(&example))?;
        Ok(FileMode::Private.write(&layout.config, &contents)?)
    }

    /// The configuration the new binary will run with, checked before the
    /// running service is touched. Operator keys live in the service's
    /// environment, so only their presence in the file is checked.
    fn load_config(layout: &Layout) -> Result<Config, InstallError> {
        let error = |source| InstallError::Config {
            path: layout.config.clone(),
            source,
        };
        let config = Config::load(Some(&layout.config)).map_err(error)?;
        config.check().map_err(error)?;
        Ok(config)
    }

    fn service_active() -> Result<bool, InstallError> {
        Self::active(SERVICE)
    }

    pub(crate) fn active(service: &str) -> Result<bool, InstallError> {
        Ok(Invocation::new("systemctl")
            .args(["--user", "is-active", "--quiet", service])
            .succeeds(Echo::Hide)?)
    }

    /// Seconds between checks for running calls.
    const POLL: Duration = Duration::from_secs(2);
    /// The `retry-after` clients get while the daemon is replaced.
    const RETRY_AFTER: &str = "10";

    /// Registers this checkout as the project `ren`, built as Ren itself
    /// (docs/plans/hierarchy.md), unless a project already is: a promotion
    /// installs from the deployer's own worktree, never the checkout Ren is
    /// registered at. The install is done by now, so a refusal (another
    /// project holds the name or the repository) is reported, not failed.
    fn register(layout: &Layout) {
        let listed = layout
            .ren(&layout.binary)
            .args(["project", "list", "--json"])
            .json("ren project list", Listed::parse);
        // A list that does not read leaves it to `project add` to say.
        if let Ok(listed) = &listed
            && let Some(ren) = listed.ren()
        {
            Progress(&format!(
                "Ren is registered as {} at {}",
                ren.name, ren.repository
            ))
            .show();
            return;
        }
        let registered = layout
            .ren(&layout.binary)
            .args([
                "project",
                "add",
                "--name",
                "ren",
                "--factory",
                "ren",
                "--dir",
            ])
            .arg(&layout.repo)
            .capture();
        match registered {
            Ok(said) => Progress(said.trim()).show(),
            Err(error) => {
                println!("warning: this checkout is not registered as project ren: {error}");
            }
        }
    }

    fn control_state(layout: &Layout) -> Result<ControlState, InstallError> {
        Ok(layout
            .ren(&layout.client)
            .args(["control", "state"])
            .json("ren control state", ControlState::parse)?)
    }

    /// Waits until no call is running and no run is mid-way (unless `policy`
    /// drains them), and pauses admission, so the restart interrupts nothing.
    /// A call can start, or a run be prompted, between the check and the
    /// pause; then admission resumes and the wait continues. Polls are
    /// counted rather than timed, so this reads no clock.
    fn quiesce(
        layout: &Layout,
        idle_timeout: u64,
        policy: RunPolicy,
    ) -> Result<Quiesced, InstallError> {
        if !Self::service_active()? || !layout.binary.exists() {
            return Ok(Quiesced {
                admission: Quiet::Stopped,
                ending: Ending::default(),
            });
        }
        let ren = || layout.ren(&layout.client);
        let polls = idle_timeout.div_ceil(Self::POLL.as_secs()).max(1);
        let mut reported = None;
        for _ in 0..polls {
            let state = Self::control_state(layout)?;
            let runs = Runs::read(ren())?;
            match state.quiesce(&runs, policy) {
                Quiesce::Pause => {
                    Self::control(layout, &["pause", "--retry-after", Self::RETRY_AFTER])?;
                    let runs = Runs::read(ren())?;
                    let paused = Self::control_state(layout)?;
                    match paused.after_pause(&runs, policy) {
                        AfterPause::Quiet => {
                            // Marked, so an install that does not get to
                            // resume leaves a pause the next one knows.
                            PauseMark::of(layout).write(paused.revision())?;
                            Progress("Paused admission; clients wait and retry").show();
                            return Ok(Quiesced {
                                admission: Quiet::Paused,
                                ending: runs.ending(),
                            });
                        }
                        AfterPause::SlippedIn(busy) => {
                            Self::control(layout, &["resume"])?;
                            busy.report(&mut reported, idle_timeout);
                        }
                    }
                }
                Quiesce::LeaveAlone if state.paused_by(PauseMark::of(layout).read()) => {
                    Progress(
                        "The daemon is still paused by an earlier install that did not finish; this one resumes it",
                    ).show();
                    return Ok(Quiesced {
                        admission: Quiet::Paused,
                        ending: runs.ending(),
                    });
                }
                Quiesce::LeaveAlone => {
                    // A mark from an older pause is not this one's.
                    PauseMark::of(layout).clear();
                    Progress("The daemon was already paused; leaving it paused").show();
                    return Ok(Quiesced {
                        admission: Quiet::LeftAlone,
                        ending: runs.ending(),
                    });
                }
                Quiesce::Wait(busy) => busy.report(&mut reported, idle_timeout),
            }
            thread::sleep(Self::POLL);
        }
        Err(StillBusy {
            busy: reported.unwrap_or_else(|| Busy::of_runs(&Runs::default(), policy)),
            waited: idle_timeout,
        }
        .into())
    }

    fn control(layout: &Layout, args: &[&str]) -> Result<(), InstallError> {
        layout
            .ren(&layout.client)
            .arg("control")
            .args(args)
            .capture()?;
        Ok(())
    }

    /// Resumes admission if this task paused it, whether or not the upgrade
    /// succeeded: leaving clients refused would not help either way.
    fn release(layout: &Layout, quiet: Quiet) {
        match quiet {
            Quiet::Stopped | Quiet::LeftAlone => {}
            Quiet::Paused => match Self::control(layout, &["resume"]) {
                Ok(()) => {
                    PauseMark::of(layout).clear();
                    Progress("Resumed admission").show();
                }
                Err(error) => eprintln!(
                    "warning: could not resume admission ({error}); once the daemon runs, run `{} control resume`",
                    layout.binary.display()
                ),
            },
        }
    }

    /// Everything that changes a running installation.
    /// The daemon's container, its image built and the Podman socket its
    /// runs' containers are started through turned on: Podman's own, or,
    /// for runs under gVisor, `ren-gvisor`'s.
    fn contain(
        layout: &Layout,
        config: &Config,
        env: &HostEnv,
    ) -> Result<DaemonContainer, InstallError> {
        let container = DaemonContainer::new(
            "production",
            Pod::image(),
            &layout.binary,
            (&layout.config, config),
            &layout.data_dir,
            env,
            Vec::new(),
        )?;
        Pod::ensure_image(&layout.repo)?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now", "podman.socket"])
            .run()?;
        // Runs under gVisor are started through a Podman service of their
        // own, which [runs.container] podman_socket names.
        match config
            .runs
            .container
            .relayed
            .as_ref()
            .map(|relayed| relayed.runtime)
        {
            None | Some(RunRuntime::HostKernel) => {}
            Some(RunRuntime::Gvisor) => crate::gvisor_service::GvisorService::new(env)?.ensure()?,
        }
        Ok(container)
    }

    fn replace(
        layout: &Layout,
        built: &Path,
        backup: Backup,
        tree: Tree,
        contained: Option<&DaemonContainer>,
    ) -> Result<(), InstallError> {
        let held = Self::hold_binary(layout, backup)?;
        Upgrade::attempt(
            held,
            SERVICE,
            || {
                Progress(&format!("Installing {}", layout.binary.display())).show();
                let contents = fs::read(built).map_err(FileError::at(built))?;
                FileMode::Executable.write(&layout.binary, &contents)?;
                Self::install_unit(layout, contained)?;
                Self::enable_linger(layout)?;
                Self::doctor(layout)?;
                Self::verify_build(layout, tree)
            },
            || Self::doctor(layout),
        )
        .finish(Part::Daemon {
            data_dir: layout.data_dir.clone(),
        })
    }

    /// The installed CLI and the daemon must report the same build.
    fn verify_build(layout: &Layout, tree: Tree) -> Result<(), InstallError> {
        let report = layout
            .ren(&layout.binary)
            .args(["doctor", "--json"])
            .json("ren doctor --json", DoctorReport::parse)?;
        let Some(check) = report.build() else {
            return Err(InstallError::BuildMismatch(
                "doctor reported no build check".to_owned(),
            ));
        };
        match Self::verdict(check.status(), tree) {
            Verdict::Confirmed => {
                Progress(check.message()).show();
                Ok(())
            }
            Verdict::Unconfirmed => {
                eprintln!("warning: {}", check.message());
                Ok(())
            }
            Verdict::Mismatch => Err(InstallError::BuildMismatch(check.message().to_owned())),
        }
    }

    /// Doctor's build check, read for an install from a clean or modified
    /// checkout: only a clean one must be confirmed.
    fn verdict(status: &CheckStatus, tree: Tree) -> Verdict {
        match (status, tree) {
            (CheckStatus::Ok, Tree::Clean | Tree::Modified) => Verdict::Confirmed,
            (
                CheckStatus::Warning
                | CheckStatus::Failed
                | CheckStatus::Skipped
                | CheckStatus::Other(_),
                Tree::Modified,
            ) => Verdict::Unconfirmed,
            (
                CheckStatus::Warning
                | CheckStatus::Failed
                | CheckStatus::Skipped
                | CheckStatus::Other(_),
                Tree::Clean,
            ) => Verdict::Mismatch,
        }
    }

    /// Backs up history with the running daemon, stops it, and holds its
    /// binary aside for the new one.
    fn hold_binary(layout: &Layout, backup: Backup) -> Result<Option<Held>, InstallError> {
        let running = Self::service_active()?;
        if running && layout.binary.exists() {
            match backup {
                Backup::Run => {
                    Progress("Backing up history with the running daemon").show();
                    if !layout
                        .ren(&layout.client)
                        .args(["control", "backup"])
                        .succeeds(Echo::Show)?
                    {
                        return Err(InstallError::Backup {
                            data_dir: layout.data_dir.clone(),
                        });
                    }
                }
                Backup::Skip => Progress("Skipping the history backup").show(),
            }
        }
        if running {
            Progress("Stopping the service").show();
            Invocation::new("systemctl")
                .args(["--user", "stop", SERVICE])
                .run()?;
        }
        Held::hold(&layout.binary, &layout.rollback, &layout.unit)
    }

    /// The unit `ren service render` writes, running the daemon in
    /// `contained` when given.
    fn install_unit(
        layout: &Layout,
        contained: Option<&DaemonContainer>,
    ) -> Result<(), InstallError> {
        let rendered = layout
            .ren(&layout.binary)
            .args(["service", "render", "--manager", "systemd", "--binary"])
            .arg(&layout.binary)
            .capture()?;
        let unit = match contained {
            Some(container) => container.unit(&rendered)?,
            None => rendered,
        };
        Self::start_unit(&layout.unit, &unit, SERVICE)
    }

    /// Writes `rendered` to `unit` if it differs (verified first), then
    /// enables and starts `service`.
    pub(crate) fn start_unit(
        unit: &Path,
        rendered: &str,
        service: &str,
    ) -> Result<(), InstallError> {
        let current = fs::read_to_string(unit).ok();
        if current.as_deref() == Some(rendered) {
            Progress(&format!("Unit {} is up to date", unit.display())).show();
        } else {
            Progress(&format!("Writing {}", unit.display())).show();
            let dir = unit.parent().unwrap_or(Path::new("/"));
            let mut staged = tempfile::Builder::new()
                .prefix(".ren.")
                .suffix(".service")
                .tempfile_in(dir)
                .map_err(FileError::at(dir))?;
            staged
                .write_all(rendered.as_bytes())
                .map_err(FileError::at(unit))?;
            staged.as_file().sync_all().map_err(FileError::at(unit))?;
            Invocation::new("systemd-analyze")
                .args(["--user", "verify"])
                .arg(staged.path())
                .capture()?;
            staged
                .persist(unit)
                .map_err(|error| FileError::at(unit)(error.error))?;
        }
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Progress(&format!("Enabling and starting {service}")).show();
        Invocation::new("systemctl")
            .args(["--user", "reset-failed", service])
            .succeeds(Echo::Hide)?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now", service])
            .run()?;
        Ok(())
    }

    fn enable_linger(layout: &Layout) -> Result<(), InstallError> {
        let linger = Invocation::new("loginctl")
            .args(["show-user", &layout.user, "-p", "Linger", "--value"])
            .capture()
            .map_or(Linger::Disabled, |text| Linger::parse(&text));
        match linger {
            Linger::Enabled => Progress("Lingering is already enabled").show(),
            Linger::Disabled => {
                Progress("Enabling lingering so the service runs after logout").show();
                if !Invocation::new("loginctl")
                    .args(["enable-linger", &layout.user])
                    .succeeds(Echo::Show)?
                {
                    eprintln!(
                        "warning: could not enable lingering; ask an administrator to run `loginctl enable-linger {}`",
                        layout.user
                    );
                }
            }
        }
        Ok(())
    }

    fn doctor(layout: &Layout) -> Result<(), InstallError> {
        Progress("Checking the daemon with ren doctor").show();
        let doctor = layout.ren(&layout.binary).arg("doctor");
        // The unit is Type=exec: started is not yet listening.
        for _ in 0..10 {
            if doctor.succeeds(Echo::Hide)? {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
        if doctor.succeeds(Echo::Show)? {
            Ok(())
        } else {
            Err(InstallError::Doctor)
        }
    }

    /// `https://<node>:<port>`, or a description when the node has no name.
    pub(crate) fn url(dns: Option<&str>, port: u16) -> String {
        dns.map_or_else(
            || format!("(port {port} on this node)"),
            |name| format!("https://{name}:{port}"),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ren_is_registered_when_a_project_not_archived_is_built_as_ren() {
        let listed = |projects: &str| {
            super::Listed::parse(&format!("{{\"projects\":[{projects}]}}")).unwrap()
        };
        let project = |name: &str, factory: &str, archived: &str| {
            format!(
                "{{\"id\":\"x\",\"name\":\"{name}\",\"repository\":\"/home/op/{name}\",\"factory\":\"{factory}\",\"archived_at\":{archived},\"sessions\":0}}"
            )
        };
        let ren = listed(&project("ren", "ren", "null"));
        assert_eq!(
            ren.ren().map(|found| found.name.to_string()).as_deref(),
            Some("ren")
        );
        assert_eq!(
            ren.ren()
                .map(|found| found.repository.to_string())
                .as_deref(),
            Some("/home/op/ren")
        );
        // Under another name, Ren is still registered.
        assert!(listed(&project("factory", "ren", "null")).ren().is_some());
        // Archived, or only contract projects: not yet.
        assert!(listed(&project("ren", "ren", "1")).ren().is_none());
        assert!(
            listed(&project("acme-web", "contract", "null"))
                .ren()
                .is_none()
        );
        assert!(listed("").ren().is_none());
    }

    use super::*;

    #[test]
    fn a_clean_install_requires_a_confirmed_build() {
        assert_eq!(
            InstallVps::verdict(&CheckStatus::Ok, Tree::Clean),
            Verdict::Confirmed
        );
        assert_eq!(
            InstallVps::verdict(&CheckStatus::Ok, Tree::Modified),
            Verdict::Confirmed
        );
        for status in [
            CheckStatus::Warning,
            CheckStatus::Failed,
            CheckStatus::Skipped,
            CheckStatus::Other("new".to_owned()),
        ] {
            assert_eq!(InstallVps::verdict(&status, Tree::Clean), Verdict::Mismatch);
            assert_eq!(
                InstallVps::verdict(&status, Tree::Modified),
                Verdict::Unconfirmed
            );
        }
    }

    #[test]
    fn an_install_inside_the_service_it_restarts_knows_it() {
        use super::Placement;
        let inside = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/ren.service\n";
        assert_eq!(Placement::read(inside, "ren.service"), Placement::InService);
        let shell = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-term.scope\n";
        assert_eq!(Placement::read(shell, "ren.service"), Placement::Elsewhere);
        // Another unit whose name only starts the same is not the service.
        let ui = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/ren.service-ui\n";
        assert_eq!(Placement::read(ui, "ren.service"), Placement::Elsewhere);
        // The relaunched install's own unit is outside it.
        let relaunched =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/ren-install-vps.service\n";
        assert_eq!(
            Placement::read(relaunched, "ren.service"),
            Placement::Elsewhere
        );
        // Hybrid hierarchies list several lines; any one places it.
        let hybrid = "12:pids:/user.slice\n1:name=systemd:/user.slice/user@1000.service/app.slice/ren.service\n";
        assert_eq!(Placement::read(hybrid, "ren.service"), Placement::InService);
        assert_eq!(Placement::read("", "ren.service"), Placement::Elsewhere);
    }
}
