//! `ren tui`, and `ren runs` with no subcommand: the shell around `ren-tui`'s
//! [`Tui`], opened on the wall or on the console. Built with the `tui`
//! feature.
//!
//! It performs the reads and sends the commands the routes ask for, alongside
//! one another, so a slow gateway never stalls the keys: snapshots of
//! sessions and calls every couple of seconds and whenever the live feed says
//! a call opened or ended, the turns and calls the wall shows, the daemon's
//! runs, recorded runs, projects, and recorded turns for the console. It
//! follows two feeds, the live observations (with progress on streaming
//! calls) and the run feed, and draws the shown route. Everything goes
//! through the gateway's read and control APIs, local or remote.

use crate::commands::{control::ControlTarget, history::TargetArgs};
use crate::context::Context;
use crate::management::{Management, ManagementError};
use crate::tail::{LiveDecoder, LiveEvent, LiveUpdates};
use crate::tui_settings::TuiSettings;
use anyhow::Result;
use clap::Args;
use clap::builder::{PossibleValuesParser, TypedValueParser};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use ren_daemon::host::SystemClock;
use ren_harness::host::HostEnv;
use ren_protocol::label::Labeled;
use ren_protocol::sse::{EventName, SseFramer, SseLimits, SseOutput};
use ren_recorder::id::{CallId, RunId, SessionId, TurnId};
use ren_recorder::time::{Clock, Monotonic};
use ren_store::activity::{CallActivity, TurnActivity};
use ren_store::query::PageLimit;
use ren_store::read::ReadRequest;
use ren_store::view::RunView;
use ren_supervisor::{Project, RunSnapshot, RunUpdate};
use ren_tui::console::{self, Answered};
use ren_tui::wall::{self, Snapshot, Wall};
use ren_tui::{Input, Out, Route, Screen, ScreenEvent, Theme, ThemeName, Tui};
use reqwest::Method;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::process::ExitCode;
use std::time::Duration;

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

/// A read or command that finished.
enum Done {
    Snapshot(Result<Snapshot, ManagementError>),
    Turn(TurnId, Result<TurnActivity, ManagementError>),
    Call(CallId, Result<CallActivity, ManagementError>),
    Runs(Result<Vec<RunSnapshot>, String>),
    Console(Box<console::Input>),
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

/// The run feed: connected, or waiting to reconnect.
enum RunFeed {
    Connected {
        response: Box<reqwest::Response>,
        framer: SseFramer,
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
    runs: bool,
    /// When the last snapshot was asked for.
    asked: Monotonic,
}

/// Whether to go on after acting.
enum Flow {
    Go,
    Quit,
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
    /// How many recorded turns a run's conversation shows.
    const TURNS: u32 = 10;

    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        if let Some(theme) = self.save_theme {
            let path = TuiSettings::save(theme, &ctx.env)?;
            println!("Saved TUI theme {theme} to {}", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        Self::open(&self.target, self.theme, ctx, Route::Wall).await
    }

    /// Opens the terminal on `route`.
    pub(crate) async fn open(
        target: &TargetArgs,
        theme: Option<ThemeName>,
        ctx: &Context,
        route: Route,
    ) -> Result<ExitCode> {
        let selected = TuiSettings::select(theme, &ctx.env)?;
        let gateway = target.gateway(ctx)?;
        // Runs need control access: this host's token, or a Tailscale
        // identity through a gateway. Without it the wall still shows
        // sessions, and the console says why it cannot drive runs.
        let control = ControlTarget::select(target, ctx).map_err(|error| error.to_string());
        let clock = SystemClock::start();
        // The first reads happen before the terminal is taken over, so a
        // gateway that cannot be reached is reported as any command's error.
        let (first, recorded) = futures_util::try_join!(
            async { Ok(Self::snapshot(&gateway, &clock).await?) },
            Self::recorded(&gateway),
        )?;

        let mut screen = Screen::open()?;
        screen.set_theme(Self::theme(selected, &ctx.env));
        let mut events = screen.events();
        let mut tui = Tui::new(screen.size()?, route);
        let mut shell = Shell {
            gateway: &gateway,
            control: control.as_ref().map_err(Clone::clone),
            clock: &clock,
            pending: FuturesUnordered::new(),
            in_flight: InFlight {
                snapshot: false,
                turns: Vec::new(),
                calls: Vec::new(),
                runs: false,
                asked: clock.now().monotonic,
            },
            later: HashMap::new(),
            theme: selected.unwrap_or_default(),
        };
        let mut outs = tui.apply(Input::Wall(wall::Input::Snapshot(first)));
        outs.extend(tui.apply(Input::Console(console::Input::Recorded(recorded))));
        outs.extend(tui.start());
        let mut feed = Self::connect(&gateway, &clock).await;
        let mut runs_feed = shell.connect_runs().await;
        if let Flow::Quit = shell.act(outs, &mut screen) {
            return Ok(ExitCode::SUCCESS);
        }
        let mut ticker = tokio::time::interval(Self::TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        screen.draw(|size| tui.frame(clock.now().wall).view(size))?;

        loop {
            let outs = tokio::select! {
                event = events.next() => match event {
                    None => break,
                    Some(event) => match event? {
                        ScreenEvent::Key(key) => tui.press(key),
                        ScreenEvent::Resize(size) => tui.apply(Input::Resize(size)),
                    },
                },
                Some(done) = shell.pending.next(), if !shell.pending.is_empty() => {
                    shell.finish(done, &mut tui)
                }
                chunk = Self::chunk(&gateway, &mut feed) => {
                    Self::feed(chunk, &mut feed, &mut tui, &clock)
                }
                inputs = Shell::next_runs(&mut runs_feed) => match inputs {
                    Some(inputs) => inputs
                        .into_iter()
                        .flat_map(|input| tui.apply(Input::Console(input)))
                        .collect(),
                    None => {
                        runs_feed = RunFeed::Waiting { since: clock.now().monotonic };
                        Vec::new()
                    }
                },
                _ = ticker.tick() => {
                    let now = clock.now().monotonic;
                    let mut outs = Vec::new();
                    if let Feed::Waiting { since } = feed
                        && now.saturating_since(since) >= Self::RECONNECT
                    {
                        feed = Self::connect(&gateway, &clock).await;
                    }
                    if let RunFeed::Waiting { since } = runs_feed
                        && now.saturating_since(since) >= Self::RECONNECT
                    {
                        runs_feed = shell.connect_runs().await;
                        // What changed while the run feed was down.
                        if let RunFeed::Connected { .. } = runs_feed {
                            outs.push(Out::Console(console::Read::Runs));
                        }
                    }
                    if now.saturating_since(shell.in_flight.asked) >= Self::SNAPSHOT_EVERY {
                        outs.extend([Out::Wall(wall::Read::Snapshot), Out::Wall(wall::Read::Runs)]);
                    }
                    outs
                }
            };
            if let Flow::Quit = shell.act(outs, &mut screen) {
                break;
            }
            screen.draw(|size| tui.frame(clock.now().wall).view(size))?;
        }
        drop(events);
        drop(screen);
        Ok(ExitCode::SUCCESS)
    }

    /// Explicit per-application preferences override NO_COLOR; otherwise it
    /// selects monochrome. Environment access stays in the CLI shell.
    pub(crate) fn theme(selected: Option<ThemeName>, env: &HostEnv) -> Theme {
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
        let query = Wall::snapshot_query(read_at);
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

    async fn recorded(gateway: &Management) -> Result<Vec<RunView>> {
        #[derive(serde::Deserialize)]
        struct Listed {
            runs: Vec<RunView>,
        }
        let body = gateway
            .serve(&ReadRequest::Runs(PageLimit::DEFAULT))
            .await?;
        Ok(serde_json::from_value::<Listed>(body)?.runs)
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

    /// Decodes the feed's bytes into the wall; a feed that ends or fails
    /// waits to reconnect, and the next snapshot catches up.
    fn feed(
        chunk: Result<Option<Vec<u8>>, ManagementError>,
        feed: &mut Feed,
        tui: &mut Tui,
        clock: &SystemClock,
    ) -> Vec<Out> {
        let bytes = match chunk {
            Ok(Some(bytes)) => bytes,
            Ok(None) | Err(_) => {
                *feed = Feed::Waiting {
                    since: clock.now().monotonic,
                };
                return vec![Out::Wall(wall::Read::Snapshot)];
            }
        };
        let Feed::Connected { decoder, .. } = feed else {
            return Vec::new();
        };
        let mut events = Vec::new();
        decoder.feed(&bytes, &mut events);
        let mut outs = Vec::new();
        for event in events {
            let input = match event {
                LiveEvent::Observation(observation) => wall::Input::Observation(observation),
                LiveEvent::Progress(progress) => wall::Input::Progress(progress),
                // Behind or pruned: a snapshot reads the present again.
                LiveEvent::Lagged { .. } | LiveEvent::Gone { .. } => {
                    outs.push(Out::Wall(wall::Read::Snapshot));
                    continue;
                }
                LiveEvent::Failed(reason) => {
                    outs.extend(tui.apply(Input::Wall(wall::Input::ReadFailed(format!(
                        "live feed: {reason}"
                    )))));
                    *feed = Feed::Waiting {
                        since: clock.now().monotonic,
                    };
                    return outs;
                }
                LiveEvent::Unrecognized { .. } => continue,
            };
            outs.extend(tui.apply(Input::Wall(input)));
        }
        outs
    }
}

/// The reads and commands in flight, and what they go to.
struct Shell<'a> {
    gateway: &'a Management,
    /// Where run reads and commands go, or why they cannot.
    control: Result<&'a ControlTarget, String>,
    clock: &'a SystemClock,
    pending: Pending<'a>,
    in_flight: InFlight,
    /// History reads after a turn ended, per run, for the pause before the next.
    later: HashMap<RunId, u32>,
    /// The theme shown, which `t` cycles.
    theme: ThemeName,
}

impl<'a> Shell<'a> {
    /// Starts what the routes asked for.
    fn act(&mut self, outs: Vec<Out>, screen: &mut Screen) -> Flow {
        for out in outs {
            match out {
                Out::Quit => return Flow::Quit,
                Out::CycleTheme => {
                    self.theme = self.theme.next();
                    screen.set_theme(self.theme.theme());
                }
                Out::Wall(wall::Read::Runs) | Out::Console(console::Read::Runs) => self.runs(),
                Out::Wall(read) => self.wall(read),
                Out::Console(read) => self.console(read),
                Out::Send(command) => self.send(command),
            }
        }
        Flow::Go
    }

    fn wall(&mut self, read: wall::Read) {
        let gateway = self.gateway;
        match read {
            wall::Read::Snapshot if !self.in_flight.snapshot => {
                self.in_flight.snapshot = true;
                self.in_flight.asked = self.clock.now().monotonic;
                let clock = self.clock;
                self.pending.push(Box::pin(async move {
                    Done::Snapshot(TuiCmd::snapshot(gateway, clock).await)
                }));
            }
            wall::Read::Turn(turn) if !self.in_flight.turns.contains(&turn) => {
                self.in_flight.turns.push(turn);
                self.pending.push(Box::pin(async move {
                    Done::Turn(turn, gateway.turn(turn).await)
                }));
            }
            wall::Read::Call(call) if !self.in_flight.calls.contains(&call) => {
                self.in_flight.calls.push(call);
                self.pending.push(Box::pin(async move {
                    Done::Call(call, gateway.call_activity(call).await)
                }));
            }
            wall::Read::Snapshot | wall::Read::Turn(_) | wall::Read::Call(_) => {}
            wall::Read::Runs => self.runs(),
        }
    }

    /// The daemon's live runs, for both routes.
    fn runs(&mut self) {
        if self.in_flight.runs {
            return;
        }
        self.in_flight.runs = true;
        let control = self.control.clone();
        self.pending.push(Box::pin(async move {
            Done::Runs(match control {
                Ok(control) => Self::list_runs(control).await,
                Err(why) => Err(why),
            })
        }));
    }

    fn console(&mut self, read: console::Read) {
        let gateway = self.gateway;
        let control = self.control.clone();
        let done = |input: console::Input| Done::Console(Box::new(input));
        match read {
            console::Read::Runs => self.runs(),
            console::Read::Recorded => self.pending.push(Box::pin(async move {
                // Recorded runs are a convenience; live ones still work.
                done(console::Input::Recorded(
                    TuiCmd::recorded(gateway).await.unwrap_or_default(),
                ))
            })),
            console::Read::Projects => self.pending.push(Box::pin(async move {
                done(console::Input::Projects(match control {
                    Ok(control) => Self::projects(control).await,
                    Err(why) => Err(why),
                }))
            })),
            console::Read::History { run, session } => {
                self.later.remove(&run);
                self.pending.push(Box::pin(async move {
                    done(console::Input::History {
                        run,
                        turns: Self::turns(gateway, session).await,
                    })
                }));
            }
            console::Read::HistoryLater { run, session } => {
                // 0.5 s, then twice as long each time: a turn's record lands
                // shortly after its calls finish.
                let tries = self.later.entry(run).or_insert(0);
                let pause = Duration::from_millis(500) * 2u32.saturating_pow(*tries);
                *tries += 1;
                self.pending.push(Box::pin(async move {
                    tokio::time::sleep(pause).await;
                    done(console::Input::History {
                        run,
                        turns: Self::turns(gateway, session).await,
                    })
                }));
            }
        }
    }

    fn send(&mut self, command: console::Command) {
        let control = match &self.control {
            Ok(control) => *control,
            Err(why) => {
                let why = why.clone();
                self.pending.push(Box::pin(async move {
                    Done::Console(Box::new(console::Input::Answered(Err(why))))
                }));
                return;
            }
        };
        let starts = matches!(command, console::Command::Start { .. });
        let (path, body) = match command {
            console::Command::Start {
                harness,
                dir,
                resume,
                prompt,
            } => {
                let mut body = serde_json::json!({ "harness": harness, "dir": dir });
                if let Some(resume) = resume {
                    body["resume"] = resume.to_string().into();
                }
                if let Some(prompt) = prompt {
                    body["prompt"] = prompt.as_str().into();
                }
                ("runs".to_owned(), Some(body))
            }
            console::Command::Prompt { run, prompt } => (
                format!("runs/{run}/prompt"),
                Some(serde_json::json!({ "text": prompt.as_str() })),
            ),
            console::Command::Answer {
                run,
                approval,
                decision,
            } => (
                format!("runs/{run}/approvals/{approval}"),
                Some(serde_json::json!({ "decision": decision })),
            ),
            console::Command::Interrupt(run) => (format!("runs/{run}/interrupt"), None),
            console::Command::Stop(run) => (format!("runs/{run}/stop"), None),
        };
        self.pending.push(Box::pin(async move {
            let answer = match control.send(Method::POST, &path, body).await {
                Ok((status, body)) if status.is_success() => match body["run"]
                    .as_str()
                    .and_then(|run| run.parse::<uuid::Uuid>().ok())
                {
                    Some(run) if starts => Ok(Answered::Started(RunId::restore(run))),
                    Some(_) | None => Ok(Answered::Sent),
                },
                Ok((_, body)) => Err(body["error"]
                    .as_str()
                    .unwrap_or("the daemon refused")
                    .to_owned()),
                Err(error) => Err(error.to_string()),
            };
            Done::Console(Box::new(console::Input::Answered(answer)))
        }));
    }

    /// Hands a finished read to the routes; returns what they ask for next.
    fn finish(&mut self, done: Done, tui: &mut Tui) -> Vec<Out> {
        let input = match done {
            Done::Snapshot(result) => {
                self.in_flight.snapshot = false;
                result.map(|snapshot| Input::Wall(wall::Input::Snapshot(snapshot)))
            }
            Done::Turn(turn, result) => {
                self.in_flight.turns.retain(|running| *running != turn);
                result.map(|activity| Input::Wall(wall::Input::Turn(Box::new(activity))))
            }
            Done::Call(call, result) => {
                self.in_flight.calls.retain(|running| *running != call);
                result.map(|activity| Input::Wall(wall::Input::Call(Box::new(activity))))
            }
            Done::Runs(runs) => {
                self.in_flight.runs = false;
                Ok(Input::Runs(runs))
            }
            Done::Console(input) => Ok(Input::Console(*input)),
        };
        match input {
            Ok(input) => tui.apply(input),
            Err(error) => tui.apply(Input::Wall(wall::Input::ReadFailed(error.to_string()))),
        }
    }

    /// The daemon's runs, or why they cannot be read.
    async fn list_runs(control: &ControlTarget) -> Result<Vec<RunSnapshot>, String> {
        #[derive(serde::Deserialize)]
        struct Listed {
            runs: Vec<RunSnapshot>,
        }
        match control.send(Method::GET, "runs", None).await {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Listed>(body)
                .map(|listed| listed.runs)
                .map_err(|_| "the daemon's runs are not readable".to_owned()),
            Ok((_, body)) => Err(match body["error"].as_str() {
                Some("runs_disabled") => "runs are off on this daemon; set [runs] roots".to_owned(),
                Some(error) => error.to_owned(),
                None => "the daemon refused".to_owned(),
            }),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn projects(control: &ControlTarget) -> Result<Vec<Project>, String> {
        #[derive(serde::Deserialize)]
        struct Listed {
            projects: Vec<Project>,
        }
        match control.send(Method::GET, "runs/projects", None).await {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Listed>(body)
                .map(|listed| listed.projects)
                .map_err(|_| "the daemon's projects are not readable".to_owned()),
            Ok((_, body)) => Err(body["error"]
                .as_str()
                .unwrap_or("the daemon refused")
                .to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// A session's newest turns, oldest first. A failed read reads as none.
    async fn turns(gateway: &Management, session: SessionId) -> Vec<TurnActivity> {
        #[derive(serde::Deserialize)]
        struct Listed {
            turns: Vec<Summary>,
        }
        #[derive(serde::Deserialize)]
        struct Summary {
            turn: TurnId,
        }
        let Ok(limit) = PageLimit::new(TuiCmd::TURNS) else {
            return Vec::new();
        };
        let Ok(body) = gateway.serve(&ReadRequest::Turns { session, limit }).await else {
            return Vec::new();
        };
        let Ok(listed) = serde_json::from_value::<Listed>(body) else {
            return Vec::new();
        };
        let mut turns = Vec::new();
        for summary in listed.turns {
            if let Ok(turn) = gateway.turn(summary.turn).await {
                turns.push(turn);
            }
        }
        // Listed newest first.
        turns.reverse();
        turns
    }

    /// Follow the run feed; waiting to reconnect when it cannot be opened.
    async fn connect_runs(&self) -> RunFeed {
        let waiting = RunFeed::Waiting {
            since: self.clock.now().monotonic,
        };
        let Ok(control) = self.control else {
            return waiting;
        };
        match control.stream("runs/events").await {
            Ok(response) => RunFeed::Connected {
                response: Box::new(response),
                framer: SseFramer::new(SseLimits::default()),
            },
            Err(_) => waiting,
        }
    }

    /// The run feed's next updates; `None` once it ends. Never resolves while
    /// waiting to reconnect.
    async fn next_runs(feed: &mut RunFeed) -> Option<Vec<console::Input>> {
        let RunFeed::Connected { response, framer } = feed else {
            return std::future::pending().await;
        };
        let chunk = response.chunk().await.ok()??;
        let mut inputs = Vec::new();
        framer.feed(&chunk, |output| match output {
            SseOutput::Event {
                name: EventName::Present("run"),
                data,
            } => {
                // An update this console cannot read (from a newer daemon)
                // is skipped; the next listing catches up.
                if let Ok(update) = serde_json::from_slice::<RunUpdate>(data) {
                    inputs.push(console::Input::Update(update));
                }
            }
            SseOutput::Event {
                name: EventName::Present("lagged"),
                ..
            } => inputs.push(console::Input::Lagged),
            SseOutput::Event { .. } | SseOutput::Discarded(_) => {}
        });
        Some(inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::TuiCmd;
    use ren_harness::host::HostEnv;
    use ren_tui::{Theme, ThemeName};

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
