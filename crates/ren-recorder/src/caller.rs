//! Who called the gateway: the recorded identity of a call's caller.
//!
//! A caller is an operator-named user, proven by a Tailscale login or a
//! user token; a Tailscale login no user lists; or nobody Ren can
//! identify. A request without identity is not evidence that it came from
//! this host: Tailscale Serve forwards tagged devices and Funnel traffic to
//! loopback without identity headers.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// An operator-named user of the gateway, from configuration: lowercase
/// ASCII letters, digits, `-` and `_`, at most 64 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct UserId(Box<str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("user ids are 1 to 64 lowercase letters, digits, '-' or '_'")]
pub struct InvalidUserId;

impl UserId {
    pub fn parse(id: &str) -> Result<Self, InvalidUserId> {
        let valid = !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            });
        if valid {
            Ok(Self(id.into()))
        } else {
            Err(InvalidUserId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for UserId {
    type Err = InvalidUserId;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::parse(id)
    }
}

impl<'de> serde::Deserialize<'de> for UserId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for UserId {
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

/// How a user proved who it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Proof {
    /// A Tailscale login the user lists.
    Tailscale(TailscaleLogin),
    /// The user's token. A Tailscale login the request also carried, which
    /// no user lists, is kept.
    Token { login: Option<TailscaleLogin> },
}

/// Who made a call.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Caller {
    /// A configured user.
    User { user: UserId, proof: Proof },
    /// A tailnet user no user lists.
    Unlisted(TailscaleLogin),
    /// No identity: a local process, a tagged tailnet device, or Funnel
    /// traffic, which Ren cannot tell apart.
    Unidentified,
}

impl Caller {
    pub fn kind(&self) -> CallerKind {
        match self {
            Self::User {
                proof: Proof::Tailscale(_),
                ..
            } => CallerKind::UserTailscale,
            Self::User {
                proof: Proof::Token { .. },
                ..
            } => CallerKind::UserToken,
            Self::Unlisted(_) => CallerKind::Unlisted,
            Self::Unidentified => CallerKind::Unidentified,
        }
    }

    pub fn user(&self) -> Option<&UserId> {
        match self {
            Self::User { user, .. } => Some(user),
            Self::Unlisted(_) | Self::Unidentified => None,
        }
    }

    /// The Tailscale login the request carried, if any.
    pub fn login(&self) -> Option<&TailscaleLogin> {
        match self {
            Self::User {
                proof: Proof::Tailscale(login),
                ..
            }
            | Self::Unlisted(login) => Some(login),
            Self::User {
                proof: Proof::Token { login },
                ..
            } => login.as_ref(),
            Self::Unidentified => None,
        }
    }
}

impl serde::Serialize for Caller {
    /// As recorded: its kind, user, and login, flat.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(serde::Serialize)]
        struct Fields<'a> {
            kind: CallerKind,
            user: Option<&'a UserId>,
            login: Option<&'a TailscaleLogin>,
        }
        Fields {
            kind: self.kind(),
            user: self.user(),
            login: self.login(),
        }
        .serialize(serializer)
    }
}

/// A caller's kind, as stored.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CallerKind {
    /// A user, by a Tailscale login it lists.
    UserTailscale,
    /// A user, by its token.
    UserToken,
    /// A Tailscale login no user lists.
    Unlisted,
    Unidentified,
}

#[cfg(test)]
mod tests {
    use super::{Caller, CallerKind, Proof, TailscaleLogin, UserId};

    #[test]
    fn ids_and_logins_parse_only_their_grammar() {
        assert!(UserId::parse("alice-2_ci").is_ok());
        for bad in ["", "Alice", "a b", "a/b", &"a".repeat(65)] {
            assert!(UserId::parse(bad).is_err(), "{bad:?}");
        }
        assert!(TailscaleLogin::parse("alice@example.com").is_ok());
        for bad in ["", "a b", "é", &"a".repeat(257)] {
            assert!(TailscaleLogin::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_caller_names_its_kind_client_and_login() {
        let alice = UserId::parse("alice").unwrap();
        let login = TailscaleLogin::parse("alice@example.com").unwrap();
        let cases = [
            (
                Caller::User {
                    user: alice.clone(),
                    proof: Proof::Tailscale(login.clone()),
                },
                CallerKind::UserTailscale,
                Some(&alice),
                Some(&login),
            ),
            (
                Caller::User {
                    user: alice.clone(),
                    proof: Proof::Token {
                        login: Some(login.clone()),
                    },
                },
                CallerKind::UserToken,
                Some(&alice),
                Some(&login),
            ),
            (
                Caller::User {
                    user: alice.clone(),
                    proof: Proof::Token { login: None },
                },
                CallerKind::UserToken,
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
        for (caller, kind, user, login) in cases {
            assert_eq!(caller.kind(), kind, "{caller:?}");
            assert_eq!(caller.user(), user, "{caller:?}");
            assert_eq!(caller.login(), login, "{caller:?}");
        }
    }
}
