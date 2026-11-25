//! Which source a binary was compiled from: a commit, and whether tracked
//! files differed from it. The binary's package works out its own when it
//! is compiled (`REN_REVISION`); everything else reports or compares it.

use crate::{Commit, Tree};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};

/// The source a binary was compiled from. Written `<commit>`,
/// `<commit>-dirty`, or `unknown`.
#[derive(Debug, Clone, PartialEq, Eq, SerializeDisplay, DeserializeFromStr)]
pub enum SourceRevision {
    /// A commit, and whether tracked files differed from it.
    Commit { commit: Commit, tree: Tree },
    /// The build had neither git metadata nor packaging metadata.
    Unknown,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a revision (`<commit>`, `<commit>-dirty`, or `unknown`): {0:?}")]
pub struct NotASourceRevision(String);

impl SourceRevision {
    const DIRTY: &str = "-dirty";
    const UNKNOWN: &str = "unknown";
}

impl fmt::Display for SourceRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Commit { commit, tree } => {
                write!(f, "{commit}")?;
                match tree {
                    Tree::Clean => Ok(()),
                    Tree::Modified => f.write_str(Self::DIRTY),
                }
            }
            Self::Unknown => f.write_str(Self::UNKNOWN),
        }
    }
}

impl FromStr for SourceRevision {
    type Err = NotASourceRevision;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == Self::UNKNOWN {
            return Ok(Self::Unknown);
        }
        let (commit, tree) = match text.strip_suffix(Self::DIRTY) {
            Some(commit) => (commit, Tree::Modified),
            None => (text, Tree::Clean),
        };
        let commit = commit
            .parse()
            .map_err(|_| NotASourceRevision(text.to_owned()))?;
        Ok(Self::Commit { commit, tree })
    }
}

#[cfg(test)]
mod tests {
    use super::SourceRevision;

    const A: &str = "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708";

    #[test]
    fn revisions_round_trip_through_their_text_and_malformed_ones_are_refused() {
        for text in [A.to_owned(), format!("{A}-dirty"), "unknown".to_owned()] {
            assert_eq!(text.parse::<SourceRevision>().unwrap().to_string(), text);
        }
        let sha256 = "a".repeat(64);
        assert_eq!(
            sha256.parse::<SourceRevision>().unwrap().to_string(),
            sha256
        );
        assert_eq!(
            A.to_uppercase()
                .parse::<SourceRevision>()
                .unwrap()
                .to_string(),
            A
        );
        for text in [
            "",
            "-dirty",
            "07f4bb7",
            &format!("{A}-modified"),
            "unknown-dirty",
            &format!("{}g", &A[..39]),
        ] {
            assert!(text.parse::<SourceRevision>().is_err(), "{text:?}");
        }
    }
}
