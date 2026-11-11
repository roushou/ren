//! `ren runs` with no subcommand: the shell around `ren-console`'s
//! [`Console`]. It follows the daemon's run feed, reads runs, recorded runs,
//! projects, and recorded turns, sends the console's commands through the
//! control API, and draws to `ren-tui`'s `Screen`.

use super::control::ControlTarget;
use crate::management::Management;
use anyhow::Result;
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use ren_console::{Answered, Command, Console, Input, Out, Read};
use ren_protocol::sse::{EventName, SseFramer, SseLimits, SseOutput};
use ren_recorder::id::{RunId, SessionId, TurnId};
use ren_store::{activity::TurnActivity, query::PageLimit, read::ReadRequest, view::RunView};
use ren_supervisor::{Project, RunSnapshot, RunUpdate};
use ren_tui::{Screen, ScreenEvent, Theme};
use reqwest::Method;
use std::{collections::HashMap, future::Future, pin::Pin, process::ExitCode, time::Duration};

/// A finished read or command, for the console.
enum Done {
    Input(Box<Input>),
    /// The feed ended or failed: connect again after a pause.
    FeedLost,
}

impl Done {
    fn of(input: Input) -> Self {
        Self::Input(Box::new(input))
    }
}

type Pending<'a> = FuturesUnordered<Pin<Box<dyn Future<Output = Done> + 'a>>>;

/// The run feed, as it is read.
struct Feed {
    response: reqwest::Response,
    framer: SseFramer,
}

/// The console's reads and commands, against one gateway.
struct Shell<'a> {
    gateway: &'a Management,
    control: Result<&'a ControlTarget, String>,
    pending: Pending<'a>,
    /// History reads after a turn ended, per run, for the pause before the next.
    later: HashMap<RunId, u32>,
}

pub struct ConsoleShell;

impl ConsoleShell {
    /// How many recorded turns a run's conversation shows.
    const TURNS: u32 = 10;
    const RECONNECT: Duration = Duration::from_secs(2);

    pub async fn run(
        gateway: &Management,
        control: Result<ControlTarget, String>,
        theme: Theme,
    ) -> Result<ExitCode> {
        // The first reads happen before the terminal is taken over, so an
        // unreachable gateway is reported as any command's error.
        let recorded = Shell::recorded(gateway).await?;
        let mut screen = Screen::open()?;
        screen.set_theme(theme);
        let mut console = Console::new(screen.size()?);
        let mut events = screen.events();
        let mut shell = Shell {
            gateway,
            control: control.as_ref().map_err(Clone::clone),
            pending: FuturesUnordered::new(),
            later: HashMap::new(),
        };
        let mut outs = console.apply(Input::Recorded(recorded));
        outs.extend(console.start());
        let mut feed = shell.connect().await;
        if !shell.act(outs) {
            return Ok(ExitCode::SUCCESS);
        }
        screen.draw(&console.frame())?;
        loop {
            let outs = tokio::select! {
                event = events.next() => match event {
                    None => break,
                    Some(event) => match event? {
                        ScreenEvent::Key(key) => console.press(key),
                        ScreenEvent::Resize(size) => console.apply(Input::Resize(size)),
                    },
                },
                Some(done) = shell.pending.next(), if !shell.pending.is_empty() => match done {
                    Done::Input(input) => console.apply(*input),
                    Done::FeedLost => {
                        feed = shell.connect().await;
                        // What changed while the feed was down.
                        vec![Out::Read(Read::Runs)]
                    }
                },
                inputs = Shell::next(&mut feed), if feed.is_some() => match inputs {
                    Some(inputs) => inputs
                        .into_iter()
                        .flat_map(|input| console.apply(input))
                        .collect(),
                    None => {
                        feed = None;
                        shell.pending.push(Box::pin(async {
                            tokio::time::sleep(Self::RECONNECT).await;
                            Done::FeedLost
                        }));
                        Vec::new()
                    }
                },
            };
            if !shell.act(outs) {
                break;
            }
            screen.draw(&console.frame())?;
        }
        Ok(ExitCode::SUCCESS)
    }
}

impl<'a> Shell<'a> {
    /// Start what the console asked for; false to quit.
    fn act(&mut self, outs: Vec<Out>) -> bool {
        for out in outs {
            match out {
                Out::Quit => return false,
                Out::Read(read) => self.read(read),
                Out::Send(command) => self.send(command),
            }
        }
        true
    }

    fn read(&mut self, read: Read) {
        let gateway = self.gateway;
        match read {
            Read::Runs => match self.control {
                Ok(control) => self.pending.push(Box::pin(async move {
                    Done::of(Input::Runs(Self::runs(control).await))
                })),
                Err(ref why) => {
                    let why = why.clone();
                    self.pending
                        .push(Box::pin(async move { Done::of(Input::Runs(Err(why))) }));
                }
            },
            Read::Recorded => self.pending.push(Box::pin(async move {
                match Self::recorded(gateway).await {
                    Ok(recorded) => Done::of(Input::Recorded(recorded)),
                    // Recorded runs are a convenience; live ones still work.
                    Err(_) => Done::of(Input::Recorded(Vec::new())),
                }
            })),
            Read::Projects => match self.control {
                Ok(control) => self.pending.push(Box::pin(async move {
                    Done::of(Input::Projects(Self::projects(control).await))
                })),
                Err(ref why) => {
                    let why = why.clone();
                    self.pending
                        .push(Box::pin(async move { Done::of(Input::Projects(Err(why))) }));
                }
            },
            Read::History { run, session } => {
                self.later.remove(&run);
                self.pending.push(Box::pin(async move {
                    Done::of(Input::History {
                        run,
                        turns: Self::turns(gateway, session).await,
                    })
                }));
            }
            Read::HistoryLater { run, session } => {
                // 0.5 s, then twice as long each time: a turn's record lands
                // shortly after its calls finish.
                let tries = self.later.entry(run).or_insert(0);
                let pause = Duration::from_millis(500) * 2u32.saturating_pow(*tries);
                *tries += 1;
                self.pending.push(Box::pin(async move {
                    tokio::time::sleep(pause).await;
                    Done::of(Input::History {
                        run,
                        turns: Self::turns(gateway, session).await,
                    })
                }));
            }
        }
    }

    fn send(&mut self, command: Command) {
        let Ok(control) = self.control else {
            return;
        };
        let starts = matches!(command, Command::Start { .. });
        let (path, body) = match command {
            Command::Start {
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
            Command::Prompt { run, prompt } => (
                format!("runs/{run}/prompt"),
                Some(serde_json::json!({ "text": prompt.as_str() })),
            ),
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
            Done::of(Input::Answered(answer))
        }));
    }

    async fn runs(control: &ControlTarget) -> Result<Vec<RunSnapshot>, String> {
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
        let Ok(limit) = PageLimit::new(ConsoleShell::TURNS) else {
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

    /// Follow the run feed; `None` when it cannot be opened now.
    async fn connect(&mut self) -> Option<Feed> {
        let control = self.control.as_ref().ok()?;
        let response = control.stream("runs/events").await.ok()?;
        Some(Feed {
            response,
            framer: SseFramer::new(SseLimits::default()),
        })
    }

    /// The next updates from the feed; `None` once it ends.
    async fn next(feed: &mut Option<Feed>) -> Option<Vec<Input>> {
        let feed = feed.as_mut()?;
        let chunk = feed.response.chunk().await.ok()??;
        let mut inputs = Vec::new();
        feed.framer.feed(&chunk, |output| match output {
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
