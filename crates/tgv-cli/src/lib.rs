//! The `tgv` command line, with [`Cli`] as its primary type: arguments and
//! dispatch, where each command owns its argument struct and execution, plus
//! the doctor report and service definitions the commands produce.
mod commands;
mod context;
pub mod doctor;
pub mod service;

use crate::context::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode};
use tgv_harness::host::HostEnv;

#[derive(Parser)]
// The program is `tgv`, whatever the package is called: clap would otherwise
// print the package name (`tgv-cli`) in `--version`.
#[command(
    name = "tgv",
    version,
    about = "Personal subscription gateway and traffic recorder"
)]
pub struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Override the capture/history directory.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve(commands::serve::ServeCmd),
    Service(commands::service::ServiceCmd),
    Doctor(commands::doctor::DoctorCmd),
    Run(commands::run::RunCmd),
    History(commands::history::HistoryCmd),
    Sessions(commands::history::SessionsCmd),
    Timeline(commands::history::TimelineCmd),
    Inspect(commands::history::InspectCmd),
    Control(commands::control::ControlCmd),
}

impl Cli {
    pub async fn run(self, env: HostEnv) -> Result<ExitCode> {
        let ctx = Context::new(self.config, self.data_dir, env);
        match self.command {
            Command::Serve(cmd) => cmd.run(&ctx).await,
            Command::Service(cmd) => cmd.run(&ctx),
            Command::Doctor(cmd) => cmd.run(&ctx).await,
            Command::Run(cmd) => cmd.run(&ctx).await,
            Command::History(cmd) => cmd.run(&ctx).await,
            Command::Sessions(cmd) => cmd.run(&ctx).await,
            Command::Timeline(cmd) => cmd.run(&ctx).await,
            Command::Inspect(cmd) => cmd.run(&ctx).await,
            Command::Control(cmd) => cmd.run(&ctx).await,
        }
    }
}
