//! `ren runs`: sessions and their runs through a daemon's control API. A
//! session is a client (Codex, Claude Code, Pi, or OpenCode) working in a folder under
//! the daemon's `[runs]` roots, driven through the client's structured mode;
//! each run is one process of its client serving it. On this host the
//! daemon's control token is read from its data directory; through a gateway,
//! the daemon authorizes the Tailscale identity that Serve adds.
use crate::commands::{Client, control::ControlTarget, history::TargetArgs};
use crate::context::Context;
use crate::management::Target;
use crate::style::Tone;
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use ren_gateway::api::control::request::{
    AnswerRequest, Checkout, ControlFeed, ControlRequest, CreateRequest, PromptRequest,
    ReplyRequest,
};
use ren_harness::Harness;
use ren_harness::drive::Decision;
use ren_protocol::sse::{EventName, SseFramer, SseLimits, SseOutput};
use ren_recorder::id::{RunId, SessionId};
use ren_store::{
    query::{PageLimit, Shelf},
    read::ReadRequest,
};
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
    /// The places a session can start in: the git repositories under the
    /// run roots, and each root's folders with none below them.
    Places,
    /// Start a session: a client in a folder under the daemon's run roots.
    Start(StartArgs),
    /// Send a prompt to a session: start a turn, starting its client again
    /// when none runs.
    Say(SayArgs),
    /// Answer an approval the session's client is waiting for. One that
    /// asks questions is only declined or cancelled this way: `reply`
    /// answers it.
    Answer(AnswerArgs),
    /// Answer the questions an approval asks.
    Reply(ReplyArgs),
    /// Interrupt the session's running turn.
    Interrupt(SessionArg),
    /// Stop the session's client; the session goes on.
    Stop(SessionArg),
    /// Print run updates as JSON lines as they happen (Ctrl-C to stop).
    Watch(WatchArgs),
    /// Recorded runs, newest first, from history.
    History(HistoryArgs),
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
    /// Where in the folder it works: `folder`, the folder itself (the
    /// default), or `worktree`, a new worktree of its own of the repository
    /// in the folder, on a branch of its own.
    #[arg(long, value_name = "WHERE")]
    checkout: Option<Checkout>,
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

impl From<Answer> for Decision {
    fn from(answer: Answer) -> Self {
        match answer {
            Answer::Accept => Self::Accept,
            Answer::AcceptForSession => Self::AcceptForSession,
            Answer::Decline => Self::Decline,
            Answer::Cancel => Self::Cancel,
        }
    }
}

#[derive(Args)]
struct ReplyArgs {
    session: SessionId,
    /// The approval id, as `ren runs ls` shows it.
    approval: String,
    /// The replies as JSON, by question key, for example
    /// `{"scope": {"as": "chose", "picked": ["Route only"]}}`. Each is
    /// `chose` (`picked` labels, and `written` for an answer in your own
    /// words), `wrote` or `secret` (`text`), `number` (`value`),
    /// `confirmed` (`answer`: yes or no), or `skipped`; a question left
    /// out is skipped.
    replies: String,
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
        let request = match self.action {
            RunsAction::History(args) => {
                let answer = self
                    .target
                    .history(ctx)
                    .await?
                    .serve(&ReadRequest::Runs {
                        limit: args.limit,
                        shelf: args.shelf,
                        project: None,
                    })
                    .await?;
                println!("{}", serde_json::to_string_pretty(&answer)?);
                return Ok(ExitCode::SUCCESS);
            }
            RunsAction::Watch(args) => {
                let control = ControlTarget::select(&self.target, ctx)?;
                return Self::watch(ctx, &control, args.run).await;
            }
            RunsAction::Ls => ControlRequest::Runs,
            RunsAction::Sessions => ControlRequest::Sessions,
            RunsAction::Places => ControlRequest::Places,
            RunsAction::Start(args) => {
                let local = matches!(self.target.select(ctx)?, Target::Local);
                let dir = Self::dir(args.dir.clone(), local)?;
                let control = ControlTarget::select(&self.target, ctx)?;
                return Self::start(&control, args, dir).await;
            }
            RunsAction::Say(args) => ControlRequest::Prompt {
                session: args.session,
                prompt: PromptRequest::text(args.text.join(" ")),
            },
            RunsAction::Answer(args) => ControlRequest::Answer {
                session: args.session,
                approval: args.approval,
                answer: AnswerRequest {
                    decision: args.decision.into(),
                },
            },
            RunsAction::Reply(args) => ControlRequest::Reply {
                session: args.session,
                approval: args.approval,
                reply: ReplyRequest {
                    replies: serde_json::from_str(&args.replies)
                        .context("the replies are not JSON replies by question key")?,
                },
            },
            RunsAction::Interrupt(args) => ControlRequest::Interrupt(args.session),
            RunsAction::Stop(args) => ControlRequest::Stop(args.session),
        };
        let control = ControlTarget::select(&self.target, ctx)?;
        Ok(control.send(&request).await?.report()?)
    }

    /// Make a session in `dir`, then send its first prompt, if one is given;
    /// prints the session made.
    async fn start(control: &ControlTarget, args: StartArgs, dir: String) -> Result<ExitCode> {
        let mut create = CreateRequest::new(Harness::from(args.client), Some(dir));
        create.resume = args.resume;
        create.checkout = args.checkout;
        let made = control.send(&ControlRequest::Create(create)).await?;
        if !made.status.is_success() {
            return Ok(made.report()?);
        }
        let session = made.body["session"]
            .as_str()
            .and_then(|session| session.parse::<SessionId>().ok());
        if let (Some(prompt), Some(session)) = (args.prompt, session) {
            let answer = control
                .send(&ControlRequest::Prompt {
                    session,
                    prompt: PromptRequest::text(prompt),
                })
                .await?;
            if !answer.status.is_success() {
                return Ok(answer.report()?);
            }
        }
        Ok(made.report()?)
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
        let mut response = control.stream(ControlFeed::Runs).await?;
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
