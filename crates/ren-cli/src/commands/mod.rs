pub mod control;
pub mod credential;
pub mod deploy;
pub mod deployer;
pub mod doctor;
pub mod history;
pub mod init;
pub mod mcp;
pub mod packages;
pub mod project;
pub mod release;
pub mod run;
pub mod run_host;
pub mod runs;
pub mod seed;
pub mod serve;
pub mod service;
pub mod tail;
pub mod users;

use clap::ValueEnum;
use ren_harness::Harness;

/// The command-line names of the clients `run` launches and `runs start`
/// drives.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Client {
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
