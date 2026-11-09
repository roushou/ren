//! Who is calling: the configured users, and how a forwarded request is
//! matched to one.
//!
//! A request names its user by a Tailscale login, which Tailscale Serve
//! adds after removing any copy the caller sent, or by a user token in
//! [`Users::TOKEN_HEADER`]. The token header is in TGV's `x-tgv-`
//! namespace, which the header policy removes before forwarding, so it never
//! reaches a provider. Configuration holds only each token's BLAKE3 digest.
//!
//! A process on this host can reach the loopback listener directly and set
//! any header, including `Tailscale-User-Login`; identity guards against
//! other machines, not against this one.

use http::HeaderMap;
use std::{collections::HashMap, num::NonZeroU32};
use tgv_recorder::{
    caller::{Caller, Proof, TailscaleLogin, UserId},
    id::IdSource,
};

/// A user's secret token. Never shown in `Debug`.
#[derive(Clone)]
pub struct UserToken(Box<str>);

impl UserToken {
    /// Marks the token's kind, so it reads as a TGV credential wherever it
    /// turns up.
    const PREFIX: &'static str = "tgvu_";

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

    pub fn digest(&self) -> TokenDigest {
        TokenDigest(*blake3::hash(self.0.as_bytes()).as_bytes())
    }
}

impl std::fmt::Debug for UserToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UserToken(<redacted>)")
    }
}

/// The BLAKE3 digest of a user token, as configuration holds it: 64
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

/// One `[[users]]` entry: a user's name and how it proves it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    pub id: UserId,
    /// Tailscale logins that are this user.
    #[serde(default)]
    pub tailscale_logins: Vec<TailscaleLogin>,
    /// The BLAKE3 digest of the user's token (`tgv users token`).
    #[serde(default)]
    pub token_blake3: Option<TokenDigest>,
    /// Calls the user may have running at once, at most 1024; unlimited
    /// beyond the gateway's own limit when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<NonZeroU32>,
}

/// Why the configured users are not a valid set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsersError {
    #[error("user {0} is configured twice")]
    DuplicateId(UserId),
    #[error("Tailscale login {0} belongs to two users")]
    DuplicateLogin(TailscaleLogin),
    #[error("users {0} and {1} have the same token")]
    DuplicateToken(UserId, UserId),
    #[error("user {0} has neither a Tailscale login nor a token, so no request can be it")]
    NoProof(UserId),
    #[error("user {0}: max_concurrent must be between 1 and 1024")]
    Concurrency(UserId),
}

/// Why a request's identity was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentifyError {
    /// A token was presented that no user has. It is refused rather than
    /// ignored, so a caller never believes it is identified when it is not.
    #[error("the user token is not one this gateway knows")]
    UnknownToken,
    #[error("the request carries more than one user token")]
    SeveralTokens,
    #[error("the Tailscale login is malformed")]
    MalformedLogin,
    /// The Tailscale login and the token name different users.
    #[error("the Tailscale login is user {login}, but the token is user {token}")]
    Conflicting { login: UserId, token: UserId },
}

/// The configured users, indexed by what proves each one.
#[derive(Debug, Clone, Default)]
pub struct Users {
    configs: Vec<UserConfig>,
    by_login: HashMap<TailscaleLogin, UserId>,
    by_token: HashMap<TokenDigest, UserId>,
}

impl Users {
    /// The request header that carries a user token.
    pub const TOKEN_HEADER: &'static str = "x-tgv-user-token";
    const LOGIN_HEADER: &'static str = "tailscale-user-login";

    pub fn new(configs: Vec<UserConfig>) -> Result<Self, UsersError> {
        let mut by_login = HashMap::new();
        let mut by_token: HashMap<TokenDigest, UserId> = HashMap::new();
        for (index, config) in configs.iter().enumerate() {
            if configs[..index]
                .iter()
                .any(|earlier| earlier.id == config.id)
            {
                return Err(UsersError::DuplicateId(config.id.clone()));
            }
            if config.tailscale_logins.is_empty() && config.token_blake3.is_none() {
                return Err(UsersError::NoProof(config.id.clone()));
            }
            if config
                .max_concurrent
                .is_some_and(|limit| limit.get() > 1024)
            {
                return Err(UsersError::Concurrency(config.id.clone()));
            }
            for login in &config.tailscale_logins {
                if by_login.insert(login.clone(), config.id.clone()).is_some() {
                    return Err(UsersError::DuplicateLogin(login.clone()));
                }
            }
            if let Some(digest) = config.token_blake3
                && let Some(other) = by_token.insert(digest, config.id.clone())
            {
                return Err(UsersError::DuplicateToken(other, config.id.clone()));
            }
        }
        Ok(Self {
            configs,
            by_login,
            by_token,
        })
    }

    pub fn configs(&self) -> &[UserConfig] {
        &self.configs
    }

    /// Whether `user` is configured.
    pub fn knows(&self, user: &UserId) -> bool {
        self.configs.iter().any(|config| &config.id == user)
    }

    /// Who sent a request, from its headers. A presented token must be
    /// valid, and must name the same user as a listed Tailscale login.
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
                    .and_then(UserToken::parse)
                    .and_then(|token| self.by_token.get(&token.digest()))
                    .cloned()
                    .ok_or(IdentifyError::UnknownToken)
            })
            .transpose()?;
        let Some(login) = login else {
            return Ok(by_token.map_or(Caller::Unidentified, |user| Caller::User {
                user,
                proof: Proof::Token { login: None },
            }));
        };
        match (self.by_login.get(&login).cloned(), by_token) {
            (Some(user), None) => Ok(Caller::User {
                user,
                proof: Proof::Tailscale(login),
            }),
            (Some(user), Some(token)) if user == token => Ok(Caller::User {
                user,
                proof: Proof::Tailscale(login),
            }),
            (Some(user), Some(token)) => Err(IdentifyError::Conflicting { login: user, token }),
            (None, Some(user)) => Ok(Caller::User {
                user,
                proof: Proof::Token { login: Some(login) },
            }),
            (None, None) => Ok(Caller::Unlisted(login)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IdentifyError, TokenDigest, UserConfig, UserToken, Users, UsersError};
    use http::HeaderMap;
    use tgv_recorder::{
        caller::{Caller, Proof, TailscaleLogin, UserId},
        id::SequentialIds,
    };

    fn id(text: &str) -> UserId {
        UserId::parse(text).unwrap()
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
        users: Users,
        alice: UserToken,
        bob: UserToken,
    }

    fn fixture() -> Fixture {
        let ids = SequentialIds::default();
        let alice = UserToken::generate(&ids);
        let bob = UserToken::generate(&ids);
        let users = Users::new(vec![
            UserConfig {
                id: id("alice"),
                tailscale_logins: vec![login("alice@example.com")],
                token_blake3: Some(alice.digest()),
                max_concurrent: None,
            },
            UserConfig {
                id: id("bob"),
                tailscale_logins: vec![],
                token_blake3: Some(bob.digest()),
                max_concurrent: None,
            },
        ])
        .unwrap();
        Fixture { users, alice, bob }
    }

    #[test]
    fn tokens_parse_their_format_and_never_show_in_debug() {
        let token = UserToken::generate(&SequentialIds::default());
        assert!(token.expose().starts_with("tgvu_"));
        assert_eq!(token.expose().len(), 5 + 64);
        assert_eq!(format!("{token:?}"), "UserToken(<redacted>)");
        assert!(UserToken::parse(token.expose()).is_some());
        for bad in ["", "tgvu_", "abc", "tgvu_a b", "Bearer tgvu_abc"] {
            assert!(UserToken::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn digests_round_trip_through_their_text() {
        let digest = UserToken::generate(&SequentialIds::default()).digest();
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
        let Fixture { users, alice, bob } = fixture();
        let cases = [
            (
                headers(&[("tailscale-user-login", "alice@example.com")]),
                Caller::User {
                    user: id("alice"),
                    proof: Proof::Tailscale(login("alice@example.com")),
                },
            ),
            (
                headers(&[(Users::TOKEN_HEADER, bob.expose())]),
                Caller::User {
                    user: id("bob"),
                    proof: Proof::Token { login: None },
                },
            ),
            (
                headers(&[
                    ("tailscale-user-login", "carol@example.com"),
                    (Users::TOKEN_HEADER, bob.expose()),
                ]),
                Caller::User {
                    user: id("bob"),
                    proof: Proof::Token {
                        login: Some(login("carol@example.com")),
                    },
                },
            ),
            (
                headers(&[
                    ("tailscale-user-login", "alice@example.com"),
                    (Users::TOKEN_HEADER, alice.expose()),
                ]),
                Caller::User {
                    user: id("alice"),
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
            assert_eq!(users.identify(&request), Ok(caller), "{request:?}");
        }
    }

    #[test]
    fn bad_or_conflicting_identity_is_refused_not_ignored() {
        let Fixture { users, alice, bob } = fixture();
        let unknown = UserToken::parse(&format!("tgvu_{}", "f".repeat(64))).unwrap();
        let cases = [
            (
                headers(&[(Users::TOKEN_HEADER, "not-a-token")]),
                IdentifyError::UnknownToken,
            ),
            (
                headers(&[
                    ("tailscale-user-login", "alice@example.com"),
                    (Users::TOKEN_HEADER, "tgvu_unknown"),
                ]),
                IdentifyError::UnknownToken,
            ),
            (
                headers(&[(Users::TOKEN_HEADER, unknown.expose())]),
                IdentifyError::UnknownToken,
            ),
            (
                headers(&[
                    (Users::TOKEN_HEADER, alice.expose()),
                    (Users::TOKEN_HEADER, bob.expose()),
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
                    (Users::TOKEN_HEADER, bob.expose()),
                ]),
                IdentifyError::Conflicting {
                    login: id("alice"),
                    token: id("bob"),
                },
            ),
        ];
        for (request, error) in cases {
            assert_eq!(users.identify(&request), Err(error), "{request:?}");
        }
    }

    #[test]
    fn the_client_set_refuses_duplicates_and_unprovable_clients() {
        let token = UserToken::generate(&SequentialIds::default()).digest();
        let user = |name: &str, logins: &[&str], token: Option<TokenDigest>| UserConfig {
            id: id(name),
            tailscale_logins: logins.iter().map(|text| login(text)).collect(),
            token_blake3: token,
            max_concurrent: None,
        };
        let cases = [
            (
                vec![user("a", &["x@y"], None), user("a", &["z@y"], None)],
                UsersError::DuplicateId(id("a")),
            ),
            (
                vec![user("a", &["x@y"], None), user("b", &["x@y"], None)],
                UsersError::DuplicateLogin(login("x@y")),
            ),
            (
                vec![user("a", &[], Some(token)), user("b", &[], Some(token))],
                UsersError::DuplicateToken(id("a"), id("b")),
            ),
            (vec![user("a", &[], None)], UsersError::NoProof(id("a"))),
        ];
        for (configs, error) in cases {
            assert_eq!(Users::new(configs).unwrap_err(), error);
        }
    }
}
