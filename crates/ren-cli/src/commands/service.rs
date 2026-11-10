use crate::context::Context;
use crate::service::{Definition, Manager, ServiceOptions};
use anyhow::Result;
use clap::{Args, Subcommand};
use std::{path::PathBuf, process::ExitCode};

/// Generate native user-service definitions; never install or start them.
#[derive(Args)]
pub struct ServiceCmd {
    #[command(subcommand)]
    command: ServiceCommand,
}
#[derive(Subcommand)]
enum ServiceCommand {
    Render(RenderCmd),
}
impl ServiceCmd {
    pub fn run(self, ctx: &Context) -> Result<ExitCode> {
        match self.command {
            ServiceCommand::Render(cmd) => cmd.run(ctx),
        }
    }
}

/// Render a launchd plist or systemd unit using fixed absolute paths.
#[derive(Args)]
struct RenderCmd {
    #[arg(long, value_enum)]
    manager: Manager,
    /// Installed executable path. Prefer a stable release binary.
    #[arg(long)]
    binary: PathBuf,
    /// Launchd log directory (default: <data-dir>/logs). Create before loading.
    #[arg(long)]
    log_dir: Option<PathBuf>,
    /// Launchd label (default: dev.ren.gateway).
    #[arg(long)]
    label: Option<String>,
    /// Write a new owner-only file; existing paths are never overwritten.
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,
}
impl RenderCmd {
    fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ctx.config()?;
        let data_dir = ctx.data_dir(&config)?;
        let definition = Definition::new(
            &config,
            ServiceOptions {
                manager: self.manager,
                config_path: ctx.config_path.as_deref(),
                data_dir: &data_dir,
                binary: &self.binary,
                log_dir: self.log_dir.as_deref(),
                label: self.label.as_deref(),
            },
        )?;
        let contents = definition.render();
        if let Some(path) = self.output {
            Definition::write_new(&path, &contents)?;
        } else {
            print!("{contents}");
        }
        Ok(ExitCode::SUCCESS)
    }
}
