//! Which headers cross the gateway, as data.
//!
//! A policy names headers and prefixes to remove; `apply` interprets it.
//! Credentials and provider feature headers pass through untouched: removing a
//! header is the exception, and each one is listed here with its reason.

use http::{HeaderMap, HeaderName};

/// Headers removed on one side of the gateway, beyond the hop-by-hop headers
/// (RFC 9110 §7.6.1) and the extension headers a `Connection` header names,
/// which describe one connection and are always removed.
#[derive(Debug, Clone, Copy)]
pub struct HeaderPolicy {
    /// Exact names to remove.
    names: &'static [&'static str],
    /// Name prefixes to remove.
    prefixes: &'static [&'static str],
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "proxy-connection",
];

impl HeaderPolicy {
    /// Client → provider.
    ///
    /// - `host`: the configured upstream owns it.
    /// - `forwarded`, `x-forwarded-`, `tailscale-`: reverse-proxy and tailnet
    ///   identity describe access to this server only.
    /// - `x-ren-`: Ren's local namespace (Pi attribution hints), never provider
    ///   identity. The whole namespace goes, not only the names Ren reads.
    pub const REQUEST: Self = Self {
        names: &["host", "forwarded"],
        prefixes: &["x-ren-", "tailscale-", "x-forwarded-"],
    };

    /// Provider → client: hop-by-hop headers only.
    pub const RESPONSE: Self = Self {
        names: &[],
        prefixes: &[],
    };

    /// Client → provider on a WebSocket handshake: [`Self::REQUEST`], plus the
    /// headers that negotiate one socket. The upstream leg sends its own
    /// version and key, and negotiates no extensions: compression is per
    /// connection, and the two legs are separate connections.
    pub const HANDSHAKE_REQUEST: Self = Self {
        names: &[
            "host",
            "forwarded",
            "sec-websocket-key",
            "sec-websocket-version",
            "sec-websocket-extensions",
        ],
        prefixes: &["x-ren-", "tailscale-", "x-forwarded-"],
    };

    /// Provider → client on an accepted handshake: the client leg answers its
    /// own key and negotiates no extensions.
    pub const HANDSHAKE_RESPONSE: Self = Self {
        names: &["sec-websocket-accept", "sec-websocket-extensions"],
        prefixes: &[],
    };

    pub fn apply(&self, headers: &mut HeaderMap) {
        let connection_named = headers
            .get_all(http::header::CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .filter_map(|name| name.trim().parse::<HeaderName>().ok());
        let prefixed = headers.keys().filter(|name| {
            self.prefixes
                .iter()
                .any(|prefix| name.as_str().starts_with(prefix))
        });
        let removed: Vec<HeaderName> = connection_named.chain(prefixed.cloned()).collect();
        for name in removed {
            headers.remove(name);
        }
        for name in HOP_BY_HOP.iter().chain(self.names) {
            headers.remove(*name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HeaderPolicy;
    use http::{HeaderMap, HeaderName};

    fn headers(names: &[&'static str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for name in names {
            headers.append(HeaderName::from_static(name), "value".parse().unwrap());
        }
        headers
    }

    fn remaining(headers: &HeaderMap) -> Vec<&str> {
        let mut names: Vec<_> = headers.keys().map(HeaderName::as_str).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn hop_by_hop_and_connection_extensions_go_but_provider_headers_stay() {
        let mut map = headers(&[
            "keep-alive",
            "x-private-hop",
            "x-another-hop",
            "transfer-encoding",
            "authorization",
            "anthropic-beta",
            "chatgpt-account-id",
        ]);
        map.append("connection", "keep-alive, X-Private-Hop".parse().unwrap());
        map.append("connection", "x-another-hop".parse().unwrap());
        for policy in [HeaderPolicy::REQUEST, HeaderPolicy::RESPONSE] {
            let mut map = map.clone();
            policy.apply(&mut map);
            assert_eq!(
                remaining(&map),
                ["anthropic-beta", "authorization", "chatgpt-account-id"]
            );
        }
    }

    #[test]
    fn requests_lose_the_whole_ren_namespace_and_proxy_identity() {
        let mut map = headers(&[
            "x-ren-pi-session-id",
            "x-ren-pi-turn-id",
            "x-ren-pi-future-hint",
            "x-ren-other",
            "x-renish",
            "tailscale-user-login",
            "x-forwarded-for",
            "forwarded",
            "host",
            "authorization",
        ]);
        HeaderPolicy::REQUEST.apply(&mut map);
        assert_eq!(remaining(&map), ["authorization", "x-renish"]);
    }

    #[test]
    fn responses_keep_provider_headers_that_requests_would_drop() {
        let mut map = headers(&["x-ren-other", "retry-after", "content-type"]);
        HeaderPolicy::RESPONSE.apply(&mut map);
        assert_eq!(
            remaining(&map),
            ["content-type", "retry-after", "x-ren-other"]
        );
    }
}
