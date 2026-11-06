//! `cargo xtask`: repository tasks, run from a checkout.
mod daemon;
mod install;
mod invocation;
mod tailscale;

use clap::Parser;
use std::process::ExitCode;
use tgv_harness::host::HostEnv;

/// Repository tasks for TGV.
#[derive(Parser)]
#[command(name = "cargo xtask", bin_name = "cargo xtask")]
enum Xtask {
    InstallVps(install::InstallVps),
}

impl Xtask {
    fn run(self, env: &HostEnv) -> Result<(), install::InstallError> {
        match self {
            Self::InstallVps(task) => task.run(env),
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
