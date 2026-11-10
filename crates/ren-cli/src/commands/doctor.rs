use crate::{client::GatewayChoice, context::Context, doctor::Report};
use anyhow::Result;
use clap::Args;
use std::process::ExitCode;

/// Diagnose configuration, storage, and gateway health without inference.
#[derive(Args)]
pub struct DoctorCmd {
    /// Gateway root URL (default: REN_GATEWAY, then client.toml, then the
    /// configured local listener). A selected gateway skips local storage checks.
    #[arg(long)]
    gateway: Option<String>,
    #[arg(long)]
    json: bool,
}
impl DoctorCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let report = match GatewayChoice::select(self.gateway.as_deref(), &ctx.env) {
            Ok(gateway) => {
                Report::local(
                    &ctx.env,
                    ctx.config_path.as_deref(),
                    ctx.data_dir_override.clone(),
                    &gateway,
                )
                .await
            }
            Err(error) => Report::failed("gateway URL", error.to_string()),
        };
        report.print(self.json, ctx.terminal.stdout)?;
        Ok(if report.success() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        })
    }
}
