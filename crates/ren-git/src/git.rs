use crate::{CommitId, Exit, GitError, NotACommitId, Repository};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The git program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    program: PathBuf,
}

/// Whether tracked files differ from the commit. Untracked files don't count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tree {
    Clean,
    Modified,
}

/// Whether a remote branch holds the current commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    OnRemote,
    LocalOnly,
}

/// The reference `HEAD` names, or none when detached.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HeadRef {
    /// The full reference, such as `refs/heads/main`.
    Named(String),
    Detached,
}

/// A work tree's git folders, absolute: its own (`HEAD`, `index`), and the
/// one its references live in, which differ in a linked worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GitDirs {
    own: PathBuf,
    common: PathBuf,
}

impl Git {
    /// The git on `search_path`, the first folder first.
    pub fn find(search_path: &[PathBuf]) -> Option<Self> {
        search_path.iter().find_map(|dir| {
            #[cfg(windows)]
            let program = dir.join("git.exe");
            #[cfg(not(windows))]
            let program = dir.join("git");
            Self::executable(&program).then_some(Self { program })
        })
    }

    /// The git this process's own search path finds, looked up as it starts.
    pub fn inherited() -> Self {
        Self {
            program: PathBuf::from("git"),
        }
    }

    /// Whether `dir` holds a repository, and where its `HEAD` stands, read
    /// from disk without the program ([`Repository`]).
    pub fn repository(dir: &Path) -> Repository {
        Repository::read(dir)
    }

    /// Make `dir` a new, empty repository.
    pub fn init(&self, dir: &Path) -> Result<(), GitError> {
        self.answer(dir, &["init", "--quiet"]).map(drop)
    }

    /// The commit `dir`'s `HEAD` is at.
    pub fn head_commit(&self, dir: &Path) -> Result<CommitId, GitError> {
        let args = ["rev-parse", "--verify", "HEAD"];
        let output = self.answer(dir, &args)?;
        output
            .parse()
            .map_err(|_: NotACommitId| GitError::Unexpected {
                args: args.join(" "),
                output,
            })
    }

    /// Whether `dir`'s tracked files differ from its commit.
    pub fn tree(&self, dir: &Path) -> Result<Tree, GitError> {
        let status = self.answer(dir, &["status", "--porcelain", "--untracked-files=no"])?;
        Ok(if status.is_empty() {
            Tree::Clean
        } else {
            Tree::Modified
        })
    }

    /// Whether a remote branch holds `dir`'s current commit.
    pub fn published(&self, dir: &Path) -> Result<Published, GitError> {
        let remotes = self.answer(dir, &["branch", "--remotes", "--contains", "HEAD"])?;
        Ok(if remotes.is_empty() {
            Published::LocalOnly
        } else {
            Published::OnRemote
        })
    }

    /// The files git rewrites when `dir`'s commit or staged tree changes:
    /// its `HEAD` and index, the packed references, and the file of the
    /// branch `HEAD` names. A file listed may not exist (a branch kept only
    /// in `packed-refs`). Editing a tracked file without staging it changes
    /// none of them.
    pub fn revision_files(&self, dir: &Path) -> Result<Vec<PathBuf>, GitError> {
        let dirs = self.dirs(dir)?;
        let mut files = vec![
            dirs.own.join("HEAD"),
            dirs.own.join("index"),
            dirs.common.join("packed-refs"),
        ];
        match self.head_ref(dir)? {
            HeadRef::Named(reference) => files.push(dirs.common.join(reference)),
            HeadRef::Detached => {}
        }
        Ok(files)
    }

    /// The reference `dir`'s `HEAD` names.
    fn head_ref(&self, dir: &Path) -> Result<HeadRef, GitError> {
        // `-q`: a detached `HEAD` exits 1, silently.
        match self.answer(dir, &["symbolic-ref", "-q", "HEAD"]) {
            Ok(reference) => Ok(HeadRef::Named(reference)),
            Err(GitError::Failed {
                exit: Exit::Code(1),
                ..
            }) => Ok(HeadRef::Detached),
            Err(error) => Err(error),
        }
    }

    /// `dir`'s git folders.
    fn dirs(&self, dir: &Path) -> Result<GitDirs, GitError> {
        Ok(GitDirs {
            own: self
                .answer(dir, &["rev-parse", "--absolute-git-dir"])?
                .into(),
            common: self
                .answer(
                    dir,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )?
                .into(),
        })
    }

    fn executable(path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|metadata| {
            if !metadata.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
    }

    /// `git <args>` in `dir`: its trimmed standard output, once it exits
    /// successfully.
    fn answer(&self, dir: &Path, args: &[&str]) -> Result<String, GitError> {
        let joined = || args.join(" ");
        let output = self.run(dir, args).map_err(|source| GitError::NotStarted {
            args: joined(),
            source,
        })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        Err(GitError::Failed {
            args: joined(),
            exit: output.status.into(),
            detail: if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            },
        })
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned place that runs git"
    )]
    fn run(&self, dir: &Path, args: &[&str]) -> io::Result<Output> {
        Command::new(&self.program)
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .output()
    }
}
