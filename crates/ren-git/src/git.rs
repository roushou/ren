use crate::{Branch, Commit, Exit, GitError, NotACommit, Repository, Revision};
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

/// Who a commit Ren makes is by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    pub email: String,
}

/// What became of a branch asked to go once merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchRemoval {
    Deleted,
    /// It holds commits its upstream (or `HEAD`) does not: kept.
    Unmerged,
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

    /// Make `dir` a new, empty repository, on `main` (the branch a canary
    /// follows unless told otherwise), whatever git's own default is.
    pub fn init(&self, dir: &Path) -> Result<(), GitError> {
        self.answer(dir, &["init", "--quiet", "--initial-branch=main"])
            .map(drop)
    }

    /// The commit `dir`'s `HEAD` is at.
    pub fn head_commit(&self, dir: &Path) -> Result<Commit, GitError> {
        let args = ["rev-parse", "--verify", "HEAD"];
        let output = self.answer(dir, &args)?;
        output
            .parse()
            .map_err(|_: NotACommit| GitError::Unexpected {
                args: args.join(" "),
                output,
            })
    }

    /// The URL of `dir`'s repository's remote `remote`, as configured;
    /// none when it has no such remote.
    pub fn remote_url(&self, dir: &Path, remote: &str) -> Result<Option<String>, GitError> {
        let key = format!("remote.{remote}.url");
        let url = self.answer(dir, &["config", "--default", "", "--get", key.as_str()])?;
        Ok((!url.is_empty()).then_some(url))
    }

    /// The commit `revision` names in `dir`'s repository.
    pub fn commit_of(&self, dir: &Path, revision: &Revision) -> Result<Commit, GitError> {
        let named = format!("{revision}^{{commit}}");
        let args = ["rev-parse", "--verify", "--quiet", named.as_str()];
        let output = self.answer(dir, &args)?;
        output
            .parse()
            .map_err(|_: NotACommit| GitError::Unexpected {
                args: args.join(" "),
                output,
            })
    }

    /// The files `commit` holds, by their paths from the repository's root,
    /// `/`-separated as git writes them.
    pub fn files_at(&self, dir: &Path, commit: &Commit) -> Result<Vec<String>, GitError> {
        let args = [
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            "--full-tree",
            commit.as_str(),
        ];
        let output = self.output(dir, &args)?;
        output
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| {
                String::from_utf8(name.to_vec()).map_err(|_| GitError::Unexpected {
                    args: args.join(" "),
                    output: String::from_utf8_lossy(name).into_owned(),
                })
            })
            .collect()
    }

    /// The bytes of the file at `path` (from the repository's root,
    /// `/`-separated) as `commit` holds it.
    pub fn file_at(&self, dir: &Path, commit: &Commit, path: &str) -> Result<Vec<u8>, GitError> {
        let object = format!("{commit}:{path}");
        self.output(dir, &["cat-file", "blob", object.as_str()])
    }

    /// Commit everything in `dir`'s work tree with `message`, as `author`
    /// (`Name <email>`), whatever identity the host's git is set up with.
    pub fn commit_all(&self, dir: &Path, message: &str, author: &Author) -> Result<(), GitError> {
        self.answer(dir, &["add", "--all"])?;
        let name = format!("user.name={}", author.name);
        let email = format!("user.email={}", author.email);
        self.answer(
            dir,
            &[
                "-c",
                &name,
                "-c",
                &email,
                "commit",
                "--quiet",
                "--message",
                message,
            ],
        )
        .map(drop)
    }

    /// A new worktree of `repo` at `path`, on a new branch `branch` from
    /// `repo`'s `HEAD`.
    pub fn add_worktree(&self, repo: &Path, path: &Path, branch: &Branch) -> Result<(), GitError> {
        let path = Self::text(path)?;
        self.answer(
            repo,
            &["worktree", "add", "--quiet", "-b", branch.as_str(), path],
        )
        .map(drop)
    }

    /// Fetch `branch` from `remote` into `repo`, and return the commit it
    /// names there now.
    pub fn fetch(&self, repo: &Path, remote: &str, branch: &Branch) -> Result<Commit, GitError> {
        self.answer(repo, &["fetch", "--quiet", remote, branch.as_str()])?;
        self.commit_of(repo, &Revision::fetched())
    }

    /// Fetch `commit` itself from `remote` into `repo`, by its id: a pull
    /// request's head, or a commit no branch names any more. The remote must
    /// serve it (GitHub serves any commit a ref of the repository reaches,
    /// pull requests' included).
    pub fn fetch_commit(&self, repo: &Path, remote: &str, commit: &Commit) -> Result<(), GitError> {
        self.answer(repo, &["fetch", "--quiet", remote, commit.as_str()])
            .map(drop)
    }

    /// Check `commit` out at `path`, detached: a new worktree of `repo` when
    /// `path` is none yet, else the worktree there, whatever it held. For
    /// a worktree Ren alone uses, such as the one it builds `main` in.
    pub fn check_out_detached(
        &self,
        repo: &Path,
        path: &Path,
        commit: &Commit,
    ) -> Result<(), GitError> {
        if path.exists() {
            self.answer(
                path,
                &[
                    "checkout",
                    "--quiet",
                    "--force",
                    "--detach",
                    commit.as_str(),
                ],
            )
            .map(drop)
        } else {
            let text = Self::text(path)?;
            self.answer(
                repo,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "--detach",
                    text,
                    commit.as_str(),
                ],
            )
            .map(drop)
        }
    }

    /// The newest `limit` commits reachable from `tip`, newest first, each
    /// with its whole message.
    pub fn log(
        &self,
        dir: &Path,
        tip: &Commit,
        limit: u32,
    ) -> Result<Vec<(Commit, String)>, GitError> {
        let count = format!("--max-count={limit}");
        let args = ["log", count.as_str(), "--format=%H%x00%B%x01", tip.as_str()];
        let output = String::from_utf8_lossy(&self.output(dir, &args)?).into_owned();
        output
            .split('\u{1}')
            .map(str::trim_start)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (id, message) = entry.split_once('\0').unwrap_or((entry, ""));
                id.parse()
                    .map(|id| (id, message.to_owned()))
                    .map_err(|_| GitError::Unexpected {
                        args: args.join(" "),
                        output: entry.to_owned(),
                    })
            })
            .collect()
    }

    /// Whether `ancestor` is `descendant` or in its history; a commit this
    /// repository does not have is in no history.
    pub fn contains(
        &self,
        dir: &Path,
        descendant: &Commit,
        ancestor: &Commit,
    ) -> Result<bool, GitError> {
        let args = [
            "merge-base",
            "--is-ancestor",
            ancestor.as_str(),
            descendant.as_str(),
        ];
        match self.answer(dir, &args) {
            Ok(_) => Ok(true),
            // 1: not an ancestor; 128: a commit it does not have.
            Err(GitError::Failed {
                exit: Exit::Code(1 | 128),
                ..
            }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// How many commits `to` has that `from` does not.
    pub fn commits_between(&self, dir: &Path, from: &Commit, to: &Commit) -> Result<u64, GitError> {
        let range = format!("{from}..{to}");
        let args = ["rev-list", "--count", range.as_str()];
        let output = self.answer(dir, &args)?;
        output.parse().map_err(|_| GitError::Unexpected {
            args: args.join(" "),
            output,
        })
    }

    /// Remove the worktree at `path`, from its repository. One holding
    /// changes not committed is refused, and kept.
    pub fn remove_worktree(&self, path: &Path) -> Result<(), GitError> {
        let main = self.main_worktree(path)?;
        let path = Self::text(path)?;
        self.answer(&main, &["worktree", "remove", path]).map(drop)
    }

    /// The main worktree of the repository `path` is a worktree of: the
    /// folder its git folder is in.
    pub fn main_worktree(&self, path: &Path) -> Result<PathBuf, GitError> {
        let dirs = self.dirs(path)?;
        dirs.common
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| GitError::Unexpected {
                args: "rev-parse --git-common-dir".into(),
                output: dirs.common.display().to_string(),
            })
    }

    /// Delete `branch` of `repo` if it is merged; keep it otherwise.
    pub fn delete_merged(&self, repo: &Path, branch: &Branch) -> Result<BranchRemoval, GitError> {
        match self.answer(repo, &["branch", "--delete", "--quiet", branch.as_str()]) {
            Ok(_) => Ok(BranchRemoval::Deleted),
            // `git branch -d` exits 1 for a branch not fully merged.
            Err(GitError::Failed {
                exit: Exit::Code(1),
                ..
            }) => Ok(BranchRemoval::Unmerged),
            Err(error) => Err(error),
        }
    }

    fn text(path: &Path) -> Result<&str, GitError> {
        path.to_str().ok_or_else(|| GitError::Unexpected {
            args: "a path".into(),
            output: path.display().to_string(),
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
        let output = self.output(dir, args)?;
        Ok(String::from_utf8_lossy(&output).trim().to_owned())
    }

    /// `git <args>` in `dir`: its standard output as it wrote it, once it
    /// exits successfully.
    fn output(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let joined = || args.join(" ");
        let output = self.run(dir, args).map_err(|source| GitError::NotStarted {
            args: joined(),
            source,
        })?;
        if output.status.success() {
            return Ok(output.stdout);
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
