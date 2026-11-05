//! `tgv control`: operator controls through the local daemon's control API.
//! The daemon's control token is read from its data directory.
use crate::context::Context;
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
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
        let config = ctx.config()?;
        let data_dir = ctx.data_dir(&config)?;
        let token = std::fs::read_to_string(data_dir.join(ControlToken::FILE))
            .ok()
            .as_deref()
            .and_then(ControlToken::parse)
            .context("no control token; is the daemon running with this data directory?")?;
        let base = format!("http://{}/control/v1", config.listen);
        let client = reqwest::Client::builder().no_proxy().build()?;
        let (path, body) = match self.action {
            ControlAction::State => {
                let response = client
                    .get(format!("{base}/state"))
                    .bearer_auth(token.expose())
                    .send()
                    .await
                    .context("cannot reach the daemon")?;
                return Self::report(response).await;
            }
            ControlAction::Pause(args) => (
                "pause".to_owned(),
                Some(serde_json::json!({ "retry_after_seconds": args.retry_after })),
            ),
            ControlAction::Resume => ("resume".to_owned(), None),
            ControlAction::Block(args) => (
                format!("sessions/{}/block", args.session.session.as_uuid()),
                Some(serde_json::json!({ "duration_seconds": args.duration })),
            ),
            ControlAction::Unblock(args) => {
                (format!("sessions/{}/unblock", args.session.as_uuid()), None)
            }
            ControlAction::Cancel(args) => (
                format!("calls/{}/cancel", args.call),
                Some(serde_json::json!({ "block_session_seconds": args.block_session })),
            ),
            ControlAction::Backup => ("backup".to_owned(), None),
        };
        let mut request = client
            .post(format!("{base}/{path}"))
            .bearer_auth(token.expose());
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.context("cannot reach the daemon")?;
        Self::report(response).await
    }

    async fn report(response: reqwest::Response) -> Result<ExitCode> {
        let status = response.status();
        let body: serde_json::Value = serde_json::from_slice(&response.bytes().await?)
            .context("the daemon's answer is not JSON")?;
        println!("{}", serde_json::to_string_pretty(&body)?);
        if !status.is_success() {
            bail!("control request failed with {status}");
        }
        Ok(ExitCode::SUCCESS)
    }
}
