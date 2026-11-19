//! Tailscale as xtask runs it: publishing a local address on the tailnet
//! with Serve, and what `tailscale status --json` and `tailscale serve
//! status --json` report, read only as far as publishing the gateway and
//! the UI needs.
use crate::{
    invocation::{Invocation, InvocationError},
    progress::Progress,
};
pub use ren_daemon::tailscale::{NodeStatus, Route, ServeStatus};

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
    Invocation(#[from] InvocationError),
}

/// How to run `tailscale` for the Serve step.
#[derive(Debug, Clone, Copy)]
pub enum TailscaleCli {
    Direct,
    Sudo,
}

impl TailscaleCli {
    pub fn invocation(self) -> Invocation {
        match self {
            Self::Direct => Invocation::new("tailscale"),
            Self::Sudo => Invocation::new("sudo").args(["-n", "tailscale"]),
        }
    }

    /// Publishes `target` privately on `port`, and returns the node's
    /// MagicDNS name.
    pub fn publish(self, port: u16, target: &str) -> Result<Option<String>, TailscaleError> {
        let node = Invocation::new("tailscale")
            .args(["status", "--json"])
            .json("tailscale status --json", NodeStatus::parse)?;
        if let Some(state) = node.not_running() {
            return Err(TailscaleError::Down(state.to_owned()));
        }
        let serve = Invocation::new("tailscale")
            .args(["serve", "status", "--json"])
            .json("tailscale serve status --json", ServeStatus::parse)?;
        match serve.route(port, target) {
            Route::Free => {
                Progress(&format!(
                    "Publishing {target} on the tailnet at port {port}"
                ))
                .show();
                let served = self
                    .invocation()
                    .args(["serve", "--bg", &format!("--https={port}"), target])
                    .run();
                match (served, self) {
                    (Ok(()), Self::Direct | Self::Sudo) => {}
                    // Tailscale refuses Serve changes from a user who is not its
                    // operator; its own message is already on the terminal.
                    (Err(InvocationError::Failed { .. }), Self::Direct) => {
                        return Err(TailscaleError::ServeRefused { port });
                    }
                    (Err(error), Self::Direct | Self::Sudo) => return Err(error.into()),
                }
            }
            Route::Target => Progress(&format!(
                "Tailscale Serve port {port} already proxies to {target}"
            ))
            .show(),
            route @ (Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward) => {
                return Err(TailscaleError::PortTaken { port, route });
            }
        }
        Ok(node.dns_name().map(str::to_owned))
    }

    /// Removes each of `routes` (a port and the target it proxies to) only
    /// while it still proxies to that target; any other route is left.
    pub fn unpublish(self, routes: &[(u16, String)]) -> Result<(), TailscaleError> {
        let serve = Invocation::new("tailscale")
            .args(["serve", "status", "--json"])
            .json("tailscale serve status --json", ServeStatus::parse)?;
        for (port, target) in routes {
            match serve.route(*port, target) {
                Route::Target => {
                    Progress(&format!("Unpublishing port {port}")).show();
                    self.invocation()
                        .args(["serve", &format!("--https={port}"), "off"])
                        .run()?;
                }
                Route::Free | Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward => {}
            }
        }
        Ok(())
    }
}
