//! `cargo xtask install-vps`: install or upgrade TGV from this checkout as a
//! Linux systemd user service, optionally published with Tailscale Serve.
//!
//! An upgrade restarts the daemon only between calls: it waits until none is
//! running, pauses admission (clients get `503` with `retry-after` and wait),
//! replaces the binary, checks that the daemon runs the new build, and
//! resumes.
use crate::{
    daemon::{Admission, CheckStatus, ControlState, DoctorReport},
    invocation::{Echo, Invocation},
    publish::Publish,
    tailscale::{NodeStatus, Route, ServeStatus},
    ui::{UiDeploy, UiUrls},
};
use clap::Args;
use std::{
    fs,
    io::Write as _,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::ExitStatus,
    thread,
    time::Duration,
};
use tgv_daemon::config::{Config, ConfigError};
use tgv_harness::host::HostEnv;

/// Install or upgrade TGV from this checkout as a systemd user service.
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
    /// Skip `tgv control backup` before replacing a running daemon.
    #[arg(long)]
    skip_backup: bool,
    /// Install even though tracked files differ from the commit; the daemon
    /// then reports a `-dirty` revision.
    #[arg(long)]
    allow_dirty: bool,
    /// How long to wait for running calls to finish before giving up,
    /// without having changed anything.
    #[arg(long, value_name = "SECONDS", default_value_t = 900)]
    idle_timeout: u64,
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
    #[error(
        "doctor reported a failed check; see `journalctl --user -u tgv`. To roll back, stop the service, move {} to {}, and start it",
        previous.display(),
        binary.display()
    )]
    Doctor { previous: PathBuf, binary: PathBuf },
    #[error(
        "tracked files differ from the commit; commit them, or pass --allow-dirty to deploy a -dirty build"
    )]
    Dirty,
    #[error(
        "{running} calls were still running after {waited} seconds; nothing was changed. Run again between sessions, or raise --idle-timeout"
    )]
    Busy { running: usize, waited: u64 },
    #[error("the daemon does not run the build just installed: {0}")]
    BuildMismatch(String),
    #[error("{} does not hold valid install-vps flags; fix or delete it", .0.display())]
    SavedOptions(PathBuf),
    #[error("{0} is not on PATH")]
    MissingProgram(&'static str),
    #[error("deploy/tgv-ui.service no longer has the line {0:?} to render")]
    UiTemplate(&'static str),
    #[error(
        "the UI does not answer {url}; see `journalctl --user -u tgv-ui`. To roll back, stop tgv-ui, move {} to {}, and start it",
        previous.display(),
        dir.display()
    )]
    UiHealth {
        url: String,
        previous: PathBuf,
        dir: PathBuf,
    },
    #[error("Tailscale reports no MagicDNS name for this node, which the UI's origin needs")]
    NoDnsName,
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

/// What the upgrade did to admission before replacing a running daemon.
#[must_use]
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
#[derive(Clone, Copy)]
enum TailscaleCli {
    Direct,
    Sudo,
}

impl TailscaleCli {
    fn invocation(self) -> Invocation {
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
    previous: PathBuf,
    config: PathBuf,
    data_dir: PathBuf,
    unit: PathBuf,
}

const SERVICE: &str = "tgv.service";

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
            binary: bin_dir.join("tgv"),
            previous: bin_dir.join("tgv.previous"),
            bin_dir,
            config: home.join(".config/tgv/tgv.toml"),
            data_dir: home.join(".local/state/tgv"),
            unit: home.join(".config/systemd/user").join(SERVICE),
        })
    }

    /// `tgv` with this layout's configuration and data directory.
    fn tgv(&self, binary: &Path) -> Invocation {
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
        Self::preflight()?;
        let layout = Layout::new(env)?;
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
        let quiet = Self::quiesce(&layout, self.idle_timeout)?;
        let replaced = Self::replace(&layout, &built, backup, tree);
        Self::release(&layout, quiet);
        replaced?;
        let dns = match publish.https_port {
            Some(port) => {
                let dns = Self::publish(tailscale, port, &format!("http://{}", config.listen))?;
                let url = Self::url(dns.as_deref(), port);
                println!(
                    "\nGateway root URL for clients: {url}\n  tgv init --gateway {url}\n  tgv run claude\n"
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
        Ok(())
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

    fn build(layout: &Layout) -> Result<PathBuf, InstallError> {
        Self::step("Building the release binary");
        Invocation::new(&layout.cargo)
            .args(["build", "--release", "--locked", "--package", "tgv-rs"])
            .args(["--bin", "tgv", "--manifest-path"])
            .arg(layout.repo.join("Cargo.toml"))
            .run()?;
        let built = layout.repo.join("target/release/tgv");
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
            "Creating {} from tgv.example.toml (review it)",
            layout.config.display()
        ));
        let example = layout.repo.join("tgv.example.toml");
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
            layout.tgv(&layout.binary).args(["control", "state"]),
            "tgv control state",
            ControlState::parse,
        )
    }

    /// Waits until no call is running and pauses admission, so the restart
    /// interrupts nothing. A call can start between the check and the pause;
    /// then admission resumes and the wait continues. Polls are counted
    /// rather than timed, so this reads no clock.
    fn quiesce(layout: &Layout, idle_timeout: u64) -> Result<Quiet, InstallError> {
        if !Self::service_active()? || !layout.binary.exists() {
            return Ok(Quiet::Stopped);
        }
        let polls = idle_timeout.div_ceil(Self::POLL.as_secs()).max(1);
        let mut reported = None;
        let mut running = 0;
        for _ in 0..polls {
            let state = Self::control_state(layout)?;
            running = state.running();
            match state.admission() {
                Admission::Running if running == 0 => {
                    Self::control(layout, &["pause", "--retry-after", Self::RETRY_AFTER])?;
                    if Self::control_state(layout)?.running() == 0 {
                        Self::step("Paused admission; clients wait and retry");
                        return Ok(Quiet::Paused);
                    }
                    Self::control(layout, &["resume"])?;
                }
                Admission::Paused | Admission::Other(_) if running == 0 => {
                    Self::step("The daemon was already paused; leaving it paused");
                    return Ok(Quiet::LeftAlone);
                }
                Admission::Running | Admission::Paused | Admission::Other(_) => {
                    if reported != Some(running) {
                        Self::step(&format!(
                            "Waiting for {running} running calls to finish (up to {idle_timeout} seconds)"
                        ));
                        reported = Some(running);
                    }
                }
            }
            thread::sleep(Self::POLL);
        }
        Err(InstallError::Busy {
            running,
            waited: idle_timeout,
        })
    }

    fn control(layout: &Layout, args: &[&str]) -> Result<(), InstallError> {
        layout
            .tgv(&layout.binary)
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
                Ok(()) => Self::step("Resumed admission"),
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
        Self::install_binary(layout, built, backup)?;
        Self::install_unit(layout)?;
        Self::enable_linger(layout)?;
        Self::doctor(layout)?;
        Self::verify_build(layout, tree)
    }

    /// The installed CLI and the daemon must report the same build.
    fn verify_build(layout: &Layout, tree: Tree) -> Result<(), InstallError> {
        let report = Self::json(
            layout.tgv(&layout.binary).args(["doctor", "--json"]),
            "tgv doctor --json",
            DoctorReport::parse,
        )?;
        let Some(check) = report.build() else {
            return Err(InstallError::BuildMismatch(
                "doctor reported no build check".to_owned(),
            ));
        };
        match (check.status(), tree) {
            (CheckStatus::Ok, Tree::Clean | Tree::Modified) => {
                Self::step(check.message());
                Ok(())
            }
            (
                CheckStatus::Warning
                | CheckStatus::Failed
                | CheckStatus::Skipped
                | CheckStatus::Other(_),
                Tree::Modified,
            ) => {
                eprintln!("warning: {}", check.message());
                Ok(())
            }
            (
                CheckStatus::Warning
                | CheckStatus::Failed
                | CheckStatus::Skipped
                | CheckStatus::Other(_),
                Tree::Clean,
            ) => Err(InstallError::BuildMismatch(check.message().to_owned())),
        }
    }

    fn install_binary(layout: &Layout, built: &Path, backup: Backup) -> Result<(), InstallError> {
        let running = Self::service_active()?;
        if running && layout.binary.exists() {
            match backup {
                Backup::Run => {
                    Self::step("Backing up history with the running daemon");
                    if !layout
                        .tgv(&layout.binary)
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
        if layout.binary.exists() {
            fs::copy(&layout.binary, &layout.previous)
                .map_err(InstallError::file(&layout.previous))?;
        }
        Self::step(&format!("Installing {}", layout.binary.display()));
        let contents = fs::read(built).map_err(InstallError::file(built))?;
        Self::write_private(&layout.binary, &contents, 0o755)
    }

    /// Publishes `contents` at `path` atomically, with `mode`.
    pub(crate) fn write_private(
        path: &Path,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), InstallError> {
        let dir = path.parent().unwrap_or(Path::new("/"));
        let mut file = tempfile::Builder::new()
            .prefix(".tgv.")
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
            .tgv(&layout.binary)
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
                .prefix(".tgv.")
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
        Self::step("Checking the daemon with tgv doctor");
        let doctor = layout.tgv(&layout.binary).arg("doctor");
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
            Err(InstallError::Doctor {
                previous: layout.previous.clone(),
                binary: layout.binary.clone(),
            })
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
    fn publish(
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
                tailscale
                    .invocation()
                    .args(["serve", "--bg", &format!("--https={port}"), target])
                    .run()?;
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
    fn url(dns: Option<&str>, port: u16) -> String {
        dns.map_or_else(
            || format!("(port {port} on this node)"),
            |name| format!("https://{name}:{port}"),
        )
    }
}
