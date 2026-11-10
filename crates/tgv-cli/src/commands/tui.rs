//! `tgv tui`: the targeted gateway's live sessions in the terminal, one
//! pane per session (`tgv-tui`). Built with the `tui` feature.
//!
//! This is the shell around the view's sans-IO core: it reads snapshots of
//! sessions and calls every couple of seconds and whenever the live feed
//! says a call opened or ended, reads the turns the view asks for, passes
//! the feed's progress updates and the terminal's keys and sizes in, and
//! draws. Reads run alongside one another, so a slow gateway never stalls
//! the keys. Everything goes through the gateway's API, local or remote:
//! progress on streaming calls exists only in a running daemon.

use crate::commands::history::TargetArgs;
use crate::context::Context;
use crate::management::{Management, ManagementError};
use crate::tail::{LiveDecoder, LiveEvent, LiveUpdates};
use crate::tui_settings::TuiSettings;
use anyhow::Result;
use clap::Args;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use std::future::Future;
use std::pin::Pin;
use std::process::ExitCode;
use std::time::Duration;
use tgv_daemon::host::SystemClock;
use tgv_harness::host::HostEnv;
use tgv_protocol::label::Labeled;
use tgv_recorder::id::{CallId, TurnId};
use tgv_recorder::time::{Clock, Monotonic};
use tgv_store::activity::{CallActivity, TurnActivity};
use tgv_tui::{Input, Pressed, Read, Screen, ScreenEvent, Snapshot, Theme, ThemeName, Tui};

/// Watch live sessions in the terminal, one pane per session.
#[derive(Args)]
pub struct TuiCmd {
    #[command(flatten)]
    target: TargetArgs,
    /// Theme for this run (terminal, dark, light); overrides the saved preference.
    #[arg(long, value_parser = Self::theme_parser())]
    theme: Option<ThemeName>,
    /// Save a local theme preference and exit without connecting to a daemon.
    #[arg(long, conflicts_with = "theme", value_parser = Self::theme_parser())]
    save_theme: Option<ThemeName>,
}

/// A read that finished.
enum Done {
    Snapshot(Result<Snapshot, ManagementError>),
    Turn(TurnId, Result<TurnActivity, ManagementError>),
    Call(CallId, Result<CallActivity, ManagementError>),
}

type Pending<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = Done> + 'a>>>;

/// The live feed: connected, or waiting to reconnect.
enum Feed {
    Connected {
        response: Box<reqwest::Response>,
        decoder: LiveDecoder,
    },
    Waiting {
        since: Monotonic,
    },
}

/// Reads in flight, so the same one is never asked for twice at once.
struct InFlight {
    snapshot: bool,
    turns: Vec<TurnId>,
    calls: Vec<CallId>,
    /// When the last snapshot was asked for.
    asked: Monotonic,
}

impl TuiCmd {
    fn theme_parser() -> impl TypedValueParser<Value = ThemeName> {
        PossibleValuesParser::new(ThemeName::ALL.iter().map(|name| name.label()))
            .try_map(|value| value.parse::<ThemeName>())
    }

    /// A snapshot at least this often, however quiet the feed.
    const SNAPSHOT_EVERY: Duration = Duration::from_secs(2);
    /// How often the clock and elapsed times on the wall move.
    const TICK: Duration = Duration::from_millis(250);
    const RECONNECT: Duration = Duration::from_secs(2);

    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        if let Some(theme) = self.save_theme {
            let path = TuiSettings::save(theme, &ctx.env)?;
            println!("Saved TUI theme {theme} to {}", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        let selected = TuiSettings::select(self.theme, &ctx.env)?;
        let mut theme_name = selected.unwrap_or_default();
        let theme = Self::theme(selected, &ctx.env);
        let gateway = self.target.gateway(ctx)?;
        let clock = SystemClock::start();
        // The first read happens before the terminal is taken over, so a
        // gateway that cannot be reached is reported as any command's error.
        let first = Self::snapshot(&gateway, &clock).await?;
        let feed = Self::connect(&gateway, &clock).await;

        let mut screen = Screen::open()?;
        screen.set_theme(theme);
        let mut tui = Tui::new(screen.size()?);
        let mut events = screen.events();
        let pending: Pending<'_> = FuturesUnordered::new();
        let mut in_flight = InFlight {
            snapshot: false,
            turns: Vec::new(),
            calls: Vec::new(),
            asked: clock.now().monotonic,
        };
        let reads = tui.apply(Input::Snapshot(first));
        let mut shell = Shell {
            gateway: &gateway,
            clock: &clock,
            pending,
        };
        shell.ask(reads, &mut in_flight);
        let mut feed = feed;
        let mut ticker = tokio::time::interval(Self::TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        screen.draw(&tui.wall(clock.now().wall))?;

        loop {
            let reads = tokio::select! {
                event = events.next() => match event {
                    None => break,
                    Some(event) => match event? {
                        ScreenEvent::Key(key) => match tui.press(key) {
                            Pressed::Quit => break,
                            Pressed::Continue(reads) => reads,
                            Pressed::CycleTheme => {
                                theme_name = theme_name.next();
                                screen.set_theme(theme_name.theme());
                                Vec::new()
                            }
                        },
                        ScreenEvent::Resize(size) => tui.apply(Input::Resize(size)),
                    },
                },
                Some(done) = shell.pending.next(), if !shell.pending.is_empty() => {
                    shell.finish(done, &mut tui, &mut in_flight)
                }
                chunk = Self::chunk(&gateway, &mut feed) => {
                    Self::feed(chunk, &mut feed, &mut tui, &clock)
                }
                _ = ticker.tick() => {
                    let now = clock.now().monotonic;
                    if let Feed::Waiting { since } = feed
                        && now.saturating_since(since) >= Self::RECONNECT
                    {
                        feed = Self::connect(&gateway, &clock).await;
                    }
                    if now.saturating_since(in_flight.asked) >= Self::SNAPSHOT_EVERY {
                        vec![Read::Snapshot]
                    } else {
                        Vec::new()
                    }
                }
            };
            shell.ask(reads, &mut in_flight);
            screen.draw(&tui.wall(clock.now().wall))?;
        }
        drop(events);
        drop(screen);
        Ok(ExitCode::SUCCESS)
    }

    /// Explicit per-application preferences override NO_COLOR; otherwise it
    /// selects monochrome. Environment access stays in the CLI shell.
    fn theme(selected: Option<ThemeName>, env: &HostEnv) -> Theme {
        match selected {
            Some(name) => name.theme(),
            None if env.is_set("NO_COLOR") => Theme::monochrome(),
            None => Theme::default(),
        }
    }

    async fn snapshot(
        gateway: &Management,
        clock: &SystemClock,
    ) -> Result<Snapshot, ManagementError> {
        let read_at = clock.now().wall;
        let query = Tui::snapshot_query(read_at);
        let (sessions, open, recent) = futures_util::try_join!(
            gateway.sessions(query.sessions),
            gateway.calls(&query.open),
            gateway.calls(&query.recent),
        )?;
        Ok(Snapshot {
            read_at,
            sessions,
            open,
            recent,
        })
    }

    /// The live feed from now, with progress; snapshots fill in whatever
    /// happened before it.
    async fn connect(gateway: &Management, clock: &SystemClock) -> Feed {
        match gateway.live(None, LiveUpdates::WithProgress).await {
            Ok(response) => Feed::Connected {
                response: Box::new(response),
                decoder: LiveDecoder::default(),
            },
            Err(_) => Feed::Waiting {
                since: clock.now().monotonic,
            },
        }
    }

    /// The feed's next bytes; never resolves while waiting to reconnect.
    async fn chunk(
        gateway: &Management,
        feed: &mut Feed,
    ) -> Result<Option<Vec<u8>>, ManagementError> {
        match feed {
            Feed::Connected { response, .. } => gateway.chunk(response).await,
            Feed::Waiting { .. } => std::future::pending().await,
        }
    }

    /// Decodes the feed's bytes into the view; a feed that ends or fails
    /// waits to reconnect, and the next snapshot catches up.
    fn feed(
        chunk: Result<Option<Vec<u8>>, ManagementError>,
        feed: &mut Feed,
        tui: &mut Tui,
        clock: &SystemClock,
    ) -> Vec<Read> {
        let bytes = match chunk {
            Ok(Some(bytes)) => bytes,
            Ok(None) | Err(_) => {
                *feed = Feed::Waiting {
                    since: clock.now().monotonic,
                };
                return vec![Read::Snapshot];
            }
        };
        let Feed::Connected { decoder, .. } = feed else {
            return Vec::new();
        };
        let mut events = Vec::new();
        decoder.feed(&bytes, &mut events);
        let mut reads = Vec::new();
        for event in events {
            match event {
                LiveEvent::Observation(observation) => {
                    reads.extend(tui.apply(Input::Observation(observation)));
                }
                LiveEvent::Progress(progress) => reads.extend(tui.apply(Input::Progress(progress))),
                // Behind or pruned: a snapshot reads the present again.
                LiveEvent::Lagged { .. } | LiveEvent::Gone { .. } => reads.push(Read::Snapshot),
                LiveEvent::Failed(reason) => {
                    reads.extend(tui.apply(Input::ReadFailed(format!("live feed: {reason}"))));
                    *feed = Feed::Waiting {
                        since: clock.now().monotonic,
                    };
                    return reads;
                }
                LiveEvent::Unrecognized { .. } => {}
            }
        }
        reads
    }
}

/// The reads in flight, and what they read from.
struct Shell<'a> {
    gateway: &'a Management,
    clock: &'a SystemClock,
    pending: Pending<'a>,
}

impl<'a> Shell<'a> {
    /// Starts the reads the view asked for that are not already running.
    fn ask(&mut self, reads: Vec<Read>, in_flight: &mut InFlight) {
        for read in reads {
            match read {
                Read::Snapshot if !in_flight.snapshot => {
                    in_flight.snapshot = true;
                    in_flight.asked = self.clock.now().monotonic;
                    let (gateway, clock) = (self.gateway, self.clock);
                    self.pending.push(Box::pin(async move {
                        Done::Snapshot(TuiCmd::snapshot(gateway, clock).await)
                    }));
                }
                Read::Turn(turn) if !in_flight.turns.contains(&turn) => {
                    in_flight.turns.push(turn);
                    let gateway = self.gateway;
                    self.pending.push(Box::pin(async move {
                        Done::Turn(turn, gateway.turn(turn).await)
                    }));
                }
                Read::Call(call) if !in_flight.calls.contains(&call) => {
                    in_flight.calls.push(call);
                    let gateway = self.gateway;
                    self.pending.push(Box::pin(async move {
                        Done::Call(call, gateway.call_activity(call).await)
                    }));
                }
                Read::Snapshot | Read::Turn(_) | Read::Call(_) => {}
            }
        }
    }

    /// Hands a finished read to the view; returns the reads it asks for.
    fn finish(&mut self, done: Done, tui: &mut Tui, in_flight: &mut InFlight) -> Vec<Read> {
        let input = match done {
            Done::Snapshot(result) => {
                in_flight.snapshot = false;
                result.map(Input::Snapshot)
            }
            Done::Turn(turn, result) => {
                in_flight.turns.retain(|running| *running != turn);
                result.map(|activity| Input::Turn(Box::new(activity)))
            }
            Done::Call(call, result) => {
                in_flight.calls.retain(|running| *running != call);
                result.map(|activity| Input::Call(Box::new(activity)))
            }
        };
        match input {
            Ok(input) => tui.apply(input),
            Err(error) => tui.apply(Input::ReadFailed(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TuiCmd;
    use tgv_harness::host::HostEnv;
    use tgv_tui::{Theme, ThemeName};

    #[test]
    fn color_policy_uses_terminal_defaults_unless_the_user_chooses() {
        let none = HostEnv::default();
        assert_eq!(TuiCmd::theme(None, &none), Theme::default());
        let no_color = HostEnv::from_pairs([("NO_COLOR", "1")]);
        assert_eq!(TuiCmd::theme(None, &no_color), Theme::monochrome());
        for name in [ThemeName::Terminal, ThemeName::Dark, ThemeName::Light] {
            assert_eq!(TuiCmd::theme(Some(name), &no_color), name.theme());
        }
    }
}
