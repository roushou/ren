//! Runs a run host holds, which outlive the daemon: what the daemon gives
//! each process goes into its journal before the process sees it; as the
//! daemon stops, each is detached, not ended; as the next starts, each is
//! rebuilt from its journal and its spool (`crate::journal`) and taken up
//! where the last daemon left it.
//!
//! A journal entry is written before anything it leads to reaches the
//! client: each stdin line waits for the entries queued before it. The
//! journal never waits for stdin. A journal that cannot keep up, or fails,
//! is broken: its run can no longer be taken up exactly, so it is drained
//! with the daemon like a bound one.

use super::{Live, Served, SupervisorTask};
use crate::journal::{Command, Entry, Replay, ReplayError, Replayed};
use crate::process::{ProcessEffect, Rest};
use crate::run::{RunCommand, RunState};
use crate::{AgentHost, CommandError, RunUpdate, host::Tenure};
use futures_util::{FutureExt, StreamExt, future::AbortHandle};
use ren_harness::lines::Line;
use ren_recorder::id::RunId;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::{mpsc, watch},
};

/// A held run's journal and the handles that let the daemon leave it.
pub(super) struct Holding {
    journal: Journal,
    output: AbortHandle,
    exit: AbortHandle,
}

enum Journal {
    Kept {
        entries: mpsc::Sender<Vec<u8>>,
        /// Entries queued so far.
        queued: u64,
    },
    /// An entry was lost: the run cannot be taken up exactly.
    Broken,
}

/// A line for a client's stdin, written once the journal holds `after`
/// entries.
pub(super) struct Queued {
    pub line: Line,
    pub after: u64,
}

/// Which of a process's writers finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Writer {
    Stdin,
    Journal,
}

/// Why a held run could not be taken up.
#[derive(Debug, thiserror::Error)]
enum RejoinError {
    #[error(transparent)]
    Replay(#[from] ReplayError),
    #[error("its run host could not be reached: {0}")]
    Host(#[from] std::io::Error),
    #[error("the recorder has no room to record it")]
    Unrecorded,
}

impl Holding {
    /// Entries a journal holds queued before it counts as not keeping up.
    const ENTRIES: usize = 4096;

    /// A held process's journal, written by the future returned with it,
    /// and the count of entries written, which stdin lines wait on.
    pub(super) fn keep(
        journal: Box<dyn AsyncWrite + Send + Unpin>,
        output: AbortHandle,
        exit: AbortHandle,
    ) -> (
        Self,
        futures_util::future::BoxFuture<'static, std::io::Result<()>>,
        watch::Receiver<u64>,
    ) {
        let (entries, receiver) = mpsc::channel(Self::ENTRIES);
        let (written, watched) = watch::channel(0);
        let holding = Self {
            journal: Journal::Kept { entries, queued: 0 },
            output,
            exit,
        };
        (
            holding,
            Self::journaler(journal, receiver, written).boxed(),
            watched,
        )
    }

    /// Queue `entry`. A full or closed journal is broken.
    pub(super) fn journal(&mut self, run: RunId, entry: &Entry) {
        let Journal::Kept { entries, queued } = &mut self.journal else {
            return;
        };
        let sent = entry
            .line()
            .map_err(|error| error.to_string())
            .and_then(|line| entries.try_send(line).map_err(|error| error.to_string()));
        match sent {
            Ok(()) => *queued += 1,
            Err(error) => {
                tracing::warn!(%run, %error, "a run's journal broke; it ends with the daemon");
                self.journal = Journal::Broken;
            }
        }
    }

    /// The entries queued so far: a line queued now waits for them.
    pub(super) fn queued(&self) -> u64 {
        match self.journal {
            Journal::Kept { queued, .. } => queued,
            Journal::Broken => 0,
        }
    }

    pub(super) fn broke(&mut self) {
        self.journal = Journal::Broken;
    }

    /// Whether the run can be left to its host as the daemon stops.
    fn intact(&self) -> bool {
        match self.journal {
            Journal::Kept { .. } => true,
            Journal::Broken => false,
        }
    }

    async fn journaler(
        mut journal: Box<dyn AsyncWrite + Send + Unpin>,
        mut entries: mpsc::Receiver<Vec<u8>>,
        written: watch::Sender<u64>,
    ) -> std::io::Result<()> {
        while let Some(entry) = entries.recv().await {
            journal.write_all(&entry).await?;
            journal.flush().await?;
            written.send_modify(|count| *count += 1);
        }
        Ok(())
    }
}

impl<H: AgentHost> SupervisorTask<H> {
    /// Give `command` to `live`'s process, journaled first.
    pub(super) fn command(
        live: &mut Live,
        command: RunCommand,
    ) -> Result<Vec<ProcessEffect>, CommandError> {
        if let Some(holding) = live.held.as_mut() {
            holding.journal(live.process.run(), &Entry::Command(Command::from(&command)));
        }
        live.process.command(command)
    }

    /// The idle stop for `rest` ran out, journaled first.
    pub(super) fn rest_over(live: &mut Live, rest: Rest) -> Vec<ProcessEffect> {
        if let Some(holding) = live.held.as_mut() {
            holding.journal(live.process.run(), &Entry::RestOver { rest });
        }
        live.process.rest_over(rest)
    }

    /// The daemon is stopping: leave `live` to its run host when it is
    /// held, its journal is whole, and it has not ended; drain it otherwise.
    pub(super) fn leave(
        &self,
        live: &mut Live,
        idles: &mut futures_util::stream::FuturesUnordered<
            futures_util::future::BoxFuture<'static, (RunId, Rest)>,
        >,
    ) {
        let ended = matches!(live.process.state(), RunState::Ended { .. });
        match live.held.take() {
            Some(holding) if holding.intact() && !ended => {
                holding.output.abort();
                holding.exit.abort();
                // The journal's writer ends once what it holds is written.
                drop(holding.journal);
                live.stdin = None;
                // Dropped unfired: the process goes on.
                live.kill = None;
                if let Some(tap) = live.tap.take() {
                    tap.detach();
                }
                tracing::info!(run = %live.process.run(), "left a run to its run host");
            }
            // Ended, or its journal broke: it is drained, and its journal
            // closes once what it holds is written.
            Some(_) => {
                let effects = live.process.shut_down();
                self.carry_out(live, effects, idles);
            }
            None => {
                let effects = live.process.shut_down();
                self.carry_out(live, effects, idles);
            }
        }
    }

    /// Take up every run a run host held while no daemon ran. One that
    /// cannot be taken up is stopped, and recorded as recovered: what it
    /// did after the last daemon left is unknown.
    pub(super) async fn rejoin_held(&self, served: &mut Served) {
        let held = match self.host.held() {
            Ok(held) => held,
            Err(error) => {
                tracing::warn!(%error, "could not list the runs run hosts hold");
                return;
            }
        };
        for held in held {
            let run = held.run;
            if let Err(error) = self.rejoin(run, &held.journal, served).await {
                tracing::warn!(%run, %error, "could not take up a held run; stopping it");
                if let Ok(process) = self.host.rejoin(run, u64::MAX) {
                    process.kill.fire();
                }
                if let Ok(tap) = self.recorder.rejoin(run) {
                    tap.end(ren_recorder::capture::RunEnd::Recovered, None);
                }
                self.host.release(run);
            }
        }
    }

    async fn rejoin(
        &self,
        run: RunId,
        journal: &[u8],
        served: &mut Served,
    ) -> Result<(), RejoinError> {
        /// Bytes of a spool read at a time while replaying.
        const PIECE: u64 = 1024 * 1024;
        let entries = Entry::read(journal)?;
        let Some(Entry::Started(start)) = entries.first() else {
            return Err(ReplayError::NoStart.into());
        };
        let mut replay = Replay::begin(start, self.line_limit)?;
        for entry in entries.iter().skip(1) {
            if let Entry::Read { upto } = entry {
                while replay.read() < *upto {
                    let from = replay.read();
                    let to = (*upto).min(from.saturating_add(PIECE));
                    let host = self.host.clone();
                    let bytes = self
                        .host
                        .blocking(move || host.spool(run, from, to))
                        .await?;
                    if bytes.is_empty() {
                        break;
                    }
                    replay.output(&bytes);
                }
            }
            replay.entry(entry)?;
        }
        let Replayed {
            process,
            lines,
            read,
        } = replay.finish();
        let agent = self.host.rejoin(run, read)?;
        let tap = self
            .recorder
            .rejoin(run)
            .map_err(|_| RejoinError::Unrecorded)?;
        let session = process.session();
        let resting = process.resting();
        let state = process.state().clone();
        let tools = self.host.tools();
        let book = self.book_key(process.harness(), process.workdir(), &tools);
        let mut live = self.take_on(process, agent, read, lines, served);
        live.book = book;
        live.tap = Some(tap);
        let mut effects = vec![ProcessEffect::Publish(RunUpdate::State { run, state })];
        effects.extend(resting.map(ProcessEffect::Rested));
        self.carry_out(&mut live, effects, &mut served.idles);
        served.runs.insert(run, live);
        served.rejoined.insert(session, run);
        tracing::info!(%run, "took up a run its run host held");
        Ok(())
    }

    /// `process` with `agent`'s pipes, reading its output from byte `read`
    /// with `lines` as framed so far. Its writers, output, and exit are
    /// polled by the task; its tap is the caller's to give.
    pub(super) fn take_on(
        &self,
        process: crate::process::Process,
        agent: crate::AgentProcess,
        read: u64,
        lines: ren_harness::lines::Lines,
        served: &mut Served,
    ) -> Live {
        let run = process.run();
        let crate::AgentProcess {
            stdin,
            stdout,
            exit,
            kill,
            tenure,
        } = agent;
        let (output, output_handle) =
            futures_util::stream::abortable(Self::output(run, stdout, read, lines));
        let (exit, exit_handle) = futures_util::future::abortable(exit);
        let (queue, queued) = mpsc::channel(Self::STDIN_LINES);
        let (held, journaled) = match tenure {
            Tenure::Bound => (None, None),
            Tenure::Held { journal } => {
                let (holding, journaler, journaled) =
                    Holding::keep(journal, output_handle, exit_handle);
                served.writers.push(
                    journaler
                        .map(move |written| (run, Writer::Journal, written))
                        .boxed(),
                );
                (Some(holding), Some(journaled))
            }
        };
        served.writers.push(
            Self::writer(stdin, queued, journaled)
                .map(move |written| (run, Writer::Stdin, written))
                .boxed(),
        );
        served.outputs.push(output.boxed());
        served
            .exits
            .push(exit.map(move |exit| (run, exit.ok())).boxed());
        Live {
            process,
            book: None,
            booked: Vec::new(),
            stdin: Some(queue),
            kill: Some(kill),
            tap: None,
            held,
        }
    }
}
