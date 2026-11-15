//! `cargo xtask install-vps`: install or upgrade Ren from this checkout as a
//! Linux systemd user service, optionally published with Tailscale Serve.
//!
//! An upgrade restarts the daemon only between calls and between runs' turns:
//! it waits until no call is running and no run is starting, working, or
//! stopping, pauses admission (clients get `503` with `retry-after` and
//! wait), replaces the binary, checks that the daemon runs the new build, and
//! resumes. Idle runs end with the old daemon; it names each, with how to
//! continue its conversation.
use crate::{
    daemon::{
        AfterPause, Busy, CheckStatus, ControlState, DoctorReport, LiveRun, Quiesce, RunPolicy,
        Runs,
    },
    invocation::{Echo, Invocation},
    publish::Publish,
    rollback::{Held, Part, Upgrade},
    tailscale::{NodeStatus, Route, ServeStatus},
    ui::{UiDeploy, UiUrls},
};
use clap::Args;
use ren_daemon::config::{Config, ConfigError};
use ren_harness::host::HostEnv;
use std::{
    fs,
    io::Write as _,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::ExitStatus,
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
    #[arg(long)]
    allow_dirty: bool,
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
    #[error("run as the regular user that owns the service, not root")]
    Root,
    #[error("{0} is not set")]
    MissingEnv(&'static str),
    #[error("`{command}` could not start: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` exited with {status}{detail}")]
    Failed {
        command: String,
        status: ExitStatus,
        detail: String,
    },
    #[error("unexpected output from `{command}`: {detail}")]
    Output { command: String, detail: String },
    #[error("{}: {source}", path.display())]
    File {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
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
    #[error(
        "tracked files differ from the commit; commit them, or pass --allow-dirty to deploy a -dirty build"
    )]
    Dirty,
    #[error(
        "{} still running after {waited} seconds; nothing was changed. {}",
        .busy.summary(),
        .busy.advice()
    )]
    Busy { busy: Busy, waited: u64 },
    #[error("the daemon does not run the build just installed: {0}")]
    BuildMismatch(String),
    #[error("{} does not hold valid install-vps flags; fix or delete it", .0.display())]
    SavedOptions(PathBuf),
    #[error("{0} is not on PATH")]
    MissingProgram(&'static str),
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
    #[error("Tailscale reports no MagicDNS name for this node, which the UI's origin needs")]
    NoDnsName,
    #[error(
        "Tailscale Serve did not publish port {port}. If it refused this user, run again with --sudo-tailscale (the choice is saved), or allow this user once with `sudo tailscale set --operator=$USER`"
    )]
    ServeRefused { port: u16 },
    #[error("Tailscale is {0}, not running: run `tailscale up` first")]
    TailscaleDown(String),
    #[error("Tailscale Serve port {port} is in use ({route}); choose another --https-port")]
    PortTaken { port: u16, route: Route },
}

/// Whether to back up history before replacing a running daemon.
#[derive(Clone, Copy)]
enum Backup {
    Run,
    Skip,
}

/// Whether tracked files match the commit being installed.
#[derive(Clone, Copy)]
enum Tree {
    Clean,
    Modified,
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
    ending: Vec<LiveRun>,
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

/// How to run `tailscale` for the Serve step.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TailscaleCli {
    Direct,
    Sudo,
}

impl TailscaleCli {
    pub(crate) fn invocation(self) -> Invocation {
        match self {
            Self::Direct => Invocation::new("tailscale"),
            Self::Sudo => Invocation::new("sudo").args(["-n", "tailscale"]),
        }
    }
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
                .map_err(InstallError::file(&self.path)),
            None => Ok(()),
        }
    }

    /// Gone once its pause is resumed or another's; a missing mark is fine.
    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Layout {
    fn new(env: &HostEnv) -> Result<Self, InstallError> {
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
            // xtask/ sits at the root of the checkout.
            repo: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            // `cargo xtask` sets CARGO to the cargo running it.
            cargo: env
                .absolute_path("CARGO")
                .unwrap_or_else(|| PathBuf::from("cargo")),
            user,
            binary: bin_dir.join("ren"),
            rollback: bin_dir.join("ren.rollback"),
            bin_dir,
            config: home.join(".config/ren/ren.toml"),
            data_dir: home.join(".local/state/ren"),
            unit: home.join(".config/systemd/user").join(SERVICE),
        })
    }

    /// `ren` with this layout's configuration and data directory.
    fn ren(&self, binary: &Path) -> Invocation {
        Invocation::new(binary)
            .arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data_dir)
    }
}

impl InstallError {
    /// Wraps an I/O error with the path it concerns.
    pub(crate) fn file(path: &Path) -> impl FnOnce(std::io::Error) -> Self + '_ {
        move |source| Self::File {
            path: path.to_path_buf(),
            source,
        }
    }
}

impl InstallVps {
    pub fn run(self, env: &HostEnv) -> Result<(), InstallError> {
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
        let layout = Layout::new(env)?;
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
            TailscaleCli::Sudo
        } else {
            TailscaleCli::Direct
        };
        let tree = Self::check_tree(&layout, self.allow_dirty)?;
        let ui = match publish.ui_https_port {
            Some(_) => Some(UiDeploy::new(env, &layout.home, &layout.repo)?),
            None => None,
        };
        let built = Self::build(&layout)?;
        if let Some(ui) = &ui {
            ui.build()?;
        }
        Self::seed_config(&layout)?;
        let config = Self::load_config(&layout)?;
        let quiesced = Self::quiesce(&layout, self.idle_timeout, policy)?;
        Self::announce_ending(&quiesced.ending);
        let replaced = Self::replace(&layout, &built, backup, tree);
        Self::release(&layout, quiesced.admission);
        replaced?;
        let dns = match publish.https_port {
            Some(port) => {
                let dns = Self::publish(tailscale, port, &format!("http://{}", config.listen))?;
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
            let dns = dns.as_deref().ok_or(InstallError::NoDnsName)?;
            ui.install(&UiUrls {
                gateway: format!("http://{}", config.listen),
                public_gateway: format!("https://{dns}:{port}"),
                origin: format!("https://{dns}:{ui_port}"),
            })?;
            Self::publish(tailscale, ui_port, UiDeploy::TARGET)?;
            println!("\nUI: {}\n", Self::url(Some(dns), ui_port));
        }
        publish.save(&saved)?;
        Self::step("Done");
        Self::remind_ending(&quiesced.ending);
        Ok(())
    }

    /// Runs this same command again as the transient user unit
    /// [`RELAUNCH_UNIT`], outside `ren.service`, and waits for it. Its output
    /// goes to the journal: this process may be stopped with the service,
    /// and the install carries on without it.
    fn relaunch(env: &HostEnv, layout: &Layout) -> Result<(), InstallError> {
        let program = std::env::current_exe().map_err(InstallError::file(Path::new("xtask")))?;
        Self::step(&format!(
            "This runs inside {SERVICE}, which the install restarts; continuing as the unit {RELAUNCH_UNIT}. Follow it with `journalctl --user -u {RELAUNCH_UNIT} -f`"
        ));
        let mut relaunch = Invocation::new("systemd-run")
            .args([
                "--user",
                "--unit",
                RELAUNCH_UNIT,
                "--collect",
                "--wait",
                "--quiet",
            ])
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
            .run()
    }

    pub(crate) fn step(message: &str) {
        println!("==> {message}");
    }

    fn preflight() -> Result<(), InstallError> {
        if !cfg!(target_os = "linux") {
            return Err(InstallError::NotLinux);
        }
        let id = Invocation::new("id").arg("-u");
        let uid = id.capture()?;
        let uid: u32 = uid.trim().parse().map_err(|_| InstallError::Output {
            command: "id -u".to_owned(),
            detail: uid.trim().to_owned(),
        })?;
        if uid == 0 {
            return Err(InstallError::Root);
        }
        Invocation::new("systemctl")
            .args(["--user", "show-environment"])
            .capture()
            .map(drop)
    }

    /// Refuses modified tracked files unless allowed, and warns when the
    /// commit is on no remote branch, so the deployed source is saved
    /// somewhere other than this host.
    fn check_tree(layout: &Layout, allow_dirty: bool) -> Result<Tree, InstallError> {
        let git = || Invocation::new("git").arg("-C").arg(&layout.repo);
        let status = git()
            .args(["status", "--porcelain", "--untracked-files=no"])
            .capture()?;
        let tree = if status.trim().is_empty() {
            Tree::Clean
        } else if allow_dirty {
            eprintln!("warning: tracked files differ from the commit; installing a -dirty build");
            Tree::Modified
        } else {
            return Err(InstallError::Dirty);
        };
        let commit = git().args(["rev-parse", "--short=12", "HEAD"]).capture()?;
        let remotes = git()
            .args(["branch", "--remotes", "--contains", "HEAD"])
            .capture()?;
        if remotes.trim().is_empty() {
            eprintln!(
                "warning: commit {} is on no remote branch; push it so the deployed source is saved",
                commit.trim()
            );
        }
        Self::step(&format!("Installing commit {}", commit.trim()));
        Ok(tree)
    }

    /// The release binary: the installed `ren` is the operator's own, on the
    /// host they drive sessions from.
    fn build(layout: &Layout) -> Result<PathBuf, InstallError> {
        Self::step("Building the release binary");
        Invocation::new(&layout.cargo)
            .args(["build", "--release", "--locked", "--package", "ren-rs"])
            .args(["--bin", "ren", "--manifest-path"])
            .arg(layout.repo.join("Cargo.toml"))
            .run()?;
        let built = layout.repo.join("target/release/ren");
        fs::canonicalize(&built).map_err(InstallError::file(&built))
    }

    fn seed_config(layout: &Layout) -> Result<(), InstallError> {
        let unit_dir = layout.unit.parent().unwrap_or(Path::new("/"));
        let config_dir = layout.config.parent().unwrap_or(Path::new("/"));
        for dir in [config_dir, unit_dir, &layout.bin_dir, &layout.data_dir] {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(InstallError::file(dir))?;
        }
        if layout.config.exists() {
            Self::step(&format!(
                "Keeping existing configuration {}",
                layout.config.display()
            ));
            return Ok(());
        }
        Self::step(&format!(
            "Creating {} from ren.example.toml (review it)",
            layout.config.display()
        ));
        let example = layout.repo.join("ren.example.toml");
        let contents = fs::read(&example).map_err(InstallError::file(&example))?;
        Self::write_private(&layout.config, &contents, 0o600)
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
        Invocation::new("systemctl")
            .args(["--user", "is-active", "--quiet", service])
            .succeeds(Echo::Hide)
    }

    /// Seconds between checks for running calls.
    const POLL: Duration = Duration::from_secs(2);
    /// The `retry-after` clients get while the daemon is replaced.
    const RETRY_AFTER: &str = "10";

    fn control_state(layout: &Layout) -> Result<ControlState, InstallError> {
        Self::json(
            layout.ren(&layout.binary).args(["control", "state"]),
            "ren control state",
            ControlState::parse,
        )
    }

    /// The runs the daemon drives now, through `ren` (`ren runs ls`).
    pub(crate) fn runs(ren: Invocation) -> Result<Runs, InstallError> {
        let text = ren.args(["runs", "ls"]).answer()?;
        Runs::parse(&text).map_err(|error| InstallError::Output {
            command: "ren runs ls".to_owned(),
            detail: error.to_string(),
        })
    }

    /// Says what a restart waits for, when it changed since `reported`.
    pub(crate) fn report_wait(busy: &Busy, reported: &mut Option<Busy>, timeout: u64) {
        if reported.as_ref() == Some(busy) {
            return;
        }
        Self::step(&format!(
            "Waiting for {} to finish (up to {timeout} seconds)",
            busy.summary()
        ));
        for line in busy.run_lines() {
            println!("    {line}");
        }
        *reported = Some(busy.clone());
    }

    /// Names the runs whose process ends with the daemon about to stop.
    pub(crate) fn announce_ending(ending: &[LiveRun]) {
        if ending.is_empty() {
            return;
        }
        Self::step(&format!(
            "{} end with the daemon; each conversation can be continued:",
            match ending.len() {
                1 => "1 run".to_owned(),
                n => format!("{n} runs"),
            }
        ));
        for run in ending {
            println!("    {run}\n      {}", run.continuation());
        }
    }

    /// After the restart, once more, so it is the last thing shown.
    pub(crate) fn remind_ending(ending: &[LiveRun]) {
        match ending.len() {
            0 => {}
            1 => println!("1 run ended with the restart (listed above); it can be continued."),
            n => println!("{n} runs ended with the restart (listed above); each can be continued."),
        }
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
                ending: Vec::new(),
            });
        }
        let ren = || layout.ren(&layout.binary);
        let polls = idle_timeout.div_ceil(Self::POLL.as_secs()).max(1);
        let mut reported = None;
        for _ in 0..polls {
            let state = Self::control_state(layout)?;
            let runs = Self::runs(ren())?;
            match state.quiesce(&runs, policy) {
                Quiesce::Pause => {
                    Self::control(layout, &["pause", "--retry-after", Self::RETRY_AFTER])?;
                    let runs = Self::runs(ren())?;
                    let paused = Self::control_state(layout)?;
                    match paused.after_pause(&runs, policy) {
                        AfterPause::Quiet => {
                            // Marked, so an install that does not get to
                            // resume leaves a pause the next one knows.
                            PauseMark::of(layout).write(paused.revision())?;
                            Self::step("Paused admission; clients wait and retry");
                            return Ok(Quiesced {
                                admission: Quiet::Paused,
                                ending: runs.ending(),
                            });
                        }
                        AfterPause::SlippedIn(busy) => {
                            Self::control(layout, &["resume"])?;
                            Self::report_wait(&busy, &mut reported, idle_timeout);
                        }
                    }
                }
                Quiesce::LeaveAlone if state.paused_by(PauseMark::of(layout).read()) => {
                    Self::step(
                        "The daemon is still paused by an earlier install that did not finish; this one resumes it",
                    );
                    return Ok(Quiesced {
                        admission: Quiet::Paused,
                        ending: runs.ending(),
                    });
                }
                Quiesce::LeaveAlone => {
                    // A mark from an older pause is not this one's.
                    PauseMark::of(layout).clear();
                    Self::step("The daemon was already paused; leaving it paused");
                    return Ok(Quiesced {
                        admission: Quiet::LeftAlone,
                        ending: runs.ending(),
                    });
                }
                Quiesce::Wait(busy) => Self::report_wait(&busy, &mut reported, idle_timeout),
            }
            thread::sleep(Self::POLL);
        }
        Err(InstallError::Busy {
            busy: reported.unwrap_or_else(|| Busy::of_runs(&Runs::default(), policy)),
            waited: idle_timeout,
        })
    }

    fn control(layout: &Layout, args: &[&str]) -> Result<(), InstallError> {
        layout
            .ren(&layout.binary)
            .arg("control")
            .args(args)
            .capture()
            .map(drop)
    }

    /// Resumes admission if this task paused it, whether or not the upgrade
    /// succeeded: leaving clients refused would not help either way.
    fn release(layout: &Layout, quiet: Quiet) {
        match quiet {
            Quiet::Stopped | Quiet::LeftAlone => {}
            Quiet::Paused => match Self::control(layout, &["resume"]) {
                Ok(()) => {
                    PauseMark::of(layout).clear();
                    Self::step("Resumed admission");
                }
                Err(error) => eprintln!(
                    "warning: could not resume admission ({error}); once the daemon runs, run `{} control resume`",
                    layout.binary.display()
                ),
            },
        }
    }

    /// Everything that changes a running installation.
    fn replace(
        layout: &Layout,
        built: &Path,
        backup: Backup,
        tree: Tree,
    ) -> Result<(), InstallError> {
        let held = Self::hold_binary(layout, backup)?;
        Upgrade::attempt(
            held,
            SERVICE,
            || {
                Self::step(&format!("Installing {}", layout.binary.display()));
                let contents = fs::read(built).map_err(InstallError::file(built))?;
                Self::write_private(&layout.binary, &contents, 0o755)?;
                Self::install_unit(layout)?;
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
        let report = Self::json(
            layout.ren(&layout.binary).args(["doctor", "--json"]),
            "ren doctor --json",
            DoctorReport::parse,
        )?;
        let Some(check) = report.build() else {
            return Err(InstallError::BuildMismatch(
                "doctor reported no build check".to_owned(),
            ));
        };
        match Self::verdict(check.status(), tree) {
            Verdict::Confirmed => {
                Self::step(check.message());
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
                    Self::step("Backing up history with the running daemon");
                    if !layout
                        .ren(&layout.binary)
                        .args(["control", "backup"])
                        .succeeds(Echo::Show)?
                    {
                        return Err(InstallError::Backup {
                            data_dir: layout.data_dir.clone(),
                        });
                    }
                }
                Backup::Skip => Self::step("Skipping the history backup"),
            }
        }
        if running {
            Self::step("Stopping the service");
            Invocation::new("systemctl")
                .args(["--user", "stop", SERVICE])
                .run()?;
        }
        // Kept by installs before rollback was automatic.
        Held::clear(&layout.bin_dir.join("ren.previous"))?;
        Held::hold(&layout.binary, &layout.rollback, &layout.unit)
    }

    /// Publishes `contents` at `path` atomically, with `mode`.
    pub(crate) fn write_private(
        path: &Path,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), InstallError> {
        let dir = path.parent().unwrap_or(Path::new("/"));
        let mut file = tempfile::Builder::new()
            .prefix(".ren.")
            .tempfile_in(dir)
            .map_err(InstallError::file(dir))?;
        file.write_all(contents).map_err(InstallError::file(path))?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))
            .map_err(InstallError::file(path))?;
        file.as_file()
            .sync_all()
            .map_err(InstallError::file(path))?;
        file.persist(path)
            .map_err(|error| InstallError::file(path)(error.error))?;
        Ok(())
    }

    fn install_unit(layout: &Layout) -> Result<(), InstallError> {
        let rendered = layout
            .ren(&layout.binary)
            .args(["service", "render", "--manager", "systemd", "--binary"])
            .arg(&layout.binary)
            .capture()?;
        Self::start_unit(&layout.unit, &rendered, SERVICE)
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
            Self::step(&format!("Unit {} is up to date", unit.display()));
        } else {
            Self::step(&format!("Writing {}", unit.display()));
            let dir = unit.parent().unwrap_or(Path::new("/"));
            let mut staged = tempfile::Builder::new()
                .prefix(".ren.")
                .suffix(".service")
                .tempfile_in(dir)
                .map_err(InstallError::file(dir))?;
            staged
                .write_all(rendered.as_bytes())
                .map_err(InstallError::file(unit))?;
            staged
                .as_file()
                .sync_all()
                .map_err(InstallError::file(unit))?;
            Invocation::new("systemd-analyze")
                .args(["--user", "verify"])
                .arg(staged.path())
                .capture()?;
            staged
                .persist(unit)
                .map_err(|error| InstallError::file(unit)(error.error))?;
        }
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Self::step(&format!("Enabling and starting {service}"));
        Invocation::new("systemctl")
            .args(["--user", "reset-failed", service])
            .succeeds(Echo::Hide)?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now", service])
            .run()
    }

    fn enable_linger(layout: &Layout) -> Result<(), InstallError> {
        let linger = Invocation::new("loginctl")
            .args(["show-user", &layout.user, "-p", "Linger", "--value"])
            .capture()
            .map_or(Linger::Disabled, |text| Linger::parse(&text));
        match linger {
            Linger::Enabled => Self::step("Lingering is already enabled"),
            Linger::Disabled => {
                Self::step("Enabling lingering so the service runs after logout");
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
        Self::step("Checking the daemon with ren doctor");
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

    fn json<T>(
        invocation: Invocation,
        command: &str,
        parse: fn(&str) -> Result<T, serde_json::Error>,
    ) -> Result<T, InstallError> {
        let text = invocation.capture()?;
        parse(&text).map_err(|error| InstallError::Output {
            command: command.to_owned(),
            detail: error.to_string(),
        })
    }

    /// Publishes `target` privately on `port`, and returns the node's
    /// MagicDNS name.
    pub(crate) fn publish(
        tailscale: TailscaleCli,
        port: u16,
        target: &str,
    ) -> Result<Option<String>, InstallError> {
        let node = Self::json(
            Invocation::new("tailscale").args(["status", "--json"]),
            "tailscale status --json",
            NodeStatus::parse,
        )?;
        if let Some(state) = node.not_running() {
            return Err(InstallError::TailscaleDown(state.to_owned()));
        }
        let serve = Self::json(
            Invocation::new("tailscale").args(["serve", "status", "--json"]),
            "tailscale serve status --json",
            ServeStatus::parse,
        )?;
        match serve.route(port, target) {
            Route::Free => {
                Self::step(&format!(
                    "Publishing {target} on the tailnet at port {port}"
                ));
                let served = tailscale
                    .invocation()
                    .args(["serve", "--bg", &format!("--https={port}"), target])
                    .run();
                match (served, tailscale) {
                    (Ok(()), TailscaleCli::Direct | TailscaleCli::Sudo) => {}
                    // Tailscale refuses Serve changes from a user who is not its
                    // operator; its own message is already on the terminal.
                    (Err(InstallError::Failed { .. }), TailscaleCli::Direct) => {
                        return Err(InstallError::ServeRefused { port });
                    }
                    (Err(error), TailscaleCli::Direct | TailscaleCli::Sudo) => return Err(error),
                }
            }
            Route::Target => Self::step(&format!(
                "Tailscale Serve port {port} already proxies to {target}"
            )),
            route @ (Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward) => {
                return Err(InstallError::PortTaken { port, route });
            }
        }
        Ok(node.dns_name().map(str::to_owned))
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
