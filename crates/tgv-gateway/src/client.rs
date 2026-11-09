//! Who is calling: the configured clients, and how a forwarded request is
//! matched to one.
//!
//! A request names its client by a Tailscale login, which Tailscale Serve
//! adds after removing any copy the client sent, or by a client token in
//! [`Clients::TOKEN_HEADER`]. The token header is in TGV's `x-tgv-`
//! namespace, which the header policy removes before forwarding, so it never
//! reaches a provider. Configuration holds only each token's BLAKE3 digest.
//!
//! A process on this host can reach the loopback listener directly and set
//! any header, including `Tailscale-User-Login`; identity guards against
//! other machines, not against this one.

use http::HeaderMap;
use std::collections::HashMap;
use tgv_recorder::{
    client::{Caller, ClientId, Proof, TailscaleLogin},
    id::IdSource,
};

/// A client's secret token. Never shown in `Debug`.
#[derive(Clone)]
pub struct ClientToken(Box<str>);

impl ClientToken {
    /// Marks the token's kind, so it reads as a TGV credential wherever it
    /// turns up.
    const PREFIX: &'static str = "tgvc_";

    pub fn generate<S: IdSource + ?Sized>(ids: &S) -> Self {
        Self(
            format!(
                "{}{}{}",
                Self::PREFIX,
                ids.next_uuid().simple(),
                ids.next_uuid().simple()
            )
            .into(),
        )
    }

    /// A token as a client presents it: the prefix, then ASCII letters and
    /// digits.
    pub fn parse(text: &str) -> Option<Self> {
        let rest = text.strip_prefix(Self::PREFIX)?;
        (!rest.is_empty() && rest.len() <= 128 && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
            .then(|| Self(text.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn digest(&self) -> TokenDigest {
        TokenDigest(*blake3::hash(self.0.as_bytes()).as_bytes())
    }
}

impl std::fmt::Debug for ClientToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientToken(<redacted>)")
    }
}

/// The BLAKE3 digest of a client token, as configuration holds it: 64
/// lowercase hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenDigest([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("token digests are 64 lowercase hexadecimal digits")]
pub struct InvalidTokenDigest;

impl TokenDigest {
    pub fn parse(text: &str) -> Result<Self, InvalidTokenDigest> {
        let digits = text.as_bytes();
        if digits.len() != 64 {
            return Err(InvalidTokenDigest);
        }
        let mut bytes = [0_u8; 32];
        let (pairs, _) = digits.as_chunks::<2>();
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = (Self::nibble(*high)? << 4) | Self::nibble(*low)?;
        }
        Ok(Self(bytes))
    }

    fn nibble(digit: u8) -> Result<u8, InvalidTokenDigest> {
        match digit {
            b'0'..=b'9' => Ok(digit - b'0'),
            b'a'..=b'f' => Ok(digit - b'a' + 10),
            _ => Err(InvalidTokenDigest),
        }
    }
}

impl std::fmt::Display for TokenDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl std::fmt::Debug for TokenDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TokenDigest({self})")
    }
}

impl serde::Serialize for TokenDigest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for TokenDigest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// One `[[clients]]` entry: a client's name and how it proves it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub id: ClientId,
    /// Tailscale logins that are this client.
    #[serde(default)]
    pub tailscale_logins: Vec<TailscaleLogin>,
    /// The BLAKE3 digest of the client's token (`tgv client token`).
    #[serde(default)]
    pub token_blake3: Option<TokenDigest>,
}

/// Why the configured clients are not a valid set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientsError {
    #[error("client {0} is configured twice")]
    DuplicateId(ClientId),
    #[error("Tailscale login {0} belongs to two clients")]
    DuplicateLogin(TailscaleLogin),
    #[error("clients {0} and {1} have the same token")]
    DuplicateToken(ClientId, ClientId),
    #[error("client {0} has neither a Tailscale login nor a token, so no request can be it")]
    NoProof(ClientId),
}

/// Why a request's identity was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentifyError {
    /// A token was presented that no client has. It is refused rather than
    /// ignored, so a client never believes it is identified when it is not.
    #[error("the client token is not one this gateway knows")]
    UnknownToken,
    #[error("the request carries more than one client token")]
    SeveralTokens,
    #[error("the Tailscale login is malformed")]
    MalformedLogin,
    /// The Tailscale login and the token name different clients.
    #[error("the Tailscale login is client {login}, but the token is client {token}")]
    Conflicting { login: ClientId, token: ClientId },
}

/// The configured clients, indexed by what proves each one.
#[derive(Debug, Clone, Default)]
pub struct Clients {
    configs: Vec<ClientConfig>,
    by_login: HashMap<TailscaleLogin, ClientId>,
    by_token: HashMap<TokenDigest, ClientId>,
}

impl Clients {
    /// The request header that carries a client token.
    pub const TOKEN_HEADER: &'static str = "x-tgv-client-token";
    const LOGIN_HEADER: &'static str = "tailscale-user-login";

    pub fn new(configs: Vec<ClientConfig>) -> Result<Self, ClientsError> {
        let mut by_login = HashMap::new();
        let mut by_token: HashMap<TokenDigest, ClientId> = HashMap::new();
        for (index, config) in configs.iter().enumerate() {
            if configs[..index]
                .iter()
                .any(|earlier| earlier.id == config.id)
            {
                return Err(ClientsError::DuplicateId(config.id.clone()));
            }
            if config.tailscale_logins.is_empty() && config.token_blake3.is_none() {
                return Err(ClientsError::NoProof(config.id.clone()));
            }
            for login in &config.tailscale_logins {
                if by_login.insert(login.clone(), config.id.clone()).is_some() {
                    return Err(ClientsError::DuplicateLogin(login.clone()));
                }
            }
            if let Some(digest) = config.token_blake3
                && let Some(other) = by_token.insert(digest, config.id.clone())
            {
                return Err(ClientsError::DuplicateToken(other, config.id.clone()));
            }
        }
        Ok(Self {
            configs,
            by_login,
            by_token,
        })
    }

    pub fn configs(&self) -> &[ClientConfig] {
        &self.configs
    }

    /// Who sent a request, from its headers. A presented token must be
    /// valid, and must name the same client as a listed Tailscale login.
    /// Comparing digests reveals nothing usable about a token: finding one
    /// that matches means inverting BLAKE3.
    pub fn identify(&self, headers: &HeaderMap) -> Result<Caller, IdentifyError> {
        let login = headers
            .get(Self::LOGIN_HEADER)
            .map(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(|text| TailscaleLogin::parse(text).ok())
                    .ok_or(IdentifyError::MalformedLogin)
            })
            .transpose()?;
        let mut tokens = headers.get_all(Self::TOKEN_HEADER).iter();
        let token = tokens.next();
        if tokens.next().is_some() {
            return Err(IdentifyError::SeveralTokens);
        }
        let by_token = token
            .map(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(ClientToken::parse)
                    .and_then(|token| self.by_token.get(&token.digest()))
                    .cloned()
                    .ok_or(IdentifyError::UnknownToken)
            })
            .transpose()?;
        let Some(login) = login else {
            return Ok(
                by_token.map_or(Caller::Unidentified, |client| Caller::Client {
                    client,
                    proof: Proof::Token { login: None },
                }),
            );
        };
        match (self.by_login.get(&login).cloned(), by_token) {
            (Some(client), None) => Ok(Caller::Client {
                client,
                proof: Proof::Tailscale(login),
            }),
            (Some(client), Some(token)) if client == token => Ok(Caller::Client {
                client,
                proof: Proof::Tailscale(login),
            }),
            (Some(client), Some(token)) => Err(IdentifyError::Conflicting {
                login: client,
                token,
            }),
            (None, Some(client)) => Ok(Caller::Client {
                client,
                proof: Proof::Token { login: Some(login) },
            }),
            (None, None) => Ok(Caller::Unlisted(login)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientConfig, ClientToken, Clients, ClientsError, IdentifyError, TokenDigest};
    use http::HeaderMap;
    use tgv_recorder::{
        client::{Caller, ClientId, Proof, TailscaleLogin},
        id::SequentialIds,
    };

    fn id(text: &str) -> ClientId {
        ClientId::parse(text).unwrap()
    }

    fn login(text: &str) -> TailscaleLogin {
        TailscaleLogin::parse(text).unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, value.parse().unwrap());
        }
        map
    }

    struct Fixture {
        clients: Clients,
        alice: ClientToken,
        bob: ClientToken,
    }

    fn fixture() -> Fixture {
        let ids = SequentialIds::default();
        let alice = ClientToken::generate(&ids);
        let bob = ClientToken::generate(&ids);
        let clients = Clients::new(vec![
            ClientConfig {
                id: id("alice"),
                tailscale_logins: vec![login("alice@example.com")],
                token_blake3: Some(alice.digest()),
            },
            ClientConfig {
                id: id("bob"),
                tailscale_logins: vec![],
                token_blake3: Some(bob.digest()),
            },
        ])
        .unwrap();
        Fixture {
            clients,
            alice,
            bob,
        }
    }

    #[test]
    fn tokens_parse_their_format_and_never_show_in_debug() {
        let token = ClientToken::generate(&SequentialIds::default());
        assert!(token.expose().starts_with("tgvc_"));
        assert_eq!(token.expose().len(), 5 + 64);
        assert_eq!(format!("{token:?}"), "ClientToken(<redacted>)");
        assert!(ClientToken::parse(token.expose()).is_some());
        for bad in ["", "tgvc_", "abc", "tgvc_a b", "Bearer tgvc_abc"] {
            assert!(ClientToken::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn digests_round_trip_through_their_text() {
        let digest = ClientToken::generate(&SequentialIds::default()).digest();
        let text = digest.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(TokenDigest::parse(&text), Ok(digest));
        for bad in [
            "",
            &text[1..],
            &text.to_uppercase(),
            &format!("{}g", &text[1..]),
        ] {
            assert!(TokenDigest::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_request_is_its_listed_login_or_its_token() {
        let Fixture {
            clients,
            alice,
            bob,
        } = fixture();
        let cases = [
            (
                headers(&[("tailscale-user-login", "alice@example.com")]),
                Caller::Client {
                    client: id("alice"),
                    proof: Proof::Tailscale(login("alice@example.com")),
                },
            ),
            (
                headers(&[(Clients::TOKEN_HEADER, bob.expose())]),
                Caller::Client {
                    client: id("bob"),
                    proof: Proof::Token { login: None },
                },
            ),
            (
                headers(&[
                    ("tailscale-user-login", "carol@example.com"),
                    (Clients::TOKEN_HEADER, bob.expose()),
                ]),
                Caller::Client {
                    client: id("bob"),
                    proof: Proof::Token {
                        login: Some(login("carol@example.com")),
                    },
                },
            ),
            (
                headers(&[
                    ("tailscale-user-login", "alice@example.com"),
                    (Clients::TOKEN_HEADER, alice.expose()),
                ]),
                Caller::Client {
                    client: id("alice"),
                    proof: Proof::Tailscale(login("alice@example.com")),
                },
            ),
            (
                headers(&[("tailscale-user-login", "carol@example.com")]),
                Caller::Unlisted(login("carol@example.com")),
            ),
            (headers(&[]), Caller::Unidentified),
        ];
        for (request, caller) in cases {
            assert_eq!(clients.identify(&request), Ok(caller), "{request:?}");
        }
    }

    #[test]
    fn bad_or_conflicting_identity_is_refused_not_ignored() {
        let Fixture {
            clients,
            alice,
            bob,
        } = fixture();
        let unknown = ClientToken::parse(&format!("tgvc_{}", "f".repeat(64))).unwrap();
        let cases = [
            (
                headers(&[(Clients::TOKEN_HEADER, "not-a-token")]),
                IdentifyError::UnknownToken,
            ),
            (
                headers(&[
                    ("tailscale-user-login", "alice@example.com"),
                    (Clients::TOKEN_HEADER, "tgvc_unknown"),
                ]),
                IdentifyError::UnknownToken,
            ),
            (
                headers(&[(Clients::TOKEN_HEADER, unknown.expose())]),
                IdentifyError::UnknownToken,
            ),
            (
                headers(&[
                    (Clients::TOKEN_HEADER, alice.expose()),
                    (Clients::TOKEN_HEADER, bob.expose()),
                ]),
                IdentifyError::SeveralTokens,
            ),
            (
                headers(&[("tailscale-user-login", "alice smith")]),
                IdentifyError::MalformedLogin,
            ),
            (
                headers(&[
                    ("tailscale-user-login", "alice@example.com"),
                    (Clients::TOKEN_HEADER, bob.expose()),
                ]),
                IdentifyError::Conflicting {
                    login: id("alice"),
                    token: id("bob"),
                },
            ),
        ];
        for (request, error) in cases {
            assert_eq!(clients.identify(&request), Err(error), "{request:?}");
        }
    }

    #[test]
    fn the_client_set_refuses_duplicates_and_unprovable_clients() {
        let token = ClientToken::generate(&SequentialIds::default()).digest();
        let client = |name: &str, logins: &[&str], token: Option<TokenDigest>| ClientConfig {
            id: id(name),
            tailscale_logins: logins.iter().map(|text| login(text)).collect(),
            token_blake3: token,
        };
        let cases = [
            (
                vec![client("a", &["x@y"], None), client("a", &["z@y"], None)],
                ClientsError::DuplicateId(id("a")),
            ),
            (
                vec![client("a", &["x@y"], None), client("b", &["x@y"], None)],
                ClientsError::DuplicateLogin(login("x@y")),
            ),
            (
                vec![client("a", &[], Some(token)), client("b", &[], Some(token))],
                ClientsError::DuplicateToken(id("a"), id("b")),
            ),
            (vec![client("a", &[], None)], ClientsError::NoProof(id("a"))),
        ];
        for (configs, error) in cases {
            assert_eq!(Clients::new(configs).unwrap_err(), error);
        }
    }
}
