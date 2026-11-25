//! `ren deployer`: deploying builds of Ren from a checkout, with
//! `ren-deployer`: production's install and upgrade, the canary and its
//! promotion, the previews, and the service the daemon asks.
use crate::context::Context;
use anyhow::Result;
use clap::{Args, Subcommand};
use ren_deployer::{
    canary::{Canary, ServeArgs},
    install::InstallVps,
    preview::Preview,
};
use std::{path::PathBuf, process::ExitCode};

/// Deploy builds of Ren: install production, run the canary and promote
/// it, and preview commits and pull requests.
#[derive(Args)]
pub struct DeployerCmd {
    /// The checkout to build from; this folder by default.
    #[arg(long, global = true, default_value = ".")]
    checkout: PathBuf,
    #[command(subcommand)]
    action: DeployerAction,
}

#[derive(Subcommand)]
enum DeployerAction {
    /// Install or upgrade Ren from the checkout as a systemd user service.
    InstallVps(InstallVps),
    /// Take up each ask of the daemon at once: previews, the canary, and
    /// promotions.
    Serve(ServeArgs),
    /// Build main's tip once, check it as the canary, and promote it.
    Canary(Canary),
    /// Preview this worktree's branch on a copy of production's history.
    Preview(Preview),
}

impl DeployerCmd {
    pub fn run(self, ctx: &Context) -> Result<ExitCode> {
        let checkout = std::path::absolute(&self.checkout)?;
        match self.action {
            DeployerAction::InstallVps(task) => task.run(&ctx.env, &checkout)?,
            DeployerAction::Serve(task) => task.run(&ctx.env, &checkout)?,
            DeployerAction::Canary(task) => task.run(&ctx.env, &checkout)?,
            DeployerAction::Preview(task) => task.run(&ctx.env, &checkout)?,
        }
        Ok(ExitCode::SUCCESS)
    }
}
