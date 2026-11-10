//! Ren executable: argument parsing, logging setup, and exit handling.
use clap::Parser;
use ren_cli::style::Terminal;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let env = ren_harness::host::HostEnv::capture();
    let terminal = Terminal::detect(&env);
    // Dependency debug logs may include credential-bearing request URLs. Only
    // enable Ren targets here; transport internals are deliberately excluded.
    tracing_subscriber::fmt()
        .with_env_filter("ren=info")
        .with_writer(std::io::stderr)
        .with_ansi(terminal.stderr.is_color())
        .init();
    ren_cli::Cli::parse().run(env, terminal).await
}
