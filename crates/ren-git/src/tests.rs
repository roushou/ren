use crate::{CommitId, Exit, Git, GitError, Head, NotACommitId, Published, Repository, Tree};
use std::path::Path;

/// A repository at `dir` whose `HEAD` holds `head`, written by hand.
fn written(dir: &Path, head: &str) {
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join(".git/HEAD"), head).unwrap();
}

fn branch(name: &str) -> Repository {
    Repository::At(Head::Branch(name.into()))
}

#[test]
fn a_repository_reads_as_its_head_says() {
    let dir = tempfile::tempdir().unwrap();
    let at = |head: &str| {
        let folder = tempfile::tempdir_in(dir.path()).unwrap();
        written(folder.path(), head);
        Git::repository(folder.path())
    };
    assert_eq!(at("ref: refs/heads/feat/upload\n"), branch("feat/upload"));
    assert_eq!(
        at("ref: refs/remotes/origin/main"),
        branch("refs/remotes/origin/main")
    );
    let Repository::At(Head::Detached(commit)) = at("0123456789ABCDEF0123456789abcdef01234567\n")
    else {
        panic!("a full SHA-1 id is a detached HEAD");
    };
    assert_eq!(commit.abbreviated(7), "0123456");
    assert_eq!(commit.as_str(), "0123456789abcdef0123456789abcdef01234567");
    assert!(matches!(
        at(&"a".repeat(64)),
        Repository::At(Head::Detached(_))
    ));
    for garbage in ["", "ref:", "ref: refs/heads/", "0123456", "garbage"] {
        assert_eq!(at(garbage), Repository::Unreadable, "{garbage:?}");
    }
    assert_eq!(Git::repository(dir.path()), Repository::None);
}

#[test]
fn a_commit_id_parses_only_whole() {
    let id: CommitId = "0123456789ABCDEF0123456789abcdef01234567".parse().unwrap();
    assert_eq!(id.to_string(), "0123456789abcdef0123456789abcdef01234567");
    assert!("f".repeat(64).parse::<CommitId>().is_ok());
    for text in [
        "",
        "0123456",
        " 0123456789abcdef0123456789abcdef01234567",
        &"g".repeat(40),
    ] {
        assert_eq!(text.parse::<CommitId>(), Err(NotACommitId(text.to_owned())));
    }
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
            Repository::Unreadable,
            "{text:?}"
        );
    }
    #[cfg(unix)]
    {
        std::fs::remove_file(worktree.join(".git")).unwrap();
        std::os::unix::fs::symlink(main.join(".git"), worktree.join(".git")).unwrap();
        assert_eq!(Git::repository(&worktree), Repository::Unreadable);
    }
}

/// A fake `git` in a folder of its own under `dir`, running `body`.
#[cfg(unix)]
fn fake(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join(name);
    std::fs::create_dir(&bin).unwrap();
    let program = bin.join("git");
    std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[cfg(unix)]
#[test]
fn the_first_git_on_the_path_runs_and_failures_keep_their_exit() {
    let dir = tempfile::tempdir().unwrap();
    let works = fake(
        dir.path(),
        "works",
        "[ \"$*\" = 'init --quiet' ] && touch made",
    );
    let fails = fake(dir.path(), "fails", "echo 'no such thing' >&2; exit 3");
    let not_executable = dir.path().join("plain");
    std::fs::create_dir(&not_executable).unwrap();
    std::fs::write(not_executable.join("git"), "").unwrap();

    let found = Git::find(&[not_executable, works.clone(), fails.clone()]).unwrap();
    found.init(dir.path()).unwrap();
    assert!(dir.path().join("made").is_file(), "git ran in the folder");

    let failed = Git::find(&[fails]).unwrap().init(dir.path()).unwrap_err();
    assert!(matches!(
        failed,
        GitError::Failed {
            exit: Exit::Code(3),
            ..
        }
    ));
    assert_eq!(
        failed.to_string(),
        "`git init --quiet` exited with code 3: no such thing"
    );
    assert!(Git::find(&[dir.path().join("none")]).is_none());

    // Found, then removed before it runs.
    let removed = fake(dir.path(), "removed", "");
    let gone = Git::find(std::slice::from_ref(&removed)).unwrap();
    std::fs::remove_file(removed.join("git")).unwrap();
    assert!(matches!(
        gone.init(dir.path()),
        Err(GitError::NotStarted { .. })
    ));
}

#[cfg(unix)]
#[test]
fn answers_are_read_from_gits_output() {
    let dir = tempfile::tempdir().unwrap();
    let full = "0123456789abcdef0123456789abcdef01234567";
    let git = Git::find(&[fake(
        dir.path(),
        "answers",
        &format!(
            "case \"$*\" in
  'rev-parse --verify HEAD') echo {full} ;;
  'status --porcelain --untracked-files=no') echo ' M src/lib.rs' ;;
  'branch --remotes --contains HEAD') ;;
  'symbolic-ref -q HEAD') exit 1 ;;
  'rev-parse --absolute-git-dir') echo /r/.git/worktrees/w ;;
  'rev-parse --path-format=absolute --git-common-dir') echo /r/.git ;;
esac"
        ),
    )])
    .unwrap();
    assert_eq!(
        git.head_commit(dir.path()).unwrap(),
        full.parse::<CommitId>().unwrap()
    );
    assert_eq!(git.tree(dir.path()).unwrap(), Tree::Modified);
    assert_eq!(git.published(dir.path()).unwrap(), Published::LocalOnly);
    // A detached linked worktree: its own HEAD and index, the shared
    // packed references, and no branch file.
    assert_eq!(
        git.revision_files(dir.path()).unwrap(),
        [
            Path::new("/r/.git/worktrees/w/HEAD"),
            Path::new("/r/.git/worktrees/w/index"),
            Path::new("/r/.git/packed-refs"),
        ]
    );

    let odd = Git::find(&[fake(dir.path(), "odd", "echo not-a-commit")]).unwrap();
    assert!(matches!(
        odd.head_commit(dir.path()),
        Err(GitError::Unexpected { .. })
    ));
}

/// The real git, on a repository it made: what the build script and the
/// installer ask of it. Skipped where no git is installed.
#[test]
fn the_real_git_answers_on_its_own_repository() {
    let dir = tempfile::tempdir().unwrap();
    let git = Git::inherited();
    if git.init(dir.path()).is_err() {
        return;
    }
    // No commit yet.
    assert!(git.head_commit(dir.path()).is_err());
    let files = git.revision_files(dir.path()).unwrap();
    let dot_git = std::fs::canonicalize(dir.path().join(".git")).unwrap();
    assert_eq!(
        files.get(..3).unwrap(),
        ["HEAD", "index", "packed-refs"].map(|file| dot_git.join(file))
    );
    let [.., branch] = files.as_slice() else {
        panic!("{files:?}");
    };
    assert_eq!(files.len(), 4, "on a branch: {files:?}");
    assert!(branch.starts_with(dot_git.join("refs/heads")), "{branch:?}");
    assert_eq!(git.tree(dir.path()).unwrap(), Tree::Clean);
    assert!(matches!(
        Git::repository(dir.path()),
        Repository::At(Head::Branch(_))
    ));
}
