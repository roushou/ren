//! A git repository as Ren reads it from disk: whether a folder holds one,
//! where its git folder is (a `.git` folder, or in a linked worktree a
//! `.git` file naming it), and what its `HEAD` says. The reading is pure:
//! the host says what is at each path (`Entry`), and this module decides
//! what to read next and what it means.

use crate::run::ProcessExit;
use std::path::{Path, PathBuf};

/// What the host found at one path, links not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Missing,
    Folder,
    /// A file's first bytes, as text.
    File(String),
    /// A link, or anything else that is neither a folder nor a file.
    Other,
    /// It is there, but could not be read.
    Unreadable,
}

/// Whether a folder holds a repository, and where its `HEAD` stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Repository {
    /// No `.git` in the folder.
    #[default]
    None,
    At(Head),
    /// A `.git` is there, but its `HEAD` could not be read.
    Unreadable(Unreadable),
}

/// Why a repository's `HEAD` could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// `.git` is neither a folder nor a readable file.
    DotGit,
    /// The `.git` file does not name a git folder.
    Gitlink(GitlinkError),
    /// The git folder holds no readable `HEAD`.
    NoHead,
    Head(HeadError),
}

impl Repository {
    /// `dir`'s repository, asking `at` what is at each path it reads.
    pub fn find(dir: &Path, mut at: impl FnMut(&Path) -> Entry) -> Self {
        let dot_git = dir.join(".git");
        let git_dir = match at(&dot_git) {
            Entry::Missing => return Self::None,
            Entry::Folder => dot_git,
            Entry::File(text) => match Gitlink::parse(&text) {
                Ok(link) => link.resolve(dir),
                Err(error) => return Self::Unreadable(Unreadable::Gitlink(error)),
            },
            Entry::Other | Entry::Unreadable => return Self::Unreadable(Unreadable::DotGit),
        };
        match at(&git_dir.join("HEAD")) {
            Entry::File(text) => Head::parse(&text)
                .map_or_else(|error| Self::Unreadable(Unreadable::Head(error)), Self::At),
            Entry::Missing | Entry::Folder | Entry::Other | Entry::Unreadable => {
                Self::Unreadable(Unreadable::NoHead)
            }
        }
    }

    /// Whether the folder holds a repository, readable or not.
    pub fn exists(&self) -> bool {
        match self {
            Self::None => false,
            Self::At(_) | Self::Unreadable(_) => true,
        }
    }
}

/// A linked worktree's `.git` file, `gitdir: <path>`: its git folder,
/// relative to the worktree or absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Gitlink(PathBuf);

/// Why a `.git` file names no git folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GitlinkError {
    #[error("the .git file does not start with `gitdir:`")]
    NotGitdir,
    #[error("the .git file names no folder")]
    Empty,
}

impl Gitlink {
    fn parse(text: &str) -> Result<Self, GitlinkError> {
        let path = text
            .trim()
            .strip_prefix("gitdir:")
            .ok_or(GitlinkError::NotGitdir)?
            .trim();
        if path.is_empty() {
            return Err(GitlinkError::Empty);
        }
        Ok(Self(PathBuf::from(path)))
    }

    /// The git folder it names; an absolute path stays as it is.
    fn resolve(&self, worktree: &Path) -> PathBuf {
        worktree.join(&self.0)
    }
}

/// What a repository's `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// `ref: refs/heads/<name>`, or another reference, kept whole.
    Branch(BranchName),
    /// A commit, not on a branch.
    Detached(CommitId),
}

/// Why a `HEAD` could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HeadError {
    #[error("HEAD is neither a reference nor a commit id")]
    Unrecognized,
}

impl Head {
    pub fn parse(text: &str) -> Result<Self, HeadError> {
        let text = text.trim();
        if let Some(reference) = text.strip_prefix("ref:") {
            let reference = reference.trim();
            let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
            return BranchName::new(name)
                .map(Self::Branch)
                .ok_or(HeadError::Unrecognized);
        }
        CommitId::parse(text)
            .map(Self::Detached)
            .ok_or(HeadError::Unrecognized)
    }
}

/// A branch's name, never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchName(String);

impl BranchName {
    fn new(name: &str) -> Option<Self> {
        (!name.is_empty()).then(|| Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A full commit id: 40 (SHA-1) or 64 (SHA-256) hex digits, lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitId(String);

impl CommitId {
    fn parse(text: &str) -> Option<Self> {
        (matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| Self(text.to_ascii_lowercase()))
    }

    /// The seven-digit form people read.
    pub fn short(&self) -> &str {
        self.0.get(..7).unwrap_or(&self.0)
    }
}

/// What Ren asks git to do. The host's git runs it, with the program
/// found on the run's search path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCommand {
    /// Make the folder a new, empty repository.
    Init,
}

impl GitCommand {
    /// Its arguments to `git`.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Init => &["init", "--quiet"],
        }
    }
}

/// How a scratch folder was made: a repository of its own, or a plain
/// folder whose changes are not tracked, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Scratch {
    Repository,
    Untracked(Untracked),
}

/// Why a scratch folder is not a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Untracked {
    /// No `git` on the run's search path.
    NoGit,
    /// `git` was found but could not be started.
    NotStarted,
    /// `git init` ran and failed.
    InitFailed(ProcessExit),
}

impl Scratch {
    /// What a `git init` that exited with `exit` made of the folder.
    pub fn after_init(exit: ProcessExit) -> Self {
        match exit {
            ProcessExit::Code(0) => Self::Repository,
            ProcessExit::Code(_) | ProcessExit::Signal(_) | ProcessExit::Unknown => {
                Self::Untracked(Untracked::InitFailed(exit))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BranchName, Entry, GitlinkError, Head, HeadError, ProcessExit, Repository, Scratch,
        Unreadable, Untracked,
    };
    use std::path::Path;

    /// A fake disk: what is at each path; anything else is missing.
    fn disk<'a>(files: &'a [(&'a str, Entry)]) -> impl FnMut(&Path) -> Entry + 'a {
        move |path| {
            files
                .iter()
                .find(|(at, _)| Path::new(at) == path)
                .map_or(Entry::Missing, |(_, entry)| entry.clone())
        }
    }

    fn file(text: &str) -> Entry {
        Entry::File(text.to_owned())
    }

    fn branch(name: &str) -> Repository {
        Repository::At(Head::Branch(BranchName::new(name).unwrap()))
    }

    #[test]
    fn a_folder_without_dot_git_holds_no_repository() {
        assert_eq!(
            Repository::find(Path::new("/w"), disk(&[])),
            Repository::None
        );
        assert!(!Repository::None.exists());
    }

    #[test]
    fn a_repository_reads_its_head() {
        let files = [
            ("/w/.git", Entry::Folder),
            ("/w/.git/HEAD", file("ref: refs/heads/feat/upload\n")),
        ];
        assert_eq!(
            Repository::find(Path::new("/w"), disk(&files)),
            branch("feat/upload")
        );
    }

    #[test]
    fn a_linked_worktree_reads_head_through_its_gitlink() {
        let relative = [
            ("/w/.git", file("gitdir: ../main/.git/worktrees/w\n")),
            (
                "/w/../main/.git/worktrees/w/HEAD",
                file("ref: refs/heads/feature"),
            ),
        ];
        assert_eq!(
            Repository::find(Path::new("/w"), disk(&relative)),
            branch("feature")
        );
        let absolute = [
            ("/w/.git", file("gitdir:/repo/.git/worktrees/w")),
            ("/repo/.git/worktrees/w/HEAD", file("ref: refs/heads/main")),
        ];
        assert_eq!(
            Repository::find(Path::new("/w"), disk(&absolute)),
            branch("main")
        );
    }

    #[test]
    fn a_dot_git_that_cannot_be_followed_is_unreadable_never_absent() {
        let cases = [
            (
                file("nonsense"),
                Unreadable::Gitlink(GitlinkError::NotGitdir),
            ),
            (
                file("gitdir:  \n"),
                Unreadable::Gitlink(GitlinkError::Empty),
            ),
            (Entry::Other, Unreadable::DotGit),
            (Entry::Unreadable, Unreadable::DotGit),
            // Names a folder with no HEAD in it.
            (file("gitdir: /gone"), Unreadable::NoHead),
        ];
        for (dot_git, why) in cases {
            let files = [("/w/.git", dot_git)];
            let found = Repository::find(Path::new("/w"), disk(&files));
            assert!(found.exists());
            assert_eq!(found, Repository::Unreadable(why));
        }
        let garbage = [
            ("/w/.git", Entry::Folder),
            ("/w/.git/HEAD", file("garbage")),
        ];
        assert_eq!(
            Repository::find(Path::new("/w"), disk(&garbage)),
            Repository::Unreadable(Unreadable::Head(HeadError::Unrecognized))
        );
    }

    #[test]
    fn head_reads_branches_and_full_commit_ids() {
        assert_eq!(
            Head::parse("ref: refs/remotes/origin/main"),
            Ok(Head::Branch(
                BranchName::new("refs/remotes/origin/main").unwrap()
            ))
        );
        let Ok(Head::Detached(commit)) = Head::parse("0123456789ABCDEF0123456789abcdef01234567\n")
        else {
            panic!("a full SHA-1 id is a detached HEAD");
        };
        assert_eq!(commit.short(), "0123456");
        assert!(matches!(
            Head::parse(&"a".repeat(64)),
            Ok(Head::Detached(_))
        ));
        for text in [
            "",
            "ref:",
            "ref: refs/heads/",
            "0123456",
            "g123456789abcdef0123456789abcdef01234567",
        ] {
            assert_eq!(Head::parse(text), Err(HeadError::Unrecognized), "{text:?}");
        }
    }

    #[test]
    fn only_a_clean_init_makes_a_scratch_repository() {
        assert_eq!(
            Scratch::after_init(ProcessExit::Code(0)),
            Scratch::Repository
        );
        for exit in [
            ProcessExit::Code(128),
            ProcessExit::Signal(9),
            ProcessExit::Unknown,
        ] {
            assert_eq!(
                Scratch::after_init(exit),
                Scratch::Untracked(Untracked::InitFailed(exit))
            );
        }
    }
}
