//! The host capability a supervisor needs: resolving directories and starting
//! agent processes. Declared here, implemented by the daemon; tests use
//! in-memory processes.

use crate::{
    files::{FoundPath, WalkLimits},
    project::Look,
    run::ProcessExit,
    workdir::Workdir,
};
use futures_util::future::BoxFuture;
use ren_harness::launch::LaunchPlan;
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

    /// What `dir` holds: its folders (links not followed), and whether it
    /// is a git repository, with its `HEAD`'s text when readable.
    fn look(&self, dir: &Path) -> io::Result<Look>;

    /// The files and folders under `dir`, relative to it, leaving out what
    /// git ignores there and hidden ones, walked no further than `limits`
    /// allow. Links are not followed.
    fn files(&self, dir: &Path, limits: WalkLimits) -> io::Result<Vec<FoundPath>>;

    /// Whether `program` is on the path a started process searches.
    fn finds(&self, program: &str) -> bool;

    /// Create `dir` and the folders above it, where they are missing.
    fn make_dir(&self, dir: &Path) -> io::Result<()>;

    /// Create `dir`, a new empty folder a scratch run works in, as a git
    /// repository, so what the run changes can be read as changes.
    fn make_scratch(&self, dir: &Path) -> io::Result<()>;

    /// Start `plan` in `workdir`, with piped stdin and stdout.
    fn spawn(&self, plan: &LaunchPlan, workdir: &Workdir) -> io::Result<AgentProcess>;
}

/// A started agent process: its pipes, its exit, and the means to end it.
pub struct AgentProcess {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    /// Resolves once the process has exited.
    pub exit: BoxFuture<'static, ProcessExit>,
    pub kill: KillSwitch,
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
