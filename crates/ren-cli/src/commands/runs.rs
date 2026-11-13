//! `ren runs`: sessions and their runs through a daemon's control API. A
//! session is a client (Codex, Claude Code, Pi, or OpenCode) working in a folder under
//! the daemon's `[runs]` roots, driven through the client's structured mode;
//! each run is one process of its client serving it. On this host the
//! daemon's control token is read from its data directory; through a gateway,
//! the daemon authorizes the Tailscale identity that Serve adds.
use crate::commands::{control::ControlTarget, history::TargetArgs};
use crate::context::Context;
use crate::management::Target;
use crate::style::Tone;
use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use ren_harness::Harness;
use ren_protocol::sse::{EventName, SseFramer, SseLimits, SseOutput};
use ren_recorder::id::{RunId, SessionId};
use ren_store::{
    query::{PageLimit, Shelf},
    read::ReadRequest,
};
use reqwest::Method;
use std::{path::PathBuf, process::ExitCode};
use uuid::Uuid;

/// Sessions and their runs on the daemon.
#[derive(Args)]
pub struct RunsCmd {
    #[command(subcommand)]
    action: RunsAction,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Subcommand)]
enum RunsAction {
    /// List the daemon's runs (each a session's client process) and their
    /// states.
    Ls,
    /// List the daemon's sessions: each as stored, with its runs, and as
    /// served now.
    Sessions,
    /// The projects a session can start in: the git repositories under the
    /// run roots, and each root's folders with none below them.
    Projects,
    /// Start a session: a client in a folder under the daemon's run roots.
    Start(StartArgs),
    /// Send a prompt to a session: start a turn, starting its client again
    /// when none runs.
    Say(SayArgs),
    /// Answer an approval the session's client is waiting for.
    Answer(AnswerArgs),
    /// Interrupt the session's running turn.
    Interrupt(SessionArg),
    /// Stop the session's client; the session goes on.
    Stop(SessionArg),
    /// Print run updates as JSON lines as they happen (Ctrl-C to stop).
    Watch(WatchArgs),
    /// Recorded runs, newest first, from history.
    History(HistoryArgs),
}

/// The clients a run can drive.
#[derive(Clone, Copy, ValueEnum)]
enum Client {
    Codex,
    /// Claude Code.
    Claude,
    Pi,
    Opencode,
}

impl From<Client> for Harness {
    fn from(client: Client) -> Self {
        match client {
            Client::Codex => Self::Codex,
            Client::Claude => Self::ClaudeCode,
            Client::Pi => Self::Pi,
            Client::Opencode => Self::OpenCode,
        }
    }
}

#[derive(Args)]
struct StartArgs {
    #[arg(value_enum)]
    client: Client,
    /// The folder to work in (default: the current one). Relative paths
    /// are resolved here, so they name this host's folders only.
    dir: Option<PathBuf>,
    /// Go on with the client's conversation of this thread id (one begun
    /// elsewhere) instead of starting a new one.
    #[arg(long)]
    resume: Option<Uuid>,
    /// The first prompt, sent once the session is made.
    #[arg(long)]
    prompt: Option<String>,
}

#[derive(Args)]
struct SessionArg {
    session: SessionId,
}

#[derive(Args)]
struct SayArgs {
    session: SessionId,
    /// The prompt; several words are joined with spaces.
    #[arg(required = true, num_args = 1..)]
    text: Vec<String>,
}

/// How to answer an approval.
#[derive(Clone, Copy, ValueEnum)]
enum Answer {
    /// Allow this one action.
    Accept,
    /// Allow it, and the same kind of action for the rest of the session.
    AcceptForSession,
    /// Refuse it; the turn goes on.
    Decline,
    /// Refuse it and interrupt the turn.
    Cancel,
}

impl Answer {
    fn label(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::AcceptForSession => "accept_for_session",
            Self::Decline => "decline",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Args)]
struct AnswerArgs {
    session: SessionId,
    /// The approval id, as `ren runs ls` shows it.
    approval: String,
    #[arg(value_enum, default_value_t = Answer::Accept)]
    decision: Answer,
}

#[derive(Args)]
struct WatchArgs {
    /// Only this run's updates.
    #[arg(long)]
    run: Option<RunId>,
}

#[derive(Args)]
struct HistoryArgs {
    #[arg(long, default_value_t = PageLimit::DEFAULT)]
    limit: PageLimit,
    /// `current`, or `archived`: the ended runs that were archived.
    #[arg(long, default_value_t = Shelf::Current)]
    shelf: Shelf,
}

impl RunsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let (method, path, body) = match self.action {
            RunsAction::History(args) => {
                let answer = self
                    .target
                    .history(ctx)
                    .await?
                    .serve(&ReadRequest::Runs {
                        limit: args.limit,
                        shelf: args.shelf,
                    })
                    .await?;
                println!("{}", serde_json::to_string_pretty(&answer)?);
                return Ok(ExitCode::SUCCESS);
            }
            RunsAction::Watch(args) => {
                let control = ControlTarget::select(&self.target, ctx)?;
                return Self::watch(ctx, &control, args.run).await;
            }
            RunsAction::Ls => (Method::GET, "runs".to_owned(), None),
            RunsAction::Sessions => (Method::GET, "sessions".to_owned(), None),
            RunsAction::Projects => (Method::GET, "runs/projects".to_owned(), None),
            RunsAction::Start(args) => {
                let local = matches!(self.target.select(ctx)?, Target::Local);
                let dir = Self::dir(args.dir.clone(), local)?;
                let control = ControlTarget::select(&self.target, ctx)?;
                return Self::start(&control, args, dir).await;
            }
            RunsAction::Say(args) => (
                Method::POST,
                format!("sessions/{}/prompt", args.session),
                Some(serde_json::json!({ "text": args.text.join(" ") })),
            ),
            RunsAction::Answer(args) => (
                Method::POST,
                format!("sessions/{}/approvals/{}", args.session, args.approval),
                Some(serde_json::json!({ "decision": args.decision.label() })),
            ),
            RunsAction::Interrupt(args) => (
                Method::POST,
                format!("sessions/{}/interrupt", args.session),
                None,
            ),
            RunsAction::Stop(args) => (
                Method::POST,
                format!("sessions/{}/stop", args.session),
                None,
            ),
        };
        let control = ControlTarget::select(&self.target, ctx)?;
        let (status, body) = control.send(method, &path, body).await?;
        ControlTarget::report(status, &body)
    }

    /// Make a session in `dir`, then send its first prompt, if one is given;
    /// prints the session made.
    async fn start(control: &ControlTarget, args: StartArgs, dir: String) -> Result<ExitCode> {
        let mut body = serde_json::json!({ "harness": Harness::from(args.client), "dir": dir });
        if let Some(resume) = args.resume {
            body["resume"] = resume.to_string().into();
        }
        let (status, made) = control.send(Method::POST, "sessions", Some(body)).await?;
        if !status.is_success() {
            return ControlTarget::report(status, &made);
        }
        if let (Some(prompt), Some(session)) = (args.prompt, made["session"].as_str()) {
            let (status, answer) = control
                .send(
                    Method::POST,
                    &format!("sessions/{session}/prompt"),
                    Some(serde_json::json!({ "text": prompt })),
                )
                .await?;
            if !status.is_success() {
                return ControlTarget::report(status, &answer);
            }
        }
        ControlTarget::report(status, &made)
    }

    /// The folder as the daemon must read it: absolute. A relative one is
    /// resolved against this process's folder, which only means something to
    /// a daemon on this host.
    fn dir(dir: Option<PathBuf>, local: bool) -> Result<String> {
        let dir = dir.unwrap_or_else(|| PathBuf::from("."));
        let dir = if dir.is_absolute() {
            dir
        } else if local {
            std::path::absolute(&dir)?
        } else {
            bail!("through a gateway, give the folder as an absolute path on the gateway's host");
        };
        match dir.to_str() {
            Some(dir) => Ok(dir.to_owned()),
            None => bail!("the folder's path is not valid UTF-8"),
        }
    }

    async fn watch(
        ctx: &Context,
        control: &ControlTarget,
        only: Option<RunId>,
    ) -> Result<ExitCode> {
        let mut response = control.stream("runs/events").await?;
        let stderr = ctx.terminal.stderr;
        let mut framer = SseFramer::new(SseLimits::default());
        while let Some(chunk) = response.chunk().await? {
            framer.feed(&chunk, |output| match output {
                SseOutput::Event {
                    name: EventName::Present("run"),
                    data,
                } => {
                    let Ok(update) = serde_json::from_slice::<serde_json::Value>(data) else {
                        return;
                    };
                    let wanted = only.is_none_or(|run| update["run"] == run.to_string().as_str());
                    if wanted {
                        println!("{update}");
                    }
                }
                SseOutput::Event {
                    name: EventName::Present("lagged"),
                    ..
                } => eprintln!(
                    "{}",
                    stderr.paint(Tone::Warn, "fell behind; some updates were skipped")
                ),
                SseOutput::Event { .. } | SseOutput::Discarded(_) => {}
            });
        }
        eprintln!(
            "{}",
            stderr.paint(Tone::Muted, "the daemon closed the feed")
        );
        Ok(ExitCode::SUCCESS)
    }
}
