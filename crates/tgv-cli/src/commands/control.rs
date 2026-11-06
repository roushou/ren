//! `tgv control`: operator controls through a daemon's control API. On this
//! host the daemon's control token is read from its data directory; through
//! a gateway, the request carries no token and the daemon authorizes the
//! Tailscale identity that Serve adds.
use crate::commands::history::TargetArgs;
use crate::context::Context;
use crate::management::Target;
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use reqwest::Method;
use std::process::ExitCode;
use tgv_gateway::ControlToken;
use tgv_protocol::native::NativeSessionId;
use tgv_recorder::id::CallId;
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
    Unblock(SessionArg),
    /// Stop a running call.
    Cancel(CancelArgs),
    /// Copy the history into <data-dir>/backup while the daemon runs. Only
    /// new chunks are copied; keep this running until it reports.
    Backup,
}

#[derive(Args)]
struct PauseArgs {
    /// Seconds clients are told to wait.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..=3600))]
    retry_after: u32,
}

#[derive(Args)]
struct SessionArg {
    /// The client's native session id (a UUID).
    #[arg(value_parser = SessionArg::parse)]
    session: NativeSessionId,
}

#[derive(Args)]
struct BlockArgs {
    #[command(flatten)]
    session: SessionArg,
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

impl SessionArg {
    fn parse(text: &str) -> Result<NativeSessionId, String> {
        Uuid::try_parse(text)
            .ok()
            .and_then(NativeSessionId::new)
            .ok_or_else(|| "a session id is a non-nil UUID".to_owned())
    }
}

impl ControlCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let target = self.target.select(ctx)?;
        let (method, path, body) = self.action.request();
        match target {
            Target::Local => Self::local(ctx, method, &path, body).await,
            Target::Remote(gateway) => {
                let answer = gateway.control(method, &path, body).await?;
                Self::report(answer.status, &answer.body)
            }
        }
    }

    async fn local(
        ctx: &Context,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<ExitCode> {
        let config = ctx.config()?;
        let data_dir = ctx.data_dir(&config)?;
        let token = std::fs::read_to_string(data_dir.join(ControlToken::FILE))
            .ok()
            .as_deref()
            .and_then(ControlToken::parse)
            .context("no control token; is the daemon running with this data directory?")?;
        let client = reqwest::Client::builder().no_proxy().build()?;
        let mut request = client
            .request(
                method,
                format!("http://{}/control/v1/{path}", config.listen),
            )
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
        Self::report(status, &body)
    }

    fn report(status: reqwest::StatusCode, body: &serde_json::Value) -> Result<ExitCode> {
        println!("{}", serde_json::to_string_pretty(body)?);
        if !status.is_success() {
            bail!("control request failed with {status}");
        }
        Ok(ExitCode::SUCCESS)
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
            Self::Cancel(args) => (
                Method::POST,
                format!("calls/{}/cancel", args.call),
                Some(serde_json::json!({ "block_session_seconds": args.block_session })),
            ),
            Self::Backup => (Method::POST, "backup".to_owned(), None),
        }
    }
}
