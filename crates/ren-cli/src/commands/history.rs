//! History commands. Each reads this host's storage, or, when a gateway is
//! selected, the same views through that gateway's management API; the JSON
//! printed is the same either way.
use crate::context::Context;
use crate::management::{History, HostFlags, Management, Target};
use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use ren_harness::Harness;
use ren_harness::endpoint::GatewayEndpoint;
use ren_protocol::direction::Direction;
use ren_recorder::caller::UserId;
use ren_recorder::id::{CallId, RecordingId, TurnId};
use ren_store::query::{CallFilter, CallQuery, Cursor, PageLimit, Shelf, TimeRange};
use ren_store::read::ReadRequest;
use ren_store::search::{ExposureQuery, SearchField, SearchQuery, SearchTerms};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

/// Which daemon a history or control command reads from.
#[derive(Args)]
pub struct TargetArgs {
    /// Gateway root URL whose API to use (default: this host when --config
    /// or --data-dir is given, else REN_GATEWAY, then client.toml, then this
    /// host's daemon and storage).
    #[arg(long, global = true)]
    gateway: Option<String>,
}

impl TargetArgs {
    pub fn select(&self, ctx: &Context) -> Result<Target> {
        let host = if ctx.config_path.is_some() || ctx.data_dir_override.is_some() {
            HostFlags::Named
        } else {
            HostFlags::Absent
        };
        Ok(Target::select(self.gateway.as_deref(), host, &ctx.env)?)
    }

    /// Where history is read: this host's storage, or the selected gateway.
    pub async fn history(&self, ctx: &Context) -> Result<History> {
        Ok(match self.select(ctx)? {
            Target::Local => History::Local(ctx.reader().await?),
            Target::Remote(gateway) => History::Remote(gateway),
        })
    }

    /// Print the answer to one read.
    async fn print(&self, ctx: &Context, request: ReadRequest) -> Result<ExitCode> {
        let answer = self.history(ctx).await?.serve(&request).await?;
        println!("{}", serde_json::to_string_pretty(&answer)?);
        Ok(ExitCode::SUCCESS)
    }

    /// The gateway to read through, for a command that needs the API even
    /// on this host: the local daemon's listener when no gateway is named.
    pub fn gateway(&self, ctx: &Context) -> Result<Management> {
        match self.select(ctx)? {
            Target::Remote(gateway) => Ok(gateway),
            Target::Local => Ok(Management::new(GatewayEndpoint::resolve(
                None,
                ctx.config()?.listen,
            )?)?),
        }
    }
}

/// List recent calls as JSON, newest first.
#[derive(Args)]
pub struct HistoryCmd {
    #[arg(long, default_value = "20")]
    limit: PageLimit,
    /// Only calls this named user made.
    #[arg(long)]
    user: Option<UserId>,
    #[command(flatten)]
    target: TargetArgs,
}
impl HistoryCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let request = ReadRequest::Calls(CallQuery {
            limit: self.limit,
            filter: CallFilter {
                user: self.user,
                ..CallFilter::default()
            },
            ..CallQuery::default()
        });
        self.target.print(ctx, request).await
    }
}

/// List sessions grouped by explicit client-provided identifiers.
#[derive(Args)]
pub struct SessionsCmd {
    #[arg(long, default_value = "20")]
    limit: PageLimit,
    /// `current`, or `archived`: the sessions archived and not active since.
    #[arg(long, default_value_t = Shelf::Current)]
    shelf: Shelf,
    #[command(flatten)]
    target: TargetArgs,
}
impl SessionsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        self.target
            .print(
                ctx,
                ReadRequest::Recordings {
                    limit: self.limit,
                    shelf: self.shelf,
                },
            )
            .await
    }
}

/// List a session's calls chronologically, with optional pagination.
#[derive(Args)]
pub struct TimelineCmd {
    id: RecordingId,
    #[arg(long, default_value = "100")]
    limit: PageLimit,
    /// The `next` cursor of the previous page.
    #[arg(long)]
    after: Option<Cursor>,
    #[command(flatten)]
    target: TargetArgs,
}
impl TimelineCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let request = ReadRequest::RecordingCalls {
            recording: self.id,
            limit: self.limit,
            after: self.after,
        };
        self.target.print(ctx, request).await
    }
}

/// List a session's turns, newest first, each with the start of its prompt.
#[derive(Args)]
pub struct TurnsCmd {
    id: RecordingId,
    #[arg(long, default_value = "20")]
    limit: PageLimit,
    #[command(flatten)]
    target: TargetArgs,
}
impl TurnsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let request = ReadRequest::Turns {
            recording: self.id,
            limit: self.limit,
        };
        self.target.print(ctx, request).await
    }
}

/// Search prompts, replies, and tool input and output, newest first. Words
/// must all match; "quoted text" matches as a phrase. The daemon indexes
/// calls as they finish.
#[derive(Args)]
pub struct SearchCmd {
    /// What to look for.
    #[arg(required = true, num_args = 1..)]
    text: Vec<String>,
    /// Only this Ren session.
    #[arg(long)]
    session: Option<RecordingId>,
    /// Only calls from this client.
    #[arg(long)]
    client: Option<Harness>,
    /// Only this part of a call.
    #[arg(long)]
    field: Option<SearchField>,
    #[arg(long, default_value = "20")]
    limit: PageLimit,
    #[command(flatten)]
    target: TargetArgs,
}
impl SearchCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let request = ReadRequest::Search(SearchQuery {
            terms: SearchTerms::parse(&self.text.join(" "))?,
            session: self.session,
            client: self.client,
            field: self.field,
            time: TimeRange::default(),
            limit: self.limit,
        });
        self.target.print(ctx, request).await
    }
}

/// List credentials calls carried (API keys, tokens, private keys), newest
/// first: their format, where they appeared, a masked hint, and a
/// fingerprint. The secrets themselves are never stored or shown.
#[derive(Args)]
pub struct ExposuresCmd {
    /// Only this Ren session.
    #[arg(long)]
    session: Option<RecordingId>,
    #[arg(long, default_value = "50")]
    limit: PageLimit,
    #[command(flatten)]
    target: TargetArgs,
}
impl ExposuresCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let request = ReadRequest::Exposures(ExposureQuery {
            session: self.session,
            time: TimeRange::default(),
            limit: self.limit,
        });
        self.target.print(ctx, request).await
    }
}

/// Report a session's context and cost: per turn, context size, prompt-cache
/// hits and writes, output, and idle time, with cache rebuilds and the
/// largest context growth. Token counts are provider-reported, not cost.
#[derive(Args)]
pub struct ReportCmd {
    id: RecordingId,
    #[command(flatten)]
    target: TargetArgs,
}
impl ReportCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        self.target.print(ctx, ReadRequest::Report(self.id)).await
    }
}

/// Show what a turn did: its prompt, tool calls and their results, and the
/// files and commands they touched, read from the recorded bodies.
#[derive(Args)]
pub struct TurnCmd {
    id: TurnId,
    #[command(flatten)]
    target: TargetArgs,
}
impl TurnCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        self.target.print(ctx, ReadRequest::Turn(self.id)).await
    }
}

/// Show what one call's bodies show: user input, tool calls, and reply.
#[derive(Args)]
pub struct ActivityCmd {
    id: CallId,
    #[command(flatten)]
    target: TargetArgs,
}
impl ActivityCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        self.target
            .print(ctx, ReadRequest::CallActivity(self.id))
            .await
    }
}

/// Inspect a call, or export one of its complete captured bodies to a new file.
#[derive(Args)]
pub struct InspectCmd {
    id: CallId,
    #[arg(long, value_enum, requires = "output")]
    body: Option<BodyDirection>,
    #[arg(long, requires = "body", value_name = "FILE")]
    output: Option<PathBuf>,
    #[command(flatten)]
    target: TargetArgs,
}
impl InspectCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let (Some(direction), Some(output)) = (self.body, self.output) else {
            return self.target.print(ctx, ReadRequest::Call(self.id)).await;
        };
        let target = self.target.select(ctx)?;
        let direction = Direction::from(direction);
        match target {
            Target::Local => {
                let reader = ctx.reader().await?;
                let key = reader.body_key(self.id, direction).await?;
                let mut export = Export::create(&output)?;
                let written = reader
                    .export(key, |bytes| export.file.write_all(bytes))
                    .await;
                export.finish(written.map_err(anyhow::Error::from))?;
            }
            Target::Remote(gateway) => {
                let mut export = Export::create(&output)?;
                let written = gateway
                    .export(self.id, direction, |bytes| export.file.write_all(bytes))
                    .await;
                export.finish(written.map_err(anyhow::Error::from))?;
            }
        }
        Ok(ExitCode::SUCCESS)
    }
}

/// A new, owner-only export file, removed unless the export completes.
struct Export<'a> {
    path: &'a Path,
    file: File,
}

impl<'a> Export<'a> {
    fn create(path: &'a Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .context("cannot create export file (existing files are not overwritten)")?;
        Ok(Self { path, file })
    }

    /// Never leave a partial export that looks complete.
    fn finish(self, written: Result<u64>) -> Result<()> {
        match written {
            Ok(_) => Ok(self.file.sync_all()?),
            Err(error) => {
                drop(self.file);
                let _ = std::fs::remove_file(self.path);
                Err(error)
            }
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum BodyDirection {
    Request,
    Response,
}
impl From<BodyDirection> for Direction {
    fn from(value: BodyDirection) -> Self {
        match value {
            BodyDirection::Request => Self::Request,
            BodyDirection::Response => Self::Response,
        }
    }
}
