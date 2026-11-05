use crate::context::Context;
use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use std::{fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode};
use tgv_protocol::direction::Direction;
use tgv_recorder::id::{CallId, SessionId};
use tgv_store::query::{CallQuery, Cursor, PageLimit};

/// List recent calls as JSON, newest first.
#[derive(Args)]
pub struct HistoryCmd {
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
}
impl HistoryCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let query = CallQuery {
            limit: PageLimit::new(self.limit)?,
            ..CallQuery::default()
        };
        let page = ctx.reader().await?.calls(&query).await?;
        println!("{}", serde_json::to_string_pretty(&page.calls)?);
        Ok(ExitCode::SUCCESS)
    }
}

/// List sessions grouped by explicit client-provided identifiers.
#[derive(Args)]
pub struct SessionsCmd {
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    limit: u32,
}
impl SessionsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let sessions = ctx
            .reader()
            .await?
            .sessions(PageLimit::new(self.limit)?)
            .await?;
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        Ok(ExitCode::SUCCESS)
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
}
impl TimelineCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let page = ctx
            .reader()
            .await?
            .session_calls(self.id, PageLimit::new(self.limit)?, self.after)
            .await?;
        println!("{}", serde_json::to_string_pretty(&page)?);
        Ok(ExitCode::SUCCESS)
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
}
impl InspectCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let reader = ctx.reader().await?;
        if let (Some(direction), Some(output)) = (self.body, self.output) {
            let key = reader.body_key(self.id, direction.into()).await?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&output)
                .context("cannot create export file (existing files are not overwritten)")?;
            let exported = reader.export(key, |bytes| file.write_all(bytes)).await;
            if let Err(error) = exported {
                // Never leave a partial export that looks complete.
                drop(file);
                let _ = std::fs::remove_file(&output);
                return Err(error.into());
            }
            file.sync_all()?;
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&reader.call(self.id).await?)?
            );
        }
        Ok(ExitCode::SUCCESS)
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
