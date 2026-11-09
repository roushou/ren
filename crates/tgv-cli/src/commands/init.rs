use crate::{
    client::ClientConfig,
    context::Context,
    doctor::Report,
    style::{Styling, Tone},
};
use anyhow::{Result, bail};
use clap::Args;
use std::{
    io::{BufRead as _, IsTerminal as _, Write as _},
    process::ExitCode,
};
use tgv_harness::endpoint::GatewayEndpoint;

/// Choose the gateway this machine's clients use, saved to client.toml.
///
/// Checks the gateway's health first and saves only if it passes. `run` and
/// `doctor` then use it unless --gateway or TGV_GATEWAY names another.
#[derive(Args)]
pub struct InitCmd {
    /// Gateway root URL, such as https://host.tailnet.ts.net:8443. Asked for
    /// when omitted on a terminal.
    #[arg(long)]
    gateway: Option<String>,
}

impl InitCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ClientConfig::load(ClientConfig::locate(&ctx.env)?)?;
        let value = match self.gateway {
            Some(value) => value,
            None => Self::ask(&config, ctx.terminal.stderr)?,
        };
        let gateway = GatewayEndpoint::parse(&value)?;
        let report = Report::gateway(&gateway, &ctx.env).await;
        report.print(false, ctx.terminal.stdout)?;
        if !report.success() {
            bail!("{} failed its check; nothing was saved", gateway.root());
        }
        let config = config.with_gateway(gateway);
        config.save()?;
        if let Some(gateway) = config.gateway() {
            println!(
                "{} {} to {}",
                ctx.terminal.stdout.paint(Tone::Good, "Saved gateway"),
                ctx.terminal.stdout.paint(Tone::Strong, gateway.root()),
                config.path().display()
            );
            println!("`tgv run` and `tgv doctor` now use it.");
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Asks on the terminal; an empty answer keeps the saved gateway.
    fn ask(config: &ClientConfig, stderr_style: Styling) -> Result<String> {
        let stdin = std::io::stdin();
        if !stdin.is_terminal() {
            bail!("no terminal to ask on: pass --gateway <url>");
        }
        let current = config.gateway().map(GatewayEndpoint::root);
        let mut stderr = std::io::stderr();
        let hint = match current {
            Some(current) => format!("[{current}]"),
            None => "(for example https://host.tailnet.ts.net:8443)".to_owned(),
        };
        write!(
            stderr,
            "{} {}: ",
            stderr_style.paint(Tone::Strong, "Gateway root URL"),
            stderr_style.paint(Tone::Muted, &hint)
        )?;
        stderr.flush()?;
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        match (line.trim(), current) {
            ("", Some(current)) => Ok(current.to_owned()),
            ("", None) => bail!("no gateway entered; nothing was saved"),
            (answer, _) => Ok(answer.to_owned()),
        }
    }
}
