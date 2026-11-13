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

mod editor;

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
use ren_recorder::id::{CallId, RunId, TurnId};
use ren_recorder::time::{Clock, Monotonic};
use ren_store::activity::{CallActivity, TurnActivity};
use ren_store::query::{PageLimit, Shelf};
use ren_store::read::ReadRequest;
use ren_store::view::{ConversationView, RunView};
use ren_supervisor::{Client, Places, RunSnapshot, RunUpdate};
use ren_tui::{
    Answered, Command, Input, Out, Read, Snapshot, Terminal, TerminalEvent, Theme, ThemeName, Tui,
};
use reqwest::Method;
use std::future::Future;
use std::pin::Pin;
use std::process::ExitCode;
use std::time::Duration;

/// Watch and drive agent sessions in the terminal.
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
    Other(Box<Input>),
    /// A read that gave nothing to apply.
    Nothing,
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
    /// Edit this text in the operator's editor.
    Edit(String),
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

    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        if let Some(theme) = self.save_theme {
            let path = TuiSettings::save(theme, &ctx.env)?;
            println!("Saved TUI theme {theme} to {}", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        Self::open(&self.target, self.theme, ctx).await
    }

    /// Opens the app in the terminal.
    pub(crate) async fn open(
        target: &TargetArgs,
        theme: Option<ThemeName>,
        ctx: &Context,
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

        let mut screen = Terminal::open()?;
        screen.set_theme(Self::theme(selected, &ctx.env));
        let mut events = screen.events();
        let editor = editor::Editor::of(&ctx.env);
        let mut tui = Tui::new(screen.size()?);
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
            theme: selected.unwrap_or_default(),
        };
        let mut outs = tui.apply(Input::Snapshot(first));
        outs.extend(tui.apply(Input::Recorded(recorded)));
        // Snapshots and recorded runs were just read: not again at once.
        outs.extend(
            tui.start()
                .into_iter()
                .filter(|out| !matches!(out, Out::Read(Read::Snapshot | Read::Recorded))),
        );
        let mut feed = Self::connect(&gateway, &clock).await;
        let mut runs_feed = shell.connect_runs().await;
        if let Flow::Quit = shell.act(outs, &mut screen) {
            return Ok(ExitCode::SUCCESS);
        }
        let mut ticker = tokio::time::interval(Self::TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        screen.draw(|_| tui.view(clock.now().wall))?;

        loop {
            let outs = tokio::select! {
                event = events.next() => match event {
                    None => break,
                    Some(event) => match event? {
                        TerminalEvent::Key(key) => tui.press(key),
                        TerminalEvent::Paste(text) => tui.paste(&text),
                        TerminalEvent::Resize(size) => tui.apply(Input::Resize(size)),
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
                        .flat_map(|input| tui.apply(input))
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
                            outs.push(Out::Read(Read::Runs));
                        }
                    }
                    if now.saturating_since(shell.in_flight.asked) >= Self::SNAPSHOT_EVERY {
                        outs.extend([Out::Read(Read::Snapshot), Out::Read(Read::Runs)]);
                    }
                    outs
                }
            };
            let mut flow = shell.act(outs, &mut screen);
            // The editor has the terminal until it exits; the app's events
            // stop meanwhile, so they do not read its keys.
            while let Flow::Edit(text) = flow {
                drop(events);
                screen.suspend()?;
                let edited = editor.edit(&text);
                screen.resume()?;
                events = screen.events();
                let outs = match &edited {
                    Ok(text) => tui.edited(Ok(text)),
                    Err(error) => tui.edited(Err(&error.to_string())),
                };
                flow = shell.act(outs, &mut screen);
            }
            if let Flow::Quit = flow {
                break;
            }
            screen.draw(|_| tui.view(clock.now().wall))?;
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
        let query = Tui::snapshot_query(read_at);
        let (recordings, open, recent) = futures_util::try_join!(
            gateway.recordings(query.recordings),
            gateway.calls(&query.open),
            gateway.calls(&query.recent),
        )?;
        Ok(Snapshot {
            read_at,
            recordings,
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
            .serve(&ReadRequest::Runs {
                limit: PageLimit::DEFAULT,
                shelf: Shelf::Current,
            })
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
                return vec![Out::Read(Read::Snapshot)];
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
                LiveEvent::Observation(observation) => Input::Observation(observation),
                LiveEvent::Progress(progress) => Input::Progress(progress),
                // Behind or pruned: a snapshot reads the present again.
                LiveEvent::Lagged { .. } | LiveEvent::Gone { .. } => {
                    outs.push(Out::Read(Read::Snapshot));
                    continue;
                }
                LiveEvent::Failed(reason) => {
                    outs.extend(tui.apply(Input::ReadFailed(format!("live feed: {reason}"))));
                    *feed = Feed::Waiting {
                        since: clock.now().monotonic,
                    };
                    return outs;
                }
                LiveEvent::Unrecognized { .. } => continue,
            };
            outs.extend(tui.apply(input));
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
    /// The theme shown, which `t` cycles.
    theme: ThemeName,
}

impl<'a> Shell<'a> {
    /// Starts what the routes asked for.
    fn act(&mut self, outs: Vec<Out>, screen: &mut Terminal) -> Flow {
        let mut flow = Flow::Go;
        for out in outs {
            match out {
                Out::Quit => return Flow::Quit,
                Out::Edit(text) => flow = Flow::Edit(text),
                Out::CycleTheme => {
                    self.theme = self.theme.next();
                    screen.set_theme(self.theme.theme());
                }
                Out::Read(read) => self.read(read),
                Out::Send(command) => self.send(command),
            }
        }
        flow
    }

    fn read(&mut self, read: Read) {
        let gateway = self.gateway;
        let control = self.control.clone();
        let done = |input: Input| Done::Other(Box::new(input));
        match read {
            Read::Snapshot if !self.in_flight.snapshot => {
                self.in_flight.snapshot = true;
                self.in_flight.asked = self.clock.now().monotonic;
                let clock = self.clock;
                self.pending.push(Box::pin(async move {
                    Done::Snapshot(TuiCmd::snapshot(gateway, clock).await)
                }));
            }
            Read::Turn(turn) if !self.in_flight.turns.contains(&turn) => {
                self.in_flight.turns.push(turn);
                self.pending.push(Box::pin(async move {
                    Done::Turn(turn, gateway.turn(turn).await)
                }));
            }
            Read::Call(call) if !self.in_flight.calls.contains(&call) => {
                self.in_flight.calls.push(call);
                self.pending.push(Box::pin(async move {
                    Done::Call(call, gateway.call_activity(call).await)
                }));
            }
            Read::Snapshot | Read::Turn(_) | Read::Call(_) => {}
            Read::Runs => self.runs(),
            Read::Recorded => self.pending.push(Box::pin(async move {
                // Recorded runs are a convenience; live ones still work.
                done(Input::Recorded(
                    TuiCmd::recorded(gateway).await.unwrap_or_default(),
                ))
            })),
            Read::Projects => self.pending.push(Box::pin(async move {
                done(Input::Projects(match control {
                    Ok(control) => Self::projects(control).await,
                    Err(why) => Err(why),
                }))
            })),
            Read::Clients => self.pending.push(Box::pin(async move {
                done(Input::Clients(match control {
                    Ok(control) => Self::clients(control).await,
                    Err(why) => Err(why),
                }))
            })),
            Read::Models(harness) => self.pending.push(Box::pin(async move {
                done(Input::Models {
                    harness,
                    models: match control {
                        Ok(control) => Self::models(control, harness).await,
                        Err(why) => Err(why),
                    },
                })
            })),
            Read::Commands(run) => self.pending.push(Box::pin(async move {
                done(Input::Commands {
                    run,
                    commands: match control {
                        Ok(control) => Self::commands(control, run).await,
                        Err(why) => Err(why),
                    },
                })
            })),
            Read::Files { run, query } => self.pending.push(Box::pin(async move {
                let files = match control {
                    Ok(control) => Self::files(control, run, &query).await,
                    Err(why) => Err(why),
                };
                done(Input::Files { run, query, files })
            })),
            Read::Conversation(run) => self.pending.push(Box::pin(async move {
                // A failed read leaves what the feed shows; selecting the run
                // again reads it again.
                match Shell::conversation(gateway, run).await {
                    Some(conversation) => done(Input::Conversation { run, conversation }),
                    None => Done::Nothing,
                }
            })),
        }
    }

    /// The daemon's live runs.
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

    fn send(&mut self, command: Command) {
        let control = match &self.control {
            Ok(control) => *control,
            Err(why) => {
                let why = why.clone();
                self.pending.push(Box::pin(async move {
                    Done::Other(Box::new(Input::Answered(Err(why))))
                }));
                return;
            }
        };
        let starts = matches!(command, Command::Start { .. });
        let (path, body) = match command {
            Command::Start {
                harness,
                dir,
                resume,
                prompt,
                settings,
            } => {
                let mut body = serde_json::json!({ "harness": harness });
                // Without a folder, the run gets a scratch folder of its own.
                if let Some(dir) = dir {
                    body["dir"] = dir.into();
                }
                if let Some(model) = settings.model {
                    body["model"] = model.as_str().into();
                }
                if let Some(effort) = settings.effort {
                    body["effort"] = effort.to_string().into();
                }
                if let Some(approvals) = settings.approvals {
                    body["approvals"] = approvals.to_string().into();
                }
                if let Some(resume) = resume {
                    body["resume"] = resume.to_string().into();
                }
                if let Some(prompt) = prompt {
                    body["prompt"] = prompt.as_str().into();
                }
                ("runs".to_owned(), Some(body))
            }
            Command::Prompt {
                run,
                prompt,
                delivery,
            } => (
                format!("runs/{run}/prompt"),
                Some(serde_json::json!({ "text": prompt.as_str(), "delivery": delivery })),
            ),
            Command::Unqueue(run) => (format!("runs/{run}/unqueue"), None),
            Command::Answer {
                run,
                approval,
                decision,
            } => (
                format!("runs/{run}/approvals/{approval}"),
                Some(serde_json::json!({ "decision": decision })),
            ),
            Command::Interrupt(run) => (format!("runs/{run}/interrupt"), None),
            Command::Stop(run) => (format!("runs/{run}/stop"), None),
            Command::Edit { subject, change } => (
                match subject {
                    ren_recorder::batch::HistorySubject::Session(session) => {
                        format!("history/sessions/{session}/{change}")
                    }
                    ren_recorder::batch::HistorySubject::Run(run) => {
                        format!("history/runs/{run}/{change}")
                    }
                },
                None,
            ),
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
            Done::Other(Box::new(Input::Answered(answer)))
        }));
    }

    /// Hands a finished read to the routes; returns what they ask for next.
    fn finish(&mut self, done: Done, tui: &mut Tui) -> Vec<Out> {
        let input = match done {
            Done::Snapshot(result) => {
                self.in_flight.snapshot = false;
                result.map(Input::Snapshot)
            }
            Done::Turn(turn, result) => {
                self.in_flight.turns.retain(|running| *running != turn);
                result.map(|activity| Input::Turn(Box::new(activity)))
            }
            Done::Call(call, result) => {
                self.in_flight.calls.retain(|running| *running != call);
                result.map(|activity| Input::Call(Box::new(activity)))
            }
            Done::Runs(runs) => {
                self.in_flight.runs = false;
                Ok(Input::Runs(runs))
            }
            Done::Other(input) => Ok(*input),
            Done::Nothing => return Vec::new(),
        };
        match input {
            Ok(input) => tui.apply(input),
            Err(error) => tui.apply(Input::ReadFailed(error.to_string())),
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

    async fn clients(control: &ControlTarget) -> Result<Vec<Client>, String> {
        #[derive(serde::Deserialize)]
        struct Listed {
            clients: Vec<Client>,
        }
        match control.send(Method::GET, "runs/clients", None).await {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Listed>(body)
                .map(|listed| listed.clients)
                .map_err(|_| "the daemon's clients are not readable".to_owned()),
            Ok((_, body)) => Err(body["error"]
                .as_str()
                .unwrap_or("the daemon refused")
                .to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// The models `harness` offers, as it lists them on the daemon's host.
    async fn models(
        control: &ControlTarget,
        harness: ren_harness::Harness,
    ) -> Result<Vec<ren_harness::models::Model>, String> {
        #[derive(serde::Deserialize)]
        struct Listed {
            models: Vec<ren_harness::models::Model>,
        }
        let path = format!("runs/clients/{harness}/models");
        match control.send(Method::GET, &path, None).await {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Listed>(body)
                .map(|listed| listed.models)
                .map_err(|_| "the client's models are not readable".to_owned()),
            Ok((_, body)) => Err(body["message"]
                .as_str()
                .or_else(|| body["error"].as_str())
                .unwrap_or("the daemon refused")
                .to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// What a run's client offers a prompt.
    async fn commands(
        control: &ControlTarget,
        run: RunId,
    ) -> Result<Vec<ren_harness::commands::ClientCommand>, String> {
        #[derive(serde::Deserialize)]
        struct Listed {
            commands: Vec<ren_harness::commands::ClientCommand>,
        }
        match control
            .send(Method::GET, &format!("runs/{run}/commands"), None)
            .await
        {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Listed>(body)
                .map(|listed| listed.commands)
                .map_err(|_| "the run's commands are not readable".to_owned()),
            Ok((_, body)) => Err(body["error"]
                .as_str()
                .unwrap_or("the daemon refused")
                .to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// The paths of a run's folder that best match `query`.
    async fn files(
        control: &ControlTarget,
        run: RunId,
        query: &str,
    ) -> Result<Vec<ren_supervisor::FileMatch>, String> {
        #[derive(serde::Deserialize)]
        struct Found {
            files: Vec<ren_supervisor::FileMatch>,
        }
        let mut search =
            reqwest::Url::parse("http://query.invalid/").map_err(|error| error.to_string())?;
        search
            .query_pairs_mut()
            .append_pair("q", query)
            .append_pair("limit", "20");
        let path = format!("runs/{run}/files?{}", search.query().unwrap_or_default());
        match control.send(Method::GET, &path, None).await {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Found>(body)
                .map(|found| found.files)
                .map_err(|_| "the folder's files are not readable".to_owned()),
            Ok((_, body)) => Err(body["error"]
                .as_str()
                .unwrap_or("the daemon refused")
                .to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn projects(control: &ControlTarget) -> Result<Places, String> {
        match control.send(Method::GET, "runs/projects", None).await {
            Ok((status, body)) if status.is_success() => serde_json::from_value::<Places>(body)
                .map_err(|_| "the daemon's projects are not readable".to_owned()),
            Ok((_, body)) => Err(body["error"]
                .as_str()
                .unwrap_or("the daemon refused")
                .to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// A run's conversation, whole, as the daemon stored it.
    async fn conversation(gateway: &Management, run: RunId) -> Option<ConversationView> {
        let body = gateway.serve(&ReadRequest::Conversation(run)).await.ok()?;
        serde_json::from_value(body).ok()
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
    async fn next_runs(feed: &mut RunFeed) -> Option<Vec<Input>> {
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
                    inputs.push(Input::Update(update));
                }
            }
            SseOutput::Event {
                name: EventName::Present("lagged"),
                ..
            } => inputs.push(Input::Lagged),
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
