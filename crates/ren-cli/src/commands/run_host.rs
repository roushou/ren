use anyhow::{Context as _, Result};
use clap::Args;
use ren_daemon::host::RunHost;
use std::{path::PathBuf, process::ExitCode};

/// Hold one run's client process, from the run's folder, so it outlives the
/// daemon that started it. Started by the daemon, never by hand.
#[derive(Args)]
pub struct RunHostCmd {
    /// The run's folder under the data directory's `runs`.
    folder: PathBuf,
}

impl RunHostCmd {
    pub async fn run(self) -> Result<ExitCode> {
        RunHost::new(self.folder)
            .serve()
            .await
            .context("the run host stopped")?;
        Ok(ExitCode::SUCCESS)
    }
}
