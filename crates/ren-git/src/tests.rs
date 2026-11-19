use crate::{
    Branch, BranchRemoval, CommitId, Exit, Git, GitError, Head, NotACommitId, NotARevision,
    Published, Repository, Revision, Tree,
};
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
fn a_revision_is_never_an_option_or_a_path() {
    for text in ["origin/main", "HEAD~1", "v0.1.0", "0123456789abcdef"] {
        assert_eq!(text.parse::<Revision>().unwrap().as_str(), text);
    }
    for text in [
        "",
        "-n",
        "--output=x",
        "main:Cargo.toml",
        "main branch",
        "a\nb",
    ] {
        assert_eq!(
            text.parse::<Revision>(),
            Err(NotARevision(text.to_owned())),
            "{text:?}"
        );
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

/// Taken by every test that writes a fake `git` or starts a process. A
/// script is written through an open file; a process another test thread
/// starts meanwhile inherits that file until it execs, and running the
/// script while any process holds it open for writing fails with "text file
/// busy" (`ETXTBSY`). Taking turns removes the race.
static PROCESSES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// This test's turn to write fakes and start processes.
fn turn() -> std::sync::MutexGuard<'static, ()> {
    PROCESSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    let _turn = turn();
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
    let _turn = turn();
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

    let at = Git::find(&[fake(
        dir.path(),
        "at",
        &format!(
            "case \"$*\" in
  'rev-parse --verify --quiet origin/main^{{commit}}') echo {full} ;;
  'ls-tree -r -z --name-only --full-tree {full}') printf 'docs/a b.md\\0Cargo.toml\\0' ;;
  'cat-file blob {full}:docs/a b.md') printf 'line\\n\\0bytes' ;;
  *) exit 128 ;;
esac"
        ),
    )])
    .unwrap();
    let commit = at
        .commit_of(dir.path(), &"origin/main".parse().unwrap())
        .unwrap();
    assert_eq!(commit, full.parse::<CommitId>().unwrap());
    assert_eq!(
        at.files_at(dir.path(), &commit).unwrap(),
        ["docs/a b.md", "Cargo.toml"]
    );
    assert_eq!(
        at.file_at(dir.path(), &commit, "docs/a b.md").unwrap(),
        b"line\n\0bytes",
        "a file's bytes as they are, not trimmed"
    );
    assert!(matches!(
        at.file_at(dir.path(), &commit, "missing"),
        Err(GitError::Failed {
            exit: Exit::Code(128),
            ..
        })
    ));

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
    let _turn = turn();
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

#[test]
fn a_branch_name_is_one_git_takes() {
    for name in ["ren/1a2b3c4d", "feat/self-build", "main"] {
        assert_eq!(name.parse::<Branch>().unwrap().as_str(), name);
    }
    for name in [
        "", "-x", "/x", "x/", "x.", "a..b", "a b", "a:b", "a~1", "x.lock", "a//b", "a@{1}",
    ] {
        assert!(name.parse::<Branch>().is_err(), "{name:?}");
    }
}

/// `file` written and committed in `dir`, by a fixed author.
fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    let author = crate::Author {
        name: "Ren".into(),
        email: "ren@example.com".into(),
    };
    Git::inherited().commit_all(dir, file, &author).unwrap();
}

/// The real git, making, keeping, and removing worktrees. Skipped where no
/// git is installed.
#[test]
fn worktrees_are_made_on_a_new_branch_and_removed_only_when_clean() {
    let _turn = turn();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let git = Git::inherited();
    if git.init(&repo).is_err() {
        return;
    }
    commit(&repo, "README.md");
    let branch: Branch = "ren/1a2b3c4d".parse().unwrap();
    let tree = dir.path().join("tree");
    git.add_worktree(&repo, &tree, &branch).unwrap();
    assert_eq!(
        Git::repository(&tree),
        Repository::At(Head::Branch("ren/1a2b3c4d".into()))
    );
    // Changes not committed keep it.
    std::fs::write(tree.join("notes.txt"), "draft").unwrap();
    assert!(git.remove_worktree(&tree).is_err());
    assert!(tree.join("notes.txt").is_file());
    std::fs::remove_file(tree.join("notes.txt")).unwrap();
    git.remove_worktree(&tree).unwrap();
    assert!(!tree.exists());
    // Nothing on it but what main has: merged, and deleted.
    assert_eq!(
        git.delete_merged(&repo, &branch).unwrap(),
        BranchRemoval::Deleted
    );

    // A branch with its own commit is kept.
    let kept: Branch = "ren/5e6f7a8b".parse().unwrap();
    git.add_worktree(&repo, &tree, &kept).unwrap();
    commit(&tree, "change.txt");
    git.remove_worktree(&tree).unwrap();
    assert_eq!(
        git.delete_merged(&repo, &kept).unwrap(),
        BranchRemoval::Unmerged
    );
}

/// The real git: what a fetch brings is checked out in a detached worktree
/// of Ren's own, moved on by the next fetch, and counted behind. Skipped
/// where no git is installed.
#[test]
fn a_fetched_branch_is_checked_out_detached_and_counted() {
    let _turn = turn();
    let dir = tempfile::tempdir().unwrap();
    let (origin, local) = (dir.path().join("origin"), dir.path().join("local"));
    std::fs::create_dir(&origin).unwrap();
    std::fs::create_dir(&local).unwrap();
    let git = Git::inherited();
    if git.init(&origin).is_err() {
        return;
    }
    git.init(&local).unwrap();
    commit(&origin, "README.md");
    let main = git.head_commit(&origin).unwrap();
    let Repository::At(Head::Branch(name)) = Git::repository(&origin) else {
        panic!("origin is on a branch");
    };
    let branch: Branch = name.parse().unwrap();
    let remote = origin.to_str().unwrap();
    assert_eq!(git.fetch(&local, remote, &branch).unwrap(), main);

    let tree = dir.path().join("factory");
    git.check_out_detached(&local, &tree, &main).unwrap();
    assert_eq!(git.head_commit(&tree).unwrap(), main);
    assert_eq!(
        Git::repository(&tree),
        Repository::At(Head::Detached(main.clone()))
    );

    commit(&origin, "CHANGELOG.md");
    commit(&origin, "LICENSE");
    let tip = git.fetch(&local, remote, &branch).unwrap();
    assert_eq!(git.commits_between(&local, &main, &tip).unwrap(), 2);
    assert_eq!(git.commits_between(&local, &tip, &main).unwrap(), 0);
    let log = git.log(&local, &tip, 2).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].0, tip);
    assert_eq!(log[0].1.trim(), "LICENSE");
    assert_eq!(log[1].1.trim(), "CHANGELOG.md");
    assert!(git.contains(&local, &tip, &main).unwrap());
    assert!(git.contains(&local, &tip, &tip).unwrap());
    assert!(!git.contains(&local, &main, &tip).unwrap());
    let absent: crate::CommitId = "0123456789012345678901234567890123456789".parse().unwrap();
    assert!(!git.contains(&local, &tip, &absent).unwrap());
    // What a build left in the worktree does not hold the next checkout.
    std::fs::write(tree.join("README.md"), "built over").unwrap();
    git.check_out_detached(&local, &tree, &tip).unwrap();
    assert_eq!(git.head_commit(&tree).unwrap(), tip);
    assert!(tree.join("LICENSE").is_file());
}
