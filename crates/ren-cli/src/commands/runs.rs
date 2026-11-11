//! `ren runs`: agent runs through a daemon's control API. A run is a client
//! (Codex so far) that the daemon starts in a folder under its `[runs]` roots
//! and drives through the client's structured mode. On this host the
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
use ren_recorder::id::RunId;
use ren_store::{query::PageLimit, read::ReadRequest};
use reqwest::Method;
use std::{path::PathBuf, process::ExitCode};
use uuid::Uuid;

/// Start, prompt, approve, and stop agent runs on the daemon.
#[derive(Args)]
pub struct RunsCmd {
    #[command(subcommand)]
    action: RunsAction,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Subcommand)]
enum RunsAction {
    /// List the daemon's runs and their states.
    Ls,
    /// The folders a run can start in: each run root and the folders inside it.
    Folders,
    /// Start a client in a folder under the daemon's run roots.
    Start(StartArgs),
    /// Send a prompt: start a turn.
    Say(SayArgs),
    /// Answer an approval the run is waiting for.
    Answer(AnswerArgs),
    /// Interrupt the running turn.
    Interrupt(RunArg),
    /// Stop the run's process.
    Stop(RunArg),
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
}

impl From<Client> for Harness {
    fn from(client: Client) -> Self {
        match client {
            Client::Codex => Self::Codex,
            Client::Claude => Self::ClaudeCode,
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
    /// Continue the client's conversation with this thread id instead of
    /// starting a new one.
    #[arg(long)]
    resume: Option<Uuid>,
}

#[derive(Args)]
struct RunArg {
    run: RunId,
}

#[derive(Args)]
struct SayArgs {
    run: RunId,
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
    run: RunId,
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
}

impl RunsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let (method, path, body) = match self.action {
            RunsAction::History(args) => {
                let answer = self
                    .target
                    .history(ctx)
                    .await?
                    .serve(&ReadRequest::Runs(args.limit))
                    .await?;
                println!("{}", serde_json::to_string_pretty(&answer)?);
                return Ok(ExitCode::SUCCESS);
            }
            RunsAction::Watch(args) => {
                let control = ControlTarget::select(&self.target, ctx)?;
                return Self::watch(ctx, &control, args.run).await;
            }
            RunsAction::Ls => (Method::GET, "runs".to_owned(), None),
            RunsAction::Folders => (Method::GET, "runs/folders".to_owned(), None),
            RunsAction::Start(args) => {
                let local = matches!(self.target.select(ctx)?, Target::Local);
                let dir = Self::dir(args.dir, local)?;
                (
                    Method::POST,
                    "runs".to_owned(),
                    Some(serde_json::json!({
                        "harness": Harness::from(args.client),
                        "dir": dir,
                        "resume": args.resume,
                    })),
                )
            }
            RunsAction::Say(args) => (
                Method::POST,
                format!("runs/{}/prompt", args.run),
                Some(serde_json::json!({ "text": args.text.join(" ") })),
            ),
            RunsAction::Answer(args) => (
                Method::POST,
                format!("runs/{}/approvals/{}", args.run, args.approval),
                Some(serde_json::json!({ "decision": args.decision.label() })),
            ),
            RunsAction::Interrupt(args) => {
                (Method::POST, format!("runs/{}/interrupt", args.run), None)
            }
            RunsAction::Stop(args) => (Method::POST, format!("runs/{}/stop", args.run), None),
        };
        let control = ControlTarget::select(&self.target, ctx)?;
        let (status, body) = control.send(method, &path, body).await?;
        ControlTarget::report(status, &body)
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
