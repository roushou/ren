//! Which build of Ren is running: a package version and the source
//! revision it was compiled from, so a client can tell whether it and the
//! daemon come from the same source, and the deployer which commit an
//! instance runs. The binary's package works out the revision when it is
//! compiled; everything else only reports or compares it.

use ren_git::{SourceRevision, Tree};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A package version and the source revision it was compiled from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    version: String,
    revision: SourceRevision,
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

impl Build {
    pub fn new(version: impl Into<String>, revision: SourceRevision) -> Self {
        Self {
            version: version.into(),
            revision,
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn revision(&self) -> &SourceRevision {
        &self.revision
    }

    pub fn compare(&self, other: &Self) -> Comparison {
        if self.version != other.version {
            return Comparison::Different;
        }
        match (&self.revision, &other.revision) {
            (
                SourceRevision::Commit {
                    commit: ours,
                    tree: our_tree,
                },
                SourceRevision::Commit {
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
            (SourceRevision::Unknown, SourceRevision::Commit { .. } | SourceRevision::Unknown)
            | (SourceRevision::Commit { .. }, SourceRevision::Unknown) => Comparison::Unconfirmed,
        }
    }
}

/// `0.1.0 (07f4bb7c1e2a)`, or `0.1.0 (07f4bb7c1e2a, modified)`.
impl fmt::Display for Build {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.revision {
            SourceRevision::Commit { commit, tree } => {
                write!(f, "{} ({}", self.version, commit.short())?;
                match tree {
                    Tree::Clean => f.write_str(")"),
                    Tree::Modified => f.write_str(", modified)"),
                }
            }
            SourceRevision::Unknown => write!(f, "{} (unknown revision)", self.version),
        }
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
