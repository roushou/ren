//! Gateway roots shared by diagnostics and session launchers, and the user
//! token a harness presents to the gateway.
use crate::host::HostEnv;
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

    /// The root URL as text, for saving and display.
    pub fn root(&self) -> &str {
        self.0.as_str()
    }

    pub fn url(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        url.set_path(path);
        url
    }
}

/// [`UserToken::ENV`] is set but holds no user token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("REN_USER_TOKEN is not a Ren user token; make one with `ren users token`")]
pub struct InvalidUserToken;

/// A user's secret token for the gateway, presented in [`Self::HEADER`].
/// Never shown in `Debug`. The gateway keeps only its digest.
#[derive(Clone)]
pub struct UserToken(Box<str>);

impl UserToken {
    /// The request header that carries a user token. It is in Ren's
    /// `x-ren-` namespace, which the gateway removes before forwarding.
    pub const HEADER: &'static str = "x-ren-user-token";
    /// The environment variable that holds a user token: for `ren run` and
    /// `ren doctor`, and for Codex and the Pi extension to send.
    pub const ENV: &'static str = "REN_USER_TOKEN";
    /// Marks the token's kind, so it reads as a Ren credential wherever it
    /// turns up.
    const PREFIX: &'static str = "renu_";

    /// The token in [`Self::ENV`], if it is set. Its value is never echoed.
    pub fn from_env(env: &HostEnv) -> Result<Option<Self>, InvalidUserToken> {
        env.get(Self::ENV)
            .filter(|value| !value.is_empty())
            .map(|value| Self::parse(value).ok_or(InvalidUserToken))
            .transpose()
    }

    /// A token from 256 bits the caller drew from its source of randomness.
    pub fn from_random(first: u128, second: u128) -> Self {
        Self(format!("{}{first:032x}{second:032x}", Self::PREFIX).into())
    }

    /// A token as a user presents it: the prefix, then ASCII letters and
    /// digits.
    pub fn parse(text: &str) -> Option<Self> {
        let rest = text.strip_prefix(Self::PREFIX)?;
        (!rest.is_empty() && rest.len() <= 128 && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
            .then(|| Self(text.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for UserToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserToken(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::{GatewayEndpoint, UserToken};
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
            "https://ren.example.ts.net",
        ] {
            assert!(GatewayEndpoint::parse(value).is_ok());
        }
    }

    #[test]
    fn user_tokens_parse_their_format_and_never_show_in_debug() {
        let token = UserToken::from_random(1, u128::MAX);
        assert_eq!(token.expose(), format!("renu_{:032x}{}", 1, "f".repeat(32)));
        assert_eq!(format!("{token:?}"), "UserToken(<redacted>)");
        assert!(UserToken::parse(token.expose()).is_some());
        for bad in ["", "renu_", "abc", "renu_a b", "Bearer renu_abc"] {
            assert!(UserToken::parse(bad).is_none(), "{bad:?}");
        }
    }
}
