//! `cargo xtask compat`: whether this checkout keeps the compatibility
//! contract with a base revision. An install of the base must be able to
//! upgrade to this checkout: no shipped migration or store fixture changes,
//! no new migration takes data or structure away, and no label an earlier
//! release stored or sent disappears. A change that breaks any of these
//! must say so in its message (`docs/plans/self-build.md`, phase 1).
//!
//! The base is read through git, the checkout from the working tree. The
//! comparison itself is pure (`contract`).
mod contract;

use self::contract::{Comparison, Declaration, Impact, Missing, Snapshot, Surface, Verdict};
use clap::Args;
use ren_git::{Git, GitError, Revision};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Check that this checkout keeps the compatibility contract with a base
/// revision: shipped migrations and store fixtures unchanged, new
/// migrations only adding, and no label removed, unless the change's
/// message declares the break.
#[derive(Args)]
pub struct Compat {
    /// The revision an existing install runs: a pull request's base, or
    /// main before a push.
    #[arg(long, value_name = "REVISION")]
    base: Revision,
    /// A file holding the change's message: its pull request's title and
    /// body, or its commit message. Without one, no break is declared.
    #[arg(long, value_name = "FILE")]
    message: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompatError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "this change breaks compatibility with {base}, and its message {missing}. A breaking change says so with a `!` in its header (`refactor(store)!: …`) and a `BREAKING CHANGE:` footer naming what breaks and how an existing install moves on"
    )]
    Undeclared { base: Revision, missing: Undeclared },
}

/// What a breaking change's message lacks, as the refusal words it.
#[derive(Debug)]
pub struct Undeclared(Declaration);

impl std::fmt::Display for Undeclared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            Declaration::Undeclared | Declaration::Breaking => "does not declare it",
            Declaration::Partial(Missing::Marker) => "has a `BREAKING CHANGE:` footer but no `!`",
            Declaration::Partial(Missing::Note) => "has a `!` but no `BREAKING CHANGE:` footer",
        })
    }
}

impl Compat {
    pub fn run(self) -> Result<(), CompatError> {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let git = Git::inherited();
        let commit = git.commit_of(&repo, &self.base)?;
        let mut base = Snapshot::default();
        for path in git.files_at(&repo, &commit)? {
            if Surface::of(&path).is_some() {
                let bytes = git.file_at(&repo, &commit, &path)?;
                base.keep(path, bytes);
            }
        }
        let head = Self::working_tree(&repo)?;
        let declaration = match &self.message {
            Some(path) => Declaration::read(&std::fs::read_to_string(path).map_err(|source| {
                CompatError::Read {
                    path: path.clone(),
                    source,
                }
            })?),
            None => Declaration::Undeclared,
        };
        let comparison = Comparison::between(&base, &head);
        println!("Compared with {} ({}):", self.base, commit.abbreviated(12));
        if comparison.findings().is_empty() {
            println!("  no change to the store's migrations, fixtures, or labels");
        }
        for finding in comparison.findings() {
            let impact = match finding.change.impact() {
                Impact::Compatible => "compatible",
                Impact::Breaking => "BREAKING  ",
            };
            println!("  {impact}  {finding}");
        }
        match comparison.verdict(declaration) {
            Verdict::Compatible => {
                println!("Compatible: an install of the base upgrades to this change.");
                Ok(())
            }
            Verdict::DeclaredBreaking => {
                println!("Breaking, and declared as such in the change's message.");
                Ok(())
            }
            Verdict::UndeclaredBreaking(declaration) => Err(CompatError::Undeclared {
                base: self.base,
                missing: Undeclared(declaration),
            }),
        }
    }

    /// The watched files in the working tree under `repo`.
    fn working_tree(repo: &Path) -> Result<Snapshot, CompatError> {
        let mut paths = vec![Surface::PINS.to_owned()];
        for folder in [Surface::STORE_FIXTURES, Surface::MIGRATIONS] {
            for name in Self::names(&repo.join(folder))? {
                paths.push(format!("{folder}/{name}"));
            }
        }
        for name in Self::names(&repo.join("crates"))? {
            paths.push(format!("crates/{name}/{}", Surface::LABELS));
        }
        let mut snapshot = Snapshot::default();
        for path in paths {
            let file = repo.join(&path);
            match std::fs::read(&file) {
                Ok(bytes) => snapshot.keep(path, bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(CompatError::Read { path: file, source }),
            }
        }
        Ok(snapshot)
    }

    /// The names in `folder`, or none when it does not exist.
    fn names(folder: &Path) -> Result<Vec<String>, CompatError> {
        let entries = match std::fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(CompatError::Read {
                    path: folder.to_path_buf(),
                    source,
                });
            }
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| CompatError::Read {
                path: folder.to_path_buf(),
                source,
            })?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }
}
