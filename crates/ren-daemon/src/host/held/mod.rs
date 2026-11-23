//! Runs held by run hosts, which outlive the daemon (self-build plan,
//! phase 2). Each run's client process is started by a run host,
//! `ren run-host <folder>`, a process of its own: under systemd, in a
//! transient user unit of its own, so stopping `ren.service` leaves it be.
//! The host keeps the client's stdin open, appends every byte the client
//! writes to the run's spool, writes its exit when it exits, and serves one
//! daemon at a time over its socket ([`frame`]).
//!
//! A run's folder, `<data>/runs/<run>`, holds:
//!
//! - `spec.json`: the command the host starts;
//! - `host.sock`: the host's socket, while it runs;
//! - `spool`: every byte the client wrote;
//! - `exit`: how the client exited, once it has;
//! - `journal`: what the daemon gave the client (the supervisor's).
//!
//! The daemon's side is [`HeldRuns`]; the host's is [`RunHost`].

mod container;
pub mod frame;
mod link;
mod serve;

pub use container::{Access, Mount, PodmanReach, RunContainers};
pub use serve::RunHost;

use container::RunNeeds;
use ren_harness::profile::HomeEntry;

use ren_recorder::id::RunId;
use ren_supervisor::{AgentProcess, HeldRun, ProcessExit, Tenure};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::io::{self, Read as _, Seek as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How a run host is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launcher {
    /// In a transient systemd user unit of its own, `ren-run-<run>`, so the
    /// daemon's unit stopping leaves it be.
    Systemd,
    /// As a child of the daemon, in a process group of its own. It outlives
    /// the daemon's process, not its service's stop: for tests, and hosts
    /// without a user manager.
    Direct,
    /// In a rootless Podman container of its own, `ren-run-<run>`, that
    /// sees only what the run works on and what its client keeps, started
    /// from a systemd scope of its own.
    Container(RunContainers),
}

/// The runs this daemon holds through run hosts: their folders, and how a
/// host is started.
#[derive(Debug, Clone)]
pub struct HeldRuns {
    root: PathBuf,
    launcher: Launcher,
    /// The `ren` a host runs as.
    program: PathBuf,
}

/// The command a run host starts: what the daemon would have run itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub program: String,
    pub args: Vec<String>,
    /// Variables set for it, over the host's own environment.
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

impl Spec {
    /// `command`, as set up: its program, arguments, the variables it sets,
    /// and its folder. Text that is not UTF-8 is refused.
    pub fn of(command: &std::process::Command) -> io::Result<Self> {
        let text = |value: &OsStr| {
            value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("a run's command is not UTF-8"))
        };
        Ok(Self {
            program: text(command.get_program())?,
            args: command.get_args().map(text).collect::<io::Result<_>>()?,
            env: command
                .get_envs()
                .filter_map(|(key, value)| Some((key, value?)))
                .map(|(key, value)| Ok((text(key)?, text(value)?)))
                .collect::<io::Result<_>>()?,
            cwd: command
                .get_current_dir()
                .map(Path::to_path_buf)
                .ok_or_else(|| io::Error::other("a run's command has no folder"))?,
        })
    }
}

impl Spec {
    /// The program's file: as given when it is a path, else the first on
    /// the search path the command sets (a held run's always sets one).
    fn program(&self) -> io::Result<PathBuf> {
        let given = Path::new(&self.program);
        if given.components().count() > 1 {
            return std::fs::canonicalize(given);
        }
        let path = self
            .env
            .iter()
            .find(|(key, _)| key == "PATH")
            .map(|(_, value)| std::ffi::OsString::from(value))
            .unwrap_or_default();
        std::env::split_paths(&path)
            .map(|dir| dir.join(given))
            .find(|candidate| candidate.is_file())
            .map(std::fs::canonicalize)
            .unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} is not on the run's path", self.program),
                ))
            })
    }

    /// The git folder of the repository `workdir` is in, where it is not
    /// under `workdir` itself: a worktree's common folder, or the folder of
    /// a repository `workdir` is a subfolder of.
    fn git_folder(workdir: &Path) -> Option<PathBuf> {
        let (root, dot_git) = workdir
            .ancestors()
            .map(|dir| (dir, dir.join(".git")))
            .find(|(_, dot_git)| dot_git.exists())?;
        if dot_git.is_dir() {
            return (root != workdir).then_some(dot_git);
        }
        // A worktree's `.git` names its folder in the common one, which
        // names the common folder relative to itself.
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let own = root.join(text.trim().strip_prefix("gitdir:")?.trim());
        let common = std::fs::read_to_string(own.join("commondir")).ok()?;
        std::fs::canonicalize(own.join(common.trim())).ok()
    }
}

impl HeldRuns {
    pub const FOLDER: &str = "runs";
    const SPEC: &str = "spec.json";
    const SOCKET: &str = "host.sock";
    const SPOOL: &str = "spool";
    const EXIT: &str = "exit";
    const JOURNAL: &str = "journal";
    /// The Podman client's settings, under the runs' folder, for a daemon
    /// in a container.
    const PODMAN_SETTINGS: &str = ".podman";

    /// Runs held under `root` (`<data>/runs`), their hosts started by
    /// `launcher` running `program`.
    pub fn new(root: PathBuf, launcher: Launcher, program: PathBuf) -> Self {
        Self {
            root,
            launcher,
            program,
        }
    }

    /// The systemd unit a run's host runs in, under [`Launcher::Systemd`].
    pub fn unit(run: RunId) -> String {
        format!("ren-run-{run}")
    }

    /// The runs whose folders are under `root`, by their names: what
    /// stopping their hosts needs, whatever state the folders are in.
    pub fn folders(root: &Path) -> io::Result<Vec<RunId>> {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut runs = Vec::new();
        for entry in entries {
            if let Some(run) = entry?
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<uuid::Uuid>().ok())
            {
                runs.push(RunId::restore(run));
            }
        }
        runs.sort();
        Ok(runs)
    }

    fn folder(&self, run: RunId) -> PathBuf {
        self.root.join(run.to_string())
    }

    /// Start `command`, `run`'s client, under a run host, and attach to it.
    /// `keeps` is what the client keeps under the home, which a run host
    /// in a container is given.
    pub fn start(
        &self,
        run: RunId,
        command: &std::process::Command,
        keeps: &[HomeEntry],
    ) -> io::Result<AgentProcess> {
        let mut spec = Spec::of(command)?;
        let folder = self.folder(run);
        Self::private_dir(&self.root)?;
        Self::private_dir(&folder)?;
        let needs = match &self.launcher {
            Launcher::Container(containers) => {
                // The container sees the program's file, not the folders on
                // the path: it is started by the path it was found at.
                let program = spec.program()?;
                spec.program = program.display().to_string();
                Some(self.needs(containers, &spec, &folder, keeps))
            }
            Launcher::Systemd | Launcher::Direct => None,
        };
        let json = serde_json::to_vec(&spec).map_err(io::Error::other)?;
        Self::private_file(&folder.join(Self::SPEC))?;
        std::fs::write(folder.join(Self::SPEC), json)?;
        self.launch(run, &folder, needs.as_ref())?;
        let stream = Self::reach(&folder.join(Self::SOCKET))?;
        link::Link::open(stream, 0, self.journal(&folder)?)
    }

    /// What `spec`'s run needs to see in its container: its folder, its
    /// workdir and the repository's git folder, this `ren`, the files its
    /// command reads, and what its client keeps that exists.
    fn needs(
        &self,
        containers: &RunContainers,
        spec: &Spec,
        folder: &Path,
        keeps: &[HomeEntry],
    ) -> RunNeeds {
        let reads = std::iter::once(PathBuf::from(&spec.program))
            .chain(
                spec.args
                    .iter()
                    .map(Path::new)
                    .filter(|arg| arg.is_absolute() && arg.is_file())
                    .map(Path::to_path_buf),
            )
            .collect();
        RunNeeds {
            folder: folder.to_path_buf(),
            workdir: spec.cwd.clone(),
            git: Spec::git_folder(&spec.cwd),
            host: self.program.clone(),
            reads,
            keeps: keeps
                .iter()
                .map(|entry| entry.under(containers.home()))
                .filter(|path| path.exists())
                .collect(),
        }
    }

    /// The runs whose folders are here: each held by a host still running,
    /// or whose client exited while no daemon watched. A folder with no
    /// journal is left out.
    pub fn list(&self) -> io::Result<Vec<HeldRun>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut held = Vec::new();
        for entry in entries {
            let entry = entry?;
            let Some(run) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<uuid::Uuid>().ok())
                .map(RunId::restore)
            else {
                continue;
            };
            let journal = match std::fs::read(entry.path().join(Self::JOURNAL)) {
                Ok(journal) => journal,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            held.push(HeldRun { run, journal });
        }
        held.sort_by_key(|held| held.run);
        Ok(held)
    }

    /// Bytes `from..to` of `run`'s spool; fewer when it holds fewer.
    pub fn spool(&self, run: RunId, from: u64, to: u64) -> io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(self.folder(run).join(Self::SPOOL))?;
        file.seek(io::SeekFrom::Start(from))?;
        let mut bytes = Vec::new();
        file.take(to.saturating_sub(from)).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Attach to `run` again, its output from byte `from` of its spool: to
    /// its host when it still runs, else to what it left (its spool and
    /// its exit).
    pub fn rejoin(&self, run: RunId, from: u64) -> io::Result<AgentProcess> {
        let folder = self.folder(run);
        let journal = self.journal(&folder)?;
        match std::os::unix::net::UnixStream::connect(folder.join(Self::SOCKET)) {
            Ok(stream) => link::Link::open(stream, from, journal),
            Err(_) => {
                let exit: ProcessExit =
                    serde_json::from_slice(&std::fs::read(folder.join(Self::EXIT))?)
                        .map_err(io::Error::other)?;
                link::Link::left(&folder.join(Self::SPOOL), from, exit, journal)
            }
        }
    }

    /// `run` has ended and its end is recorded: its folder goes.
    pub fn release(&self, run: RunId) {
        let folder = self.folder(run);
        if let Err(error) = std::fs::remove_dir_all(&folder)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%run, %error, "could not remove a run's folder");
        }
    }

    fn journal(&self, folder: &Path) -> io::Result<Tenure> {
        let path = folder.join(Self::JOURNAL);
        Self::private_file(&path)?;
        let file = std::fs::OpenOptions::new().append(true).open(path)?;
        Ok(Tenure::Held {
            journal: Box::new(tokio::fs::File::from_std(file)),
        })
    }

    /// Start the run host for `folder`.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned place that starts run hosts"
    )]
    fn launch(&self, run: RunId, folder: &Path, needs: Option<&RunNeeds>) -> io::Result<()> {
        match &self.launcher {
            Launcher::Systemd => {
                let output = std::process::Command::new("systemd-run")
                    .args(["--user", "--quiet", "--collect"])
                    .arg(format!("--unit={}", Self::unit(run)))
                    // The client's own processes go with it when its host is
                    // stopped; SIGTERM goes to the host first.
                    .arg("--property=KillMode=mixed")
                    .arg("--")
                    .arg(&self.program)
                    .arg("run-host")
                    .arg(folder)
                    .stdin(std::process::Stdio::null())
                    .output()?;
                if output.status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!(
                        "systemd-run could not start the run host: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )))
                }
            }
            Launcher::Container(containers) => {
                let needs = needs.ok_or_else(|| {
                    io::Error::other("a run host in a container was started without its needs")
                })?;
                let container = containers.container(run, needs);
                let (program, args) = container.start();
                let mut podman = std::process::Command::new(program);
                podman.args(args);
                if container.through_socket() {
                    // In a container, the home's folders and the runtime
                    // folder are made for its mounts and are not the
                    // operator's, which the Podman client refuses: it keeps
                    // its settings and runtime files here instead.
                    let settings = self.root.join(Self::PODMAN_SETTINGS);
                    let runtime = settings.join("run");
                    Self::private_dir(&settings)?;
                    Self::private_dir(&runtime)?;
                    podman
                        .env("XDG_CONFIG_HOME", settings)
                        .env("XDG_RUNTIME_DIR", runtime);
                }
                let output = podman.stdin(std::process::Stdio::null()).output()?;
                if output.status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!(
                        "podman could not start the run host {}: {}",
                        container.name(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    )))
                }
            }
            Launcher::Direct => {
                use std::os::unix::process::CommandExt as _;
                std::process::Command::new(&self.program)
                    .arg("run-host")
                    .arg(folder)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .process_group(0)
                    .spawn()
                    .map(drop)
            }
        }
    }

    /// The host's socket, once it listens.
    fn reach(socket: &Path) -> io::Result<std::os::unix::net::UnixStream> {
        const TRIES: u32 = 250;
        let mut last = io::Error::from(io::ErrorKind::NotFound);
        for _ in 0..TRIES {
            match std::os::unix::net::UnixStream::connect(socket) {
                Ok(stream) => return Ok(stream),
                Err(error) => last = error,
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(io::Error::new(
            last.kind(),
            format!("the run host did not listen: {last}"),
        ))
    }

    fn private_dir(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::DirBuilderExt as _;
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            other => other,
        }
    }

    fn private_file(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::Spec;
    use std::{fs, path::Path};

    #[test]
    fn a_workdir_at_its_repositorys_root_needs_no_other_git_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        assert_eq!(Spec::git_folder(dir.path()), None);
    }

    #[test]
    fn a_subfolder_needs_its_repositorys_git_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let sub = dir.path().join("crates/app");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(Spec::git_folder(&sub), Some(dir.path().join(".git")));
    }

    #[test]
    fn a_worktree_needs_its_repositorys_common_git_folder() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("app");
        let own = repo.join(".git/worktrees/fix");
        fs::create_dir_all(&own).unwrap();
        fs::write(own.join("commondir"), "../..\n").unwrap();
        let worktree = dir.path().join("fix");
        fs::create_dir(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", own.display()),
        )
        .unwrap();
        assert_eq!(
            Spec::git_folder(&worktree),
            Some(fs::canonicalize(repo.join(".git")).unwrap())
        );
    }

    #[test]
    fn a_program_is_found_on_the_path_the_command_sets() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("client"), "").unwrap();
        let spec = Spec {
            program: "client".into(),
            args: Vec::new(),
            env: vec![("PATH".into(), bin.display().to_string())],
            cwd: dir.path().into(),
        };
        assert_eq!(
            spec.program().unwrap(),
            fs::canonicalize(bin.join("client")).unwrap()
        );
        let missing = Spec {
            program: "absent".into(),
            ..spec
        };
        assert!(missing.program().is_err());
        assert!(Path::new(&missing.program).is_relative());
    }
}
