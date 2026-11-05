//! TGV executable: argument parsing, logging setup, and exit handling.
use anyhow::Result;
use clap::Parser;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> Result<ExitCode> {
    // Dependency debug logs may include credential-bearing request URLs. Only
    // enable TGV targets here; transport internals are deliberately excluded.
    tracing_subscriber::fmt()
        .with_env_filter("tgv=info")
        .with_writer(std::io::stderr)
        .init();
    let env = tgv_harness::host::HostEnv::capture();
    tgv_cli::Cli::parse().run(env).await
}
