//! Tailscale: the login Serve names a request's caller by
//! (`Tailscale-User-Login`), the node's state and name (`tailscale status
//! --json`), and how a Serve port is routed (`tailscale serve status
//! --json`), read only as far as publishing a local listener on the tailnet
//! needs; and running `tailscale` for them ([`Cli`]): xtask publishes the
//! gateway, the UI, the canary, and previews; `ren project preview` a
//! project's.

mod cli;
pub use cli::{Change, Cli, CliError, Publication, PublishError};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::DeserializeFromStr;
use std::{collections::BTreeMap, fmt};

/// A tailnet login name, as Tailscale Serve reports it in
/// `Tailscale-User-Login`: 1 to 256 visible ASCII characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, DeserializeFromStr)]
#[serde(transparent)]
pub struct Login(Box<str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Tailscale logins are 1 to 256 visible ASCII characters")]
pub struct InvalidLogin;

impl Login {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Login {
    type Err = InvalidLogin;

    fn from_str(login: &str) -> Result<Self, Self::Err> {
        if (1..=256).contains(&login.len()) && login.bytes().all(|byte| byte.is_ascii_graphic()) {
            Ok(Self(login.into()))
        } else {
            Err(InvalidLogin)
        }
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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
    use super::{Login, NodeStatus, Route, ServeStatus};

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

    #[test]
    fn logins_parse_only_their_grammar() {
        assert!("alice@example.com".parse::<Login>().is_ok());
        for bad in ["", "a b", "é", &"a".repeat(257)] {
            assert!(bad.parse::<Login>().is_err(), "{bad:?}");
        }
    }
}
