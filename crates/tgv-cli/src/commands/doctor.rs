use crate::context::Context;
use anyhow::Result;
use clap::Args;
use std::process::ExitCode;

/// Diagnose configuration, storage, and gateway health without inference.
#[derive(Args)]
pub struct DoctorCmd {
    /// Gateway root URL; skips local storage checks when supplied.
    #[arg(long)]
    gateway: Option<String>,
    #[arg(long)]
    json: bool,
}
impl DoctorCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let report = crate::doctor::Report::local(
            &ctx.env,
            ctx.config_path.as_deref(),
            ctx.data_dir_override.clone(),
            self.gateway.as_deref(),
        )
        .await;
        report.print(self.json)?;
        Ok(if report.success() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        })
    }
}
