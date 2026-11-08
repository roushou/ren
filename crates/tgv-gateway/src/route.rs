//! The native routes TGV forwards, as one table.
//!
//! Each route names its ingress path, the one method it accepts, and where it
//! goes. The router is built by interpreting this table; nothing else decides
//! which requests reach a provider.

use tgv_harness::Harness;
use tgv_protocol::route::{Api, Endpoint, Method, Provider, Wire};
use tgv_recorder::routing::UpstreamId;

/// What a route accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepts {
    Post,
    Get,
    /// POST, and a WebSocket handshake answered with `426 Upgrade Required`:
    /// a client that supports it falls back to HTTP/SSE.
    PostOrRefusedHandshake,
    /// POST, and a WebSocket handshake relayed to the upstream (Codex), unless
    /// the gateway's WebSocket mode refuses it.
    PostOrWebSocket,
}

impl Accepts {
    /// The HTTP method of a forwarded request on this route.
    pub fn method(self) -> Method {
        match self {
            Self::Post | Self::PostOrRefusedHandshake | Self::PostOrWebSocket => Method::Post,
            Self::Get => Method::Get,
        }
    }
}

/// How one native endpoint is forwarded, wherever its route is mounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Forward {
    pub accepts: Accepts,
    pub endpoint: Endpoint,
    /// The native path below the entry's base URL.
    pub upstream_path: &'static str,
    /// The harness whose correlation headers this route reads, if any.
    pub harness: Option<Harness>,
}

impl Forward {
    /// The native endpoints of each wire format, below an entry's base URL:
    /// the routes a configured entry with direct ingress mounts below
    /// [`Route::DIRECT_PREFIX`]. Messages routes read Claude Code's headers;
    /// public Responses headers establish no client identity, and its
    /// WebSocket mode is refused, as on `/openai`.
    pub const BY_WIRE: [(Wire, Self); 6] = [
        (
            Wire::AnthropicMessages,
            Self {
                accepts: Accepts::Post,
                endpoint: Endpoint::Messages,
                upstream_path: "/v1/messages",
                harness: Some(Harness::ClaudeCode),
            },
        ),
        (
            Wire::AnthropicMessages,
            Self {
                accepts: Accepts::Post,
                endpoint: Endpoint::CountTokens,
                upstream_path: "/v1/messages/count_tokens",
                harness: Some(Harness::ClaudeCode),
            },
        ),
        (
            Wire::AnthropicMessages,
            Self {
                accepts: Accepts::Get,
                endpoint: Endpoint::Models,
                upstream_path: "/v1/models",
                harness: Some(Harness::ClaudeCode),
            },
        ),
        (
            Wire::OpenAiResponses,
            Self {
                accepts: Accepts::PostOrRefusedHandshake,
                endpoint: Endpoint::Responses,
                upstream_path: "/v1/responses",
                harness: None,
            },
        ),
        (
            Wire::OpenAiResponses,
            Self {
                accepts: Accepts::Post,
                endpoint: Endpoint::ResponsesCompact,
                upstream_path: "/v1/responses/compact",
                harness: None,
            },
        ),
        (
            Wire::OpenAiResponses,
            Self {
                accepts: Accepts::Get,
                endpoint: Endpoint::Models,
                upstream_path: "/v1/models",
                harness: None,
            },
        ),
    ];
}

/// One forwarded endpoint of a provider's native entry.
#[derive(Debug, Clone, Copy)]
pub struct Route {
    pub path: &'static str,
    /// The provider whose native entry this route forwards to.
    pub provider: Provider,
    pub forward: Forward,
}

impl Route {
    /// Where configured entries with direct ingress are mounted:
    /// `/upstreams/{id}`, then their wire format's native paths.
    pub const DIRECT_PREFIX: &'static str = "/upstreams";

    pub const ALL: [Self; 9] = [
        Self {
            path: "/anthropic/v1/messages",
            provider: Provider::Anthropic,
            forward: Forward {
                accepts: Accepts::Post,
                endpoint: Endpoint::Messages,
                upstream_path: "/v1/messages",
                harness: Some(Harness::ClaudeCode),
            },
        },
        Self {
            path: "/anthropic/v1/messages/count_tokens",
            provider: Provider::Anthropic,
            forward: Forward {
                accepts: Accepts::Post,
                endpoint: Endpoint::CountTokens,
                upstream_path: "/v1/messages/count_tokens",
                harness: Some(Harness::ClaudeCode),
            },
        },
        Self {
            path: "/anthropic/v1/models",
            provider: Provider::Anthropic,
            forward: Forward {
                accepts: Accepts::Get,
                endpoint: Endpoint::Models,
                upstream_path: "/v1/models",
                harness: Some(Harness::ClaudeCode),
            },
        },
        Self {
            path: "/codex/responses",
            provider: Provider::Codex,
            forward: Forward {
                accepts: Accepts::PostOrWebSocket,
                endpoint: Endpoint::Responses,
                upstream_path: "/responses",
                harness: Some(Harness::Codex),
            },
        },
        Self {
            path: "/codex/responses/compact",
            provider: Provider::Codex,
            forward: Forward {
                accepts: Accepts::Post,
                endpoint: Endpoint::ResponsesCompact,
                upstream_path: "/responses/compact",
                harness: Some(Harness::Codex),
            },
        },
        Self {
            path: "/codex/models",
            provider: Provider::Codex,
            forward: Forward {
                accepts: Accepts::Get,
                endpoint: Endpoint::Models,
                upstream_path: "/models",
                harness: Some(Harness::Codex),
            },
        },
        // Public Responses headers do not establish a client identity. Its
        // WebSocket mode is deferred (docs/architecture.md).
        Self {
            path: "/openai/v1/responses",
            provider: Provider::OpenAi,
            forward: Forward {
                accepts: Accepts::PostOrRefusedHandshake,
                endpoint: Endpoint::Responses,
                upstream_path: "/v1/responses",
                harness: None,
            },
        },
        Self {
            path: "/openai/v1/responses/compact",
            provider: Provider::OpenAi,
            forward: Forward {
                accepts: Accepts::Post,
                endpoint: Endpoint::ResponsesCompact,
                upstream_path: "/v1/responses/compact",
                harness: None,
            },
        },
        Self {
            path: "/openai/v1/models",
            provider: Provider::OpenAi,
            forward: Forward {
                accepts: Accepts::Get,
                endpoint: Endpoint::Models,
                upstream_path: "/v1/models",
                harness: None,
            },
        },
    ];
}

/// A route as the router mounts it: its ingress path, how it forwards, the
/// upstream entry its calls go to first, and that entry's API. Handlers read
/// the call's provider and first upstream here.
#[derive(Debug, Clone)]
pub struct BoundRoute {
    pub path: Box<str>,
    pub forward: Forward,
    pub upstream: UpstreamId,
    pub api: Api,
}

#[cfg(test)]
mod tests {
    use super::{Forward, Route};
    use tgv_protocol::{label::Labeled, route::Provider};

    #[test]
    fn a_wire_mounts_the_routes_its_providers_own_entry_has() {
        // Codex's native paths are the ChatGPT backend's own layout, not the
        // public Responses one.
        for provider in [Provider::Anthropic, Provider::OpenAi] {
            let wire = provider.native_api().wire();
            let native: Vec<(String, Forward)> = Route::ALL
                .iter()
                .filter(|route| route.provider == provider)
                .map(|route| (route.path.to_owned(), route.forward))
                .collect();
            let by_wire: Vec<(String, Forward)> = Forward::BY_WIRE
                .iter()
                .filter(|(of, _)| *of == wire)
                .map(|(_, forward)| {
                    (
                        format!("/{}{}", provider.label(), forward.upstream_path),
                        *forward,
                    )
                })
                .collect();
            assert_eq!(native, by_wire, "{provider}");
        }
    }

    #[test]
    fn every_ingress_path_is_routed_once() {
        for (index, route) in Route::ALL.iter().enumerate() {
            assert!(
                Route::ALL[..index]
                    .iter()
                    .all(|earlier| earlier.path != route.path),
                "{} is routed twice",
                route.path
            );
            assert!(route.path.starts_with('/') && route.forward.upstream_path.starts_with('/'));
        }
    }
}
