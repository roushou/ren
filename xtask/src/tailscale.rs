//! What `tailscale status --json` and `tailscale serve status --json` report,
//! read only as far as publishing the gateway and the UI needs.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fmt};

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

    fn route(json: &str) -> Route {
        ServeStatus::parse(json).unwrap().route(8443, TARGET)
    }

    #[test]
    fn unused_serve_leaves_the_port_free() {
        assert_eq!(route(""), Route::Free);
        assert_eq!(route("null"), Route::Free);
        assert_eq!(route("{}"), Route::Free);
    }

    #[test]
    fn other_ports_do_not_hold_this_one() {
        let json = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"vps.t.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert_eq!(route(json), Route::Free);
    }

    #[test]
    fn a_proxy_to_the_gateway_is_ours() {
        let json = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787/"}}}}}"#;
        assert_eq!(route(json), Route::Target);
    }

    #[test]
    fn another_application_holds_the_port() {
        let json = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert_eq!(
            route(json),
            Route::Other {
                host_port: "vps.t.ts.net:8443".to_owned(),
                handlers: r#"{"/":{"Proxy":"http://127.0.0.1:3000"}}"#.to_owned(),
            }
        );
    }

    #[test]
    fn extra_handlers_or_fields_are_not_ours() {
        let paths = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"},"/ui":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert!(matches!(route(paths), Route::Other { .. }));
        let files = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787","Path":"/srv"}}}}}"#;
        assert!(matches!(route(files), Route::Other { .. }));
    }

    #[test]
    fn funnel_is_refused_even_to_the_gateway() {
        let json = r#"{"TCP":{"8443":{"HTTPS":true}},"Web":{"vps.t.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"}}}},"AllowFunnel":{"vps.t.ts.net:8443":true}}"#;
        assert_eq!(
            route(json),
            Route::Funnel {
                host_port: "vps.t.ts.net:8443".to_owned()
            }
        );
    }

    #[test]
    fn a_tcp_forward_holds_the_port() {
        assert_eq!(
            route(r#"{"TCP":{"8443":{"TCPForward":"127.0.0.1:22"}}}"#),
            Route::TcpForward
        );
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
