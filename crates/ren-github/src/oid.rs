//! A git object id, as GitHub reports a commit.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};

/// A commit's id: 40 lowercase hex digits (SHA-1), or 64 (SHA-256).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct Oid(String);

/// Why text is not an [`Oid`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a commit id is 40 or 64 lowercase hex digits: {0:?}")]
pub struct NotAnOid(pub String);

impl Oid {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its first 12 digits, as git abbreviates in a large repository.
    pub fn short(&self) -> &str {
        self.0.get(..12).unwrap_or(&self.0)
    }
}

impl FromStr for Oid {
    type Err = NotAnOid;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let hex = text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if hex && matches!(text.len(), 40 | 64) {
            Ok(Self(text.to_owned()))
        } else {
            Err(NotAnOid(text.to_owned()))
        }
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Oid;

    #[test]
    fn an_oid_is_40_or_64_lowercase_hex_digits() {
        let oid: Oid = "a".repeat(40).parse().unwrap();
        assert_eq!(oid.short(), "aaaaaaaaaaaa");
        assert!("b".repeat(64).parse::<Oid>().is_ok());
        for text in ["", "abc", &"A".repeat(40), &"g".repeat(40)] {
            assert!(text.parse::<Oid>().is_err(), "{text:?}");
        }
    }
}
