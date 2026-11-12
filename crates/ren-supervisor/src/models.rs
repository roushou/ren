//! Listing a client's models: the client started briefly in its structured
//! mode with no conversation, asked, and stopped. The task runs one listing
//! per client at a time, off its loop, and keeps the first non-empty answer.

use crate::host::AgentProcess;
use futures_util::{FutureExt, future::BoxFuture, stream::FuturesUnordered};
use ren_harness::{
    Harness,
    drive::{Framed, Line, LineLimit, Lines, Listing, Model, ModelListing},
    launch::NotDrivable,
};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::oneshot,
};

/// Why a client's models could not be listed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelsError {
    #[error(transparent)]
    NotDrivable(NotDrivable),
    #[error("no run root to list models in")]
    NoRoot,
    #[error("cannot start the client; check it is installed on PATH")]
    Spawn,
    #[error("the client refused to list its models: {0}")]
    Refused(String),
    #[error("the client exited before listing its models")]
    Exited,
    #[error("the client did not list its models in time")]
    TimedOut,
    #[error("the supervisor has stopped")]
    Gone,
}

pub(crate) type ModelsReply = oneshot::Sender<Result<Vec<Model>, ModelsError>>;
type Running = FuturesUnordered<BoxFuture<'static, (Harness, Result<Vec<Model>, ModelsError>)>>;

/// The task's listings: answers kept, and those still running with who
/// waits on them.
#[derive(Default)]
pub(crate) struct Listings {
    kept: HashMap<Harness, Vec<Model>>,
    waiting: HashMap<Harness, Vec<ModelsReply>>,
    pub(crate) running: Running,
}

impl Listings {
    /// Answer from what is kept, or join a listing already running. Gives
    /// the reply back when a listing has to start.
    pub(crate) fn ask(&mut self, harness: Harness, reply: ModelsReply) -> Option<ModelsReply> {
        if let Some(models) = self.kept.get(&harness) {
            let _ = reply.send(Ok(models.clone()));
            return None;
        }
        match self.waiting.get_mut(&harness) {
            Some(waiting) => {
                waiting.push(reply);
                None
            }
            None => Some(reply),
        }
    }

    /// Run `listing` for `harness`, answering `reply` and whoever asks
    /// meanwhile when it ends.
    pub(crate) fn start(
        &mut self,
        harness: Harness,
        reply: ModelsReply,
        listing: BoxFuture<'static, Result<Vec<Model>, ModelsError>>,
    ) {
        self.waiting.insert(harness, vec![reply]);
        self.running
            .push(listing.map(move |outcome| (harness, outcome)).boxed());
    }

    /// A listing ended. A client that lists nothing (not signed in yet) is
    /// asked again next time.
    pub(crate) fn finish(&mut self, harness: Harness, outcome: Result<Vec<Model>, ModelsError>) {
        if let Ok(models) = &outcome
            && !models.is_empty()
        {
            self.kept.insert(harness, models.clone());
        }
        for reply in self.waiting.remove(&harness).unwrap_or_default() {
            let _ = reply.send(outcome.clone());
        }
    }
}

/// One listing process, talked to until it lists its models.
pub(crate) struct ModelProbe;

impl ModelProbe {
    /// How long a client may take to list its models: under the UI proxy's
    /// 30 seconds.
    const TIMEOUT: Duration = Duration::from_secs(20);
    const CHUNK: usize = 64 * 1024;

    /// List through `process`, sending `first` before anything else, then
    /// stop it.
    pub(crate) async fn run(
        mut listing: ModelListing,
        first: Vec<Line>,
        process: AgentProcess,
        limit: LineLimit,
    ) -> Result<Vec<Model>, ModelsError> {
        let AgentProcess {
            mut stdin,
            mut stdout,
            exit,
            kill,
        } = process;
        let talk = Self::talk(&mut listing, first, &mut stdin, &mut stdout, limit);
        let outcome = tokio::time::timeout(Self::TIMEOUT, talk)
            .await
            .unwrap_or(Err(ModelsError::TimedOut));
        drop(stdin);
        kill.fire();
        let _ = exit.await;
        outcome
    }

    async fn talk(
        listing: &mut ModelListing,
        first: Vec<Line>,
        stdin: &mut (dyn AsyncWrite + Send + Unpin),
        stdout: &mut (dyn AsyncRead + Send + Unpin),
        limit: LineLimit,
    ) -> Result<Vec<Model>, ModelsError> {
        Self::send(stdin, &first).await?;
        let mut lines = Lines::new(limit);
        let mut buffer = vec![0; Self::CHUNK];
        loop {
            let read = stdout.read(&mut buffer).await.unwrap_or(0);
            let frames: Vec<Framed> = match read {
                0 => std::mem::replace(&mut lines, Lines::new(limit))
                    .finish()
                    .into_iter()
                    .collect(),
                read => lines.push(buffer.get(..read).unwrap_or_default()),
            };
            for frame in frames {
                match listing.receive(frame) {
                    Listing::Waiting(replies) => Self::send(stdin, &replies).await?,
                    Listing::Listed(models) => return Ok(models),
                    Listing::Failed(message) => return Err(ModelsError::Refused(message)),
                }
            }
            if read == 0 {
                return Err(ModelsError::Exited);
            }
        }
    }

    async fn send(
        stdin: &mut (dyn AsyncWrite + Send + Unpin),
        lines: &[Line],
    ) -> Result<(), ModelsError> {
        for line in lines {
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|_| ModelsError::Exited)?;
        }
        stdin.flush().await.map_err(|_| ModelsError::Exited)
    }
}
