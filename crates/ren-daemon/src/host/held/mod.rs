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
//! A relayed run (phase 12) reaches out only through the doorways in
//! `<data>/runs/.reach` ([`Doors`]): Ren's gateway, and the relay, which
//! admits it by the ticket its spec carries ([`Relaying`]). A run working
//! in a checkout of a repository the GitHub App is granted pushes to it
//! with a token git asks the grants door for ([`Grants`]).
//!
//! The daemon's side is [`HeldRuns`]; the host's is [`RunHost`].

mod doorway;
pub mod frame;
mod grant;
mod link;
mod listing;
#[cfg(test)]
mod purity;
mod sandbox;
mod serve;
mod watched;

pub use doorway::{Doors, Doorway};
pub use grant::{CredentialAsk, Grants};
pub use listing::{Listing, RelayedRun};
use ren_container::{Engine, EngineError, Launch, Reach};
pub use sandbox::{RunReach, SandboxError, Sandboxes};
pub use serve::RunHost;
pub use watched::{ProjectReachError, WatchedProjects};

use ren_harness::profile::ClientNeeds;
use sandbox::{RenPlaces, Sandbox, Sight};

use ren_github::RepositorySlug;
use ren_recorder::id::{IdSource, RunId};
use ren_recorder::passage::Purpose;
use ren_relay::{Destination, PassHandle, Relay, RelayPolicy, Ticket};
use ren_supervisor::{AgentProcess, HeldRun, ProcessExit, Tenure};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{self, Read as _, Seek as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
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
    /// In a rootless Podman container of its own, `ren-run-<run>`, its
    /// run's [`sandbox::Sandbox`]: it sees only what the run works on and
    /// what its client keeps, and reaches what its reach says.
    Container(Sandboxes),
}

/// How one run's host is started: as its [`Launcher`] says, a container's
/// with the run's sandbox made.
enum Start {
    Systemd,
    Direct,
    Sandbox(Box<Sandbox>),
}

/// The runs this daemon holds through run hosts: their folders, and how a
/// host is started.
#[derive(Debug, Clone)]
pub struct HeldRuns {
    root: PathBuf,
    launcher: Launcher,
    /// The `ren` a host runs as.
    program: PathBuf,
    /// How relayed runs reach out, when runs are relayed.
    relaying: Option<Relaying>,
}

/// How relayed runs reach out: the relay that admits their tunnels, the
/// doors their containers mount, the gateway's address (theirs too, on
/// their own loopback), and the passes the relay holds for them.
#[derive(Clone)]
pub struct Relaying {
    relay: Arc<Relay<RelayedRun>>,
    doors: Doors,
    gateway: SocketAddr,
    ids: Arc<dyn IdSource>,
    passes: Arc<Mutex<HashMap<RunId, PassHandle<RelayedRun>>>>,
    /// What each watched project's manifest lets its runs reach.
    projects: WatchedProjects,
    /// The GitHub App's grants, when one is set up.
    grants: Option<Arc<Grants>>,
    /// Each granted run's ticket and the repository it may push to.
    granted: Arc<Mutex<HashMap<RunId, (Ticket, RepositorySlug)>>>,
    /// The `ren` git runs as its credential helper inside a run.
    helper: PathBuf,
    /// Where each relayed run's [`Admission`] is kept: in the runs' root,
    /// which no run sees, never in a run's own folder, which it writes.
    admitted: PathBuf,
}

/// What a relayed run's spec carries for its host: the doorways it opens.
/// Where the run may go is the daemon's ([`Admission`]), never the spec's,
/// which the run can write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayedSpec {
    pub doorways: Vec<Doorway>,
}

/// A relayed run's pass as the daemon keeps it, so a daemon that takes the
/// run up again issues the same: its ticket, where it may go and why, and
/// the repository it may push to.
#[derive(Clone, Serialize, Deserialize)]
struct Admission {
    #[serde(
        serialize_with = "Admission::write_ticket",
        deserialize_with = "Admission::read_ticket"
    )]
    ticket: Ticket,
    reach: Listing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grant: Option<RepositorySlug>,
}

impl Admission {
    /// The file `run`'s admission is kept in, under `admitted`.
    fn file(admitted: &Path, run: RunId) -> PathBuf {
        admitted.join(format!("{run}.json"))
    }

    fn write_ticket<S: serde::Serializer>(ticket: &Ticket, to: S) -> Result<S::Ok, S::Error> {
        to.serialize_str(ticket.reveal())
    }

    fn read_ticket<'de, D: serde::Deserializer<'de>>(from: D) -> Result<Ticket, D::Error> {
        let secret = String::deserialize(from)?;
        Ticket::new(secret).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Debug for Admission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admission")
            .field("ticket", &self.ticket)
            .field("reach", &self.reach)
            .field("grant", &self.grant)
            .finish()
    }
}

impl std::fmt::Debug for Relaying {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relaying")
            .field("doors", &self.doors)
            .field("gateway", &self.gateway)
            .finish_non_exhaustive()
    }
}

impl Relaying {
    /// Where a run reaches the relay on its own loopback: the port proxies
    /// usually take, or the next when the gateway has it.
    const RELAY_PORT: u16 = 3128;
    /// What a relayed run connects to directly, never through the relay.
    const NO_PROXY: &str = "127.0.0.1,localhost,::1";

    /// Where a granted run reaches GitHub, through the relay.
    const GITHUB: &str = "github.com:443";
    /// The folder of relayed runs' admissions, in the runs' root.
    pub const ADMITTED: &str = ".admitted";

    pub fn new(
        relay: Arc<Relay<RelayedRun>>,
        doors: Doors,
        gateway: SocketAddr,
        ids: Arc<dyn IdSource>,
        helper: PathBuf,
        projects: WatchedProjects,
        admitted: PathBuf,
    ) -> Self {
        Self {
            relay,
            doors,
            gateway,
            ids,
            passes: Arc::new(Mutex::new(HashMap::new())),
            projects,
            grants: None,
            granted: Arc::new(Mutex::new(HashMap::new())),
            helper,
            admitted,
        }
    }

    /// Runs working in a repository `grants` covers are granted it.
    pub fn granting(mut self, grants: Arc<Grants>) -> Self {
        self.grants = Some(grants);
        self
    }

    /// Answers each ask on the grants socket `listener` until the returned
    /// future is dropped: a token for the asking run's repository, or
    /// nothing.
    pub fn serve_grants(
        &self,
        listener: tokio::net::UnixListener,
    ) -> impl std::future::Future<Output = ()> + use<> {
        use futures_util::{StreamExt as _, stream::FuturesUnordered};
        let relaying = self.clone();
        async move {
            let mut answering = FuturesUnordered::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        if let Ok((stream, _)) = accepted {
                            answering.push(relaying.clone().answer(stream));
                        }
                    }
                    Some(()) = answering.next(), if !answering.is_empty() => {}
                }
            }
        }
    }

    /// One helper's ask, answered.
    async fn answer(self, mut stream: tokio::net::UnixStream) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut read = Vec::new();
        let mut chunk = [0_u8; 1024];
        while read.len() < CredentialAsk::LIMIT && !read.ends_with(b"\n\n") {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => read.extend_from_slice(&chunk[..n]),
            }
        }
        let ask = CredentialAsk::read(&String::from_utf8_lossy(&read));
        let granted = ask.ticket().and_then(|ticket| {
            self.granted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .find(|(held, _)| *held == ticket)
                .map(|(_, repository)| repository.clone())
        });
        let answer = match (&self.grants, granted) {
            (Some(grants), Some(repository)) if ask.is_for(&repository) => {
                match grants.token(&repository).await {
                    Ok(token) => CredentialAsk::answer(&token),
                    Err(error) => {
                        tracing::warn!(%repository, %error, "a run's grant could not be minted");
                        "\n".to_owned()
                    }
                }
            }
            (Some(_) | None, _) => "\n".to_owned(),
        };
        let _ = stream.write_all(answer.as_bytes()).await;
        let _ = stream.shutdown().await;
    }

    fn relay_inside(&self) -> SocketAddr {
        let port = if self.gateway.port() == Self::RELAY_PORT {
            Self::RELAY_PORT + 1
        } else {
            Self::RELAY_PORT
        };
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    /// Admits `run` to where `policy`, its client's `needs`, its project's
    /// manifest, and its grant list: a ticket minted for it and a pass the
    /// relay holds until it is released. What its spec carries, and the
    /// variables its client gets.
    fn admit(
        &self,
        run: RunId,
        policy: &RelayPolicy,
        needs: ClientNeeds,
        workdir: &Path,
    ) -> io::Result<(RelayedSpec, Vec<(String, String)>)> {
        // Its project by its folder, as Ren registered it; never by what
        // its checkout says, which the run can change.
        let repository = self.projects.repository_of(workdir);
        let grant = repository.clone().filter(|repository| {
            self.grants
                .as_ref()
                .is_some_and(|grants| grants.covers(repository))
        });
        let client = needs
            .reaches
            .iter()
            .map(|text| text.parse::<Destination>().map_err(io::Error::other))
            .collect::<io::Result<Vec<_>>>()?;
        let github = grant
            .as_ref()
            .map(|_| {
                Self::GITHUB
                    .parse::<Destination>()
                    .map_err(io::Error::other)
            })
            .transpose()?;
        let reach = Listing::default()
            .list(
                Purpose::Configuration,
                policy.destinations().iter().cloned(),
            )
            .list(Purpose::Client, client)
            .list(
                Purpose::Project,
                repository
                    .iter()
                    .flat_map(|repository| self.projects.reach_of(repository)),
            )
            .list(Purpose::Grant, github);
        let ticket = Ticket::new(format!(
            "{}{}",
            self.ids.next_uuid().simple(),
            self.ids.next_uuid().simple()
        ))
        .map_err(io::Error::other)?;
        let admission = Admission {
            ticket,
            reach,
            grant,
        };
        self.keep(run, &admission)?;
        self.pass(run, &admission)?;
        let spec = RelayedSpec {
            doorways: vec![
                Doorway {
                    inside: self.gateway,
                    socket: self.doors.socket(Doors::GATEWAY),
                },
                Doorway {
                    inside: self.relay_inside(),
                    socket: self.doors.socket(Doors::RELAY),
                },
            ],
        };
        let proxy = admission.ticket.proxy_url("run", self.relay_inside());
        let git = admission
            .grant
            .as_ref()
            .map(|_| self.git_env())
            .unwrap_or_default();
        let env = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
            .into_iter()
            .map(|name| (name.to_owned(), proxy.clone()))
            .chain(
                ["NO_PROXY", "no_proxy"]
                    .into_iter()
                    .map(|name| (name.to_owned(), Self::NO_PROXY.to_owned())),
            )
            .chain(git)
            .collect();
        Ok((spec, env))
    }

    /// What a granted run's git is given: `ren credential` as its helper
    /// for github.com, asking with the repository's path, the grants
    /// socket it asks, and GitHub's SSH addresses read as HTTPS, so a
    /// checkout cloned over SSH pushes through the relay.
    fn git_env(&self) -> Vec<(String, String)> {
        let config = [
            (
                "credential.https://github.com.helper",
                format!("{} credential", self.helper.display()),
            ),
            (
                "credential.https://github.com.useHttpPath",
                "true".to_owned(),
            ),
            (
                "url.https://github.com/.insteadOf",
                "git@github.com:".to_owned(),
            ),
            (
                "url.https://github.com/.insteadOf",
                "ssh://git@github.com/".to_owned(),
            ),
        ];
        let mut env = vec![("GIT_CONFIG_COUNT".to_owned(), config.len().to_string())];
        for (index, (key, value)) in config.into_iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{index}"), key.to_owned()));
            env.push((format!("GIT_CONFIG_VALUE_{index}"), value));
        }
        env.push((
            Grants::SOCKET_VARIABLE.to_owned(),
            self.doors.socket(Doors::GRANTS).display().to_string(),
        ));
        env
    }

    fn grant(&self, run: RunId, ticket: Ticket, repository: RepositorySlug) {
        self.granted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run, (ticket, repository));
    }

    /// Admits `run` again, as the daemon kept it, for a daemon that took
    /// it up: the pass went with the daemon that issued it. A run with no
    /// kept admission is not relayed.
    fn readmit(&self, run: RunId) -> io::Result<()> {
        if self
            .passes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&run)
        {
            return Ok(());
        }
        match self.admission(run)? {
            Some(admission) => self.pass(run, &admission),
            None => Ok(()),
        }
    }

    /// `run`'s kept admission, if it has one.
    fn admission(&self, run: RunId) -> io::Result<Option<Admission>> {
        match std::fs::read(Admission::file(&self.admitted, run)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Keeps `run`'s admission where no run sees it, readable by the
    /// operator alone.
    fn keep(&self, run: RunId, admission: &Admission) -> io::Result<()> {
        HeldRuns::private_dir(&self.admitted)?;
        let file = Admission::file(&self.admitted, run);
        HeldRuns::private_file(&file)?;
        std::fs::write(
            &file,
            serde_json::to_vec(admission).map_err(io::Error::other)?,
        )
    }

    /// The relay holds `admission`'s pass for `run`, and its grant.
    fn pass(&self, run: RunId, admission: &Admission) -> io::Result<()> {
        self.hold(run, admission.ticket.clone(), admission.reach.clone())?;
        if let Some(repository) = &admission.grant {
            self.grant(run, admission.ticket.clone(), repository.clone());
        }
        Ok(())
    }

    fn hold(&self, run: RunId, ticket: Ticket, reach: Listing) -> io::Result<()> {
        let policy = reach.policy();
        let pass = self
            .relay
            .issue(ticket, RelayedRun::new(run, reach), policy)
            .map_err(io::Error::other)?;
        self.passes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run, pass);
        Ok(())
    }

    /// `run` reaches nothing more.
    fn release(&self, run: RunId) {
        let kept = Admission::file(&self.admitted, run);
        if let Err(error) = std::fs::remove_file(&kept)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%run, %error, "could not remove a relayed run's admission");
        }
        self.passes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&run);
        self.granted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&run);
    }
}

/// The command a run host starts: what the daemon would have run itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub program: String,
    pub args: Vec<String>,
    /// Variables set for it, over the host's own environment.
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    /// What a relayed run reaches out through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relayed: Option<RelayedSpec>,
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
            relayed: None,
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
            relaying: None,
        }
    }

    /// These runs, relayed ones reaching out through `relaying`.
    pub fn relaying(mut self, relaying: Relaying) -> Self {
        self.relaying = Some(relaying);
        self
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
    /// `needs` is what the client needs beyond its folder, which a run host
    /// in a container is given.
    pub fn start(
        &self,
        run: RunId,
        command: &std::process::Command,
        needs: ClientNeeds,
    ) -> io::Result<AgentProcess> {
        let started = self.start_held(run, command, needs);
        if started.is_err()
            && let Some(relaying) = &self.relaying
        {
            relaying.release(run);
        }
        started
    }

    fn start_held(
        &self,
        run: RunId,
        command: &std::process::Command,
        needs: ClientNeeds,
    ) -> io::Result<AgentProcess> {
        let mut spec = Spec::of(command)?;
        let folder = self.folder(run);
        Self::private_dir(&self.root)?;
        Self::private_dir(&folder)?;
        let start = match &self.launcher {
            Launcher::Systemd => Start::Systemd,
            Launcher::Direct => Start::Direct,
            Launcher::Container(sandboxes) => {
                // The container sees the program's file, not the folders on
                // the path: it is started by the path it was found at.
                let program = spec.program()?;
                spec.program = program.display().to_string();
                match sandboxes.reach() {
                    RunReach::Host { .. } => {}
                    RunReach::Relayed { policy, .. } => {
                        let relaying = self.relaying.as_ref().ok_or_else(|| {
                            io::Error::other("a relayed run was started with no relay")
                        })?;
                        let (relayed, env) = relaying.admit(run, policy, needs, &spec.cwd)?;
                        spec.env.extend(env);
                        spec.relayed = Some(relayed);
                    }
                }
                let sight = self.sight(sandboxes, &spec, &folder, needs);
                let ren = RenPlaces {
                    data: self.root.parent().unwrap_or(&self.root).to_path_buf(),
                    doors: Doors::new(&self.root).folder().to_path_buf(),
                };
                Start::Sandbox(Box::new(
                    sandboxes
                        .sandbox(run, &sight, &ren)
                        .map_err(io::Error::other)?,
                ))
            }
        };
        let json = serde_json::to_vec(&spec).map_err(io::Error::other)?;
        Self::private_file(&folder.join(Self::SPEC))?;
        std::fs::write(folder.join(Self::SPEC), json)?;
        self.launch(run, &folder, start)?;
        let stream = Self::reach(&folder.join(Self::SOCKET))?;
        link::Link::open(stream, 0, self.journal(&folder)?)
    }

    /// What `spec`'s run needs to see in its sandbox: its folder, its
    /// workdir and the repository's git folder, this `ren`, the files its
    /// command reads, and what its client keeps that exists.
    fn sight(
        &self,
        sandboxes: &Sandboxes,
        spec: &Spec,
        folder: &Path,
        needs: ClientNeeds,
    ) -> Sight {
        let reads = std::iter::once(PathBuf::from(&spec.program))
            .chain(
                spec.args
                    .iter()
                    .map(Path::new)
                    .filter(|arg| arg.is_absolute() && arg.is_file())
                    .map(Path::to_path_buf),
            )
            .collect();
        Sight {
            folder: folder.to_path_buf(),
            workdir: spec.cwd.clone(),
            git: Spec::git_folder(&spec.cwd),
            host: self.program.clone(),
            reads,
            keeps: needs
                .keeps
                .iter()
                .map(|entry| entry.under(sandboxes.home()))
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
            Ok(stream) => {
                // Its pass went with the daemon that issued it.
                if let Some(relaying) = &self.relaying {
                    relaying.readmit(run)?;
                }
                link::Link::open(stream, from, journal)
            }
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
        if let Some(relaying) = &self.relaying {
            relaying.release(run);
        }
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
    fn launch(&self, run: RunId, folder: &Path, start: Start) -> io::Result<()> {
        match start {
            Start::Systemd => {
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
            Start::Sandbox(sandbox) => {
                let launch = match sandbox.engine() {
                    Reach::Scope => Launch::Scope {
                        unit: sandbox.name().to_owned(),
                    },
                    Reach::Socket(socket) => {
                        // In a container, the home's folders and the runtime
                        // folder are made for its mounts and are not the
                        // operator's, which the Podman client refuses: it keeps
                        // its settings and runtime files here instead.
                        let settings = self.root.join(Self::PODMAN_SETTINGS);
                        Self::private_dir(&settings)?;
                        Self::private_dir(&settings.join("run"))?;
                        Launch::Socket {
                            socket: socket.clone(),
                            settings: Some(settings),
                        }
                    }
                };
                Engine::Podman
                    .start(sandbox.spec(), &launch)
                    .map_err(|error| match error {
                        EngineError::Spawn { source, .. } => source,
                        EngineError::Failed { stderr, .. } => io::Error::other(format!(
                            "podman could not start the run host {}: {stderr}",
                            sandbox.name(),
                        )),
                        error @ EngineError::Runtime { .. } => io::Error::other(error.to_string()),
                    })
            }
            Start::Direct => {
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

    /// A relayed run working in a granted repository: its git is given
    /// `ren credential` and the grants socket, and its ask there is
    /// answered with a token for that repository alone. Any other ask,
    /// or a run not granted it, gets nothing.
    #[tokio::test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test spawns the stand-in and the door the daemon would"
    )]
    async fn a_granted_runs_git_gets_a_token_for_its_repository_only() {
        use super::{Admission, WatchedProjects};
        use super::{Doors, Grants, Relaying};
        use aws_lc_rs::{encoding::AsDer as _, rsa::KeySize, signature};
        use base64::Engine as _;
        use ren_harness::profile::ClientNeeds;
        use ren_recorder::id::{RunId, SequentialIds};
        use ren_recorder::passage::Purpose;
        use ren_recorder::time::{ManualClock, UnixMillis};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let router = axum::Router::new()
            .route(
                "/repos/acme/web/installation",
                axum::routing::get(|| async { axum::Json(serde_json::json!({ "id": 7 })) }),
            )
            .route(
                "/app/installations/7/access_tokens",
                axum::routing::post(|| async {
                    (
                        axum::http::StatusCode::CREATED,
                        axum::Json(serde_json::json!({
                            "token": "ghs_FIXTURE",
                            "expires_at": "2027-01-15T10:00:00Z",
                        })),
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let pair = signature::RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            base64::engine::general_purpose::STANDARD.encode(pair.as_der().unwrap().as_ref())
        );
        let installations = ren_github::Installations::new(
            ren_github::Endpoint::parse(&api).unwrap(),
            ren_github::App::new(
                ren_github::AppId::new(1),
                ren_github::AppKey::from_pem(&pem).unwrap(),
            ),
        )
        .unwrap();
        let grants = Grants::new(
            Arc::new(installations),
            ["acme/web".parse().unwrap()],
            Arc::new(ManualClock::at(UnixMillis::from_millis(1_800_000_000_000))),
        );

        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let doors = Doors::new(dir.path());
        // acme/web's default branch lists the registry its runs need.
        let projects = WatchedProjects::new(None);
        projects.learn_folders([
            (dir.path().join("web"), "acme/web".parse().unwrap()),
            (dir.path().join("other"), "acme/other".parse().unwrap()),
        ]);
        projects
            .learn_manifest(
                &"acme/web".parse().unwrap(),
                &ren_github::Snapshot {
                    default_branch: None,
                    pull_requests: Vec::new(),
                    listing: ren_github::Listing::Complete,
                    manifest: Some("reach = [\"registry.npmjs.org:443\"]\n".to_owned()),
                },
            )
            .unwrap();
        let relaying = Relaying::new(
            Arc::new(ren_relay::Relay::new()),
            doors.clone(),
            "127.0.0.1:8787".parse().unwrap(),
            Arc::new(SequentialIds::default()),
            "/opt/ren/bin/ren".into(),
            projects,
            dir.path().join(Relaying::ADMITTED),
        )
        .granting(Arc::new(grants));
        tokio::spawn(relaying.serve_grants(doors.listen(Doors::GRANTS).unwrap()));

        let policy = ren_relay::RelayPolicy::default();
        let (_, env) = relaying
            .admit(
                RunId::restore(uuid::Uuid::from_u128(1)),
                &policy,
                ClientNeeds::default(),
                &dir.path().join("web"),
            )
            .unwrap();
        let variable = |name: &str| {
            env.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            variable("GIT_CONFIG_VALUE_0").as_deref(),
            Some("/opt/ren/bin/ren credential")
        );
        let github = "github.com:443".parse().unwrap();
        let registry = "registry.npmjs.org:443".parse().unwrap();
        // Kept where the run cannot write: in the runs' root, not its folder.
        let granted = relaying
            .admission(RunId::restore(uuid::Uuid::from_u128(1)))
            .unwrap()
            .unwrap();
        assert!(
            dir.path()
                .join(Relaying::ADMITTED)
                .join(format!("{}.json", RunId::restore(uuid::Uuid::from_u128(1))))
                .is_file()
        );
        assert_eq!(granted.reach.purpose(&github), Some(Purpose::Grant));
        assert_eq!(granted.reach.purpose(&registry), Some(Purpose::Project));
        let (_, other_env) = relaying
            .admit(
                RunId::restore(uuid::Uuid::from_u128(2)),
                &policy,
                ClientNeeds::default(),
                &dir.path().join("other"),
            )
            .unwrap();
        assert!(
            !other_env
                .iter()
                .any(|(key, _)| key.starts_with("GIT_CONFIG"))
        );
        let other = relaying
            .admission(RunId::restore(uuid::Uuid::from_u128(2)))
            .unwrap()
            .unwrap();
        assert_eq!(other.reach.purpose(&github), None);
        assert_eq!(other.reach.purpose(&registry), None);
        let proxy = |admission: &Admission| {
            admission
                .ticket
                .proxy_url("run", "127.0.0.1:3128".parse().unwrap())
        };

        let ask = |proxy: String, path: &'static str| {
            let socket = doors.socket(Doors::GRANTS);
            async move {
                let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
                stream
                    .write_all(
                        format!("proxy={proxy}\nprotocol=https\nhost=github.com\npath={path}\n\n")
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
                let mut answer = String::new();
                stream.read_to_string(&mut answer).await.unwrap();
                answer
            }
        };
        assert_eq!(
            ask(proxy(&granted), "acme/web.git").await,
            "username=x-access-token\npassword=ghs_FIXTURE\n\n"
        );
        assert_eq!(ask(proxy(&granted), "acme/other.git").await, "\n");
        assert_eq!(ask(proxy(&other), "acme/other.git").await, "\n");
        // A daemon that took the run up issues its pass as it was kept.
        let again = Relaying::new(
            Arc::new(ren_relay::Relay::new()),
            doors.clone(),
            "127.0.0.1:8787".parse().unwrap(),
            Arc::new(SequentialIds::default()),
            "/opt/ren/bin/ren".into(),
            WatchedProjects::new(None),
            dir.path().join(Relaying::ADMITTED),
        );
        again
            .readmit(RunId::restore(uuid::Uuid::from_u128(1)))
            .unwrap();
        assert!(
            again
                .passes
                .lock()
                .unwrap()
                .contains_key(&RunId::restore(uuid::Uuid::from_u128(1)))
        );
        relaying.release(RunId::restore(uuid::Uuid::from_u128(1)));
        assert_eq!(ask(proxy(&granted), "acme/web.git").await, "\n");
        assert!(
            relaying
                .admission(RunId::restore(uuid::Uuid::from_u128(1)))
                .unwrap()
                .is_none()
        );
    }

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
            relayed: None,
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
