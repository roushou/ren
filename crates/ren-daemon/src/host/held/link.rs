//! The daemon's end of a run host's socket, as the supervisor's
//! [`AgentProcess`]: the client's output is read from the host's output
//! frames, and the exit future carries the client's stdin to the host and
//! the kill switch, so nothing is spawned. The exit resolves once every
//! byte of output before it has been read.

use super::frame::{Frame, Frames};
use ren_supervisor::{AgentProcess, KillSwitch, ProcessExit, Tenure};
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _, ReadBuf};
use tokio::net::unix::OwnedReadHalf;
use tokio::sync::oneshot;

/// A link to one run host.
pub(super) struct Link;

/// The client's output, read from a host's frames.
struct Output {
    read: OwnedReadHalf,
    frames: Frames,
    scratch: Vec<u8>,
    pending: Vec<u8>,
    at: usize,
    /// The exit the host sent, given once its output is read.
    exited: Option<ProcessExit>,
    ended: bool,
    exit: Option<oneshot::Sender<ProcessExit>>,
}

impl Link {
    /// Bytes of stdin carried at a time.
    const PIECE: usize = 64 * 1024;

    /// Attach to the host on `stream`, its client's output from `from`.
    pub(super) fn open(
        stream: std::os::unix::net::UnixStream,
        from: u64,
        tenure: Tenure,
    ) -> io::Result<AgentProcess> {
        stream.set_nonblocking(true)?;
        let (read, mut write) = tokio::net::UnixStream::from_std(stream)?.into_split();
        let (exit_tx, mut exit_rx) = oneshot::channel();
        let stdout = Output {
            read,
            frames: Frames::default(),
            scratch: vec![0; Self::PIECE],
            pending: Vec::new(),
            at: 0,
            exited: None,
            ended: false,
            exit: Some(exit_tx),
        };
        let (stdin, mut taken) = tokio::io::duplex(Self::PIECE);
        let (kill, signal) = KillSwitch::pair();
        let exit = async move {
            let mut open = write
                .write_all(&Frame::Hello { from }.encode())
                .await
                .is_ok();
            let mut killed = false;
            let mut signal = std::pin::pin!(signal.fired());
            let mut buffer = vec![0; Self::PIECE];
            loop {
                tokio::select! {
                    exit = &mut exit_rx => return exit.unwrap_or(ProcessExit::Unknown),
                    () = &mut signal, if !killed => {
                        killed = true;
                        let _ = write.write_all(&Frame::Kill.encode()).await;
                    }
                    read = taken.read(&mut buffer), if open => match read {
                        Ok(0) | Err(_) => open = false,
                        Ok(read) => {
                            let input = Frame::Input(buffer.get(..read).unwrap_or_default().to_vec());
                            open = write.write_all(&input.encode()).await.is_ok();
                        }
                    },
                }
            }
        };
        Ok(AgentProcess {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            exit: Box::pin(exit),
            kill,
            tenure,
        })
    }

    /// What a run left when its host is gone: its spool from `from`, and
    /// its exit. Its stdin goes nowhere, and there is nothing to kill.
    pub(super) fn left(
        spool: &Path,
        from: u64,
        exit: ProcessExit,
        tenure: Tenure,
    ) -> io::Result<AgentProcess> {
        let mut file = std::fs::File::open(spool)?;
        std::io::Seek::seek(&mut file, io::SeekFrom::Start(from))?;
        let stdout = tokio::fs::File::from_std(file);
        let (kill, _gone) = KillSwitch::pair();
        Ok(AgentProcess {
            stdin: Box::new(tokio::io::sink()),
            stdout: Box::new(stdout),
            exit: Box::pin(std::future::ready(exit)),
            kill,
            tenure,
        })
    }
}

impl Output {
    fn take(&mut self, frames: Vec<Frame>) {
        for frame in frames {
            match frame {
                Frame::Output(bytes) => self.pending.extend(bytes),
                Frame::Exited(exit) => {
                    self.exited = Some(exit);
                    self.ended = true;
                }
                // Only a daemon sends these; a frame this build does not
                // know is passed over.
                Frame::Hello { .. } | Frame::Input(_) | Frame::Kill | Frame::Unknown { .. } => {}
            }
        }
    }

    /// The output has ended: the exit goes once every byte before it is
    /// read; without one, the exit is unknown.
    fn end(&mut self) {
        self.ended = true;
        if let Some(sender) = self.exit.take() {
            let _ = sender.send(self.exited.unwrap_or(ProcessExit::Unknown));
        }
    }
}

impl AsyncRead for Output {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if this.at < this.pending.len() {
                let available = this.pending.get(this.at..).unwrap_or_default();
                let len = available.len().min(buf.remaining());
                buf.put_slice(available.get(..len).unwrap_or_default());
                this.at += len;
                if this.at == this.pending.len() {
                    this.pending.clear();
                    this.at = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if this.ended {
                this.end();
                return Poll::Ready(Ok(()));
            }
            let mut read = ReadBuf::new(&mut this.scratch);
            if ready!(Pin::new(&mut this.read).poll_read(cx, &mut read)).is_err()
                || read.filled().is_empty()
            {
                this.end();
                continue;
            }
            match this.frames.push(read.filled()) {
                Ok(frames) => this.take(frames),
                Err(error) => {
                    tracing::warn!(%error, "a run host sent what is not a frame");
                    this.end();
                }
            }
        }
    }
}
