//! `ren credential`: git's credential helper inside a relayed run. Git
//! runs it with what it needs a credential for; it passes git's ask, with
//! the run's proxy URL (whose ticket says which run asks), to the daemon's
//! grants socket, and writes back what the daemon answers: a token for
//! the run's repository, or nothing, so git goes on without one.

use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use ren_daemon::host::{CredentialAsk, Grants};
use ren_harness::host::HostEnv;
use std::process::ExitCode;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Hand git a run's grant. Set up by the daemon for a relayed run's git;
/// never run by hand.
#[derive(Args)]
pub struct CredentialCmd {
    /// What git asks of its helper.
    operation: Operation,
}

/// Git's helper operations: only `get` asks; a grant is never stored or
/// erased by git.
#[derive(Clone, Copy, ValueEnum)]
enum Operation {
    Get,
    Store,
    Erase,
}

impl CredentialCmd {
    pub async fn run(self, env: &HostEnv) -> Result<ExitCode> {
        match self.operation {
            Operation::Get => {}
            Operation::Store | Operation::Erase => return Ok(ExitCode::SUCCESS),
        }
        let socket = env
            .get(Grants::SOCKET_VARIABLE)
            .context("no grants socket: this run is not granted a repository")?;
        let mut ask = String::new();
        tokio::io::stdin()
            .take(u64::try_from(CredentialAsk::LIMIT).unwrap_or(u64::MAX))
            .read_to_string(&mut ask)
            .await
            .context("git's ask did not read")?;
        let proxy = env.get("HTTPS_PROXY").unwrap_or_default();
        let mut stream = tokio::net::UnixStream::connect(socket)
            .await
            .context("the grants socket did not answer")?;
        stream
            .write_all(format!("proxy={proxy}\n{}\n", ask.trim_end()).as_bytes())
            .await?;
        stream.write_all(b"\n").await?;
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await?;
        tokio::io::stdout().write_all(&answer).await?;
        Ok(ExitCode::SUCCESS)
    }
}
