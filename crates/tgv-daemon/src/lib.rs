//! The TGV daemon: configuration, resource wiring, and service lifecycle,
//! with [`Daemon`] as its primary type. The gateway owns all HTTP handling.
//!
//! The daemon assembles gateway services and coordinates shutdown: drain HTTP,
//! abort what remains at the deadline, then drain capture before releasing
//! storage.

pub mod config;
pub mod host;

use crate::config::RuntimeConfig;
use crate::host::{RandomIds, SystemClock};
use std::{io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tgv_gateway::{
    Build, ControlToken, Gateway, ManagementSetup, ReqwestUpstream, Shutdown, Stopped,
    control::Controls,
    operator::{OperatorState, Pause, RestoreError},
};
use tgv_gateway::{IngressError, UpstreamConfigError};
use tgv_recorder::CaptureOptionsError;
use tgv_recorder::{Recorder, RecorderSetup, id::IdSource, time::Clock};
use tgv_store::StoreError;
use tgv_store::{
    Reader, Store,
    backup::Backup,
    search::{Indexer, SearchIndex},
};
use tokio::{net::TcpListener, task::JoinHandle};

/// Why the daemon could not start, run, or stop cleanly.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("daemon is already started; wait for termination before starting again")]
    AlreadyStarted,
    #[error("daemon has not started")]
    NotStarted,
    #[error("daemon has no active run to wait for")]
    NotRunning,
    #[error(transparent)]
    Upstream(#[from] UpstreamConfigError),
    #[error("cannot bind gateway listener")]
    Bind(#[source] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    OperatorState(#[from] RestoreError),
    #[error(transparent)]
    Capture(#[from] CaptureOptionsError),
    #[error("cannot write the control token")]
    ControlToken(#[source] io::Error),
    #[error("HTTP server failed")]
    Http(#[from] IngressError),
    #[error("capture persistence failed")]
    Persistence(#[source] StoreError),
    #[error("capture shutdown timed out; unfinished records will be recovered on restart")]
    CaptureTimeout,
    #[error("{0} task failed")]
    Task(&'static str, #[source] tokio::task::JoinError),
}

type Result<T, E = DaemonError> = std::result::Result<T, E>;

/// Owns configuration and the complete service lifecycle. Construction performs
/// no I/O; start, shutdown, and wait operate on this same instance.
pub struct Daemon {
    config: RuntimeConfig,
    data_dir: PathBuf,
    build: Build,
    lifecycle: Lifecycle,
}

/// Active means the daemon still owns an unjoined task, even after a shutdown
/// request. Only observing task termination transitions it to Stopped.
enum Lifecycle {
    Ready,
    Active {
        address: SocketAddr,
        shutdown: Shutdown,
        task: JoinHandle<Result<()>>,
    },
    Stopped {
        address: SocketAddr,
    },
}

impl Daemon {
    /// How long in-flight calls may continue after shutdown is requested.
    const DRAIN_DEADLINE: Duration = Duration::from_secs(10);
    const CAPTURE_DEADLINE: Duration = Duration::from_secs(10);
    /// The indexer's pause while it is backfilling, and once caught up.
    const INDEX_BUSY_PAUSE: Duration = Duration::from_millis(200);
    const INDEX_IDLE_PAUSE: Duration = Duration::from_secs(5);

    /// `build` is what the binary running this daemon was compiled from;
    /// `/health` reports it.
    pub fn new(config: RuntimeConfig, data_dir: impl Into<PathBuf>, build: Build) -> Self {
        Self {
            config,
            data_dir: data_dir.into(),
            build,
            lifecycle: Lifecycle::Ready,
        }
    }

    /// Write this run's control token where the CLI reads it, readable by
    /// this user only.
    fn write_token(data_dir: &std::path::Path, token: &ControlToken) -> Result<()> {
        use std::io::Write;
        let path = data_dir.join(ControlToken::FILE);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .and_then(|mut file| file.write_all(token.expose().as_bytes()))
            .map_err(DaemonError::ControlToken)
    }

    /// Start services. An active run must be shut down and joined before this
    /// instance can start again. Failed startup leaves it available for retry.
    pub async fn start(&mut self) -> Result<()> {
        if matches!(self.lifecycle, Lifecycle::Active { .. }) {
            return Err(DaemonError::AlreadyStarted);
        }
        let config = &self.config;
        let data_dir = &self.data_dir;
        let upstream = ReqwestUpstream::new(config.upstreams().clone(), config.transport())?;
        let listener = TcpListener::bind(config.listen())
            .await
            .map_err(DaemonError::Bind)?;
        let address = listener.local_addr().map_err(DaemonError::Bind)?;
        // The host's clock and randomness are read here, in the shell, and
        // handed to components as capabilities.
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::start());
        let ids: Arc<dyn IdSource> = Arc::new(RandomIds);
        let (store, recovery) = Store::open(data_dir, clock.as_ref(), ids.clone()).await?;
        let operator = match &recovery.operator {
            None => OperatorState::default(),
            Some(stored) => OperatorState::restore(stored.revision, &stored.document)?,
        };
        if operator.revision().get() > 0 {
            let paused = match operator.pause() {
                Pause::Running => "running",
                Pause::Paused { .. } => "paused",
            };
            tracing::warn!(
                revision = operator.revision().get(),
                pause = paused,
                blocked_sessions = operator.blocked_sessions(clock.now().wall),
                "restored operator state from the previous run"
            );
        }
        let backup = Backup::new(data_dir, store.sweep_gate());
        let (recorder, task) = Recorder::new(
            store,
            recovery.calls,
            RecorderSetup {
                options: config.capture().clone(),
                retention: config.retention().clone(),
                // A call's lifecycle slots return when its finish record is
                // dequeued, after admission is released: leave headroom.
                max_calls: config.recorder_calls(),
                live_buffer: 1024,
            },
            clock.clone(),
            ids.clone(),
        )?;
        #[expect(
            clippy::disallowed_methods,
            reason = "process wiring: the daemon owns the recorder task"
        )]
        let mut worker = tokio::spawn(task.run());
        let reader = match Reader::open(data_dir, 4).await {
            Ok(reader) => reader,
            Err(error) => {
                // No requests have been admitted. Release startup resources
                // before reporting failure so this instance can retry safely.
                drop(recorder);
                let _ = worker.await;
                return Err(error.into());
            }
        };
        // Search is derived data: the indexer has its own read connection
        // and the only writer of `search.sqlite3`, off the forwarding path.
        let search = SearchIndex::open(data_dir).await?;
        let mut indexer = Indexer::new(search.clone(), Reader::open(data_dir, 1).await?)
            .map_err(StoreError::from)?;
        #[expect(
            clippy::disallowed_methods,
            reason = "process wiring: the daemon owns the search indexer task"
        )]
        let indexing = tokio::spawn(async move {
            loop {
                let pause = match indexer.pass().await {
                    Ok(report) if report.more => Self::INDEX_BUSY_PAUSE,
                    Ok(_) => Self::INDEX_IDLE_PAUSE,
                    Err(error) => {
                        tracing::warn!(%error, "search indexing failed; retrying");
                        Self::INDEX_IDLE_PAUSE
                    }
                };
                tokio::time::sleep(pause).await;
            }
        });
        let shutdown = Shutdown::default();
        let controls = Arc::new(Controls::restore(operator));
        let token = ControlToken::generate(ids.as_ref());
        Self::write_token(data_dir, &token)?;
        let gateway = Gateway::new(
            config.gateway(),
            upstream,
            recorder,
            controls,
            clock,
            shutdown.clone(),
        );
        let ingress = gateway.into_ingress(
            listener,
            ManagementSetup {
                reader,
                search,
                backup,
                retention: config.retention().clone(),
                control: config.control().clone(),
                token,
                build: self.build.clone(),
            },
        );
        let stop = shutdown.clone();
        #[expect(
            clippy::disallowed_methods,
            reason = "process wiring: the daemon owns its serving task"
        )]
        let task = tokio::spawn(async move {
            let serve_result = ingress.serve(stop.clone(), Self::DRAIN_DEADLINE).await;
            stop.abort();
            // Indexing is resumable: a pass cut short rolls back and is
            // repeated on the next start. Wait for it to stop, so it no
            // longer reads the store while capture drains and the store closes.
            indexing.abort();
            let _ = indexing.await;
            match &serve_result {
                Ok(Stopped::Drained) => {}
                Ok(Stopped::Aborted) => {
                    tracing::warn!("shutdown deadline passed; calls still running were cancelled")
                }
                Err(error) => tracing::error!(%error, "HTTP shutdown did not complete"),
            }
            // The server dropped every request owner, so the recorder's senders
            // are closing. Drain accepted capture before releasing the lock.
            let capture_result =
                match tokio::time::timeout(Self::CAPTURE_DEADLINE, &mut worker).await {
                    Ok(result) => result
                        .map_err(|error| DaemonError::Task("capture", error))?
                        .map_err(DaemonError::Persistence),
                    Err(_) => {
                        worker.abort();
                        let _ = worker.await;
                        Err(DaemonError::CaptureTimeout)
                    }
                };
            let _logged: Stopped = serve_result?;
            capture_result
        });
        self.lifecycle = Lifecycle::Active {
            address,
            shutdown,
            task,
        };
        Ok(())
    }

    /// Address bound by the current or most recently completed run.
    pub fn address(&self) -> Result<SocketAddr> {
        match &self.lifecycle {
            Lifecycle::Ready => Err(DaemonError::NotStarted),
            Lifecycle::Active { address, .. } | Lifecycle::Stopped { address } => Ok(*address),
        }
    }

    /// Request shutdown without claiming that HTTP and capture have finished:
    /// new calls are refused and running calls get the drain deadline.
    /// Repeated requests, including before startup or after termination, are harmless.
    pub fn shutdown(&self) {
        if let Lifecycle::Active { shutdown, .. } = &self.lifecycle {
            shutdown.drain();
        }
    }

    /// Observe termination once per run. Cancelling this wait leaves task
    /// ownership in the daemon, so a later wait can still observe its result.
    pub async fn wait(&mut self) -> Result<()> {
        let Lifecycle::Active { address, task, .. } = &mut self.lifecycle else {
            return Err(DaemonError::NotRunning);
        };
        let address = *address;
        let result = task.await;
        self.lifecycle = Lifecycle::Stopped { address };
        result.map_err(|error| DaemonError::Task("daemon", error))?
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.shutdown();
    }
}
