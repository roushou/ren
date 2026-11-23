use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::io;
use std::path::Path;

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
    Detached(Commit),
}

/// A full commit id: 40 (SHA-1) or 64 (SHA-256) hex digits, lowercase.
/// Parsed from text with [`str::parse`], which lowercases it; written and
/// read as that text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub struct Commit(String);

/// Text that is not a full commit id, kept as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a full commit id: {0:?}")]
pub struct NotACommit(pub String);

/// A branch's name, as `git check-ref-format --branch` takes one: no
/// space, control character, `~^:?*[\\`, `..`, `@{`, or `//`; not starting
/// with `-` or `/`, nor ending with `/`, `.`, or `.lock`. Parsed from text
/// with [`str::parse`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Branch(String);

/// Text that is not a branch name, kept as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a branch name: {0:?}")]
pub struct NotABranch(pub String);

impl std::str::FromStr for Branch {
    type Err = NotABranch;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = text.is_empty()
            || text.starts_with(['-', '/'])
            || text.ends_with(['/', '.'])
            || text.ends_with(".lock")
            || ["..", "@{", "//", "/."]
                .iter()
                .any(|bad| text.contains(bad))
            || text
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
        if refused {
            Err(NotABranch(text.to_owned()))
        } else {
            Ok(Self(text.to_owned()))
        }
    }
}

impl Branch {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Branch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A revision as git reads one: a commit id, a branch or tag, or an
/// expression such as `origin/main` or `HEAD~1`. Never an option (no
/// leading `-`), and never a path in a revision (no `:`), so it is passed to
/// git as it is. Parsed from text with [`str::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision(String);

/// Text that cannot name a revision, kept as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a revision git reads as one: {0:?}")]
pub struct NotARevision(pub String);

impl Repository {
    /// `dir`'s repository, read from disk. Its `.git` is a folder, or in a
    /// linked worktree a file naming the git folder (`gitdir: <path>`).
    /// Links are not followed. A `.git` that cannot be followed is an
    /// unreadable repository, never none.
    pub(crate) fn read(dir: &Path) -> Self {
        let dot_git = dir.join(".git");
        let metadata = match std::fs::symlink_metadata(&dot_git) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::None,
            Err(_) => return Self::Unreadable,
        };
        let git_dir = if metadata.is_dir() {
            dot_git
        } else if metadata.is_file() {
            match Self::small(&dot_git).as_deref().and_then(Self::gitlink) {
                Some(path) => dir.join(path),
                None => return Self::Unreadable,
            }
        } else {
            return Self::Unreadable;
        };
        Self::small(&git_dir.join("HEAD"))
            .and_then(|text| Head::parse(&text))
            .map_or(Self::Unreadable, Self::At)
    }

    /// The git folder a `.git` file names, relative to its worktree or
    /// absolute.
    fn gitlink(text: &str) -> Option<&str> {
        let path = text.trim().strip_prefix("gitdir:")?.trim();
        (!path.is_empty()).then_some(path)
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
}

impl Head {
    /// What a `HEAD` says: a reference, or a full commit id.
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(reference) = text.strip_prefix("ref:") {
            let reference = reference.trim();
            let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
            return (!name.is_empty()).then(|| Self::Branch(name.to_owned()));
        }
        text.parse().ok().map(Self::Detached)
    }
}

impl std::str::FromStr for Commit {
    type Err = NotACommit;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(text.to_ascii_lowercase()))
        } else {
            Err(NotACommit(text.to_owned()))
        }
    }
}

impl Commit {
    const SHORT: usize = 12;

    /// Its first `digits` hex digits, as people read it.
    pub fn abbreviated(&self, digits: usize) -> &str {
        self.0.get(..digits).unwrap_or(&self.0)
    }

    /// The first 12 digits, as git abbreviates in a large repository.
    pub fn short(&self) -> &str {
        self.abbreviated(Self::SHORT)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for Revision {
    type Err = NotARevision;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let readable = !text.is_empty()
            && !text.starts_with('-')
            && !text
                .chars()
                .any(|char| char == ':' || char.is_whitespace() || char.is_control());
        if readable {
            Ok(Self(text.to_owned()))
        } else {
            Err(NotARevision(text.to_owned()))
        }
    }
}

impl Revision {
    /// What the last fetch brought: `FETCH_HEAD`.
    pub(crate) fn fetched() -> Self {
        Self("FETCH_HEAD".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for Commit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
