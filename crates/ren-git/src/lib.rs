//! Git as Ren reads and runs it, with [`Git`] as its primary type: the git
//! program, found on a search path, and the commands Ren runs with it. A
//! folder's repository is read from disk without the program
//! ([`Git::repository`]). This is the one place Ren reads or runs git.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The git program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    program: PathBuf,
}

/// Why a git command did not answer.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("`git {args}` could not start: {source}")]
    NotStarted {
        args: String,
        #[source]
        source: io::Error,
    },
    #[error("`git {args}` exited with {exit}{detail}")]
    Failed {
        args: String,
        exit: Exit,
        /// Its standard error, after a colon, when it printed any.
        detail: String,
    },
    #[error("unexpected output from `git {args}`: {output:?}")]
    Unexpected { args: String, output: String },
}

/// How a git command exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
    /// The host could not say how it exited.
    Unknown,
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Code(code) => write!(f, "code {code}"),
            Self::Signal(signal) => write!(f, "signal {signal}"),
            Self::Unknown => f.write_str("an unknown status"),
        }
    }
}

impl From<std::process::ExitStatus> for Exit {
    fn from(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return Self::Signal(signal);
            }
        }
        status.code().map_or(Self::Unknown, Self::Code)
    }
}

/// Whether a folder holds a repository, and where its `HEAD` stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repository {
    /// No `.git` in the folder.
    None,
    At(Head),
    /// A `.git` is there, but its `HEAD` could not be read.
    Unreadable,
}

/// What a repository's `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// `ref: refs/heads/<name>`: the branch's name. Another reference is
    /// kept whole.
    Branch(String),
    Detached(CommitId),
}

/// A full commit id: 40 (SHA-1) or 64 (SHA-256) hex digits, lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitId(String);

impl CommitId {
    fn parse(text: &str) -> Option<Self> {
        (matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| Self(text.to_ascii_lowercase()))
    }

    /// Its first `digits` hex digits, as people read it.
    pub fn abbreviated(&self, digits: usize) -> &str {
        self.0.get(..digits).unwrap_or(&self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CommitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
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
pub enum HeadRef {
    /// The full reference, such as `refs/heads/main`.
    Named(String),
    Detached,
}

/// A work tree's git folders, absolute: its own (`HEAD`, `index`), and the
/// one its references live in, which differ in a linked worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDirs {
    pub own: PathBuf,
    pub common: PathBuf,
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

    /// Make `dir` a new, empty repository.
    pub fn init(&self, dir: &Path) -> Result<(), GitError> {
        self.answer(dir, &["init", "--quiet"]).map(drop)
    }

    /// The commit `dir`'s `HEAD` is at.
    pub fn head_commit(&self, dir: &Path) -> Result<CommitId, GitError> {
        let args = ["rev-parse", "--verify", "HEAD"];
        let output = self.answer(dir, &args)?;
        CommitId::parse(&output).ok_or_else(|| GitError::Unexpected {
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

    /// The reference `dir`'s `HEAD` names.
    pub fn head_ref(&self, dir: &Path) -> Result<HeadRef, GitError> {
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
    pub fn dirs(&self, dir: &Path) -> Result<GitDirs, GitError> {
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

    /// Whether `dir` holds a repository, and where its `HEAD` stands, read
    /// from disk. Its `.git` is a folder, or in a linked worktree a file
    /// naming the git folder (`gitdir: <path>`). Links are not followed. A
    /// `.git` that cannot be followed is an unreadable repository, never none.
    pub fn repository(dir: &Path) -> Repository {
        let dot_git = dir.join(".git");
        let metadata = match std::fs::symlink_metadata(&dot_git) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Repository::None,
            Err(_) => return Repository::Unreadable,
        };
        let git_dir = if metadata.is_dir() {
            dot_git
        } else if metadata.is_file() {
            match Self::small(&dot_git).as_deref().and_then(Self::gitlink) {
                Some(path) => dir.join(path),
                None => return Repository::Unreadable,
            }
        } else {
            return Repository::Unreadable;
        };
        Self::small(&git_dir.join("HEAD"))
            .and_then(|text| Self::head(&text))
            .map_or(Repository::Unreadable, Repository::At)
    }

    /// The git folder a `.git` file names, relative to its worktree or
    /// absolute.
    fn gitlink(text: &str) -> Option<&str> {
        let path = text.trim().strip_prefix("gitdir:")?.trim();
        (!path.is_empty()).then_some(path)
    }

    /// What a `HEAD` says: a reference, or a full commit id.
    fn head(text: &str) -> Option<Head> {
        let text = text.trim();
        if let Some(reference) = text.strip_prefix("ref:") {
            let reference = reference.trim();
            let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
            return (!name.is_empty()).then(|| Head::Branch(name.to_owned()));
        }
        CommitId::parse(text).map(Head::Detached)
    }

    /// A small text file's first bytes; `HEAD` and a `.git` file are one line.
    fn small(path: &Path) -> Option<String> {
        use std::io::Read as _;
        let mut text = String::new();
        std::fs::File::open(path)
            .ok()?
            .take(1024)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
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

#[cfg(test)]
mod tests;
