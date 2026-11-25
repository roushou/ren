//! `cargo xtask dev`: development instances on this host, with fake
//! providers and fake data, separate from the installed service
//! (docs/plans/instances.md, phase 5).

mod agent;
mod claims;
pub(crate) mod index;
pub(crate) mod instance;
mod provider;
mod seed;
mod traffic;
mod work;

use crate::daemon_container::Contain;
use crate::file::FileError;
use crate::invocation::InvocationError;
use crate::progress::Progress;
use crate::shot::{Camera, Page, ShotError};
use crate::tailscale::{Tailnet, TailscaleError};
use crate::{
    daemon::{RunPolicy, StillBusy},
    install::publish::{Publish, PublishError},
    ui::UiBuildError,
};
use agent::{
    AgentError, FakeAgent, FakeClaudeAuth, FakeCodexAuth, FakeOpenCodeAuth, FakePiAuth, Played,
};
use claims::AllClaimed;
use clap::{Args, Subcommand};
use index::DevIndex;
use instance::{ClaimsFile, DevInstance, Preview, RunWait};
use provider::{FakeProvider, ProviderError};
use ren_harness::host::HostEnv;
use ren_network::tailscale::Cli;
use std::net::SocketAddr;
use std::path::PathBuf;
use traffic::{Pace, SettingsError, Traffic, TrafficError};

/// Development instances with fake providers and fake data.
#[derive(Args)]
pub struct Dev {
    /// The dev instance acted on; by default, the one this checkout
    /// claimed (`dev up` claims one).
    #[arg(long, global = true)]
    instance: Option<DevIndex>,
    #[command(subcommand)]
    action: DevAction,
}

#[derive(Subcommand)]
enum DevAction {
    /// Run the fake provider in the foreground: every wire format Ren
    /// forwards, answered with realistic scripted replies. The `x-dev-reply`
    /// request header picks the kind: tools, long, slow, cut, error,
    /// rate_limited; without it, an ordinary text answer.
    Provider(ProviderArgs),
    /// Build this checkout and (re)start the slot: the fake provider and a
    /// dev daemon, as `ren-dev-*` systemd user units; with --ui, also the UI,
    /// published privately on the tailnet (ports 9443 and 9444).
    Up(UpArgs),
    /// Stop the instance's units and remove its tailnet routes. Its data is
    /// kept; with --release, this checkout gives up its claim on it too.
    Down(DownArgs),
    /// Stop the slot, remove its data, and start it again.
    Reset,
    /// Show the instance's units, addresses, and files.
    Status,
    /// Show each dev instance's index, checkout, and dev UI.
    List,
    /// Send fake traffic through the dev gateway: harness sessions, named
    /// users, long prompts, errors, and a Codex WebSocket channel.
    Seed(SeedArgs),
    /// Show or change the live fake traffic: rate, pace, pause, mix, and
    /// bursts. Changes apply at once, with no restart.
    Traffic(TrafficArgs),
    /// Screenshot the dev UI's pages with a headless Chromium.
    Shot(ShotArgs),
    /// Act as `codex app-server`, `claude -p`, or `pi --mode rpc` on stdio
    /// for the dev daemon's runs (the slot's `codex`, `claude`, and `pi`
    /// run this).
    #[command(hide = true)]
    Agent(AgentArgs),
}

#[derive(Args)]
struct AgentArgs {
    /// The client the agent plays.
    #[arg(long, value_enum, default_value = "codex")]
    client: Played,
    /// Where the agent keeps its state between runs: whether it is signed
    /// out, and opencode's sessions. Never Ren's data, which no run sees.
    #[arg(long)]
    state_dir: PathBuf,
    /// The arguments the daemon gave the client.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    arguments: Vec<String>,
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
    /// How long to wait for the dev daemon's runs to stop working before
    /// giving up, without having changed anything.
    #[arg(long, value_name = "SECONDS", default_value_t = 300)]
    idle_timeout: u64,
    /// Restart even while runs are starting or working: their turns are
    /// lost, and each ends with the daemon (its conversation can be
    /// continued).
    #[arg(long)]
    drain_runs: bool,
    /// Keep the preview on this host's loopback; do not touch Tailscale.
    #[arg(long, requires = "ui")]
    local: bool,
    /// What runs in containers: nothing, the slot's runs, or its daemon
    /// too.
    #[arg(long, value_enum, default_value = "none")]
    contain: Contain,
    /// What runs a contained run's processes: `host_kernel`, or `gvisor`
    /// (`runsc` installed on the host).
    #[arg(long, default_value = "host_kernel")]
    runtime: ren_daemon::config::RunRuntime,
    /// Seed an instance never seeded with the history this number invents
    /// (`ren seed`); one seeded already keeps what it has.
    #[arg(long, value_name = "NUMBER")]
    seed: Option<u64>,
    #[command(flatten)]
    tailnet: TailnetArgs,
}

#[derive(Args)]
struct DownArgs {
    #[command(flatten)]
    tailnet: TailnetArgs,
    /// Also let another checkout claim the instance.
    #[arg(long)]
    release: bool,
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
    fn cli(&self, env: &HostEnv) -> Result<Tailnet, DevError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(DevError::MissingEnv("HOME"))?;
        let saved = Publish::default().resolve(&Publish::path(&home))?;
        Ok(if self.sudo_tailscale || saved.sudo_tailscale {
            Tailnet::new(Cli::Sudo)
        } else {
            Tailnet::new(Cli::Direct)
        })
    }
}

#[derive(Args)]
struct SeedArgs {
    /// The number the history is made from (`ren seed --seed`).
    #[arg(long, default_value_t = 1)]
    seed: u64,
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
    /// Run the generator in the foreground (what an instance's traffic
    /// unit runs).
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
    /// A file `acme/ren` is written in by hand (its `main` and its open
    /// pull requests), read for each of the fake GitHub's answers.
    #[arg(long)]
    github: Option<PathBuf>,
}

/// Why a dev task failed.
#[derive(Debug, thiserror::Error)]
pub enum DevError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    DaemonContainer(#[from] crate::daemon_container::DaemonContainerError),
    #[error(transparent)]
    Gvisor(#[from] crate::gvisor_service::GvisorServiceError),
    #[error("{0} is not set")]
    MissingEnv(&'static str),
    #[error(transparent)]
    SavedOptions(#[from] PublishError),
    #[error(transparent)]
    Ui(#[from] UiBuildError),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    Engine(#[from] ren_container::EngineError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Tailscale(#[from] TailscaleError),
    #[error(transparent)]
    Busy(#[from] StillBusy),
    #[error("the dev configuration is invalid; this is a bug in `xtask dev`")]
    Config,
    #[error("nothing answered on {0} in time; see `cargo xtask dev status` for its logs")]
    NotReady(SocketAddr),
    #[error("{} has no dev instance: `cargo xtask dev up` claims one", .0.display())]
    Unclaimed(PathBuf),
    #[error(transparent)]
    AllClaimed(#[from] AllClaimed),
    #[error("the dev instances' claims in {} do not read: {why}", .path.display())]
    Claims { path: PathBuf, why: String },
    #[error(transparent)]
    Shot(#[from] ShotError),
    #[error(transparent)]
    Traffic(#[from] TrafficError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Agent(#[from] AgentError),
    #[error("could not make a fake project a git repository: {0}")]
    Git(#[from] ren_git::GitError),
}

impl Dev {
    pub fn run(self, env: &HostEnv) -> Result<(), DevError> {
        let instance = self.instance;
        match self.action {
            DevAction::Provider(args) => Ok(FakeProvider::serve(
                args.listen,
                args.settings,
                args.github,
            )?),
            DevAction::Agent(args) => match args.client {
                Played::Claude if FakeClaudeAuth::asked(&args.arguments) => {
                    Ok(FakeClaudeAuth::run(&args.state_dir, &args.arguments)?)
                }
                Played::Codex if FakeCodexAuth::asked(&args.arguments) => {
                    Ok(FakeCodexAuth::run(&args.state_dir, &args.arguments)?)
                }
                Played::Pi if FakePiAuth::asked(&args.arguments) => {
                    Ok(FakePiAuth::run(&args.state_dir, &args.arguments)?)
                }
                Played::OpenCode if FakeOpenCodeAuth::asked(&args.arguments) => {
                    Ok(FakeOpenCodeAuth::run(&args.state_dir, &args.arguments)?)
                }
                Played::Codex | Played::Claude | Played::Pi | Played::OpenCode => Ok(
                    FakeAgent::new(args.client, env, &args.state_dir, &args.arguments)?.run()?,
                ),
            },
            DevAction::Up(args) => {
                let slot = DevInstance::new(env, Self::index(instance, env, Claim::Take)?)?
                    .containing(args.contain)
                    .running_on(args.runtime);
                let wait = RunWait {
                    policy: if args.drain_runs {
                        RunPolicy::Drain
                    } else {
                        RunPolicy::Wait
                    },
                    timeout: args.idle_timeout,
                };
                if args.ui {
                    let preview = if args.local {
                        Preview::Local
                    } else {
                        Preview::Tailnet(args.tailnet.cli(env)?)
                    };
                    slot.up_with_ui(env, preview, wait)?;
                } else {
                    slot.up(wait)?;
                }
                if let Some(seed) = args.seed {
                    if slot.seeded() {
                        Progress("Seeded already: the instance keeps its data").show();
                    } else {
                        slot.seed(seed)?;
                    }
                }
                slot.status()
            }
            DevAction::Down(args) => {
                let slot = DevInstance::new(env, Self::index(instance, env, Claim::Held)?)?;
                slot.down()?;
                slot.unpublish(args.tailnet.cli(env)?)?;
                if args.release {
                    let checkout = DevInstance::checkout();
                    ClaimsFile::new(env)?.change(|claims| Ok((claims.release(&checkout), ())))?;
                    Progress(&format!(
                        "{} gave up {}",
                        checkout.display(),
                        slot.index.name()
                    ))
                    .show();
                }
                Ok(())
            }
            DevAction::Reset => {
                let slot = DevInstance::new(env, Self::index(instance, env, Claim::Take)?)?;
                slot.wipe()?;
                // The wipe stopped the daemon: no run is left to wait for.
                slot.up(RunWait {
                    policy: RunPolicy::Drain,
                    timeout: 0,
                })?;
                slot.status()
            }
            DevAction::Status => {
                DevInstance::new(env, Self::index(instance, env, Claim::Held)?)?.status()
            }
            DevAction::List => {
                for (index, checkout) in ClaimsFile::new(env)?.read()?.all() {
                    println!(
                        "{:<10} http://{:<16} {}",
                        index.name(),
                        index.ui(),
                        checkout.display()
                    );
                }
                Ok(())
            }
            DevAction::Seed(args) => {
                DevInstance::new(env, Self::index(instance, env, Claim::Held)?)?.seed(args.seed)
            }
            DevAction::Shot(args) => {
                let slot = DevInstance::new(env, Self::index(instance, env, Claim::Held)?)?;
                let out = args
                    .out
                    .unwrap_or_else(|| slot.repo.join("target/dev-shots"));
                let pages = if args.pages.is_empty() {
                    Page::ALL.to_vec()
                } else {
                    args.pages
                };
                Camera::new(env, slot.index.ui(), slot.index.daemon(), out)?
                    .shoot(&pages, (args.width, args.height))?;
                Ok(())
            }
            DevAction::Traffic(args) => {
                let slot = DevInstance::new(env, Self::index(instance, env, Claim::Held)?)?;
                let traffic = Traffic::new(
                    slot.index.daemon(),
                    DevInstance::ci_token().expose().to_owned(),
                    slot.traffic_settings(),
                    slot.data_dir.clone(),
                );
                Self::traffic(&traffic, args.action.unwrap_or(TrafficAction::Show))
            }
        }
    }
}

/// What a command needs of this checkout's claim.
#[derive(Debug, Clone, Copy)]
enum Claim {
    /// It starts the instance: one is claimed if none is held.
    Take,
    /// It acts on the instance already held.
    Held,
}

impl Dev {
    /// The instance acted on: the one asked for, else this checkout's.
    fn index(
        instance: Option<DevIndex>,
        env: &HostEnv,
        claim: Claim,
    ) -> Result<DevIndex, DevError> {
        if let Some(index) = instance {
            return Ok(index);
        }
        let checkout = DevInstance::checkout();
        let claims = ClaimsFile::new(env)?;
        match claim {
            Claim::Take => {
                claims.change(|claims| Ok(claims.claim(&checkout, |held| held.exists())?))
            }
            Claim::Held => claims
                .read()?
                .of(&checkout)
                .ok_or(DevError::Unclaimed(checkout)),
        }
    }

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
