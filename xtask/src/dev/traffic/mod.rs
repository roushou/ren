//! Live fake traffic for the dev slot: a generator that sends calls through
//! the dev gateway at the rate, and with the mix, the settings file names,
//! reading it again before every call. The calls themselves come from the
//! seed scenarios' builders, so they carry each harness's real headers.

mod control;
mod settings;

pub use settings::{Flow, Pace, Settings, SettingsError};

use super::seed::{Call, Caller, Client, PROMPTS, Run, Scenario, Seeder, Side};
use control::{Control, Stats};
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use settings::{Harness, Reply, Sender};
use std::{
    future::Future,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// Why the generator stopped.
#[derive(Debug, thiserror::Error)]
pub enum TrafficError {
    #[error("cannot start the generator's runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("cannot build the HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error(transparent)]
    Seed(#[from] super::seed::SeedError),
    #[error("the traffic control API cannot serve on {address}: {source}")]
    Control {
        address: SocketAddr,
        source: std::io::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

/// The generator, for one gateway and one settings file.
#[derive(Clone)]
pub struct Traffic {
    gateway: SocketAddr,
    pub(super) token: String,
    settings: PathBuf,
    data_dir: PathBuf,
}

type Sending = Pin<Box<dyn Future<Output = ()> + Send>>;

impl Traffic {
    /// How often a paused generator looks at the settings again.
    const IDLE: Duration = Duration::from_secs(1);
    /// Calls the generator keeps in flight at most; past it, it waits.
    const MAX_IN_FLIGHT: usize = 32;
    /// How long a tool runs between a turn's calls.
    const TOOL_TIME: Duration = Duration::from_millis(1_500);

    pub fn new(gateway: SocketAddr, token: String, settings: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            gateway,
            token,
            settings,
            data_dir,
        }
    }

    /// The settings file, written with the defaults if it is missing.
    pub fn settings(&self) -> Result<Settings, TrafficError> {
        if !self.settings.exists() {
            self.save(&Settings::default())?;
        }
        Ok(Settings::read(&self.settings))
    }

    pub fn save(&self, settings: &Settings) -> Result<(), TrafficError> {
        std::fs::write(&self.settings, settings.to_toml()).map_err(|source| TrafficError::Write {
            path: self.settings.display().to_string(),
            source,
        })
    }

    /// The `n`th generated turn of `run`, its calls in the order they are
    /// sent, each after the one before has finished; `None` when a mix has
    /// every weight at 0. A turn that asks for tools calls one to three of
    /// them, each answered in the next call, then ends with a reply.
    pub fn turn(n: u64, run: Run, settings: &Settings, token: &str) -> Option<Vec<Call>> {
        let harness = settings.harnesses.pick(n)?;
        let sender = settings.senders.pick(n.wrapping_add(0x5bd1))?;
        let reply = settings.replies.pick(n.wrapping_add(0x2f71))?;
        let (client, model) = match harness {
            Harness::ClaudeCode => (Client::ClaudeCode, "claude-opus-5-5"),
            Harness::Codex => (Client::Codex, "gpt-5.5-codex"),
            Harness::Pi => (Client::Pi, "claude-sonnet-5-5"),
            Harness::Opencode => (Client::OpenCode, "gpt-5.5"),
            Harness::Deepseek => (Client::DeepSeek, "deepseek-chat"),
        };
        let ask = match reply {
            Reply::Text => None,
            Reply::Tools => Some("tools"),
            Reply::Long => Some("long"),
            Reply::Slow => Some("slow"),
            Reply::Cut => Some("cut"),
            Reply::Error => Some("error"),
            Reply::RateLimited => Some("rate_limited"),
        };
        let caller = match sender {
            Sender::Alice => Caller::Login("alice@example.com"),
            Sender::Ci => Caller::Token(token.to_owned()),
            Sender::Unlisted => Caller::Login("carol@example.com"),
            Sender::Unidentified => Caller::Unidentified,
        };
        let index = Harness::index(harness);
        // Each harness keeps a session for about eight of its calls.
        let session = 5000 + u32::try_from(n / 40).unwrap_or(0) * 8 + index;
        let turn = 100_000 + u32::try_from(n % 100_000).unwrap_or(0);
        let prompt = PROMPTS[usize::try_from(n).unwrap_or(0) % PROMPTS.len()];
        // About one Claude Code call in ten is a side call with no turn, as
        // in real use: a title, or a quota probe.
        let side = match (harness, n % 20) {
            (Harness::ClaudeCode, 7) => Some(Side::Title),
            (Harness::ClaudeCode, 15) => Some(Side::QuotaProbe),
            (
                Harness::ClaudeCode
                | Harness::Codex
                | Harness::Pi
                | Harness::Opencode
                | Harness::Deepseek,
                _,
            ) => None,
        };
        if let Some(side) = side {
            return Some(vec![Scenario::side_call(
                side, &caller, run, session, prompt,
            )]);
        }
        let ids = (session, turn);
        let step = |done| Scenario::step_call(client, &caller, run, ids, model, prompt, done);
        Some(match ask {
            Some("tools") => {
                let tools = usize::try_from(1 + (n / 7) % 3).unwrap_or(1);
                (0..tools)
                    .map(|done| step(done).asking("tools"))
                    .chain([step(tools)])
                    .collect()
            }
            Some(reply) => vec![step(0).asking(reply)],
            None => vec![step(0)],
        })
    }

    fn runtime() -> Result<tokio::runtime::Runtime, TrafficError> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(TrafficError::Runtime)
    }

    fn client() -> Result<reqwest::Client, TrafficError> {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(TrafficError::Client)
    }

    /// One call, read to its end, as a future the caller polls.
    fn send(client: &reqwest::Client, gateway: SocketAddr, call: Call) -> Sending {
        let mut request = client
            .post(format!("http://{gateway}{}", call.path))
            .body(call.body);
        for (name, value) in call.headers {
            request = request.header(name, value);
        }
        Box::pin(async move {
            let status = match request.send().await {
                Ok(response) => {
                    let status = response.status().as_u16().to_string();
                    drop(response.bytes().await);
                    status
                }
                Err(_) => "unreachable".to_owned(),
            };
            println!("{status} {}", call.label);
        })
    }

    /// Send calls as the settings say, until stopped, keeping several in
    /// flight so a slow call never holds up the next.
    pub fn run(&self, control: Option<SocketAddr>) -> Result<(), TrafficError> {
        let client = Self::client()?;
        let run = Run(Seeder::next_run(&self.data_dir)?);
        let stats = Arc::new(Stats::default());
        Self::runtime()?.block_on(async {
            let page = async {
                let Some(address) = control else {
                    return std::future::pending().await;
                };
                let listener = tokio::net::TcpListener::bind(address)
                    .await
                    .map_err(|source| TrafficError::Control { address, source })?;
                let router = Control {
                    traffic: self.clone(),
                    stats: stats.clone(),
                    client: client.clone(),
                    run,
                }
                .router();
                axum::serve(listener, router)
                    .await
                    .map_err(|source| TrafficError::Control { address, source })
            };
            tokio::select! {
                stopped = page => stopped,
                stopped = self.generate(&client, run, &stats) => stopped,
            }
        })
    }

    /// Send calls as the settings say, until stopped, keeping several in
    /// flight so a slow call never holds up the next.
    async fn generate(
        &self,
        client: &reqwest::Client,
        run: Run,
        stats: &Arc<Stats>,
    ) -> Result<(), TrafficError> {
        let mut in_flight: FuturesUnordered<Sending> = FuturesUnordered::new();
        let mut n: u64 = 0;
        loop {
            let settings = self.settings()?;
            let wait = match settings.interval() {
                Some(interval) if in_flight.len() < Self::MAX_IN_FLIGHT => {
                    // The rate counts calls: a turn of several waits as long as they would.
                    let calls = Self::turn(n, run, &settings, &self.token).unwrap_or_default();
                    let wait = interval
                        .saturating_mul(u32::try_from(calls.len().max(1)).unwrap_or(u32::MAX));
                    if !calls.is_empty() {
                        in_flight.push(self.counted(client, calls, stats));
                    }
                    n = n.wrapping_add(1);
                    wait
                }
                Some(_) | None => Self::IDLE,
            };
            let pause = tokio::time::sleep(wait);
            tokio::pin!(pause);
            loop {
                tokio::select! {
                    () = &mut pause => break,
                    Some(()) = in_flight.next() => {}
                }
            }
        }
    }

    /// A turn's calls, one after another, each counted in `stats` while it
    /// runs and once it is sent.
    fn counted(&self, client: &reqwest::Client, calls: Vec<Call>, stats: &Arc<Stats>) -> Sending {
        Self::sequence(client, self.gateway, calls, stats.clone())
    }

    fn sequence(
        client: &reqwest::Client,
        gateway: SocketAddr,
        calls: Vec<Call>,
        stats: Arc<Stats>,
    ) -> Sending {
        let client = client.clone();
        Box::pin(async move {
            for (index, call) in calls.into_iter().enumerate() {
                if index > 0 {
                    // The client runs the tool before it asks again.
                    tokio::time::sleep(Self::TOOL_TIME).await;
                }
                stats.in_flight.fetch_add(1, Ordering::Relaxed);
                Self::send(&client, gateway, call).await;
                stats.in_flight.fetch_sub(1, Ordering::Relaxed);
                stats.sent.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    /// Send `count` turns now, all at once, then return.
    pub fn burst(&self, count: u16) -> Result<(), TrafficError> {
        let client = Self::client()?;
        let settings = self.settings()?;
        let run = Run(Seeder::next_run(&self.data_dir)?);
        Self::runtime()?.block_on(async {
            let calls: FuturesUnordered<Sending> = (0..u64::from(count))
                .filter_map(|n| Self::turn(n, run, &settings, &self.token))
                .map(|calls| {
                    Self::sequence(&client, self.gateway, calls, Arc::new(Stats::default()))
                })
                .collect();
            calls.collect::<Vec<()>>().await;
        });
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.settings
    }
}

impl Harness {
    fn index(self) -> u32 {
        match self {
            Self::ClaudeCode => 0,
            Self::Codex => 1,
            Self::Pi => 2,
            Self::Opencode => 3,
            Self::Deepseek => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Settings, Traffic};
    use crate::dev::seed::Run;

    #[test]
    fn generated_calls_follow_the_mix() {
        let settings = Settings {
            replies: Settings::default()
                .replies
                .with("text=0,tools=0,long=0,slow=0,cut=0,rate_limited=0,error=1")
                .unwrap(),
            harnesses: Settings::default()
                .harnesses
                .with("claude_code=0,pi=0,opencode=0,deepseek=0,codex=1")
                .unwrap(),
            ..Settings::default()
        };
        for n in 0..20 {
            let calls = Traffic::turn(n, Run(1), &settings, "tgvu_fixture").unwrap();
            let [call] = calls.as_slice() else {
                panic!("an error ends its turn at once: {}", calls.len())
            };
            assert_eq!(call.path, "/codex/responses");
            assert!(
                call.headers
                    .contains(&("x-dev-reply".into(), "error".into())),
                "{}",
                call.label
            );
        }
        let silent = Settings {
            senders: Settings::default()
                .senders
                .with("alice=0,ci=0,unlisted=0,unidentified=0")
                .unwrap(),
            ..Settings::default()
        };
        assert!(Traffic::turn(0, Run(1), &silent, "t").is_none());
    }

    #[test]
    fn some_claude_code_calls_are_side_calls_with_no_turn() {
        let settings = Settings {
            harnesses: Settings::default()
                .harnesses
                .with("claude_code=1,pi=0,opencode=0,deepseek=0,codex=1")
                .unwrap(),
            ..Settings::default()
        };
        let named = |call: &super::Call, name: &str| call.headers.iter().any(|(n, _)| n == name);
        let (mut side, mut claude) = (0, 0);
        for n in 0..2_000 {
            let call = Traffic::turn(n, Run(1), &settings, "tgvu_fixture").unwrap()[0].clone();
            if named(&call, "x-claude-code-session-id") {
                claude += 1;
                if !named(&call, "x-claude-code-prompt-id") {
                    side += 1;
                }
            } else {
                assert!(named(&call, "x-codex-turn-metadata"), "{}", call.label);
            }
        }
        let share = f64::from(side) / f64::from(claude);
        assert!((0.05..0.15).contains(&share), "{side} of {claude}");
    }

    #[test]
    fn every_harness_gets_the_same_share_of_errors() {
        // The default mix: 5% of calls are an error or a rate limit, for
        // every harness alike.
        let settings = Settings::default();
        let mut by_harness = std::collections::BTreeMap::<String, (u32, u32)>::new();
        for n in 0..20_000 {
            let call = Traffic::turn(n, Run(1), &settings, "tgvu_fixture").unwrap()[0].clone();
            let failing = call.headers.iter().any(|(name, value)| {
                name == "x-dev-reply" && (value == "error" || value == "rate_limited")
            });
            let entry = by_harness.entry(call.path.clone()).or_default();
            entry.0 += 1;
            entry.1 += u32::from(failing);
        }
        for (path, (calls, failing)) in by_harness {
            let share = f64::from(failing) / f64::from(calls);
            assert!(
                (0.03..0.07).contains(&share),
                "{path}: {failing} of {calls}"
            );
        }
    }
}
