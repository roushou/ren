//! Git on this host: a folder's repository read from disk, and the git
//! program a run would use, run in a folder. The one place Ren reads or
//! runs git.

use crate::host::{HostTools, ProcessHost};
use ren_depot::{ToolHost, package::ProgramName};
use ren_supervisor::{ProjectGit, Scratch, Untracked};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The git program found on a search path.
#[derive(Debug, Clone)]
pub(crate) struct Git {
    program: PathBuf,
}

impl Git {
    /// The git on `search_path`, the first folder first.
    pub(crate) fn find(search_path: Vec<PathBuf>) -> Option<Self> {
        HostTools::new(search_path)
            .locate(&ProgramName::GIT)
            .map(|program| Self { program })
    }

    /// Make `dir` a new, empty repository.
    pub(crate) fn init(&self, dir: &Path) -> Scratch {
        let mut git = std::process::Command::new(&self.program);
        git.args(["init", "--quiet"])
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[expect(
            clippy::disallowed_methods,
            reason = "the one sanctioned place that runs git"
        )]
        let status = git.status();
        match status {
            Ok(status) if status.success() => Scratch::Repository,
            Ok(status) => Scratch::Untracked(Untracked::InitFailed(ProcessHost::exit(status))),
            Err(error) => {
                tracing::debug!(%error, "cannot start git");
                Scratch::Untracked(Untracked::NotStarted)
            }
        }
    }

    /// Whether `dir` holds a repository, and where its `HEAD` stands. Its
    /// `.git` is a folder, or in a linked worktree a file naming the git
    /// folder (`gitdir: <path>`). Links are not followed. A `.git` that
    /// cannot be followed is an unreadable repository, never none.
    pub(crate) fn repository(dir: &Path) -> ProjectGit {
        let dot_git = dir.join(".git");
        let metadata = match std::fs::symlink_metadata(&dot_git) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return ProjectGit::None,
            Err(_) => return ProjectGit::Unreadable,
        };
        let git_dir = if metadata.is_dir() {
            dot_git
        } else if metadata.is_file() {
            match Self::small(&dot_git).as_deref().and_then(Self::gitlink) {
                Some(path) => dir.join(path),
                None => return ProjectGit::Unreadable,
            }
        } else {
            return ProjectGit::Unreadable;
        };
        Self::small(&git_dir.join("HEAD")).map_or(ProjectGit::Unreadable, |text| Self::head(&text))
    }

    /// The git folder a `.git` file names, relative to its worktree or
    /// absolute.
    fn gitlink(text: &str) -> Option<&str> {
        let path = text.trim().strip_prefix("gitdir:")?.trim();
        (!path.is_empty()).then_some(path)
    }

    /// What a `HEAD` says: `ref: refs/heads/<branch>` (another reference
    /// is kept whole), or a full commit id, 40 or 64 hex digits.
    fn head(text: &str) -> ProjectGit {
        let text = text.trim();
        if let Some(reference) = text.strip_prefix("ref:") {
            let reference = reference.trim();
            let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
            return if name.is_empty() {
                ProjectGit::Unreadable
            } else {
                ProjectGit::Branch {
                    name: name.to_owned(),
                }
            };
        }
        if matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            ProjectGit::Detached {
                commit: text
                    .chars()
                    .take(7)
                    .collect::<String>()
                    .to_ascii_lowercase(),
            }
        } else {
            ProjectGit::Unreadable
        }
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

#[cfg(test)]
mod tests {
    use super::Git;
    use ren_supervisor::{ProcessExit, ProjectGit, Scratch, Untracked};
    use std::path::Path;

    /// A repository at `dir` whose `HEAD` holds `head`.
    fn repo(dir: &Path, head: &str) {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), head).unwrap();
    }

    fn branch(name: &str) -> ProjectGit {
        ProjectGit::Branch { name: name.into() }
    }

    #[test]
    fn a_repository_reads_as_its_head_says() {
        let dir = tempfile::tempdir().unwrap();
        let at = |head: &str| {
            let folder = tempfile::tempdir_in(dir.path()).unwrap();
            repo(folder.path(), head);
            Git::repository(folder.path())
        };
        assert_eq!(at("ref: refs/heads/feat/upload\n"), branch("feat/upload"));
        assert_eq!(
            at("ref: refs/remotes/origin/main"),
            branch("refs/remotes/origin/main")
        );
        assert_eq!(
            at("0123456789ABCDEF0123456789abcdef01234567\n"),
            ProjectGit::Detached {
                commit: "0123456".into()
            }
        );
        assert!(matches!(at(&"a".repeat(64)), ProjectGit::Detached { .. }));
        for garbage in ["", "ref:", "ref: refs/heads/", "0123456", "garbage"] {
            assert_eq!(at(garbage), ProjectGit::Unreadable, "{garbage:?}");
        }
        assert_eq!(Git::repository(dir.path()), ProjectGit::None);
    }

    #[test]
    fn a_linked_worktree_reads_head_through_its_gitdir_file() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let linked = main.join(".git/worktrees/w");
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(linked.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        let worktree = dir.path().join("w");
        std::fs::create_dir(&worktree).unwrap();

        std::fs::write(worktree.join(".git"), "gitdir: ../main/.git/worktrees/w\n").unwrap();
        assert_eq!(Git::repository(&worktree), branch("feature"));
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir:{}", linked.display()),
        )
        .unwrap();
        assert_eq!(Git::repository(&worktree), branch("feature"));

        // A `.git` that cannot be followed is unreadable, never no repository.
        for text in ["nonsense", "gitdir:  \n", "gitdir: ../gone"] {
            std::fs::write(worktree.join(".git"), text).unwrap();
            assert_eq!(
                Git::repository(&worktree),
                ProjectGit::Unreadable,
                "{text:?}"
            );
        }
        #[cfg(unix)]
        {
            std::fs::remove_file(worktree.join(".git")).unwrap();
            std::os::unix::fs::symlink(main.join(".git"), worktree.join(".git")).unwrap();
            assert_eq!(Git::repository(&worktree), ProjectGit::Unreadable);
        }
    }

    #[cfg(unix)]
    #[test]
    fn init_reports_how_the_folder_was_made() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let git = |name: &str, body: &str| {
            let bin = dir.path().join(name);
            std::fs::create_dir(&bin).unwrap();
            let program = bin.join("git");
            std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            bin
        };
        let works = git("works", "[ \"$*\" = 'init --quiet' ] && touch made");
        let fails = git("fails", "exit 3");

        // The first folder on the path wins.
        let found = Git::find(vec![works.clone(), fails.clone()]).unwrap();
        assert_eq!(found.init(dir.path()), Scratch::Repository);
        assert!(dir.path().join("made").is_file(), "git ran in the folder");
        assert_eq!(
            Git::find(vec![fails]).unwrap().init(dir.path()),
            Scratch::Untracked(Untracked::InitFailed(ProcessExit::Code(3)))
        );
        assert!(Git::find(vec![dir.path().join("none")]).is_none());
    }
}
