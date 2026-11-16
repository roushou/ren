//! Records the source revision this binary is compiled from as
//! `REN_REVISION`: `<commit>`, `<commit>-dirty`, or `unknown`
//! (`ren_gateway::build::Revision` parses it).
//!
//! A packaged crate (crates.io) carries `.cargo_vcs_info.json`; a checkout,
//! including `cargo install --git`, has git. Untracked files don't count as
//! modifications: cargo writes one into its own git checkouts.

use ren_git::{CommitId, Git, Tree};
use std::path::{Path, PathBuf};

/// Where the revision comes from.
enum Source {
    /// `.cargo_vcs_info.json`, written by `cargo package`.
    Packaged(PathBuf),
    /// The git work tree this package is in.
    Git,
}

/// What the build script found.
struct Revision {
    commit: CommitId,
    tree: Tree,
}

/// `.cargo_vcs_info.json`; `dirty` is present only when true.
#[derive(serde::Deserialize)]
struct VcsInfo {
    git: VcsGit,
}

#[derive(serde::Deserialize)]
struct VcsGit {
    sha1: String,
    #[serde(default)]
    dirty: bool,
}

impl Source {
    const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

    fn find() -> Self {
        let vcs_info = Path::new(Self::MANIFEST_DIR).join(".cargo_vcs_info.json");
        if vcs_info.is_file() {
            Self::Packaged(vcs_info)
        } else {
            Self::Git
        }
    }

    /// Reads the revision, and tells cargo which files to watch for a new one.
    fn revision(&self) -> Option<Revision> {
        match self {
            Self::Packaged(path) => {
                println!("cargo::rerun-if-changed={}", path.display());
                let text = std::fs::read_to_string(path).ok()?;
                let info: VcsInfo = serde_json::from_str(&text).ok()?;
                Some(Revision {
                    commit: info.git.sha1.parse().ok()?,
                    tree: if info.git.dirty {
                        Tree::Modified
                    } else {
                        Tree::Clean
                    },
                })
            }
            Self::Git => {
                let (git, dir) = (Git::inherited(), Path::new(Self::MANIFEST_DIR));
                Self::watch_git(&git, dir);
                Some(Revision {
                    commit: git.head_commit(dir).ok()?,
                    tree: git.tree(dir).ok()?,
                })
            }
        }
    }

    /// Watches the files git rewrites for a new commit or a staged change.
    /// Editing a tracked file without staging it changes none, so the dirty
    /// flag can lag until the next rebuild of this package. Only existing
    /// files are watched: cargo reruns on every build for a missing one.
    fn watch_git(git: &Git, dir: &Path) {
        let files = git.revision_files(dir).unwrap_or_default();
        for path in files.iter().filter(|path| path.is_file()) {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let revision = match Source::find().revision() {
        Some(Revision {
            commit,
            tree: Tree::Clean,
        }) => commit.to_string(),
        Some(Revision {
            commit,
            tree: Tree::Modified,
        }) => format!("{commit}-dirty"),
        None => "unknown".to_owned(),
    };
    println!("cargo::rustc-env=REN_REVISION={revision}");
}
