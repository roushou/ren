//! The native routes TGV forwards, as one table.
//!
//! Each route names its ingress path, the one method it accepts, and where it
//! goes. The router is built by interpreting this table; nothing else decides
//! which requests reach a provider.

use crate::upstream::UpstreamTable;
use tgv_harness::Harness;
use tgv_protocol::route::{Api, Endpoint, Method, Provider};
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

/// One forwarded endpoint of a provider's native API.
#[derive(Debug, Clone, Copy)]
pub struct Route {
    pub path: &'static str,
    pub accepts: Accepts,
    /// The provider whose native entry this route forwards to.
    pub provider: Provider,
    pub endpoint: Endpoint,
    /// The native path below the provider's base URL.
    pub upstream_path: &'static str,
    /// The harness whose correlation headers this route reads, if any.
    pub harness: Option<Harness>,
}

impl Route {
    /// This route bound to its provider's native entry in `upstreams`.
    pub fn bind(self, upstreams: &UpstreamTable) -> BoundRoute {
        let entry = upstreams.native(self.provider);
        BoundRoute {
            route: self,
            upstream: entry.id.clone(),
            api: entry.api,
        }
    }

    pub const ALL: [Self; 9] = [
        Self {
            path: "/anthropic/v1/messages",
            accepts: Accepts::Post,
            provider: Provider::Anthropic,
            endpoint: Endpoint::Messages,
            upstream_path: "/v1/messages",
            harness: Some(Harness::ClaudeCode),
        },
        Self {
            path: "/anthropic/v1/messages/count_tokens",
            accepts: Accepts::Post,
            provider: Provider::Anthropic,
            endpoint: Endpoint::CountTokens,
            upstream_path: "/v1/messages/count_tokens",
            harness: Some(Harness::ClaudeCode),
        },
        Self {
            path: "/anthropic/v1/models",
            accepts: Accepts::Get,
            provider: Provider::Anthropic,
            endpoint: Endpoint::Models,
            upstream_path: "/v1/models",
            harness: Some(Harness::ClaudeCode),
        },
        Self {
            path: "/codex/responses",
            accepts: Accepts::PostOrWebSocket,
            provider: Provider::Codex,
            endpoint: Endpoint::Responses,
            upstream_path: "/responses",
            harness: Some(Harness::Codex),
        },
        Self {
            path: "/codex/responses/compact",
            accepts: Accepts::Post,
            provider: Provider::Codex,
            endpoint: Endpoint::ResponsesCompact,
            upstream_path: "/responses/compact",
            harness: Some(Harness::Codex),
        },
        Self {
            path: "/codex/models",
            accepts: Accepts::Get,
            provider: Provider::Codex,
            endpoint: Endpoint::Models,
            upstream_path: "/models",
            harness: Some(Harness::Codex),
        },
        // Public Responses headers do not establish a client identity. Its
        // WebSocket mode is deferred (docs/architecture.md).
        Self {
            path: "/openai/v1/responses",
            accepts: Accepts::PostOrRefusedHandshake,
            provider: Provider::OpenAi,
            endpoint: Endpoint::Responses,
            upstream_path: "/v1/responses",
            harness: None,
        },
        Self {
            path: "/openai/v1/responses/compact",
            accepts: Accepts::Post,
            provider: Provider::OpenAi,
            endpoint: Endpoint::ResponsesCompact,
            upstream_path: "/v1/responses/compact",
            harness: None,
        },
        Self {
            path: "/openai/v1/models",
            accepts: Accepts::Get,
            provider: Provider::OpenAi,
            endpoint: Endpoint::Models,
            upstream_path: "/v1/models",
            harness: None,
        },
    ];
}

/// A route bound to the upstream entry its calls go to first, and that
/// entry's API. Handlers read the call's provider and first upstream here.
#[derive(Debug, Clone)]
pub struct BoundRoute {
    pub route: Route,
    pub upstream: UpstreamId,
    pub api: Api,
}

#[cfg(test)]
mod tests {
    use super::Route;

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
            assert!(route.path.starts_with('/') && route.upstream_path.starts_with('/'));
        }
    }
}
