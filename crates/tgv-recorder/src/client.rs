//! Who called the gateway: the recorded identity of a call's caller.
//!
//! A caller is an operator-named client, proven by a Tailscale login or a
//! client token; a Tailscale login no client lists; or nobody TGV can
//! identify. A request without identity is not evidence that it came from
//! this host: Tailscale Serve forwards tagged devices and Funnel traffic to
//! loopback without identity headers.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// An operator-named client of the gateway, from configuration: lowercase
/// ASCII letters, digits, `-` and `_`, at most 64 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct ClientId(Box<str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("client ids are 1 to 64 lowercase letters, digits, '-' or '_'")]
pub struct InvalidClientId;

impl ClientId {
    pub fn parse(id: &str) -> Result<Self, InvalidClientId> {
        let valid = !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            });
        if valid {
            Ok(Self(id.into()))
        } else {
            Err(InvalidClientId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for ClientId {
    type Err = InvalidClientId;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::parse(id)
    }
}

impl<'de> serde::Deserialize<'de> for ClientId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for ClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A tailnet login name, as Tailscale Serve reports it in
/// `Tailscale-User-Login`: 1 to 256 visible ASCII characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct TailscaleLogin(Box<str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Tailscale logins are 1 to 256 visible ASCII characters")]
pub struct InvalidTailscaleLogin;

impl TailscaleLogin {
    pub fn parse(login: &str) -> Result<Self, InvalidTailscaleLogin> {
        if (1..=256).contains(&login.len()) && login.bytes().all(|byte| byte.is_ascii_graphic()) {
            Ok(Self(login.into()))
        } else {
            Err(InvalidTailscaleLogin)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for TailscaleLogin {
    type Err = InvalidTailscaleLogin;

    fn from_str(login: &str) -> Result<Self, Self::Err> {
        Self::parse(login)
    }
}

impl<'de> serde::Deserialize<'de> for TailscaleLogin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for TailscaleLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a client proved who it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Proof {
    /// A Tailscale login the client lists.
    Tailscale(TailscaleLogin),
    /// The client's token. A Tailscale login the request also carried, which
    /// no client lists, is kept.
    Token { login: Option<TailscaleLogin> },
}

/// Who made a call.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Caller {
    /// A configured client.
    Client { client: ClientId, proof: Proof },
    /// A tailnet user no client lists.
    Unlisted(TailscaleLogin),
    /// No identity: a local process, a tagged tailnet device, or Funnel
    /// traffic, which TGV cannot tell apart.
    Unidentified,
}

impl Caller {
    pub fn kind(&self) -> CallerKind {
        match self {
            Self::Client {
                proof: Proof::Tailscale(_),
                ..
            } => CallerKind::ClientTailscale,
            Self::Client {
                proof: Proof::Token { .. },
                ..
            } => CallerKind::ClientToken,
            Self::Unlisted(_) => CallerKind::Unlisted,
            Self::Unidentified => CallerKind::Unidentified,
        }
    }

    pub fn client(&self) -> Option<&ClientId> {
        match self {
            Self::Client { client, .. } => Some(client),
            Self::Unlisted(_) | Self::Unidentified => None,
        }
    }

    /// The Tailscale login the request carried, if any.
    pub fn login(&self) -> Option<&TailscaleLogin> {
        match self {
            Self::Client {
                proof: Proof::Tailscale(login),
                ..
            }
            | Self::Unlisted(login) => Some(login),
            Self::Client {
                proof: Proof::Token { login },
                ..
            } => login.as_ref(),
            Self::Unidentified => None,
        }
    }
}

/// A caller's kind, as stored.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CallerKind {
    /// A client, by a Tailscale login it lists.
    ClientTailscale,
    /// A client, by its token.
    ClientToken,
    /// A Tailscale login no client lists.
    Unlisted,
    Unidentified,
}

#[cfg(test)]
mod tests {
    use super::{Caller, CallerKind, ClientId, Proof, TailscaleLogin};

    #[test]
    fn ids_and_logins_parse_only_their_grammar() {
        assert!(ClientId::parse("alice-2_ci").is_ok());
        for bad in ["", "Alice", "a b", "a/b", &"a".repeat(65)] {
            assert!(ClientId::parse(bad).is_err(), "{bad:?}");
        }
        assert!(TailscaleLogin::parse("alice@example.com").is_ok());
        for bad in ["", "a b", "é", &"a".repeat(257)] {
            assert!(TailscaleLogin::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_caller_names_its_kind_client_and_login() {
        let alice = ClientId::parse("alice").unwrap();
        let login = TailscaleLogin::parse("alice@example.com").unwrap();
        let cases = [
            (
                Caller::Client {
                    client: alice.clone(),
                    proof: Proof::Tailscale(login.clone()),
                },
                CallerKind::ClientTailscale,
                Some(&alice),
                Some(&login),
            ),
            (
                Caller::Client {
                    client: alice.clone(),
                    proof: Proof::Token {
                        login: Some(login.clone()),
                    },
                },
                CallerKind::ClientToken,
                Some(&alice),
                Some(&login),
            ),
            (
                Caller::Client {
                    client: alice.clone(),
                    proof: Proof::Token { login: None },
                },
                CallerKind::ClientToken,
                Some(&alice),
                None,
            ),
            (
                Caller::Unlisted(login.clone()),
                CallerKind::Unlisted,
                None,
                Some(&login),
            ),
            (Caller::Unidentified, CallerKind::Unidentified, None, None),
        ];
        for (caller, kind, client, login) in cases {
            assert_eq!(caller.kind(), kind, "{caller:?}");
            assert_eq!(caller.client(), client, "{caller:?}");
            assert_eq!(caller.login(), login, "{caller:?}");
        }
    }
}
