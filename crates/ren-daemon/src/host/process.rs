//! Agent processes on this host: the production [`AgentHost`], which
//! starts, signs in, and kills client processes, and does the task's
//! filesystem and git work off its thread.

use super::HeldRuns;
use crate::tools::ToolStore;
use ren_depot::toolset::Generation;
use ren_git::{Git, GitError, Head, Repository};
use ren_harness::{launch::LaunchPlan, pi::PiCache, sign_in::LocalRedirect};
use ren_recorder::id::RunId;
use ren_supervisor::{
    AgentHost, AgentProcess, ClientStamp, FoundPath, HeldRun, KillSwitch, Look, PathKind,
    ProcessExit, ProjectGit, RunTools, Scratch, Tenure, Untracked, WalkLimits, Workdir,
};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Starts agent processes on this host: the production [`AgentHost`]. Each
/// process works in its run's directory with piped stdin and stdout; stderr
/// is discarded, and a process whose exit future is dropped is killed.
/// Pi loads Ren's extension, published to `pi` before each start.
#[derive(Debug, Clone)]
pub struct ProcessHost {
    pi: Option<PiCache>,
    /// The host's search path, which a toolset's folder goes before.
    search_path: Vec<PathBuf>,
    tools: ToolStore<()>,
    /// Where runs are held through run hosts; `None` starts every run
    /// bound to the daemon.
    held: Option<HeldRuns>,
}

impl ProcessHost {
    pub fn new(pi: Option<PiCache>, search_path: Vec<PathBuf>, tools: ToolStore<()>) -> Self {
        Self {
            pi,
            search_path,
            tools,
            held: None,
        }
    }

    /// This host, holding runs through run hosts as `held` says.
    pub fn holding(mut self, held: HeldRuns) -> Self {
        self.held = Some(held);
        self
    }

    /// `plan`'s command, its search path set when a toolset goes first.
    fn command(&self, plan: &LaunchPlan, tools: &RunTools) -> io::Result<std::process::Command> {
        let extension = if plan.loads_pi_extension() {
            let cache = self.pi.as_ref().ok_or_else(|| {
                io::Error::other("set XDG_CACHE_HOME or HOME for the Pi extension cache")
            })?;
            Some(cache.materialize().map_err(io::Error::other)?)
        } else {
            None
        };
        let mut command = plan.command(extension.as_deref(), &[]);
        if let Some(path) = self.search_path_with(tools)? {
            command.env("PATH", path);
        }
        Ok(command)
    }

    /// The search path a process with `tools` gets: their folder, then the
    /// host's. `None` leaves the inherited one.
    fn search_path_with(&self, tools: &RunTools) -> io::Result<Option<std::ffi::OsString>> {
        tools
            .bin()
            .map(|_| std::env::join_paths(self.search_dirs(tools)).map_err(io::Error::other))
            .transpose()
    }

    /// The folders a process with `tools` searches for programs: their
    /// folder, then the host's.
    fn search_dirs(&self, tools: &RunTools) -> Vec<PathBuf> {
        tools
            .bin()
            .map(Path::to_path_buf)
            .into_iter()
            .chain(self.search_path.iter().cloned())
            .collect()
    }

    fn start(
        mut command: std::process::Command,
        workdir: &Path,
        stderr: Stderr,
    ) -> io::Result<AgentProcess> {
        command
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(match stderr {
                Stderr::Discarded => Stdio::null(),
                Stderr::WithStdout => Stdio::piped(),
            });
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        // A program installed a moment ago can still be open in a process
        // that is being started ("text file busy"): try again briefly.
        let mut tries = 0;
        let mut child = loop {
            #[expect(
                clippy::disallowed_methods,
                reason = "the one sanctioned place that starts agent processes"
            )]
            match command.spawn() {
                Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => break other?,
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the agent's pipes were not opened"));
        };
        let stdout: Box<dyn tokio::io::AsyncRead + Send + Unpin> = match stderr {
            Stderr::Discarded => Box::new(stdout),
            Stderr::WithStdout => match child.stderr.take() {
                Some(stderr) => Box::new(Merged::new(stdout, stderr)),
                None => return Err(io::Error::other("the agent's stderr was not opened")),
            },
        };
        let (exit, kill) = Self::exiting(child);
        Ok(AgentProcess {
            stdin: Box::new(stdin),
            stdout,
            exit,
            kill,
            tenure: Tenure::Bound,
        })
    }

    /// `child`'s exit, and the switch that kills it first.
    fn exiting(
        mut child: tokio::process::Child,
    ) -> (
        futures_util::future::BoxFuture<'static, ProcessExit>,
        KillSwitch,
    ) {
        let (kill, signal) = KillSwitch::pair();
        let exit = async move {
            let status = tokio::select! {
                status = child.wait() => status,
                () = signal.fired() => match child.kill().await {
                    Ok(()) => child.wait().await,
                    Err(error) => Err(error),
                },
            };
            match status {
                Ok(status) => Self::exit(status),
                Err(_) => ProcessExit::Unknown,
            }
        };
        (Box::pin(exit), kill)
    }

    /// Its held runs, when it holds runs.
    fn holder(&self) -> io::Result<&HeldRuns> {
        self.held
            .as_ref()
            .ok_or_else(|| io::ErrorKind::Unsupported.into())
    }

    /// The size of a login's terminal: wide, so a long link stays on one line.
    #[cfg(unix)]
    const TERMINAL: (u16, u16) = (50, 400);

    /// `command` started in `workdir` on a pseudo-terminal of its own, as
    /// its session leader with that terminal as its controlling terminal,
    /// echo off.
    #[cfg(unix)]
    fn start_in_terminal(
        command: &std::process::Command,
        workdir: &Path,
    ) -> io::Result<AgentProcess> {
        let (pty, pts) = pty_process::open().map_err(io::Error::other)?;
        let (rows, columns) = Self::TERMINAL;
        pty.resize(pty_process::Size::new(rows, columns))
            .map_err(io::Error::other)?;
        let mut terminal = pty_process::Command::new(command.get_program())
            .args(command.get_args())
            .current_dir(workdir)
            .kill_on_drop(true)
            .env("TERM", "xterm-256color");
        for (name, value) in command.get_envs() {
            terminal = match value {
                Some(value) => terminal.env(name, value),
                None => terminal.env_remove(name),
            };
        }
        // Echo off from the start: what Ren types (an API key) is never shown
        // back by the terminal; a program that wants it shown shows it.
        let mut modes = rustix::termios::tcgetattr(&pts)?;
        modes.local_modes.remove(rustix::termios::LocalModes::ECHO);
        rustix::termios::tcsetattr(&pts, rustix::termios::OptionalActions::Now, &modes)?;
        let child = terminal.spawn(pts).map_err(io::Error::other)?;
        let (read, write) = pty.into_split();
        let (exit, kill) = Self::exiting(child);
        Ok(AgentProcess {
            stdin: Box::new(write),
            stdout: Box::new(read),
            exit,
            kill,
            tenure: Tenure::Bound,
        })
    }

    /// A folder's repository, as the supervisor serves it.
    fn project_git(repository: Repository) -> ProjectGit {
        match repository {
            Repository::None => ProjectGit::None,
            Repository::At(Head::Branch(name)) => ProjectGit::Branch { name },
            Repository::At(Head::Detached(commit)) => ProjectGit::Detached {
                commit: commit.abbreviated(7).to_owned(),
            },
            Repository::Unreadable => ProjectGit::Unreadable,
        }
    }

    fn git_exit(exit: ren_git::Exit) -> ProcessExit {
        match exit {
            ren_git::Exit::Code(code) => ProcessExit::Code(code),
            ren_git::Exit::Signal(signal) => ProcessExit::Signal(signal),
            ren_git::Exit::Unknown => ProcessExit::Unknown,
        }
    }

    fn exit(status: std::process::ExitStatus) -> ProcessExit {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return ProcessExit::Signal(signal);
            }
        }
        status
            .code()
            .map_or(ProcessExit::Unknown, ProcessExit::Code)
    }
}

impl AgentHost for ProcessHost {
    /// On Tokio's blocking pool: the filesystem and short processes wait
    /// there, not on the threads that forward calls.
    fn blocking<T, W>(&self, work: W) -> futures_util::future::BoxFuture<'static, io::Result<T>>
    where
        T: Send + 'static,
        W: FnOnce() -> io::Result<T> + Send + 'static,
    {
        Box::pin(async move {
            tokio::task::spawn_blocking(work)
                .await
                .unwrap_or_else(|joined| Err(io::Error::other(joined)))
        })
    }

    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn look(&self, dir: &Path) -> io::Result<Look> {
        let mut folders = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Ok(name) = entry.file_name().into_string()
            {
                folders.push(name);
            }
        }
        Ok(Look {
            folders,
            git: Self::project_git(Git::repository(dir)),
        })
    }

    fn files(&self, dir: &Path, limits: WalkLimits) -> io::Result<Vec<FoundPath>> {
        // Hidden files and what git ignores are left out, as an editor's
        // file finder does; links are not followed.
        let walk = ignore::WalkBuilder::new(dir)
            .max_depth(Some(limits.depth))
            .follow_links(false)
            .require_git(false)
            .build();
        let mut found = Vec::new();
        for entry in walk {
            let entry = entry.map_err(io::Error::other)?;
            if entry.depth() == 0 {
                continue;
            }
            let Some(kind) = entry.file_type().map(|kind| {
                if kind.is_dir() {
                    PathKind::Directory
                } else {
                    PathKind::File
                }
            }) else {
                continue;
            };
            let Some(path) = entry
                .path()
                .strip_prefix(dir)
                .ok()
                .and_then(Path::to_str)
                .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"))
            else {
                continue;
            };
            found.push(FoundPath { path, kind });
            if found.len() >= limits.entries {
                break;
            }
        }
        Ok(found)
    }

    fn finds(&self, program: &str) -> bool {
        // The same search a started process makes: this process's path,
        // which every agent it starts inherits.
        #[expect(
            clippy::disallowed_methods,
            reason = "the host finds a program on the path its agents inherit"
        )]
        let path = std::env::var_os("PATH");
        path.is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    }

    fn client_stamp(&self, program: &str, tools: &RunTools) -> Option<ClientStamp> {
        // The program a process with these tools would start: the first on
        // its search path, read through its links.
        let found = self
            .search_dirs(tools)
            .into_iter()
            .map(|dir| dir.join(program))
            .find(|file| file.is_file())?;
        let path = std::fs::canonicalize(&found).ok()?;
        let metadata = std::fs::metadata(&path).ok()?;
        Some(ClientStamp::new(
            path,
            metadata.len(),
            metadata.modified().ok(),
        ))
    }

    fn make_dir(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn place_file(&self, incoming: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
        for dir in [incoming.parent(), target.parent()].into_iter().flatten() {
            std::fs::create_dir_all(dir)?;
        }
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(incoming)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, bytes)?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(incoming, target));
        if written.is_err() {
            let _ = std::fs::remove_file(incoming);
        }
        written
    }

    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn remove_tree(&self, dir: &Path) -> io::Result<()> {
        match std::fs::remove_dir_all(dir) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            done => done,
        }
    }

    fn make_scratch(&self, dir: &Path, tools: &RunTools) -> io::Result<Scratch> {
        std::fs::create_dir(dir)?;
        // Without git the folder still works; its changes are not tracked.
        let Some(git) = Git::find(&self.search_dirs(tools)) else {
            return Ok(Scratch::Untracked(Untracked::NoGit));
        };
        Ok(match git.init(dir) {
            Ok(()) => Scratch::Repository,
            Err(GitError::Failed { exit, .. }) => {
                Scratch::Untracked(Untracked::InitFailed(Self::git_exit(exit)))
            }
            Err(error @ GitError::NotStarted { .. }) => {
                tracing::debug!(%error, "cannot start git");
                Scratch::Untracked(Untracked::NotStarted)
            }
            // `git init` prints nothing that is read.
            Err(error @ GitError::Unexpected { .. }) => {
                tracing::debug!(%error, "git init answered oddly");
                Scratch::Untracked(Untracked::InitFailed(ProcessExit::Unknown))
            }
        })
    }

    fn make_worktree(
        &self,
        repo: &Path,
        folder: &Path,
        data: &Path,
        branch: &str,
        tools: &RunTools,
    ) -> io::Result<()> {
        let git = Git::find(&self.search_dirs(tools))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no git on the runs' path"))?;
        let branch = branch
            .parse::<ren_git::Branch>()
            .map_err(io::Error::other)?;
        git.add_worktree(repo, folder, &branch)
            .map_err(io::Error::other)?;
        std::fs::create_dir_all(data)
    }

    fn remove_worktree(&self, folder: &Path, data: &Path, tools: &RunTools) -> io::Result<()> {
        let git = Git::find(&self.search_dirs(tools))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no git on the runs' path"))?;
        let branch = match Git::repository(folder) {
            Repository::At(Head::Branch(branch)) => branch.parse::<ren_git::Branch>().ok(),
            Repository::At(Head::Detached(_)) | Repository::None | Repository::Unreadable => None,
        };
        let main = git.main_worktree(folder).map_err(io::Error::other)?;
        git.remove_worktree(folder).map_err(io::Error::other)?;
        match std::fs::remove_dir_all(data) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Ok(()) | Err(_) => {}
        }
        if let Some(branch) = branch {
            let kept = git
                .delete_merged(&main, &branch)
                .map_err(io::Error::other)?;
            tracing::debug!(%branch, ?kept, "a workspace's branch");
        }
        Ok(())
    }

    fn tools(&self) -> RunTools {
        match self.tools.current() {
            Ok(toolset) if toolset.generation() == Generation::NONE => RunTools::Empty,
            Ok(toolset) => RunTools::Toolset {
                generation: toolset.generation(),
                bin: self.tools.bin(toolset.generation()),
            },
            Err(error) => {
                tracing::warn!(%error, "cannot read the tool store; runs start without its tools");
                RunTools::Unreadable
            }
        }
    }

    fn spawn(
        &self,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess> {
        let command = self.command(plan, tools)?;
        Self::start(command, workdir.as_path(), Stderr::Discarded)
    }

    fn start_run(
        &self,
        run: RunId,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess> {
        let Some(held) = &self.held else {
            return self.spawn(plan, workdir, tools);
        };
        let mut command = self.command(plan, tools)?;
        command.current_dir(workdir.as_path());
        // A run host does not start with the daemon's environment: the
        // search path the daemon's own runs get is given to it.
        if command.get_envs().all(|(key, _)| key != "PATH") {
            command.env(
                "PATH",
                std::env::join_paths(self.search_dirs(tools)).map_err(io::Error::other)?,
            );
        }
        held.start(run, &command)
    }

    fn held(&self) -> io::Result<Vec<HeldRun>> {
        self.held.as_ref().map_or(Ok(Vec::new()), HeldRuns::list)
    }

    fn spool(&self, run: RunId, from: u64, to: u64) -> io::Result<Vec<u8>> {
        self.holder()?.spool(run, from, to)
    }

    fn rejoin(&self, run: RunId, from: u64) -> io::Result<AgentProcess> {
        self.holder()?.rejoin(run, from)
    }

    fn release(&self, run: RunId) {
        if let Some(held) = &self.held {
            held.release(run);
        }
    }

    fn spawn_with_stderr(
        &self,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess> {
        if plan.loads_pi_extension() {
            return Err(io::Error::other(
                "a client's own short commands load no extension",
            ));
        }
        let mut command = plan.command(None, &[]);
        if let Some(path) = self.search_path_with(tools)? {
            command.env("PATH", path);
        }
        Self::start(command, workdir.as_path(), Stderr::WithStdout)
    }

    fn spawn_in_terminal(
        &self,
        plan: &LaunchPlan,
        workdir: &Workdir,
        tools: &RunTools,
    ) -> io::Result<AgentProcess> {
        if plan.loads_pi_extension() {
            return Err(io::Error::other(
                "a client's own short commands load no extension",
            ));
        }
        let mut command = plan.command(None, &[]);
        if let Some(path) = self.search_path_with(tools)? {
            command.env("PATH", path);
        }
        #[cfg(unix)]
        {
            Self::start_in_terminal(&command, workdir.as_path())
        }
        #[cfg(not(unix))]
        {
            let _ = (command, workdir);
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    fn open_local(
        &self,
        address: &LocalRedirect,
    ) -> futures_util::future::BoxFuture<'static, io::Result<()>> {
        let address = address.as_str().to_owned();
        Box::pin(async move {
            // Loopback only (the address was checked to be the one the
            // login listens on): no proxy, no redirect followed.
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(io::Error::other)?;
            let response = client
                .get(&address)
                .send()
                .await
                .map_err(io::Error::other)?;
            let status = response.status();
            if status.is_success() || status.is_redirection() {
                Ok(())
            } else {
                Err(io::Error::other(format!("it answered {status}")))
            }
        })
    }
}

/// Where a started process's stderr goes.
#[derive(Debug, Clone, Copy)]
enum Stderr {
    Discarded,
    /// Read with its stdout, as one stream.
    WithStdout,
}

/// Two pipes read as one: whatever either has, in the order it is read,
/// ending once both have ended. Lines from one never split the other's
/// within a read, but a line written in pieces may interleave with the
/// other pipe's.
struct Merged<A, B> {
    first: Option<A>,
    second: Option<B>,
}

impl<A, B> Merged<A, B> {
    fn new(first: A, second: B) -> Self {
        Self {
            first: Some(first),
            second: Some(second),
        }
    }
}

impl<A, B> tokio::io::AsyncRead for Merged<A, B>
where
    A: tokio::io::AsyncRead + Unpin,
    B: tokio::io::AsyncRead + Unpin,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        let this = &mut *self;
        let before = buffer.filled().len();
        if let Some(first) = this.first.as_mut() {
            match std::pin::Pin::new(first).poll_read(context, buffer) {
                Poll::Ready(Ok(())) if buffer.filled().len() > before => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Ok(())) => this.first = None,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => {}
            }
        }
        if let Some(second) = this.second.as_mut() {
            match std::pin::Pin::new(second).poll_read(context, buffer) {
                Poll::Ready(Ok(())) if buffer.filled().len() > before => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Ok(())) => this.second = None,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => {}
            }
        }
        if this.first.is_none() && this.second.is_none() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessHost;
    use crate::tools::ToolStore;
    use ren_depot::{Depot, source::Platform, toolset::Toolset};
    use ren_supervisor::RunTools;
    use ren_supervisor::{AgentHost, FileListing, PathKind, WalkLimits};

    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str, mode: u32) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn runs_get_the_current_toolset_first_on_their_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = || ToolStore::without_downloads(dir.path().join("tools"));
        let host = ProcessHost::new(None, vec!["/usr/bin".into(), "/bin".into()], store());
        // Nothing installed: the inherited path is left alone.
        assert_eq!(host.tools(), RunTools::Empty);
        assert_eq!(host.search_path_with(&RunTools::Empty).unwrap(), None);

        let depot = Depot::builtin().unwrap();
        let empty = Toolset::empty();
        let plan = empty
            .plan(
                &depot,
                Platform::LinuxX64,
                &std::collections::BTreeSet::new(),
            )
            .unwrap();
        let one = store().activate(plan).unwrap();
        let tools = host.tools();
        let bin = dir.path().join("tools/generations/1/bin");
        assert_eq!(
            tools,
            RunTools::Toolset {
                generation: one.generation(),
                bin: bin.clone(),
            }
        );
        assert_eq!(
            host.search_path_with(&tools).unwrap(),
            Some(std::env::join_paths([bin, "/usr/bin".into(), "/bin".into()]).unwrap())
        );

        // A current link to a generation that is not there is unreadable,
        // never taken for no tools.
        std::fs::remove_file(dir.path().join("tools/current")).unwrap();
        std::os::unix::fs::symlink("generations/9", dir.path().join("tools/current")).unwrap();
        assert_eq!(host.tools(), RunTools::Unreadable);
        assert_eq!(RunTools::Unreadable.generation(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_scratch_folder_is_made_by_the_runs_own_git() {
        use ren_depot::toolset::Generation;
        use ren_supervisor::{AgentHost, Scratch, Untracked};
        let dir = tempfile::tempdir().unwrap();
        let (host_bin, tools_bin) = (dir.path().join("host"), dir.path().join("tools"));
        for bin in [&host_bin, &tools_bin] {
            std::fs::create_dir(bin).unwrap();
        }
        // The host's git fails; the toolset's, first on the run's path, works.
        script(&host_bin, "git", "exit 3", 0o755);
        script(&tools_bin, "git", "exit 0", 0o755);
        let store = ToolStore::without_downloads(dir.path().join("store"));
        let host = ProcessHost::new(None, vec![host_bin], store.clone());
        let toolset = RunTools::Toolset {
            generation: Generation::NONE,
            bin: tools_bin,
        };
        assert_eq!(
            host.make_scratch(&dir.path().join("scratch-1"), &toolset)
                .unwrap(),
            Scratch::Repository
        );
        let without = ProcessHost::new(None, Vec::new(), store);
        let made = dir.path().join("scratch-2");
        assert_eq!(
            without.make_scratch(&made, &RunTools::Empty).unwrap(),
            Scratch::Untracked(Untracked::NoGit)
        );
        assert!(made.is_dir(), "the folder is made without git");
        // A folder already there is never reused.
        assert!(without.make_scratch(&made, &RunTools::Empty).is_err());
    }

    #[test]
    fn a_folders_files_leave_out_what_git_ignores_and_hidden_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for folder in ["src/deep/er", "target/debug", ".git", ".hidden"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        for file in [
            "src/lib.rs",
            "src/deep/er/far.rs",
            "target/debug/ren",
            ".hidden/key",
            ".env",
            "README.md",
        ] {
            std::fs::write(root.join(file), "x").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let host = ProcessHost::new(
            None,
            Vec::new(),
            ToolStore::without_downloads(root.join("tools")),
        );
        let mut found: Vec<(String, PathKind)> = host
            .files(root, FileListing::LIMITS)
            .unwrap()
            .into_iter()
            .map(|found| (found.path, found.kind))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            found,
            [
                ("README.md".to_owned(), PathKind::File),
                ("src".to_owned(), PathKind::Directory),
                ("src/deep".to_owned(), PathKind::Directory),
                ("src/deep/er".to_owned(), PathKind::Directory),
                ("src/deep/er/far.rs".to_owned(), PathKind::File),
                ("src/lib.rs".to_owned(), PathKind::File),
            ]
        );
        let shallow = host
            .files(
                root,
                WalkLimits {
                    entries: 2,
                    depth: 1,
                },
            )
            .unwrap();
        assert_eq!(shallow.len(), 2);
        assert!(shallow.iter().all(|found| !found.path.contains('/')));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn starts_a_process_in_its_directory_and_kills_it_on_request() {
        use super::{ProcessExit, ProcessHost, Stderr};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let mut echo = std::process::Command::new("sh");
        echo.args([
            "-c",
            "pwd; echo hidden >&2; read line; echo \"$line\"; exit 3",
        ]);
        let mut process = ProcessHost::start(echo, dir.path(), Stderr::Discarded).unwrap();
        process.stdin.write_all(b"hello\n").await.unwrap();
        let mut output = String::new();
        process.stdout.read_to_string(&mut output).await.unwrap();
        let resolved = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(output, format!("{}\nhello\n", resolved.display()));
        assert_eq!(process.exit.await, ProcessExit::Code(3));

        let mut sleeper = std::process::Command::new("sleep");
        sleeper.arg("30");
        let sleeping = ProcessHost::start(sleeper, dir.path(), Stderr::Discarded).unwrap();
        sleeping.kill.fire();
        assert_eq!(sleeping.exit.await, ProcessExit::Signal(9));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn a_terminal_login_sees_a_terminal_and_takes_enter() {
        use super::{ProcessExit, ProcessHost};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let mut login = std::process::Command::new("sh");
        // Like `pi auth login --api-key`: a terminal, a prompt, no echo.
        login.args([
            "-c",
            "[ -t 0 ] && echo terminal; stty -echo; printf 'Enter key: '; read key; echo; echo \"Saved ${#key} characters\"",
        ]);
        let mut process = ProcessHost::start_in_terminal(&login, dir.path()).unwrap();
        process.stdin.write_all(b"sk-acme\r").await.unwrap();
        let mut output = Vec::new();
        let mut buffer = [0; 1024];
        // A terminal reads as an error once its process has exited.
        while let Ok(read @ 1..) = process.stdout.read(&mut buffer).await {
            output.extend_from_slice(&buffer[..read]);
        }
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("terminal"), "{output:?}");
        assert!(output.contains("Saved 7 characters"), "{output:?}");
        assert!(!output.contains("sk-acme"), "{output:?}");
        assert_eq!(process.exit.await, ProcessExit::Code(0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_login_is_read_from_stdout_and_stderr_together() {
        use super::{ProcessExit, ProcessHost, Stderr};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let mut login = std::process::Command::new("sh");
        // Like `claude auth login`: a link on stdout, a prompt with no
        // newline, then a failure on stderr.
        login.args([
            "-c",
            "echo 'visit: https://example.com/a'; printf 'Paste code > '; read code; echo \"Login failed: $code\" >&2; exit 1",
        ]);
        let mut process = ProcessHost::start(login, dir.path(), Stderr::WithStdout).unwrap();
        process.stdin.write_all(b"bad\n").await.unwrap();
        let mut output = String::new();
        process.stdout.read_to_string(&mut output).await.unwrap();
        assert_eq!(
            output,
            "visit: https://example.com/a\nPaste code > Login failed: bad\n"
        );
        assert_eq!(process.exit.await, ProcessExit::Code(1));
    }
}
