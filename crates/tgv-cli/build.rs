//! Records the source revision this binary is compiled from as
//! `TGV_REVISION`: `<commit>`, `<commit>-dirty`, or `unknown`
//! (`tgv_gateway::build::Revision` parses it).
//!
//! A packaged crate (crates.io) carries `.cargo_vcs_info.json`; a checkout,
//! including `cargo install --git`, has git. Untracked files don't count as
//! modifications: cargo writes one into its own git checkouts.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Where the revision comes from.
enum Source {
    /// `.cargo_vcs_info.json`, written by `cargo package`.
    Packaged(PathBuf),
    /// The git work tree this package is in.
    Git,
}

/// What the build script found.
struct Revision {
    commit: String,
    dirty: bool,
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
                    commit: info.git.sha1,
                    dirty: info.git.dirty,
                })
            }
            Self::Git => {
                Self::watch_git();
                let commit = Self::git(&["rev-parse", "--verify", "HEAD"])?;
                let status = Self::git(&["status", "--porcelain", "--untracked-files=no"])?;
                Some(Revision {
                    commit,
                    dirty: !status.is_empty(),
                })
            }
        }
    }

    /// A new commit moves HEAD or the branch it names; staging changes the
    /// index. Editing a tracked file without staging it changes neither, so
    /// the dirty flag can lag until the next rebuild of this package. Only
    /// existing files are watched: cargo reruns on every build for a missing one.
    fn watch_git() {
        let git_dir = Self::git(&["rev-parse", "--absolute-git-dir"]).map(PathBuf::from);
        let common = Self::git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .map(PathBuf::from);
        let branch = Self::git(&["symbolic-ref", "-q", "HEAD"]);
        let mut watched = Vec::new();
        if let Some(git_dir) = &git_dir {
            watched.extend(["HEAD", "index"].map(|file| git_dir.join(file)));
        }
        if let Some(common) = &common {
            watched.push(common.join("packed-refs"));
            watched.extend(branch.iter().map(|branch| common.join(branch)));
        }
        for path in watched.iter().filter(|path| path.is_file()) {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }

    /// A git command's trimmed output, if it succeeds.
    fn git(args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(Self::MANIFEST_DIR)
            .args(args)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let revision = match Source::find().revision() {
        Some(Revision {
            commit,
            dirty: false,
        }) => commit,
        Some(Revision {
            commit,
            dirty: true,
        }) => format!("{commit}-dirty"),
        None => "unknown".to_owned(),
    };
    println!("cargo::rustc-env=TGV_REVISION={revision}");
}
