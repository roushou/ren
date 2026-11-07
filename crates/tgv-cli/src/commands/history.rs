//! History commands. Each reads this host's storage, or, when a gateway is
//! selected, the same views through that gateway's management API; the JSON
//! printed is the same either way.
use crate::context::Context;
use crate::management::{HostFlags, Target};
use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};
use tgv_protocol::direction::Direction;
use tgv_recorder::id::{CallId, SessionId, TurnId};
use tgv_store::activity::ActivityBudget;
use tgv_store::query::{CallQuery, Cursor, PageLimit};

/// Which daemon a history or control command reads from.
#[derive(Args)]
pub struct TargetArgs {
    /// Gateway root URL whose API to use (default: this host when --config
    /// or --data-dir is given, else TGV_GATEWAY, then client.toml, then this
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
}

/// List recent calls as JSON, newest first.
#[derive(Args)]
pub struct HistoryCmd {
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
    #[command(flatten)]
    target: TargetArgs,
}
impl HistoryCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let limit = PageLimit::new(self.limit)?;
        match self.target.select(ctx)? {
            Target::Local => {
                let query = CallQuery {
                    limit,
                    ..CallQuery::default()
                };
                Self::print(&ctx.reader().await?.calls(&query).await?.calls)
            }
            Target::Remote(gateway) => Self::print(&gateway.calls(limit).await?),
        }
    }

    pub(crate) fn print(value: &impl Serialize) -> Result<ExitCode> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(ExitCode::SUCCESS)
    }
}

/// List sessions grouped by explicit client-provided identifiers.
#[derive(Args)]
pub struct SessionsCmd {
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
    #[command(flatten)]
    target: TargetArgs,
}
impl SessionsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let limit = PageLimit::new(self.limit)?;
        match self.target.select(ctx)? {
            Target::Local => HistoryCmd::print(&ctx.reader().await?.sessions(limit).await?),
            Target::Remote(gateway) => HistoryCmd::print(&gateway.sessions(limit).await?),
        }
    }
}

/// List a session's calls chronologically, with optional pagination.
#[derive(Args)]
pub struct TimelineCmd {
    id: SessionId,
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
    /// The `next` cursor of the previous page.
    #[arg(long)]
    after: Option<Cursor>,
    #[command(flatten)]
    target: TargetArgs,
}
impl TimelineCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let limit = PageLimit::new(self.limit)?;
        match self.target.select(ctx)? {
            Target::Local => HistoryCmd::print(
                &ctx.reader()
                    .await?
                    .session_calls(self.id, limit, self.after)
                    .await?,
            ),
            Target::Remote(gateway) => HistoryCmd::print(
                &gateway
                    .session_calls(self.id, limit, self.after.as_ref())
                    .await?,
            ),
        }
    }
}

/// List a session's turns, newest first, each with the start of its prompt.
#[derive(Args)]
pub struct TurnsCmd {
    id: SessionId,
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
    #[command(flatten)]
    target: TargetArgs,
}
impl TurnsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let limit = PageLimit::new(self.limit)?;
        match self.target.select(ctx)? {
            Target::Local => {
                HistoryCmd::print(&ctx.reader().await?.session_turns(self.id, limit).await?)
            }
            Target::Remote(gateway) => HistoryCmd::print(&gateway.turns(self.id, limit).await?),
        }
    }
}

/// Report a session's context and cost: per turn, context size, prompt-cache
/// hits and writes, output, and idle time, with cache rebuilds and the
/// largest context growth. Token counts are provider-reported, not cost.
#[derive(Args)]
pub struct ReportCmd {
    id: SessionId,
    #[command(flatten)]
    target: TargetArgs,
}
impl ReportCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        match self.target.select(ctx)? {
            Target::Local => HistoryCmd::print(&ctx.reader().await?.session_report(self.id).await?),
            Target::Remote(gateway) => HistoryCmd::print(&gateway.report(self.id).await?),
        }
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
        match self.target.select(ctx)? {
            Target::Local => HistoryCmd::print(
                &ctx.reader()
                    .await?
                    .turn_activity(self.id, ActivityBudget::DEFAULT)
                    .await?,
            ),
            Target::Remote(gateway) => HistoryCmd::print(&gateway.turn(self.id).await?),
        }
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
        match self.target.select(ctx)? {
            Target::Local => HistoryCmd::print(
                &ctx.reader()
                    .await?
                    .call_activity(self.id, ActivityBudget::DEFAULT)
                    .await?,
            ),
            Target::Remote(gateway) => HistoryCmd::print(&gateway.activity(self.id).await?),
        }
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
        let target = self.target.select(ctx)?;
        let (Some(direction), Some(output)) = (self.body, self.output) else {
            return match target {
                Target::Local => HistoryCmd::print(&ctx.reader().await?.call(self.id).await?),
                Target::Remote(gateway) => HistoryCmd::print(&gateway.call(self.id).await?),
            };
        };
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
