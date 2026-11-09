//! `cargo xtask`: repository tasks, run from a checkout.
mod daemon;
mod dev;
mod install;
mod invocation;
mod publish;
mod tailscale;
mod ui;

use clap::Parser;
use std::process::ExitCode;
use tgv_harness::host::HostEnv;

/// Repository tasks for TGV.
#[derive(Parser)]
#[command(name = "cargo xtask", bin_name = "cargo xtask")]
enum Xtask {
    InstallVps(install::InstallVps),
    Dev(dev::Dev),
}

/// Why a task failed.
#[derive(Debug, thiserror::Error)]
enum TaskError {
    #[error(transparent)]
    Install(#[from] install::InstallError),
    #[error(transparent)]
    Dev(#[from] dev::DevError),
}

impl Xtask {
    fn run(self, env: &HostEnv) -> Result<(), TaskError> {
        match self {
            Self::InstallVps(task) => Ok(task.run(env)?),
            Self::Dev(task) => Ok(task.run()?),
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
