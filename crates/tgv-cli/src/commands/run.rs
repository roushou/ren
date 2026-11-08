use crate::context::Context;
use crate::doctor;
use crate::{client::GatewayChoice, style::Tone};
use anyhow::Result;
use clap::{Args, ValueEnum};
use std::{ffi::OsString, process::ExitCode};
use tgv_harness::Harness;

/// Launch a client through TGV for this session without editing its files.
#[derive(Args)]
pub struct RunCmd {
    #[arg(value_enum)]
    client: Client,
    /// Gateway root URL (default: TGV_GATEWAY, then client.toml, then the
    /// configured local listener).
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
    Opencode,
}

impl From<Client> for Harness {
    fn from(client: Client) -> Self {
        match client {
            Client::Claude => Self::ClaudeCode,
            Client::Codex => Self::Codex,
            Client::Pi => Self::Pi,
            Client::Opencode => Self::OpenCode,
        }
    }
}

impl RunCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ctx.config()?;
        let choice = GatewayChoice::select(self.gateway.as_deref(), &ctx.env)?;
        let gateway = choice.endpoint(config.listen)?;
        let plan = Harness::from(self.client).launch(&gateway);
        if self.dry_run {
            println!("{}", serde_json::to_string_pretty(&plan)?);
            return Ok(ExitCode::SUCCESS);
        }
        let report = doctor::Report::gateway(&gateway).await;
        let stderr = ctx.terminal.stderr;
        for check in &report.checks {
            if check.status != doctor::Status::Ok {
                let name = format!("{}:", check.name);
                eprintln!(
                    "{} {}",
                    stderr.paint(check.status.tone(), &name),
                    check.message
                );
            }
        }
        if !report.success() {
            match choice {
                GatewayChoice::Selected { source, .. } => {
                    anyhow::bail!(
                        "gateway preflight failed for {} (from {source}); run tgv doctor for details",
                        gateway.root()
                    )
                }
                GatewayChoice::LocalListener => anyhow::bail!(
                    "gateway preflight failed for the local daemon at {}; to use a remote gateway, run `tgv init` or pass --gateway",
                    gateway.root()
                ),
            }
        }
        let overrides = doctor::Report::credential_overrides(&ctx.env);
        if !overrides.is_empty() {
            eprintln!(
                "{} authentication overrides are set ({}); verify the client is using your intended subscription login.",
                stderr.paint(Tone::Warn, "warning:"),
                overrides.join(", ")
            );
        }
        Ok(plan.execute(&self.args, &ctx.env)?)
    }
}
