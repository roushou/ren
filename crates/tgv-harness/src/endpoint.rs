//! Credential-free gateway roots shared by diagnostics and session launchers.
use std::net::{IpAddr, SocketAddr};
use url::Url;

/// A validated gateway root URL.
#[derive(Debug, Clone)]
pub struct GatewayEndpoint(Url);

/// Why a gateway URL was refused. The input is never echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayEndpointError {
    #[error("set --gateway to the daemon's actual address when listen uses port 0")]
    EphemeralListener,
    #[error("gateway URL must not contain whitespace or control characters")]
    Whitespace,
    #[error("invalid gateway URL")]
    Invalid,
    #[error("use a gateway root URL without credentials, query, fragment, or provider suffix")]
    NotRoot,
    #[error("gateway requires HTTPS, except HTTP on loopback with a nonzero port")]
    Insecure,
}

impl GatewayEndpoint {
    pub fn resolve(
        explicit: Option<&str>,
        listen: SocketAddr,
    ) -> Result<Self, GatewayEndpointError> {
        match explicit {
            Some(value) => Self::parse(value),
            None if listen.port() == 0 => Err(GatewayEndpointError::EphemeralListener),
            None => Self::parse(&format!("http://{listen}")),
        }
    }

    pub fn parse(value: &str) -> Result<Self, GatewayEndpointError> {
        // Reject inputs before URL normalization can hide whitespace or a suffix.
        if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(GatewayEndpointError::Whitespace);
        }
        let url = Url::parse(value).map_err(|_| GatewayEndpointError::Invalid)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(GatewayEndpointError::NotRoot);
        }
        let loopback = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if url.host_str().is_none()
            || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
            || url.port() == Some(0)
        {
            return Err(GatewayEndpointError::Insecure);
        }
        Ok(Self(url))
    }

    pub fn url(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        url.set_path(path);
        url
    }
}

#[cfg(test)]
mod tests {
    use super::GatewayEndpoint;
    #[test]
    fn validates_without_echoing_sensitive_inputs() {
        for value in [
            "http://example.com",
            "https://u:SECRET@example.com",
            "https://example.com/?SECRET",
            "https://example.com/#SECRET",
            "https://example.com/anthropic",
            "https://exa\nmple.com",
            "http://localhost:0",
        ] {
            let error = GatewayEndpoint::parse(value).err().unwrap();
            assert!(!format!("{error:?}").contains("SECRET"));
        }
        for value in [
            "http://127.0.0.1:8787",
            "http://[::1]:8787/",
            "https://tgv.example.ts.net",
        ] {
            assert!(GatewayEndpoint::parse(value).is_ok());
        }
    }
}
