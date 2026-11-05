use crate::context::Context;
use crate::doctor;
use anyhow::Result;
use clap::{Args, ValueEnum};
use std::{ffi::OsString, process::ExitCode};
use tgv_harness::{Harness, endpoint::GatewayEndpoint};

/// Launch a client through TGV for this session without editing its files.
#[derive(Args)]
pub struct RunCmd {
    #[arg(value_enum)]
    client: Client,
    /// Gateway root URL (default: the configured local listener).
    #[arg(long)]
    gateway: Option<String>,
    /// Print only TGV's overrides as JSON; do not contact or launch anything.
    #[arg(long)]
    dry_run: bool,
    /// Client arguments, passed verbatim after --.
    #[arg(last = true)]
    args: Vec<OsString>,
}
/// The command-line names of the harnesses `run` can launch.
#[derive(Clone, Copy, ValueEnum)]
enum Client {
    Claude,
    Codex,
    Pi,
}

impl From<Client> for Harness {
    fn from(client: Client) -> Self {
        match client {
            Client::Claude => Self::ClaudeCode,
            Client::Codex => Self::Codex,
            Client::Pi => Self::Pi,
        }
    }
}

impl RunCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ctx.config()?;
        let gateway = GatewayEndpoint::resolve(self.gateway.as_deref(), config.listen)?;
        let plan = Harness::from(self.client).launch(&gateway);
        if self.dry_run {
            println!("{}", serde_json::to_string_pretty(&plan)?);
            return Ok(ExitCode::SUCCESS);
        }
        let report = doctor::Report::gateway(&gateway).await;
        for check in &report.checks {
            if check.status != doctor::Status::Ok {
                eprintln!("{}: {}", check.name, check.message);
            }
        }
        anyhow::ensure!(
            report.success(),
            "gateway preflight failed; run tgv doctor for details"
        );
        let overrides = doctor::Report::credential_overrides(&ctx.env);
        if !overrides.is_empty() {
            eprintln!(
                "Authentication overrides are set ({}); verify the client is using your intended subscription login.",
                overrides.join(", ")
            );
        }
        Ok(plan.execute(&self.args, &ctx.env)?)
    }
}
