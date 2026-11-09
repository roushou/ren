//! `cargo xtask dev`: a development slot on this host, with fake providers
//! and fake data, separate from the installed service.

mod provider;
mod seed;
mod shot;
mod slot;
mod traffic;

use crate::{
    install::{InstallError, TailscaleCli},
    publish::Publish,
};
use clap::{Args, Subcommand};
use provider::{FakeProvider, ProviderError};
use seed::{Scenario, SeedError, Seeder};
use shot::{Camera, Page};
use slot::{Preview, Slot};
use std::net::SocketAddr;
use std::path::PathBuf;
use tgv_harness::host::HostEnv;
use traffic::{Pace, SettingsError, Traffic, TrafficError};

/// A development slot with fake providers and fake data.
#[derive(Args)]
pub struct Dev {
    #[command(subcommand)]
    action: DevAction,
}

#[derive(Subcommand)]
enum DevAction {
    /// Run the fake provider in the foreground: every wire format TGV
    /// forwards, answered with realistic scripted replies. The `x-dev-reply`
    /// request header picks the kind: tools, long, slow, cut, error,
    /// rate_limited; without it, an ordinary text answer.
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
    /// Show or change the live fake traffic: rate, pace, pause, mix, and
    /// bursts. Changes apply at once, with no restart.
    Traffic(TrafficArgs),
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
}

#[derive(Args)]
struct TrafficArgs {
    #[command(subcommand)]
    action: Option<TrafficAction>,
}

#[derive(Subcommand)]
enum TrafficAction {
    /// Show the current settings (the default).
    Show,
    /// Change some settings; the others keep their values.
    Set(SetArgs),
    /// Stop sending calls.
    Pause,
    /// Send calls again.
    Resume,
    /// Send this many calls now, with the current mix.
    Burst {
        #[arg(default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=500))]
        count: u16,
    },
    /// Run the generator in the foreground (what the tgv-dev-traffic unit
    /// runs).
    #[command(hide = true)]
    Run {
        /// Also serve the control API (for the dev UI's toolbar) on this
        /// loopback address.
        #[arg(long)]
        control: Option<SocketAddr>,
    },
}

#[derive(Args)]
struct SetArgs {
    /// Calls a minute (0 to 600; 0 sends none).
    #[arg(long)]
    rate: Option<u16>,
    /// How fast the fake provider streams replies.
    #[arg(long, value_enum)]
    pace: Option<Pace>,
    /// Reply weights: text, tools, long, slow, cut, error, rate_limited
    /// (for example `tools=50,error=5`).
    #[arg(long)]
    replies: Option<String>,
    /// Harness weights: claude_code, codex, pi, opencode, deepseek.
    #[arg(long)]
    harnesses: Option<String>,
    /// Caller weights: alice, ci, unlisted, unidentified.
    #[arg(long)]
    senders: Option<String>,
}

#[derive(Args)]
struct ProviderArgs {
    /// The loopback address to listen on.
    #[arg(long, default_value = "127.0.0.1:18999")]
    listen: SocketAddr,
    /// A traffic settings file whose pace the replies follow.
    #[arg(long)]
    settings: Option<PathBuf>,
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
    #[error(transparent)]
    Traffic(#[from] TrafficError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

impl Dev {
    pub fn run(self, env: &HostEnv) -> Result<(), DevError> {
        match self.action {
            DevAction::Provider(args) => Ok(FakeProvider::serve(args.listen, args.settings)?),
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
                Seeder::new(Slot::DAEMON, Slot::ci_token().expose().to_owned())
                    .play(args.scenario, &slot.data_dir)?;
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
            DevAction::Traffic(args) => {
                let slot = Slot::new(env)?;
                let traffic = Traffic::new(
                    Slot::DAEMON,
                    Slot::ci_token().expose().to_owned(),
                    slot.traffic_settings(),
                    slot.data_dir.clone(),
                );
                Self::traffic(&traffic, args.action.unwrap_or(TrafficAction::Show))
            }
        }
    }
}

impl Dev {
    fn traffic(traffic: &Traffic, action: TrafficAction) -> Result<(), DevError> {
        let mut settings = traffic.settings()?;
        match action {
            TrafficAction::Run { control } => return Ok(traffic.run(control)?),
            TrafficAction::Burst { count } => return Ok(traffic.burst(count)?),
            TrafficAction::Show => {}
            TrafficAction::Pause => settings.flow = traffic::Flow::Paused,
            TrafficAction::Resume => settings.flow = traffic::Flow::Running,
            TrafficAction::Set(set) => {
                if let Some(rate) = set.rate {
                    settings.rate = rate;
                }
                if let Some(pace) = set.pace {
                    settings.pace = pace;
                }
                if let Some(text) = set.replies {
                    settings.replies = settings.replies.with(&text)?;
                }
                if let Some(text) = set.harnesses {
                    settings.harnesses = settings.harnesses.with(&text)?;
                }
                if let Some(text) = set.senders {
                    settings.senders = settings.senders.with(&text)?;
                }
                settings.check()?;
            }
        }
        traffic.save(&settings)?;
        print!("# {}\n{}", traffic.path().display(), settings.to_toml());
        Ok(())
    }
}
