//! `cargo xtask`: repository tasks, run from a checkout.
mod compat;

use clap::Parser;
use std::process::ExitCode;

/// Repository tasks for Ren.
#[derive(Parser)]
#[command(name = "cargo xtask", bin_name = "cargo xtask")]
enum Xtask {
    Compat(compat::Compat),
}

/// Why a task failed.
#[derive(Debug, thiserror::Error)]
enum TaskError {
    #[error(transparent)]
    Compat(#[from] compat::CompatError),
}

impl Xtask {
    fn run(self) -> Result<(), TaskError> {
        match self {
            Self::Compat(task) => Ok(task.run()?),
        }
    }
}

fn main() -> ExitCode {
    match Xtask::parse().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
