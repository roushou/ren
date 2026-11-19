//! `cargo xtask`: repository tasks, run from a checkout.
mod compat;
mod daemon;
mod dev;
mod file;
mod install;
mod invocation;
mod progress;
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
}

impl Xtask {
    fn run(self, env: &HostEnv) -> Result<(), TaskError> {
        match self {
            Self::InstallVps(task) => Ok(task.run(env)?),
            Self::Dev(task) => Ok(task.run(env)?),
            Self::Compat(task) => Ok(task.run()?),
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
