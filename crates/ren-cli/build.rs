//! Records the source revision this binary is compiled from as
//! `REN_REVISION`: `<commit>`, `<commit>-dirty`, or `unknown`
//! (`ren_git::SourceRevision` parses it).
//!
//! A packaged crate (crates.io) carries `.cargo_vcs_info.json`; a checkout,
//! including `cargo install --git`, has git. Untracked files don't count as
//! modifications: cargo writes one into its own git checkouts.

use ren_git::{Commit, Git, Tree};
use std::path::{Path, PathBuf};

/// Where the revision comes from.
enum Source {
    /// `.cargo_vcs_info.json`, written by `cargo package`.
    Packaged(PathBuf),
    /// The git work tree this package is in.
    Git(PathBuf),
}

/// What the build script found.
struct Revision {
    commit: Commit,
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
    /// Where the package is, as cargo tells the running script: read when
    /// it runs, not baked in when it was compiled, so a checkout moved since
    /// (the deployer's worktrees were) is read where it is now.
    #[expect(
        clippy::disallowed_methods,
        reason = "a build script's input is the environment cargo runs it in"
    )]
    fn find() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR")?);
        let vcs_info = dir.join(".cargo_vcs_info.json");
        Some(if vcs_info.is_file() {
            Self::Packaged(vcs_info)
        } else {
            Self::Git(dir)
        })
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
            Self::Git(dir) => {
                let git = Git::inherited();
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
    let revision = match Source::find().and_then(|source| source.revision()) {
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
