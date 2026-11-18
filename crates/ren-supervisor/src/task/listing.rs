//! Listing what a client offers: its models, or its commands in a folder.
//! The client is started briefly in its structured mode with no
//! conversation kept, asked, and stopped. The task runs one listing per key
//! at a time, off its loop, and keeps the answers it is told to keep.

use crate::host::AgentProcess;
use futures_util::{FutureExt, future::BoxFuture, stream::FuturesUnordered};
use ren_harness::lines::{Framed, Line, LineLimit, Lines};
use ren_harness::models::Listing;
use std::{collections::HashMap, hash::Hash, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::oneshot,
};

/// Why a client could not list what it offers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ListingError {
    #[error("no run root to list in")]
    NoRoot,
    #[error("cannot start the client; check it is installed on PATH")]
    Spawn,
    #[error("the client refused to list: {0}")]
    Refused(String),
    #[error("the client exited before it listed")]
    Exited,
    #[error("the client did not list in time")]
    TimedOut,
    #[error("the supervisor has stopped")]
    Gone,
}

/// Where a listing's answer goes: its error is the listing's, or one that
/// says more of what was asked (`E`).
pub(crate) type ListingReply<V, E = ListingError> = oneshot::Sender<Result<V, E>>;
type Running<K, V> = FuturesUnordered<BoxFuture<'static, (K, Result<V, ListingError>)>>;

/// The task's listings of one kind, by key: answers kept, and those still
/// running with who waits on them.
pub(crate) struct Listings<K, V, E = ListingError> {
    kept: HashMap<K, V>,
    waiting: HashMap<K, Vec<ListingReply<V, E>>>,
    pub(crate) running: Running<K, V>,
}

impl<K, V, E> Default for Listings<K, V, E> {
    fn default() -> Self {
        Self {
            kept: HashMap::new(),
            waiting: HashMap::new(),
            running: FuturesUnordered::new(),
        }
    }
}

impl<K, V, E> Listings<K, V, E>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + 'static,
    E: From<ListingError>,
{
    /// Answer from what is kept, or join a listing already running. Gives
    /// the reply back when a listing has to start.
    pub(crate) fn ask(&mut self, key: &K, reply: ListingReply<V, E>) -> Option<ListingReply<V, E>> {
        if let Some(kept) = self.kept.get(key) {
            let _ = reply.send(Ok(kept.clone()));
            return None;
        }
        match self.waiting.get_mut(key) {
            Some(waiting) => {
                waiting.push(reply);
                None
            }
            None => Some(reply),
        }
    }

    /// Run `listing` for `key`, answering `reply` and whoever asks
    /// meanwhile when it ends.
    pub(crate) fn start(
        &mut self,
        key: K,
        reply: ListingReply<V, E>,
        listing: BoxFuture<'static, Result<V, ListingError>>,
    ) {
        self.waiting.insert(key.clone(), vec![reply]);
        self.running
            .push(listing.map(move |outcome| (key, outcome)).boxed());
    }

    /// Run `listing` for `key` with no one waiting yet; whoever asks
    /// meanwhile joins it.
    pub(crate) fn warm(&mut self, key: K, listing: BoxFuture<'static, Result<V, ListingError>>) {
        self.waiting.insert(key.clone(), Vec::new());
        self.running
            .push(listing.map(move |outcome| (key, outcome)).boxed());
    }

    /// A listing ended: its answer is kept when `keep` says so, and every
    /// one waiting on it gets it.
    pub(crate) fn finish(
        &mut self,
        key: K,
        outcome: Result<V, ListingError>,
        keep: impl Fn(&V) -> bool,
    ) {
        if let Ok(listed) = &outcome
            && keep(listed)
        {
            self.kept.insert(key.clone(), listed.clone());
        }
        for reply in self.waiting.remove(&key).unwrap_or_default() {
            let _ = reply.send(outcome.clone().map_err(E::from));
        }
    }

    /// What a running client listed itself, kept as if listed.
    pub(crate) fn keep(&mut self, key: K, listed: V) {
        self.kept.insert(key, listed);
    }
}

/// What a client offers in a folder is listed once per installed client
/// and folder: a new version of the client is a new key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CommandKey {
    pub(crate) harness: ren_harness::Harness,
    pub(crate) stamp: crate::host::ClientStamp,
    pub(crate) folder: std::path::PathBuf,
}

/// One listing process, talked to until it lists.
pub(crate) struct ListingProbe;

impl ListingProbe {
    /// How long a client may take to list: under the UI proxy's 30
    /// seconds.
    const TIMEOUT: Duration = Duration::from_secs(20);
    const CHUNK: usize = 64 * 1024;

    /// List through `process`, sending `first` before anything else, then
    /// stop it.
    pub(crate) async fn run<T>(
        mut listing: impl FnMut(Framed) -> Listing<T> + Send,
        first: Vec<Line>,
        process: AgentProcess,
        limit: LineLimit,
    ) -> Result<T, ListingError> {
        let AgentProcess {
            mut stdin,
            mut stdout,
            exit,
            kill,
        } = process;
        let talk = Self::talk(&mut listing, first, &mut stdin, &mut stdout, limit);
        let outcome = tokio::time::timeout(Self::TIMEOUT, talk)
            .await
            .unwrap_or(Err(ListingError::TimedOut));
        drop(stdin);
        kill.fire();
        let _ = exit.await;
        outcome
    }

    async fn talk<T>(
        listing: &mut (impl FnMut(Framed) -> Listing<T> + Send),
        first: Vec<Line>,
        stdin: &mut (dyn AsyncWrite + Send + Unpin),
        stdout: &mut (dyn AsyncRead + Send + Unpin),
        limit: LineLimit,
    ) -> Result<T, ListingError> {
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
                match listing(frame) {
                    Listing::Waiting(replies) => Self::send(stdin, &replies).await?,
                    Listing::Listed(listed) => return Ok(listed),
                    Listing::Failed(message) => return Err(ListingError::Refused(message)),
                }
            }
            if read == 0 {
                return Err(ListingError::Exited);
            }
        }
    }

    async fn send(
        stdin: &mut (dyn AsyncWrite + Send + Unpin),
        lines: &[Line],
    ) -> Result<(), ListingError> {
        for line in lines {
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|_| ListingError::Exited)?;
        }
        stdin.flush().await.map_err(|_| ListingError::Exited)
    }
}
