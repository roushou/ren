//! Recorded routing facts: the configuration revision, operator state
//! revision, and upstream entry a call was planned and sent with.

use ren_protocol::{label::Labeled, route::Provider};

/// The revision of the configuration snapshot a call used.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ConfigRevision(Box<str>);

impl ConfigRevision {
    pub fn new(revision: impl Into<Box<str>>) -> Self {
        Self(revision.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An upstream entry's identifier from configuration: lowercase ASCII
/// letters, digits, `-` and `_`, at most 64 bytes. Each provider has a default
/// entry named after it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(transparent)]
pub struct UpstreamId(Box<str>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("upstream ids are 1 to 64 lowercase letters, digits, '-' or '_'")]
pub struct InvalidUpstreamId;

impl UpstreamId {
    pub fn parse(id: &str) -> Result<Self, InvalidUpstreamId> {
        let valid = !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            });
        if valid {
            Ok(Self(id.into()))
        } else {
            Err(InvalidUpstreamId)
        }
    }

    /// The id of a provider's native entry, which the gateway's upstream
    /// table builds for every provider.
    pub fn default_for(provider: Provider) -> Self {
        Self(provider.label().into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for UpstreamId {
    type Err = InvalidUpstreamId;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::parse(id)
    }
}

impl<'de> serde::Deserialize<'de> for UpstreamId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for UpstreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Counts operator changes in this data directory. Revision 0 is the empty
/// state; the count continues across restarts.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct OperatorRevision(u64);

impl OperatorRevision {
    /// A revision read back from storage.
    pub fn restore(revision: u64) -> Self {
        Self(revision)
    }

    /// The revision after this one.
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}
