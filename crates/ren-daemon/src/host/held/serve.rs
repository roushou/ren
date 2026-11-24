//! The run host itself, `ren run-host <folder>`: a process of its own that
//! starts one run's client from the folder's spec and holds it. Every byte
//! the client writes is appended to the spool before any daemon is sent
//! it; its exit is written once its output has ended. The host serves one
//! daemon at a time: a daemon going away leaves the client running, its
//! stdin open. The host ends once the client has exited and a daemon has
//! been told, or, with no daemon to tell, after [`RunHost::LINGER`]; the
//! folder keeps what the next daemon needs.

use super::frame::{Frame, Frames};
use super::{HeldRuns, Spec};
use ren_supervisor::ProcessExit;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{Mutex, mpsc, watch};

/// Holds one run's client process.
pub struct RunHost {
    folder: PathBuf,
}

/// How a daemon's connection ended.
enum Served {
    /// It was told the client's exit: the host's work is done.
    ToldExit,
    /// It went away; the client goes on.
    Left,
}

impl RunHost {
    /// How long a host whose client exited waits for a daemon to tell.
    pub const LINGER: Duration = Duration::from_secs(60 * 60);
    const PIECE: usize = 64 * 1024;

    /// The host of the run in `folder`.
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// Start the client and hold it until it has exited and been told of.
    pub async fn serve(self) -> io::Result<()> {
        let spec: Spec = serde_json::from_slice(&std::fs::read(self.path(HeldRuns::SPEC))?)
            .map_err(io::Error::other)?;
        // A relayed run's ways out listen before its client starts, so it
        // finds its gateway from its first request.
        let mut doorways = Vec::new();
        for doorway in spec.relayed.iter().flat_map(|relayed| &relayed.doorways) {
            doorways.push(doorway.open().await?);
        }
        // Never done on its own: with no doorways, there is nothing to pass.
        let passing = async move {
            futures_util::future::join_all(doorways).await;
            std::future::pending::<()>().await;
        };
        let mut child = Self::start(&spec)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the client's pipes were not opened"));
        };
        let socket = self.path(HeldRuns::SOCKET);
        match std::fs::remove_file(&socket) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Ok(()) | Err(_) => {}
        }
        let listener = UnixListener::bind(&socket)?;
        let (spooled, spool_len) = watch::channel(0u64);
        let (exited, exit) = watch::channel(None::<ProcessExit>);
        let (kill, killed) = mpsc::channel(1);
        let stdin = Mutex::new(Some(stdin));
        let ended = async {
            let (spooling, status) =
                tokio::join!(self.spool(stdout, &spooled), Self::wait(&mut child, killed));
            if let Err(error) = spooling {
                tracing::warn!(%error, "the run host could not spool its client's output");
            }
            if let Err(error) = self.write_exit(status) {
                tracing::warn!(%error, "the run host could not write its client's exit");
            }
            exited.send_replace(Some(status));
            // Nothing more goes to a client that has exited.
            stdin.lock().await.take();
        };
        let serving = async {
            loop {
                let accepted = match *exit.borrow() {
                    Some(_) => tokio::time::timeout(Self::LINGER, listener.accept())
                        .await
                        .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into())),
                    None => listener.accept().await,
                };
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::TimedOut => return,
                    Err(error) => {
                        tracing::warn!(%error, "the run host could not accept a daemon");
                        continue;
                    }
                };
                let served = self
                    .connection(stream, spool_len.clone(), exit.clone(), &stdin, &kill)
                    .await;
                match served {
                    Ok(Served::ToldExit) => return,
                    Ok(Served::Left) => {}
                    Err(error) => tracing::debug!(%error, "a daemon's connection failed"),
                }
            }
        };
        // The doorways pass connections for as long as the host holds the
        // client, and close with it.
        tokio::select! {
            ((), ()) = async { tokio::join!(ended, serving) } => {}
            () = passing => {}
        }
        let _ = std::fs::remove_file(&socket);
        Ok(())
    }

    fn path(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the run host starts the one client process it holds"
    )]
    fn start(spec: &Spec) -> io::Result<tokio::process::Child> {
        tokio::process::Command::new(&spec.program)
            .args(&spec.args)
            .envs(spec.env.iter().map(|(key, value)| (key, value)))
            .current_dir(&spec.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
    }

    /// Append everything the client writes to the spool, telling how long
    /// it is after each piece.
    async fn spool(&self, mut stdout: ChildStdout, spooled: &watch::Sender<u64>) -> io::Result<()> {
        let mut spool = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(self.path(HeldRuns::SPOOL))
            .await?;
        let mut buffer = vec![0; Self::PIECE];
        loop {
            let read = stdout.read(&mut buffer).await?;
            if read == 0 {
                return Ok(());
            }
            spool
                .write_all(buffer.get(..read).unwrap_or_default())
                .await?;
            spool.flush().await?;
            let read = u64::try_from(read).unwrap_or(u64::MAX);
            spooled.send_modify(|len| *len = len.saturating_add(read));
        }
    }

    /// The client's exit; killed first when a daemon asks.
    async fn wait(
        child: &mut tokio::process::Child,
        mut killed: mpsc::Receiver<()>,
    ) -> ProcessExit {
        let status = tokio::select! {
            status = child.wait() => status,
            Some(()) = killed.recv() => match child.kill().await {
                Ok(()) => child.wait().await,
                Err(error) => Err(error),
            },
        };
        match status {
            Ok(status) => {
                use std::os::unix::process::ExitStatusExt as _;
                match (status.code(), status.signal()) {
                    (Some(code), _) => ProcessExit::Code(code),
                    (None, Some(signal)) => ProcessExit::Signal(signal),
                    (None, None) => ProcessExit::Unknown,
                }
            }
            Err(_) => ProcessExit::Unknown,
        }
    }

    fn write_exit(&self, exit: ProcessExit) -> io::Result<()> {
        let written = self.path("exit.new");
        std::fs::write(
            &written,
            serde_json::to_vec(&exit).map_err(io::Error::other)?,
        )?;
        std::fs::rename(written, self.path(HeldRuns::EXIT))
    }

    /// Serve one daemon: the output from where it asks, then as it comes;
    /// its input to the client; the exit once the output before it is sent.
    async fn connection(
        &self,
        stream: UnixStream,
        mut spool_len: watch::Receiver<u64>,
        mut exit: watch::Receiver<Option<ProcessExit>>,
        stdin: &Mutex<Option<ChildStdin>>,
        kill: &mpsc::Sender<()>,
    ) -> io::Result<Served> {
        let (mut read, mut write) = stream.into_split();
        let mut frames = Frames::default();
        let mut buffer = vec![0; Self::PIECE];
        let mut waiting = Vec::new();
        let from = loop {
            let got = read.read(&mut buffer).await?;
            if got == 0 {
                return Ok(Served::Left);
            }
            let mut hello = None;
            for frame in frames
                .push(buffer.get(..got).unwrap_or_default())
                .map_err(io::Error::other)?
            {
                match (frame, hello) {
                    (Frame::Hello { from }, None) => hello = Some(from),
                    (frame, _) => waiting.push(frame),
                }
            }
            if let Some(from) = hello {
                break from;
            }
        };
        for frame in waiting {
            Self::given(frame, stdin, kill).await;
        }
        let mut spool = tokio::fs::File::open(self.path(HeldRuns::SPOOL)).await?;
        spool.seek(io::SeekFrom::Start(from)).await?;
        let mut at = from;
        loop {
            let len = *spool_len.borrow_and_update();
            while at < len {
                let want = usize::try_from(len - at)
                    .unwrap_or(Self::PIECE)
                    .min(Self::PIECE);
                let got = spool
                    .read(buffer.get_mut(..want).unwrap_or_default())
                    .await?;
                if got == 0 {
                    break;
                }
                let output = Frame::Output(buffer.get(..got).unwrap_or_default().to_vec());
                write.write_all(&output.encode()).await?;
                at = at.saturating_add(u64::try_from(got).unwrap_or(u64::MAX));
            }
            let exited = *exit.borrow_and_update();
            if let Some(status) = exited
                && at >= *spool_len.borrow()
            {
                write.write_all(&Frame::Exited(status).encode()).await?;
                write.flush().await?;
                return Ok(Served::ToldExit);
            }
            tokio::select! {
                _ = spool_len.changed() => {}
                _ = exit.changed() => {}
                got = read.read(&mut buffer) => {
                    let got = got?;
                    if got == 0 {
                        return Ok(Served::Left);
                    }
                    for frame in frames
                        .push(buffer.get(..got).unwrap_or_default())
                        .map_err(io::Error::other)?
                    {
                        Self::given(frame, stdin, kill).await;
                    }
                }
            }
        }
    }

    /// A frame from the daemon.
    async fn given(frame: Frame, stdin: &Mutex<Option<ChildStdin>>, kill: &mpsc::Sender<()>) {
        match frame {
            Frame::Input(bytes) => {
                let mut stdin = stdin.lock().await;
                if let Some(pipe) = stdin.as_mut()
                    && (pipe.write_all(&bytes).await.is_err() || pipe.flush().await.is_err())
                {
                    stdin.take();
                }
            }
            Frame::Kill => {
                let _ = kill.try_send(());
            }
            // A second hello, or what only a host sends, or a frame this
            // build does not know: passed over.
            Frame::Hello { .. } | Frame::Output(_) | Frame::Exited(_) | Frame::Unknown { .. } => {}
        }
    }
}
