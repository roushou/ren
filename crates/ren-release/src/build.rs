//! Which build of Ren is running: a package version and the source
//! revision it was compiled from, so a client can tell whether it and the
//! daemon come from the same source, and the factory which commit an
//! instance runs. The binary's package works out the revision when it is
//! compiled; everything else only reports or compares it.

use ren_git::{Commit, Tree};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};

/// A package version and the source revision it was compiled from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    version: String,
    revision: Revision,
}

/// The source a binary was compiled from. Written `<commit>`,
/// `<commit>-dirty`, or `unknown`.
#[derive(Debug, Clone, PartialEq, Eq, SerializeDisplay, DeserializeFromStr)]
pub enum Revision {
    /// A commit, and whether tracked files differed from it.
    Commit { commit: Commit, tree: Tree },
    /// The build had neither git metadata nor packaging metadata.
    Unknown,
}

/// How two builds relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// The same version from the same clean commit.
    Same,
    /// Different versions or commits.
    Different,
    /// Neither the same nor provably different: a revision is unknown, or
    /// both are the same commit with local modifications.
    Unconfirmed,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a revision (`<commit>`, `<commit>-dirty`, or `unknown`): {0:?}")]
pub struct RevisionError(String);

impl Build {
    pub fn new(version: impl Into<String>, revision: Revision) -> Self {
        Self {
            version: version.into(),
            revision,
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn revision(&self) -> &Revision {
        &self.revision
    }

    pub fn compare(&self, other: &Self) -> Comparison {
        if self.version != other.version {
            return Comparison::Different;
        }
        match (&self.revision, &other.revision) {
            (
                Revision::Commit {
                    commit: ours,
                    tree: our_tree,
                },
                Revision::Commit {
                    commit: theirs,
                    tree: their_tree,
                },
            ) => {
                if ours != theirs {
                    Comparison::Different
                } else {
                    match (our_tree, their_tree) {
                        (Tree::Clean, Tree::Clean) => Comparison::Same,
                        (Tree::Clean, Tree::Modified)
                        | (Tree::Modified, Tree::Clean)
                        | (Tree::Modified, Tree::Modified) => Comparison::Unconfirmed,
                    }
                }
            }
            (Revision::Unknown, Revision::Commit { .. } | Revision::Unknown)
            | (Revision::Commit { .. }, Revision::Unknown) => Comparison::Unconfirmed,
        }
    }
}

/// `0.1.0 (07f4bb7c1e2a)`, or `0.1.0 (07f4bb7c1e2a, modified)`.
impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.revision {
            Revision::Commit { commit, tree } => {
                write!(f, "{} ({}", self.version, commit.short())?;
                match tree {
                    Tree::Clean => f.write_str(")"),
                    Tree::Modified => f.write_str(", modified)"),
                }
            }
            Revision::Unknown => write!(f, "{} (unknown revision)", self.version),
        }
    }
}

impl Revision {
    const DIRTY: &str = "-dirty";
    const UNKNOWN: &str = "unknown";
}

impl fmt::Display for Revision {
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

impl FromStr for Revision {
    type Err = RevisionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == Self::UNKNOWN {
            return Ok(Self::Unknown);
        }
        let (commit, tree) = match text.strip_suffix(Self::DIRTY) {
            Some(commit) => (commit, Tree::Modified),
            None => (text, Tree::Clean),
        };
        let commit = commit.parse().map_err(|_| RevisionError(text.to_owned()))?;
        Ok(Self::Commit { commit, tree })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708";
    const B: &str = "62e9e9e0000000000000000000000000000000ff";

    fn build(version: &str, revision: &str) -> Build {
        Build::new(version, revision.parse().unwrap())
    }

    #[test]
    fn revisions_round_trip_through_their_text_and_malformed_ones_are_refused() {
        for text in [A.to_owned(), format!("{A}-dirty"), "unknown".to_owned()] {
            assert_eq!(text.parse::<Revision>().unwrap().to_string(), text);
        }
        let sha256 = "a".repeat(64);
        assert_eq!(sha256.parse::<Revision>().unwrap().to_string(), sha256);
        assert_eq!(A.to_uppercase().parse::<Revision>().unwrap().to_string(), A);
        for text in [
            "",
            "-dirty",
            "07f4bb7",
            &format!("{A}-modified"),
            "unknown-dirty",
            &format!("{}g", &A[..39]),
        ] {
            assert!(text.parse::<Revision>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn same_requires_one_version_and_one_clean_commit() {
        assert_eq!(
            build("0.1.0", A).compare(&build("0.1.0", A)),
            Comparison::Same
        );
        assert_eq!(
            build("0.1.0", A).compare(&build("0.1.0", B)),
            Comparison::Different
        );
        assert_eq!(
            build("0.1.0", A).compare(&build("0.2.0", A)),
            Comparison::Different
        );
        let dirty = format!("{A}-dirty");
        for (ours, theirs) in [
            (A, dirty.as_str()),
            (dirty.as_str(), dirty.as_str()),
            ("unknown", A),
            ("unknown", "unknown"),
        ] {
            assert_eq!(
                build("0.1.0", ours).compare(&build("0.1.0", theirs)),
                Comparison::Unconfirmed,
                "{ours} vs {theirs}"
            );
        }
    }

    #[test]
    fn displays_a_short_commit() {
        assert_eq!(build("0.1.0", A).to_string(), "0.1.0 (07f4bb7c1e2a)");
        assert_eq!(
            build("0.1.0", &format!("{A}-dirty")).to_string(),
            "0.1.0 (07f4bb7c1e2a, modified)"
        );
        assert_eq!(
            build("0.1.0", "unknown").to_string(),
            "0.1.0 (unknown revision)"
        );
    }

    #[test]
    fn serializes_as_version_and_revision_text() {
        let json = serde_json::to_value(build("0.1.0", &format!("{A}-dirty"))).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "version": "0.1.0", "revision": format!("{A}-dirty") })
        );
        assert_eq!(
            serde_json::from_value::<Build>(json).unwrap(),
            build("0.1.0", &format!("{A}-dirty"))
        );
    }
}
