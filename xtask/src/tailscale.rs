//! Publishing on the tailnet as xtask does it: `ren-network`'s Tailscale
//! [`Cli`], with progress lines and the refusals worded for xtask's flags.
use crate::progress::Progress;
use ren_network::tailscale::{Change, Cli, CliError, PublishError, Route};

/// Why a port could not be published on the tailnet.
#[derive(Debug, thiserror::Error)]
pub enum TailscaleError {
    #[error("Tailscale reports no MagicDNS name for this node, which the UI's origin needs")]
    NoDnsName,
    #[error(
        "Tailscale Serve did not publish port {port}. If it refused this user, run again with --sudo-tailscale (the choice is saved), or allow this user once with `sudo tailscale set --operator=$USER`"
    )]
    ServeRefused { port: u16 },
    #[error("Tailscale is {0}, not running: run `tailscale up` first")]
    Down(String),
    #[error("Tailscale Serve port {port} is in use ({route}); choose another --https-port")]
    PortTaken { port: u16, route: Route },
    #[error(transparent)]
    Cli(#[from] CliError),
}

impl From<PublishError> for TailscaleError {
    fn from(error: PublishError) -> Self {
        match error {
            PublishError::Down(state) => Self::Down(state),
            PublishError::PortTaken { port, route } => Self::PortTaken { port, route },
            PublishError::Cli(error) => Self::Cli(error),
        }
    }
}

/// The tailnet xtask publishes on, through `tailscale` run as `cli` says.
#[derive(Debug, Clone, Copy)]
pub struct Tailnet {
    cli: Cli,
}

impl Tailnet {
    pub fn new(cli: Cli) -> Self {
        Self { cli }
    }

    /// Publishes `target` privately on `port`, and returns the node's
    /// MagicDNS name.
    pub fn publish(self, port: u16, target: &str) -> Result<Option<String>, TailscaleError> {
        let publication = Cli::plan(port, target)?;
        match publication.change() {
            Change::Add => Progress(&format!(
                "Publishing {target} on the tailnet at port {port}"
            ))
            .show(),
            Change::Keep => Progress(&format!(
                "Tailscale Serve port {port} already proxies to {target}"
            ))
            .show(),
        }
        match (self.cli.apply(&publication), self.cli) {
            (Ok(()), Cli::Direct | Cli::Sudo) => {}
            // Tailscale refuses Serve changes from a user who is not its
            // operator; its own message is already on the terminal.
            (Err(CliError::Failed { .. }), Cli::Direct) => {
                return Err(TailscaleError::ServeRefused { port });
            }
            (Err(error), Cli::Direct | Cli::Sudo) => return Err(error.into()),
        }
        Ok(publication.dns_name().map(str::to_owned))
    }

    /// Removes each of `routes` (a port and the target it proxies to) only
    /// while it still proxies to that target; any other route is left.
    pub fn unpublish(self, routes: &[(u16, String)]) -> Result<(), TailscaleError> {
        let serve = Cli::serve_status()?;
        for (port, target) in routes {
            match serve.route(*port, target) {
                Route::Target => {
                    Progress(&format!("Unpublishing port {port}")).show();
                    self.cli.serve_off(*port)?;
                }
                Route::Free | Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward => {}
            }
        }
        Ok(())
    }
}
