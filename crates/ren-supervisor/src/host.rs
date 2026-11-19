//! The host capability a supervisor needs: resolving directories and starting
//! agent processes. Declared here, implemented by the daemon; tests use
//! in-memory processes.

use crate::{
    files::{FoundPath, WalkLimits},
    place::Look,
    run::ProcessExit,
    workdir::Workdir,
    workspace::Workspace,
};
use futures_util::future::BoxFuture;
use ren_depot::toolset::Generation;
use ren_harness::{launch::LaunchPlan, sign_in::LocalRedirect};
use ren_recorder::id::RunId;
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::oneshot,
};

/// Starts agent processes on the daemon's host.
pub trait AgentHost: Send + Sync + 'static {
    /// The real, absolute path of `path`, with every link resolved.
    fn resolve(&self, path: &Path) -> io::Result<PathBuf>;

    /// What `dir` holds: its folders (links not followed), and its git
    /// repository.
    fn look(&self, dir: &Path) -> io::Result<Look>;

    /// The files and folders under `dir`, relative to it, leaving out what
    /// git ignores there and hidden ones, walked no further than `limits`
    /// allow. Links are not followed.
    fn files(&self, dir: &Path, limits: WalkLimits) -> io::Result<Vec<FoundPath>>;

    /// Whether `program` is on the path a started process searches.
    fn finds(&self, program: &str) -> bool;

    /// The installed client a process with `tools` would start as
    /// `program`, as its file is now; `None` when it is not found.
    fn client_stamp(&self, program: &str, tools: &RunTools) -> Option<ClientStamp>;

    /// Create `dir` and the folders above it, where they are missing.
    fn make_dir(&self, dir: &Path) -> io::Result<()>;

    /// Create `dir`, a new empty folder a scratch run works in, and make
    /// it a git repository with the git on `tools`' search path, so what
    /// the run changes can be read as changes. A folder git could not
    /// make a repository is still made, and says why.
    fn make_scratch(&self, dir: &Path, tools: &RunTools) -> io::Result<Scratch>;

    /// Write `bytes` to `incoming`, a new file, then move it to `target`,
    /// making the folders above each where they are missing: `target`
    /// appears whole or not at all.
    fn place_file(&self, incoming: &Path, target: &Path, bytes: &[u8]) -> io::Result<()>;

    /// The bytes of the file at `path`.
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Remove `dir` and everything in it; one already gone is not an error.
    fn remove_tree(&self, dir: &Path) -> io::Result<()>;

    /// The toolset a process started now gets: the current generation of
    /// the tools Ren installed here.
    fn tools(&self) -> RunTools;

    /// Start `plan` in `workdir`, with piped stdin and stdout, and `tools`'
    /// folder first on its search path.
    fn spawn(
        &self,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess>;

    /// Start `run`'s client process, as [`Self::spawn`] does. A host that
    /// can keep the process going when the daemon stops starts it under a
    /// run host and says so in its [`Tenure`]; the default starts it bound
    /// to the daemon.
    fn start_run(
        &self,
        run: RunId,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess> {
        let _ = run;
        self.spawn(plan, workdir, tools)
    }

    /// The runs whose run hosts kept them going while no daemon ran, each
    /// with its journal. None by default: a host that holds no runs.
    fn held(&self) -> io::Result<Vec<HeldRun>> {
        Ok(Vec::new())
    }

    /// Bytes `from..to` of a held run's spool, everything its client wrote;
    /// fewer when the spool holds fewer.
    fn spool(&self, run: RunId, from: u64, to: u64) -> io::Result<Vec<u8>> {
        let _ = (run, from, to);
        Err(io::ErrorKind::Unsupported.into())
    }

    /// Take up a held run again: its client's output from byte `from` of
    /// its spool, its stdin, its exit, and its journal to go on with.
    fn rejoin(&self, run: RunId, from: u64) -> io::Result<AgentProcess> {
        let _ = (run, from);
        Err(io::ErrorKind::Unsupported.into())
    }

    /// Make `folder` a worktree of the repository at `repo` on a new
    /// branch `branch`, and `data` its data folder. None by default: a host
    /// that makes no workspaces.
    fn make_worktree(
        &self,
        repo: &Path,
        folder: &Path,
        data: &Path,
        branch: &str,
        tools: &RunTools,
    ) -> io::Result<()> {
        let _ = (repo, folder, data, branch, tools);
        Err(io::ErrorKind::Unsupported.into())
    }

    /// Remove `workspace`: stop its preview's units, then remove its
    /// worktree and its data folder, and its branch once merged. A worktree
    /// holding changes not committed is kept, and the error says so.
    fn remove_worktree(&self, workspace: &Workspace, tools: &RunTools) -> io::Result<()> {
        let _ = (workspace, tools);
        Err(io::ErrorKind::Unsupported.into())
    }

    /// `run` has ended and its end is recorded: what its host kept of it
    /// can go. Nothing to do by default.
    fn release(&self, run: RunId) {
        let _ = run;
    }

    /// Start `plan`, one of a client's own short commands (its status or
    /// login), as [`Self::spawn`] does, but with its stdout and stderr read
    /// together as the process's `stdout`, so what it says on either is read.
    fn spawn_with_stderr(
        &self,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess>;

    /// Start `plan`, a client's login that needs a terminal, on a
    /// pseudo-terminal of its own: the process's `stdin` types into it and
    /// its `stdout` is what it shows.
    fn spawn_in_terminal(
        &self,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess>;

    /// Do `work`, which reads or writes the filesystem or waits on a short
    /// process, where it holds up no thread the supervisor's task or the
    /// rest of the daemon runs on. The default does it in place: for hosts
    /// whose work never blocks, as tests' in-memory ones.
    fn blocking<T, W>(&self, work: W) -> BoxFuture<'static, io::Result<T>>
    where
        T: Send + 'static,
        W: FnOnce() -> io::Result<T> + Send + 'static,
    {
        Box::pin(std::future::ready(work()))
    }

    /// Open `address`, a loopback address on this host a client's login
    /// listens on, as the operator's browser would have: one `GET`, no
    /// redirects followed.
    fn open_local(&self, address: &LocalRedirect) -> BoxFuture<'static, io::Result<()>>;
}

/// How a scratch folder was made: a repository of its own, or a plain
/// folder whose changes are not tracked, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Scratch {
    Repository,
    Untracked(Untracked),
}

/// Why a scratch folder is not a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Untracked {
    /// No `git` on the run's search path.
    NoGit,
    /// `git` was found but could not be started.
    NotStarted,
    /// `git init` ran and failed.
    InitFailed(ProcessExit),
}

/// An installed client as its program's file is: where the program
/// resolves to, links followed, its size, and when it was last written. A
/// new version, installed or updated by the client itself, is a new stamp.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientStamp {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl ClientStamp {
    pub fn new(path: PathBuf, len: u64, modified: Option<std::time::SystemTime>) -> Self {
        Self {
            path,
            len,
            modified,
        }
    }
}

/// The tools a process starts with, as the host read them when it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTools {
    /// Ren has installed no tools here (generation 0): the process gets the
    /// host's search path alone.
    Empty,
    /// A generation, and its folder of programs.
    Toolset {
        generation: Generation,
        bin: PathBuf,
    },
    /// The tool store could not be read: the process gets the host's search
    /// path alone, and which generation it would have had is unknown.
    Unreadable,
}

impl RunTools {
    /// The generation to record; `None` when unknown.
    pub fn generation(&self) -> Option<Generation> {
        match self {
            Self::Empty => Some(Generation::NONE),
            Self::Toolset { generation, .. } => Some(*generation),
            Self::Unreadable => None,
        }
    }

    /// The folder to put first on the search path, if any.
    pub fn bin(&self) -> Option<&Path> {
        match self {
            Self::Toolset { bin, .. } => Some(bin),
            Self::Empty | Self::Unreadable => None,
        }
    }
}

/// A started agent process: its pipes, its exit, the means to end it, and
/// whether it outlives the daemon.
pub struct AgentProcess {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    /// Resolves once the process has exited.
    pub exit: BoxFuture<'static, ProcessExit>,
    pub kill: KillSwitch,
    pub tenure: Tenure,
}

/// Whether a process outlives the daemon that started it.
pub enum Tenure {
    /// It ends with the daemon, which drains it as it stops.
    Bound,
    /// A run host keeps it going when the daemon stops, and keeps every
    /// byte it writes. The daemon appends what it gives the process to
    /// `journal`, so the next daemon can take it up exactly where this one
    /// left it.
    Held {
        journal: Box<dyn AsyncWrite + Send + Unpin>,
    },
}

/// A run a run host kept going while no daemon ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldRun {
    pub run: RunId,
    /// Its journal, as the daemon before left it.
    pub journal: Vec<u8>,
}

/// Ends a process when fired. The process's `exit` resolves afterwards.
#[derive(Debug)]
#[must_use]
pub struct KillSwitch(oneshot::Sender<()>);

/// The other side of a [`KillSwitch`], held by the host's exit future.
#[derive(Debug)]
pub struct KillSignal(oneshot::Receiver<()>);

impl KillSwitch {
    pub fn pair() -> (Self, KillSignal) {
        let (sender, receiver) = oneshot::channel();
        (Self(sender), KillSignal(receiver))
    }

    pub fn fire(self) {
        // A process that already exited has dropped its signal.
        let _ = self.0.send(());
    }
}

impl KillSignal {
    /// Resolves when the switch fires. If the switch is dropped unfired, the
    /// process is left to exit on its own and this never resolves.
    pub async fn fired(self) {
        if self.0.await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
