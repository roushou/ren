//! `ren control`: operator controls through a daemon's control API. On this
//! host the daemon's control token is read from its data directory; through
//! a gateway, the request carries no token and the daemon authorizes the
//! Tailscale identity that Serve adds.
use crate::commands::history::TargetArgs;
use crate::context::Context;
use crate::management::{Management, Target};
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use ren_gateway::ControlToken;
use ren_protocol::native::NativeSessionId;
use ren_recorder::{
    caller::UserId,
    id::{CallId, RecordingId, RunId},
};
use reqwest::Method;
use std::process::ExitCode;
use uuid::Uuid;

/// Pause, block, and cancel through the running daemon (operator controls).
#[derive(Args)]
pub struct ControlCmd {
    #[command(subcommand)]
    action: ControlAction,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Subcommand)]
enum ControlAction {
    /// Show the operator state and the running calls.
    State,
    /// Answer every new call with 503 and retry-after.
    Pause(PauseArgs),
    /// Admit new calls again.
    Resume,
    /// Refuse a client session's calls (non-retryable 403).
    Block(BlockArgs),
    Unblock(RecordingArg),
    /// Refuse a named user's new calls (non-retryable 403), effective at
    /// once; running calls continue.
    BlockUser(BlockUserArgs),
    UnblockUser(UserArg),
    /// Stop a running call.
    Cancel(CancelArgs),
    /// Copy the history into <data-dir>/backup while the daemon runs. Only
    /// new chunks are copied; keep this running until it reports.
    Backup,
    /// Hide a recorded session, or an ended run with its session, from
    /// listings. A session with new traffic is listed again.
    Archive {
        #[command(subcommand)]
        subject: Subject,
    },
    /// List an archived session or run again.
    Unarchive {
        #[command(subcommand)]
        subject: Subject,
    },
    /// Delete a recorded session, or a run with its session: their calls,
    /// bodies, and runs. Refused while a call is open or a run is running.
    /// It cannot be undone.
    Delete(DeleteArgs),
}

/// What an archive or delete applies to.
#[derive(Subcommand, Clone, Copy)]
enum Subject {
    /// A session, by Ren's session id (`ren sessions` lists them).
    Session { session: RecordingId },
    /// A run, by its run id, with the session its client recorded.
    Run { run: RunId },
}

impl Subject {
    fn path(self, change: &str) -> String {
        match self {
            Self::Session { session } => format!("history/sessions/{session}/{change}"),
            Self::Run { run } => format!("history/runs/{run}/{change}"),
        }
    }
}

#[derive(Args)]
struct DeleteArgs {
    #[command(subcommand)]
    subject: Subject,
    /// Confirm: what is deleted cannot be recovered.
    #[arg(long, global = true)]
    yes: bool,
}

#[derive(Args)]
struct PauseArgs {
    /// Seconds clients are told to wait.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=3600))]
    retry_after: u32,
}

#[derive(Args)]
struct RecordingArg {
    /// The client's native session id (a UUID).
    #[arg(value_parser = RecordingArg::parse)]
    session: NativeSessionId,
}

#[derive(Args)]
struct BlockArgs {
    #[command(flatten)]
    session: RecordingArg,
    /// Block for this many seconds; without it, until unblocked.
    #[arg(long = "for")]
    duration: Option<u64>,
}

#[derive(Args)]
struct UserArg {
    /// A user id from the daemon's [[users]].
    user: UserId,
}

#[derive(Args)]
struct BlockUserArgs {
    #[command(flatten)]
    user: UserArg,
    /// Block for this many seconds; without it, until unblocked.
    #[arg(long = "for")]
    duration: Option<u64>,
}

#[derive(Args)]
struct CancelArgs {
    call: CallId,
    /// Also block the call's session for this many seconds.
    #[arg(long)]
    block_session: Option<u64>,
}

impl RecordingArg {
    fn parse(text: &str) -> Result<NativeSessionId, String> {
        Uuid::try_parse(text)
            .ok()
            .and_then(NativeSessionId::new)
            .ok_or_else(|| "a session id is a non-nil UUID".to_owned())
    }
}

/// Where control requests go: this host's daemon, authorized by the control
/// token in its data directory, or a gateway, authorized by the Tailscale
/// identity Serve adds.
pub(crate) enum ControlTarget {
    Local {
        client: reqwest::Client,
        base: String,
        token: ControlToken,
    },
    Remote(Management),
}

impl ControlTarget {
    pub(crate) fn select(target: &TargetArgs, ctx: &Context) -> Result<Self> {
        Ok(match target.select(ctx)? {
            Target::Remote(gateway) => Self::Remote(gateway),
            Target::Local => {
                let config = ctx.config()?;
                let data_dir = ctx.data_dir(&config)?;
                let token = std::fs::read_to_string(data_dir.join(ControlToken::FILE))
                    .ok()
                    .as_deref()
                    .and_then(ControlToken::parse)
                    .context("no control token; is the daemon running with this data directory?")?;
                Self::Local {
                    client: reqwest::Client::builder().no_proxy().build()?,
                    base: format!("http://{}/control/v1", config.listen),
                    token,
                }
            }
        })
    }

    /// Send one request, relative to `/control/v1/`, and return the daemon's
    /// answer. Refused identities through a gateway are errors; every other
    /// answer is returned to be printed.
    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(reqwest::StatusCode, serde_json::Value)> {
        match self {
            Self::Remote(gateway) => {
                let answer = gateway.control(method, path, body).await?;
                Ok((answer.status, answer.body))
            }
            Self::Local {
                client,
                base,
                token,
            } => {
                let mut request = client
                    .request(method, format!("{base}/{path}"))
                    .bearer_auth(token.expose());
                if let Some(body) = body {
                    request = request
                        .header("content-type", "application/json")
                        .body(body.to_string());
                }
                let response = request.send().await.context("cannot reach the daemon")?;
                let status = response.status();
                let body: serde_json::Value = serde_json::from_slice(&response.bytes().await?)
                    .context("the daemon's answer is not JSON")?;
                Ok((status, body))
            }
        }
    }

    /// A streamed `GET`, relative to `/control/v1/`: server-sent events.
    pub(crate) async fn stream(&self, path: &str) -> Result<reqwest::Response> {
        match self {
            Self::Remote(gateway) => Ok(gateway.control_stream(path).await?),
            Self::Local {
                client,
                base,
                token,
            } => {
                let response = client
                    .get(format!("{base}/{path}"))
                    .bearer_auth(token.expose())
                    .header("accept", "text/event-stream")
                    .send()
                    .await
                    .context("cannot reach the daemon")?;
                let status = response.status();
                if !status.is_success() {
                    let body: serde_json::Value =
                        serde_json::from_slice(&response.bytes().await?).unwrap_or_default();
                    println!("{}", serde_json::to_string_pretty(&body)?);
                    bail!("control request failed with {status}");
                }
                Ok(response)
            }
        }
    }

    /// Print the daemon's answer; a refusal is an error after it.
    pub(crate) fn report(
        status: reqwest::StatusCode,
        body: &serde_json::Value,
    ) -> Result<ExitCode> {
        println!("{}", serde_json::to_string_pretty(body)?);
        if !status.is_success() {
            bail!("control request failed with {status}");
        }
        Ok(ExitCode::SUCCESS)
    }
}

impl ControlCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        if let ControlAction::Delete(DeleteArgs { yes: false, .. }) = self.action {
            bail!("deleting recorded history cannot be undone; pass --yes to delete");
        }
        let target = ControlTarget::select(&self.target, ctx)?;
        let (method, path, body) = self.action.request();
        let (status, body) = target.send(method, &path, body).await?;
        ControlTarget::report(status, &body)
    }
}

impl ControlAction {
    /// The control API request this action makes, relative to `/control/v1/`.
    fn request(self) -> (Method, String, Option<serde_json::Value>) {
        match self {
            Self::State => (Method::GET, "state".to_owned(), None),
            Self::Pause(args) => (
                Method::POST,
                "pause".to_owned(),
                Some(serde_json::json!({ "retry_after_seconds": args.retry_after })),
            ),
            Self::Resume => (Method::POST, "resume".to_owned(), None),
            Self::Block(args) => (
                Method::POST,
                format!("sessions/{}/block", args.session.session.as_uuid()),
                Some(serde_json::json!({ "duration_seconds": args.duration })),
            ),
            Self::Unblock(args) => (
                Method::POST,
                format!("sessions/{}/unblock", args.session.as_uuid()),
                None,
            ),
            Self::BlockUser(args) => (
                Method::POST,
                format!("users/{}/block", args.user.user),
                Some(serde_json::json!({ "duration_seconds": args.duration })),
            ),
            Self::UnblockUser(args) => (Method::POST, format!("users/{}/unblock", args.user), None),
            Self::Cancel(args) => (
                Method::POST,
                format!("calls/{}/cancel", args.call),
                Some(serde_json::json!({ "block_session_seconds": args.block_session })),
            ),
            Self::Backup => (Method::POST, "backup".to_owned(), None),
            Self::Archive { subject } => (Method::POST, subject.path("archive"), None),
            Self::Unarchive { subject } => (Method::POST, subject.path("unarchive"), None),
            Self::Delete(args) => (Method::POST, args.subject.path("delete"), None),
        }
    }
}
