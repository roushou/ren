//! `cargo xtask dev`: a development slot on this host, with fake providers
//! and fake data, separate from the installed service.

mod provider;

use clap::{Args, Subcommand};
use provider::{FakeProvider, ProviderError};
use std::net::SocketAddr;

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
}

impl Dev {
    pub fn run(self) -> Result<(), DevError> {
        match self.action {
            DevAction::Provider(args) => Ok(FakeProvider::serve(args.listen)?),
        }
    }
}
