//! `cargo xtask dev`: a development slot on this host, with fake providers
//! and fake data, separate from the installed service.

mod provider;
mod slot;

use crate::install::InstallError;
use clap::{Args, Subcommand};
use provider::{FakeProvider, ProviderError};
use slot::Slot;
use std::net::SocketAddr;
use tgv_harness::host::HostEnv;

/// A development slot with fake providers and fake data.
#[derive(Args)]
pub struct Dev {
    #[command(subcommand)]
    action: DevAction,
}

#[derive(Subcommand)]
enum DevAction {
    /// Run the fake provider in the foreground: every wire format TGV
    /// forwards, answered with scripted replies. The model name picks the
    /// reply: fake-tools, fake-long, fake-slow, fake-cut, fake-error,
    /// fake-429; anything else gets text quoting the prompt.
    Provider(ProviderArgs),
    /// Build this checkout and (re)start the slot: the fake provider and a
    /// dev daemon, as `tgv-dev-*` systemd user units.
    Up,
    /// Stop every `tgv-dev-*` unit. The slot's data is kept.
    Down,
    /// Stop the slot, remove its data, and start it again.
    Reset,
    /// Show the slot's units, addresses, and files.
    Status,
}

#[derive(Args)]
struct ProviderArgs {
    /// The loopback address to listen on.
    #[arg(long, default_value = "127.0.0.1:18999")]
    listen: SocketAddr,
}

/// Why a dev task failed.
#[derive(Debug, thiserror::Error)]
pub enum DevError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Command(#[from] InstallError),
    #[error("the dev configuration is invalid; this is a bug in `xtask dev`")]
    Config,
    #[error("nothing answered on {0} in time; see `journalctl --user -u 'tgv-dev-*'`")]
    NotReady(SocketAddr),
}

impl Dev {
    pub fn run(self, env: &HostEnv) -> Result<(), DevError> {
        match self.action {
            DevAction::Provider(args) => Ok(FakeProvider::serve(args.listen)?),
            DevAction::Up => {
                let slot = Slot::new(env)?;
                slot.up()?;
                slot.status()
            }
            DevAction::Down => Slot::down(),
            DevAction::Reset => {
                let slot = Slot::new(env)?;
                slot.wipe()?;
                slot.up()?;
                slot.status()
            }
            DevAction::Status => Slot::new(env)?.status(),
        }
    }
}
