//! The host's clocks, randomness, and the observation plane's thread, as the
//! daemon hands them to components.
//!
//! These are the production implementations of the recorder's `Clock` and
//! `IdSource` ports, and the only code that reads the host clocks or mints
//! random identities. Components receive them as values; nothing else calls
//! `SystemTime::now`, `Instant::now`, or `Uuid::new_v4`.

use ren_harness::{launch::LaunchPlan, pi::PiCache};
use ren_recorder::{
    id::IdSource,
    time::{Clock, Moment, Monotonic, UnixMillis},
};
use ren_supervisor::{AgentHost, AgentProcess, KillSwitch, Look, ProcessExit, Workdir};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;
use uuid::Uuid;

/// The host's clocks. Monotonic time counts from [`SystemClock::start`].
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned reader of the host's monotonic clock"
    )]
    pub fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    /// A host clock set before 1970 reads as the epoch: wall time is for
    /// display only, while ordering and durations use monotonic time.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned reader of the host's wall clock"
    )]
    fn now(&self) -> Moment {
        Moment {
            wall: UnixMillis::try_from(SystemTime::now()).unwrap_or(UnixMillis::EPOCH),
            monotonic: Monotonic::from_elapsed(self.origin.elapsed()),
        }
    }
}

/// Random version 4 UUIDs.
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomIds;

impl IdSource for RandomIds {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned source of random identities"
    )]
    fn next_uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}

/// The observation plane's own thread and runtime. The recorder runs there,
/// so cutting chunks, inspecting, deriving digests, and writing storage never
/// occupy a worker that forwards traffic, however much work they take.
#[must_use = "the observation task stops when this handle is aborted"]
pub struct ObservationThread<T> {
    output: oneshot::Receiver<T>,
    stop: Option<oneshot::Sender<()>>,
}

impl<T: Send + 'static> ObservationThread<T> {
    /// Run `task` on a current-thread runtime of a new thread.
    pub fn start<F>(task: F) -> io::Result<Self>
    where
        F: Future<Output = T> + Send + 'static,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("ren-observation")
            .build()?;
        let (output_sender, output) = oneshot::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("ren-observation".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    tokio::select! {
                        result = task => {
                            let _ = output_sender.send(result);
                        }
                        // Only an explicit stop ends it early; a dropped
                        // handle lets the task finish on its own.
                        Ok(()) = stopped => {}
                    }
                });
            })?;
        Ok(Self {
            output,
            stop: Some(stop),
        })
    }

    /// The task's output, once it finishes. `None` when it stopped without
    /// one: it panicked, or it was stopped.
    pub async fn join(&mut self) -> Option<T> {
        (&mut self.output).await.ok()
    }

    /// Stop the task at its next await point; [`Self::join`] then returns
    /// `None`.
    pub fn abort(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// Starts agent processes on this host: the production [`AgentHost`]. Each
/// process works in its run's directory with piped stdin and stdout; stderr
/// is discarded, and a process whose exit future is dropped is killed.
/// Pi loads Ren's extension, published to `pi` before each start.
#[derive(Debug, Default, Clone)]
pub struct ProcessHost {
    pi: Option<PiCache>,
}

impl ProcessHost {
    pub fn new(pi: Option<PiCache>) -> Self {
        Self { pi }
    }

    fn start(mut command: std::process::Command, workdir: &Path) -> io::Result<AgentProcess> {
        command
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        #[expect(
            clippy::disallowed_methods,
            reason = "the one sanctioned place that starts agent processes"
        )]
        let mut child = command.spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the agent's pipes were not opened"));
        };
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
        Ok(AgentProcess {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            exit: Box::pin(exit),
            kill,
        })
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

impl ProcessHost {
    /// Whether `dir` holds a git repository, and its `HEAD`'s text when it
    /// can be read. `.git` is a folder, or, in a linked worktree, a file
    /// naming the repository's folder (`gitdir: <path>`).
    fn git(dir: &Path) -> Option<Option<String>> {
        let dot_git = dir.join(".git");
        let kind = std::fs::symlink_metadata(&dot_git).ok()?.file_type();
        let git_dir = if kind.is_dir() {
            Some(dot_git)
        } else if kind.is_file() {
            Self::small(&dot_git).and_then(|text| {
                let path = text.trim().strip_prefix("gitdir:")?.trim().to_owned();
                Some(dir.join(path))
            })
        } else {
            None
        };
        Some(git_dir.and_then(|git_dir| Self::small(&git_dir.join("HEAD"))))
    }

    /// A small text file's first bytes; `HEAD` is one line.
    fn small(path: &Path) -> Option<String> {
        use std::io::Read as _;
        let mut text = String::new();
        std::fs::File::open(path)
            .ok()?
            .take(1024)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    }
}

impl AgentHost for ProcessHost {
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
            git: Self::git(dir),
        })
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

    fn make_dir(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn make_scratch(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir(dir)?;
        let mut git = std::process::Command::new("git");
        git.args(["init", "--quiet"])
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let status = git.status();
        // Without git the folder still works; its changes are not tracked.
        if let Err(error) = status {
            tracing::debug!(%error, "cannot make a scratch folder a repository");
        }
        Ok(())
    }

    fn spawn(&self, plan: &LaunchPlan, workdir: &Workdir) -> io::Result<AgentProcess> {
        let extension = if plan.loads_pi_extension() {
            let cache = self.pi.as_ref().ok_or_else(|| {
                io::Error::other("set XDG_CACHE_HOME or HOME for the Pi extension cache")
            })?;
            Some(cache.materialize().map_err(io::Error::other)?)
        } else {
            None
        };
        Self::start(plan.command(extension.as_deref(), &[]), workdir.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::{RandomIds, SystemClock};
    use ren_recorder::{
        id::{AttemptId, IdSource},
        time::{Clock, UnixMillis},
    };

    #[test]
    fn system_clock_is_monotonic() {
        let clock = SystemClock::start();
        let first = clock.now();
        let second = clock.now();
        assert!(second.monotonic >= first.monotonic);
        assert!(first.wall > UnixMillis::EPOCH);
    }

    #[test]
    fn random_ids_are_distinct_and_round_trip_through_text() {
        let source: &dyn IdSource = &RandomIds;
        let first = AttemptId::generate(source);
        let second = AttemptId::generate(source);
        assert_ne!(first, second);
        assert_eq!(first.to_string().parse::<AttemptId>(), Ok(first));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn work_on_the_observation_thread_never_stalls_the_callers_runtime() {
        // A task that blocks its thread, as heavy parsing would.
        let mut thread = super::ObservationThread::start(async {
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::thread::current().name().map(str::to_owned)
        })
        .unwrap();
        // Meanwhile this single-threaded runtime keeps running timers.
        let mut ticks = 0;
        let output = loop {
            tokio::select! {
                output = thread.join() => break output,
                () = tokio::time::sleep(std::time::Duration::from_millis(10)) => ticks += 1,
            }
        };
        assert_eq!(output, Some(Some("ren-observation".to_owned())));
        assert!(ticks >= 10, "the caller's runtime stalled: {ticks} ticks");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn starts_a_process_in_its_directory_and_kills_it_on_request() {
        use super::{ProcessExit, ProcessHost};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let mut echo = std::process::Command::new("sh");
        echo.args(["-c", "pwd; read line; echo \"$line\"; exit 3"]);
        let mut process = ProcessHost::start(echo, dir.path()).unwrap();
        process.stdin.write_all(b"hello\n").await.unwrap();
        let mut output = String::new();
        process.stdout.read_to_string(&mut output).await.unwrap();
        let resolved = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(output, format!("{}\nhello\n", resolved.display()));
        assert_eq!(process.exit.await, ProcessExit::Code(3));

        let mut sleeper = std::process::Command::new("sleep");
        sleeper.arg("30");
        let sleeping = ProcessHost::start(sleeper, dir.path()).unwrap();
        sleeping.kill.fire();
        assert_eq!(sleeping.exit.await, ProcessExit::Signal(9));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_stopped_task_ends_without_output() {
        let mut stopped = super::ObservationThread::start(std::future::pending::<()>()).unwrap();
        stopped.abort();
        assert_eq!(stopped.join().await, None);
        let mut finished = super::ObservationThread::start(async { 7 }).unwrap();
        assert_eq!(finished.join().await, Some(7));
    }
}
