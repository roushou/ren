//! Tailscale as xtask runs it: publishing a local address on the tailnet
//! with Serve, and what `tailscale status --json` and `tailscale serve
//! status --json` report, read only as far as publishing the gateway and
//! the UI needs.
use crate::{
    invocation::{Invocation, InvocationError},
    progress::Progress,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fmt};

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

/// The node's connection state and name (`tailscale status --json`).
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NodeStatus {
    backend_state: BackendState,
    #[serde(rename = "Self")]
    node: Option<Node>,
}

#[derive(Deserialize)]
#[serde(from = "String")]
enum BackendState {
    Running,
    Other(String),
}

impl From<String> for BackendState {
    fn from(text: String) -> Self {
        match text.as_str() {
            "Running" => Self::Running,
            _ => Self::Other(text),
        }
    }
}

#[derive(Deserialize)]
struct Node {
    #[serde(rename = "DNSName", default)]
    dns_name: String,
}

impl NodeStatus {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The backend state, unless the node is running.
    pub fn not_running(&self) -> Option<&str> {
        match &self.backend_state {
            BackendState::Running => None,
            BackendState::Other(state) => Some(state),
        }
    }

    /// The node's MagicDNS name, without the trailing dot.
    pub fn dns_name(&self) -> Option<&str> {
        self.node
            .as_ref()
            .map(|node| node.dns_name.trim_end_matches('.'))
            .filter(|name| !name.is_empty())
    }
}

/// The node's Serve configuration (`tailscale serve status --json`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct ServeStatus {
    #[serde(rename = "TCP")]
    tcp: BTreeMap<String, Value>,
    web: BTreeMap<String, WebServer>,
    allow_funnel: BTreeMap<String, bool>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct WebServer {
    handlers: BTreeMap<String, Handler>,
}

/// A Serve handler. Fields other than the proxy target are kept to report
/// what holds a port.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Handler {
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

/// How a Serve HTTPS port relates to a local listener it should proxy to.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Nothing listens on the port.
    Free,
    /// The port already proxies only to the target, privately.
    Target,
    /// The port is open to the internet through Funnel.
    Funnel { host_port: String },
    /// The port serves something else.
    Other { host_port: String, handlers: String },
    /// The port is a raw TCP forward.
    TcpForward,
}

impl ServeStatus {
    /// `tailscale serve status --json` prints nothing, `{}`, or `null` when
    /// Serve is unused.
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str::<Option<Self>>(json).map(Option::unwrap_or_default)
    }

    /// How `port` relates to a listener at `target`
    /// (`http://127.0.0.1:8787`).
    pub fn route(&self, port: u16, target: &str) -> Route {
        let port = port.to_string();
        if !self.tcp.contains_key(&port) {
            return Route::Free;
        }
        let Some((host_port, server)) = self
            .web
            .iter()
            .find(|(host_port, _)| host_port.rsplit(':').next() == Some(port.as_str()))
        else {
            return Route::TcpForward;
        };
        if self.allow_funnel.get(host_port).copied().unwrap_or(false) {
            return Route::Funnel {
                host_port: host_port.clone(),
            };
        }
        let ours = server.handlers.len() == 1
            && server.handlers.get("/").is_some_and(|handler| {
                handler.other.is_empty()
                    && handler
                        .proxy
                        .as_deref()
                        .is_some_and(|proxy| proxy.trim_end_matches('/') == target)
            });
        if ours {
            Route::Target
        } else {
            Route::Other {
                host_port: host_port.clone(),
                handlers: serde_json::to_string(&server.handlers)
                    .unwrap_or_else(|_| "unreadable handlers".to_owned()),
            }
        }
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Free => f.write_str("free"),
            Self::Target => f.write_str("proxies to the target"),
            Self::Funnel { host_port } => write!(f, "{host_port} is public through Funnel"),
            Self::Other {
                host_port,
                handlers,
            } => write!(f, "{host_port} serves {handlers}"),
            Self::TcpForward => f.write_str("a TCP forward"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NodeStatus, Route, ServeStatus};

    const TARGET: &str = "http://127.0.0.1:8787";

    #[test]
    fn serve_status_says_how_the_port_is_routed() {
        let other = |handlers: &str| Route::Other {
            host_port: "vps.t.ts.net:8443".to_owned(),
            handlers: handlers.to_owned(),
        };
        let cases = [
            ("serve unused, no output", "", Route::Free),
            ("serve unused, null", "null", Route::Free),
            ("serve unused, empty object", "{}", Route::Free),
            (
                "other ports do not hold this one",
                r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"vps.t.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#,
                Route::Free,
            ),
            (
                "a proxy to the gateway is ours",
                r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787/"}}}}}"#,
                Route::Target,
            ),
            (
                "another application holds the port",
                r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#,
                other(r#"{"/":{"Proxy":"http://127.0.0.1:3000"}}"#),
            ),
            (
                "an extra handler is not ours",
                r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"},"/ui":{"Proxy":"http://127.0.0.1:3000"}}}}}"#,
                other(
                    r#"{"/":{"Proxy":"http://127.0.0.1:8787"},"/ui":{"Proxy":"http://127.0.0.1:3000"}}"#,
                ),
            ),
            (
                "an extra handler field is not ours",
                r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787","Path":"/srv"}}}}}"#,
                other(r#"{"/":{"Proxy":"http://127.0.0.1:8787","Path":"/srv"}}"#),
            ),
            (
                "funnel is refused even to the gateway",
                r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"}}}},"AllowFunnel":{"vps.t.ts.net:8443":true}}"#,
                Route::Funnel {
                    host_port: "vps.t.ts.net:8443".to_owned(),
                },
            ),
            (
                "a TCP forward holds the port",
                r#"{"TCP":{"8443":{"TCPForward":"127.0.0.1:22"}}}"#,
                Route::TcpForward,
            ),
        ];
        for (case, json, expected) in cases {
            let status = ServeStatus::parse(json)
                .unwrap_or_else(|error| panic!("{case}: unparsed: {error}"));
            assert_eq!(status.route(8443, TARGET), expected, "{case}");
        }
    }

    #[test]
    fn node_status_reads_state_and_name() {
        let up = NodeStatus::parse(
            r#"{"BackendState":"Running","Self":{"DNSName":"vps.t.ts.net.","ID":"n1"},"Peer":{}}"#,
        )
        .unwrap();
        assert_eq!(up.not_running(), None);
        assert_eq!(up.dns_name(), Some("vps.t.ts.net"));
        let down = NodeStatus::parse(r#"{"BackendState":"NeedsLogin","Self":null}"#).unwrap();
        assert_eq!(down.not_running(), Some("NeedsLogin"));
        assert_eq!(down.dns_name(), None);
    }
}
