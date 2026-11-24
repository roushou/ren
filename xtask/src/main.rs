//! `cargo xtask`: repository tasks, run from a checkout.
mod builds;
mod canary;
mod compat;
mod daemon;
mod daemon_container;
mod dev;
mod file;
mod gvisor_service;
mod install;
mod invocation;
mod pod;
mod preview;
mod progress;
mod replica;
mod tailscale;
mod ui;
mod unit;

use clap::Parser;
use ren_harness::host::HostEnv;
use std::process::ExitCode;

/// Repository tasks for Ren.
#[derive(Parser)]
#[command(name = "cargo xtask", bin_name = "cargo xtask")]
enum Xtask {
    InstallVps(install::InstallVps),
    Dev(dev::Dev),
    Compat(compat::Compat),
    Canary(canary::Canary),
    Preview(preview::Preview),
}

/// Why a task failed.
#[derive(Debug, thiserror::Error)]
enum TaskError {
    #[error(transparent)]
    Install(#[from] install::InstallError),
    #[error(transparent)]
    Dev(#[from] dev::DevError),
    #[error(transparent)]
    Compat(#[from] compat::CompatError),
    #[error(transparent)]
    Canary(#[from] canary::CanaryError),
    #[error(transparent)]
    Preview(#[from] preview::PreviewError),
}

impl Xtask {
    fn run(self, env: &HostEnv) -> Result<(), TaskError> {
        match self {
            Self::InstallVps(task) => Ok(task.run(env)?),
            Self::Dev(task) => Ok(task.run(env)?),
            Self::Compat(task) => Ok(task.run()?),
            Self::Canary(task) => Ok(task.run(env)?),
            Self::Preview(task) => Ok(task.run(env)?),
        }
    }
}

fn main() -> ExitCode {
    let env = HostEnv::capture();
    match Xtask::parse().run(&env) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
