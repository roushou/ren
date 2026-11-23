//! Running `tailscale`: reading its status, publishing a loopback listener
//! on the tailnet with Serve, and turning a Serve port off. What to do with
//! a port is [`ServeStatus::route`]'s decision; a caller plans a
//! publication ([`Cli::plan`]), reports it, then applies it.

use super::{NodeStatus, Route, ServeStatus};
use std::process::{Command, ExitStatus, Stdio};

/// How to run `tailscale` for the Serve changes: as this user, or through
/// `sudo -n` for a user Tailscale does not take Serve changes from. Status
/// is always read as this user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cli {
    Direct,
    Sudo,
}

/// A `tailscale` command that could not start, failed, or printed what was
/// not understood.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("`{command}` could not start: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` exited with {status}{detail}")]
    Failed {
        command: String,
        status: ExitStatus,
        detail: String,
    },
    #[error("unexpected output from `{command}`: {detail}")]
    Output { command: String, detail: String },
}

/// Why a port was not published.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("Tailscale is {0}, not running")]
    Down(String),
    #[error("Tailscale Serve port {port} is in use ({route})")]
    PortTaken { port: u16, route: Route },
    #[error(transparent)]
    Cli(#[from] CliError),
}

/// What publishing does to a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Serve proxies nothing there yet: it will proxy to the target.
    Add,
    /// The port already proxies only to the target.
    Keep,
}

/// A port to publish on the tailnet, planned from what Tailscale reports.
#[derive(Debug)]
#[must_use]
pub struct Publication {
    port: u16,
    target: String,
    change: Change,
    dns_name: Option<String>,
}

impl Publication {
    pub fn change(&self) -> Change {
        self.change
    }

    /// The node's MagicDNS name, which the port is reached at.
    pub fn dns_name(&self) -> Option<&str> {
        self.dns_name.as_deref()
    }
}

impl Cli {
    /// `tailscale status --json`.
    pub fn node() -> Result<NodeStatus, CliError> {
        let args = ["status", "--json"];
        let text = Self::Direct.capture(&args)?;
        NodeStatus::parse(&text).map_err(|error| Self::Direct.unexpected(&args, error))
    }

    /// `tailscale serve status --json`.
    pub fn serve_status() -> Result<ServeStatus, CliError> {
        let args = ["serve", "status", "--json"];
        let text = Self::Direct.capture(&args)?;
        ServeStatus::parse(&text).map_err(|error| Self::Direct.unexpected(&args, error))
    }

    /// How to publish `target` (`http://127.0.0.1:8787`) privately on
    /// `port`, from what Tailscale reports; nothing changes yet. A port
    /// Serve already uses for anything else, or exposes through Funnel, is
    /// refused, never replaced.
    pub fn plan(port: u16, target: &str) -> Result<Publication, PublishError> {
        let node = Self::node()?;
        if let Some(state) = node.not_running() {
            return Err(PublishError::Down(state.to_owned()));
        }
        let change = match Self::serve_status()?.route(port, target) {
            Route::Free => Change::Add,
            Route::Target => Change::Keep,
            route @ (Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward) => {
                return Err(PublishError::PortTaken { port, route });
            }
        };
        Ok(Publication {
            port,
            target: target.to_owned(),
            change,
            dns_name: node.dns_name().map(str::to_owned),
        })
    }

    /// Carries `publication` out: `tailscale serve`, with its output on
    /// the terminal, for a port to add.
    pub fn apply(self, publication: &Publication) -> Result<(), CliError> {
        match publication.change {
            Change::Add => self.run(&[
                "serve",
                "--bg",
                &format!("--https={}", publication.port),
                &publication.target,
            ]),
            Change::Keep => Ok(()),
        }
    }

    /// Turns Serve off on `port`, whatever it proxies to: callers first
    /// check the route ([`ServeStatus::route`]).
    pub fn serve_off(self, port: u16) -> Result<(), CliError> {
        self.capture(&["serve", &format!("--https={port}"), "off"])
            .map(drop)
    }

    fn command(self, args: &[&str]) -> Command {
        let mut command = match self {
            Self::Direct => Command::new("tailscale"),
            Self::Sudo => {
                let mut sudo = Command::new("sudo");
                sudo.args(["-n", "tailscale"]);
                sudo
            }
        };
        command.args(args).stdin(Stdio::null());
        command
    }

    fn describe(self, args: &[&str]) -> String {
        let program: &[&str] = match self {
            Self::Direct => &["tailscale"],
            Self::Sudo => &["sudo", "-n", "tailscale"],
        };
        program
            .iter()
            .chain(args)
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn spawn_error(self, args: &[&str], source: std::io::Error) -> CliError {
        CliError::Spawn {
            command: self.describe(args),
            source,
        }
    }

    fn unexpected(self, args: &[&str], error: serde_json::Error) -> CliError {
        CliError::Output {
            command: self.describe(args),
            detail: error.to_string(),
        }
    }

    /// Runs with the terminal attached; fails unless it exits successfully.
    #[expect(
        clippy::disallowed_methods,
        reason = "ren-network is the one place Ren runs tailscale"
    )]
    fn run(self, args: &[&str]) -> Result<(), CliError> {
        let status = self
            .command(args)
            .status()
            .map_err(|source| self.spawn_error(args, source))?;
        if status.success() {
            Ok(())
        } else {
            Err(CliError::Failed {
                command: self.describe(args),
                status,
                detail: String::new(),
            })
        }
    }

    /// Its standard output; fails, with its standard error, unless it
    /// exits successfully.
    #[expect(
        clippy::disallowed_methods,
        reason = "ren-network is the one place Ren runs tailscale"
    )]
    fn capture(self, args: &[&str]) -> Result<String, CliError> {
        let output = self
            .command(args)
            .output()
            .map_err(|source| self.spawn_error(args, source))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            Err(CliError::Failed {
                command: self.describe(args),
                status: output.status,
                detail: if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                },
            })
        }
    }
}
