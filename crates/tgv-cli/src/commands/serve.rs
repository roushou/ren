use crate::context::Context;
use anyhow::{Context as _, Result};
use clap::Args;
use std::{io, process::ExitCode};
use tgv_daemon::Daemon;

/// Run native HTTP/SSE forwarding and recording.
#[derive(Args)]
pub struct ServeCmd {}
impl ServeCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ctx.config()?;
        let data_dir = ctx.data_dir(&config)?;
        let mut daemon = Daemon::new(
            config.resolve_with(&ctx.env)?,
            data_dir,
            crate::Cli::build(),
        );
        // Register before starting: a daemon that cannot observe shutdown
        // signals must not run, and must not report success.
        let mut signal =
            ShutdownSignal::register().context("could not register shutdown signal handlers")?;
        daemon.start().await?;
        tracing::info!(address = %daemon.address()?, "TGV listening");
        tokio::select! {
            result = daemon.wait() => result?,
            received = signal.received() => {
                daemon.shutdown();
                daemon.wait().await?;
                received.context("shutdown signal handler failed")?;
            }
        }
        Ok(ExitCode::SUCCESS)
    }
}

/// Process shutdown signals, registered eagerly so registration failures surface
/// before the daemon starts rather than as an immediate, successful shutdown.
struct ShutdownSignal {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}

impl ShutdownSignal {
    fn register() -> io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
            })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }

    /// Resolves when shutdown is requested. An error means the handler failed
    /// after registration; the caller still shuts down, then reports it.
    async fn received(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.interrupt.recv() => {},
                _ = self.terminate.recv() => {},
            }
            Ok(())
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await
    }
}
