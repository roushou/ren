//! `ren control`: operator controls through a daemon's control API. On this
//! host the daemon's control token is read from its data directory; through
//! a gateway, the request carries no token and the daemon authorizes the
//! Tailscale identity that Serve adds.
use crate::commands::history::TargetArgs;
use crate::context::Context;
use crate::management::{ControlAnswer, ControlCredential, Management, Target};
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use ren_gateway::ControlToken;
use ren_gateway::api::control::request::{
    BlockRequest, CancelRequest, ControlFeed, ControlRequest, PauseRequest,
};
use ren_harness::endpoint::GatewayEndpoint;
use ren_protocol::native::NativeSessionId;
use ren_recorder::{
    batch::{HistoryChange, HistorySubject},
    caller::UserId,
    id::{CallId, RecordingId, RunId},
};
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
    Unblock(NativeSessionArg),
    /// Refuse a named user's new calls (non-retryable 403), effective at
    /// once; running calls continue.
    BlockUser(BlockUserArgs),
    UnblockUser(UserArg),
    /// Stop a running call.
    Cancel(CancelArgs),
    /// Copy the history into <data-dir>/backup while the daemon runs. Only
    /// new chunks are copied; keep this running until it reports.
    Backup,
    /// Hide a recording, or an ended run with its recording, from listings.
    /// A recording with new traffic is listed again.
    Archive {
        #[command(subcommand)]
        subject: Subject,
    },
    /// List an archived recording or run again.
    Unarchive {
        #[command(subcommand)]
        subject: Subject,
    },
    /// Delete a recording, or a run with its recording: their calls,
    /// bodies, and runs. Refused while a call is open or a run is running.
    /// It cannot be undone.
    Delete(DeleteArgs),
}

/// What an archive or delete applies to.
#[derive(Subcommand, Clone, Copy)]
enum Subject {
    /// A recording, by its id (`ren recordings` lists them).
    Recording { recording: RecordingId },
    /// A run, by its run id, with the recording its client made.
    Run { run: RunId },
}

impl Subject {
    fn edit(self, change: HistoryChange) -> ControlRequest {
        ControlRequest::Edit {
            subject: match self {
                Self::Recording { recording } => HistorySubject::Recording(recording),
                Self::Run { run } => HistorySubject::Run(run),
            },
            change,
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
struct NativeSessionArg {
    /// The client's native session id (a UUID).
    #[arg(value_parser = NativeSessionArg::parse)]
    session: NativeSessionId,
}

#[derive(Args)]
struct BlockArgs {
    #[command(flatten)]
    session: NativeSessionArg,
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

impl NativeSessionArg {
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
pub(crate) struct ControlTarget {
    gateway: Management,
    credential: ControlCredential,
}

impl ControlTarget {
    pub(crate) fn select(target: &TargetArgs, ctx: &Context) -> Result<Self> {
        Ok(match target.select(ctx)? {
            Target::Remote(gateway) => Self {
                gateway,
                credential: ControlCredential::Tailnet,
            },
            Target::Local => {
                let config = ctx.config()?;
                let data_dir = ctx.data_dir(&config)?;
                let token = std::fs::read_to_string(data_dir.join(ControlToken::FILE))
                    .ok()
                    .as_deref()
                    .and_then(ControlToken::parse)
                    .context("no control token; is the daemon running with this data directory?")?;
                Self {
                    gateway: Management::new(GatewayEndpoint::resolve(None, config.listen)?)?,
                    credential: ControlCredential::Token(token),
                }
            }
        })
    }

    /// Send one request and return the daemon's answer. Refused identities
    /// through a gateway are errors; every other answer is returned to be
    /// printed.
    pub(crate) async fn send(&self, request: &ControlRequest) -> Result<ControlAnswer> {
        Ok(self.gateway.control(request, &self.credential).await?)
    }

    /// One of the control API's event streams.
    pub(crate) async fn stream(&self, feed: ControlFeed) -> Result<reqwest::Response> {
        Ok(self.gateway.control_stream(feed, &self.credential).await?)
    }
}

impl ControlCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        if let ControlAction::Delete(DeleteArgs { yes: false, .. }) = self.action {
            bail!("deleting recorded history cannot be undone; pass --yes to delete");
        }
        let target = ControlTarget::select(&self.target, ctx)?;
        Ok(target.send(&self.action.request()).await?.report()?)
    }
}

impl ControlAction {
    /// The control API request this action makes.
    fn request(self) -> ControlRequest {
        match self {
            Self::State => ControlRequest::State,
            Self::Pause(args) => ControlRequest::Pause(PauseRequest {
                retry_after_seconds: args.retry_after,
            }),
            Self::Resume => ControlRequest::Resume,
            Self::Block(args) => ControlRequest::BlockSession {
                session: args.session.session,
                block: BlockRequest {
                    duration_seconds: args.duration,
                },
            },
            Self::Unblock(args) => ControlRequest::UnblockSession(args.session),
            Self::BlockUser(args) => ControlRequest::BlockUser {
                user: args.user.user,
                block: BlockRequest {
                    duration_seconds: args.duration,
                },
            },
            Self::UnblockUser(args) => ControlRequest::UnblockUser(args.user),
            Self::Cancel(args) => ControlRequest::Cancel {
                call: args.call,
                cancel: CancelRequest {
                    block_session_seconds: args.block_session,
                },
            },
            Self::Backup => ControlRequest::Backup,
            Self::Archive { subject } => subject.edit(HistoryChange::Archive),
            Self::Unarchive { subject } => subject.edit(HistoryChange::Unarchive),
            Self::Delete(args) => args.subject.edit(HistoryChange::Delete),
        }
    }
}
