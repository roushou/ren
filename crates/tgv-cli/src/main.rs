//! TGV executable: argument parsing, logging setup, and exit handling.
use clap::Parser;
use std::process::ExitCode;
use tgv_cli::style::Terminal;

#[tokio::main]
async fn main() -> ExitCode {
    let env = tgv_harness::host::HostEnv::capture();
    let terminal = Terminal::detect(&env);
    // Dependency debug logs may include credential-bearing request URLs. Only
    // enable TGV targets here; transport internals are deliberately excluded.
    tracing_subscriber::fmt()
        .with_env_filter("tgv=info")
        .with_writer(std::io::stderr)
        .with_ansi(terminal.stderr.is_color())
        .init();
    tgv_cli::Cli::parse().run(env, terminal).await
}
