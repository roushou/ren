//! What a deployment's data starts as: a copy of production's latest
//! backup, or a history invented from a number (`ren seed`).

use serde::{Deserialize, Serialize};
use std::fmt;

/// The number an invented history is made from: the same number always
/// invents the same history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeedNumber(u64);

/// What a deployment's data starts as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seed {
    /// A copy of production's latest backup, every operator key a
    /// placeholder.
    #[default]
    Copy,
    /// The history this number invents, written into an empty store.
    Invented(SeedNumber),
}

impl SeedNumber {
    pub const fn new(number: u64) -> Self {
        Self(number)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::str::FromStr for SeedNumber {
    type Err = std::num::ParseIntError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

impl fmt::Display for SeedNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Seed {
    /// Whether it is a copy of production, which files written before
    /// seeds existed leave out.
    pub fn is_copy(&self) -> bool {
        matches!(self, Self::Copy)
    }
}

impl fmt::Display for Seed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Copy => f.write_str("a copy of production"),
            Self::Invented(number) => write!(f, "history {number}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Seed, SeedNumber};

    #[test]
    fn seeds_are_written_as_the_files_keep_them() {
        assert_eq!(serde_json::to_value(Seed::Copy).unwrap(), "copy");
        assert_eq!(
            serde_json::to_value(Seed::Invented(SeedNumber::new(7))).unwrap(),
            serde_json::json!({ "invented": 7 })
        );
    }
}
