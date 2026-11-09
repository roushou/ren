//! `cargo xtask dev`: a development slot on this host, with fake providers
//! and fake data, separate from the installed service.

mod provider;
mod seed;
mod shot;
mod slot;

use crate::{
    install::{InstallError, TailscaleCli},
    publish::Publish,
};
use clap::{Args, Subcommand};
use provider::{FakeProvider, ProviderError};
use seed::{Scenario, SeedError, Seeder};
use shot::{Camera, Page};
use slot::{Preview, Slot, Unit};
use std::net::SocketAddr;
use std::path::PathBuf;
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
    /// dev daemon, as `tgv-dev-*` systemd user units; with --ui, also the UI,
    /// published privately on the tailnet (ports 9443 and 9444).
    Up(UpArgs),
    /// Stop every `tgv-dev-*` unit and remove the dev preview's tailnet
    /// routes. The slot's data is kept.
    Down(TailnetArgs),
    /// Stop the slot, remove its data, and start it again.
    Reset,
    /// Show the slot's units, addresses, and files.
    Status,
    /// Send fake traffic through the dev gateway: harness sessions, named
    /// users, long prompts, errors, and a Codex WebSocket channel.
    Seed(SeedArgs),
    /// Start or stop a steady trickle of fake calls (one every few
    /// seconds), for pages that follow live traffic.
    Trickle(TrickleArgs),
    /// Screenshot the dev UI's pages with a headless Chromium.
    Shot(ShotArgs),
}

#[derive(Args)]
struct ShotArgs {
    /// The pages to capture; every page when none is named.
    #[arg(value_enum)]
    pages: Vec<Page>,
    #[arg(long, default_value_t = 1600)]
    width: u16,
    #[arg(long, default_value_t = 900)]
    height: u16,
    /// Where to write the PNGs (default: target/dev-shots in the checkout).
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Args)]
struct UpArgs {
    /// Also build and run the UI, and publish the preview.
    #[arg(long)]
    ui: bool,
    /// Keep the preview on this host's loopback; do not touch Tailscale.
    #[arg(long, requires = "ui")]
    local: bool,
    #[command(flatten)]
    tailnet: TailnetArgs,
}

#[derive(Args)]
struct TailnetArgs {
    /// Use `sudo -n` for Tailscale Serve; the default is install-vps's saved
    /// choice.
    #[arg(long)]
    sudo_tailscale: bool,
}

impl TailnetArgs {
    /// How to run `tailscale`: as asked, else as install-vps last did.
    fn cli(&self, env: &HostEnv) -> Result<TailscaleCli, DevError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(InstallError::MissingEnv("HOME"))?;
        let saved = Publish::default().resolve(&Publish::path(&home))?;
        Ok(if self.sudo_tailscale || saved.sudo_tailscale {
            TailscaleCli::Sudo
        } else {
            TailscaleCli::Direct
        })
    }
}

#[derive(Args)]
struct SeedArgs {
    #[arg(value_enum, default_value = "all")]
    scenario: Scenario,
    /// Repeat with new sessions, a call every few seconds, until stopped.
    #[arg(long)]
    forever: bool,
}

#[derive(Args)]
struct TrickleArgs {
    /// Stop the trickle instead of starting it.
    #[arg(long)]
    stop: bool,
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
    #[error(transparent)]
    Seed(#[from] SeedError),
    #[error("no headless Chromium found: set TGV_CHROME, or install chrome-headless-shell")]
    NoChrome,
}

impl Dev {
    pub fn run(self, env: &HostEnv) -> Result<(), DevError> {
        match self.action {
            DevAction::Provider(args) => Ok(FakeProvider::serve(args.listen)?),
            DevAction::Up(args) => {
                let slot = Slot::new(env)?;
                if args.ui {
                    let preview = if args.local {
                        Preview::Local
                    } else {
                        Preview::Tailnet(args.tailnet.cli(env)?)
                    };
                    slot.up_with_ui(env, preview)?;
                } else {
                    slot.up()?;
                }
                slot.status()
            }
            DevAction::Down(args) => {
                Slot::down()?;
                Slot::unpublish(args.cli(env)?)
            }
            DevAction::Reset => {
                let slot = Slot::new(env)?;
                slot.wipe()?;
                slot.up()?;
                slot.status()
            }
            DevAction::Status => Slot::new(env)?.status(),
            DevAction::Seed(args) => {
                let slot = Slot::new(env)?;
                Seeder::new(Slot::DAEMON, Slot::ci_token().expose().to_owned()).play(
                    args.scenario,
                    &slot.data_dir,
                    args.forever,
                )?;
                Ok(())
            }
            DevAction::Shot(args) => {
                let slot = Slot::new(env)?;
                let out = args
                    .out
                    .unwrap_or_else(|| slot.repo.join("target/dev-shots"));
                let pages = if args.pages.is_empty() {
                    Page::ALL.to_vec()
                } else {
                    args.pages
                };
                Camera::new(env, Slot::UI, Slot::DAEMON, out)?
                    .shoot(&pages, (args.width, args.height))?;
                Ok(())
            }
            DevAction::Trickle(args) => {
                let slot = Slot::new(env)?;
                Unit::Trickle.stop()?;
                if !args.stop {
                    slot.trickle()?;
                }
                Ok(())
            }
        }
    }
}
