//! Builds of Ren made once per commit, kept in `~/.local/share/ren/builds`:
//! each `<commit>/` holds the release binary (`ren`) and the UI's deployable
//! copy (`ui/`). The canary runs one, and production installs that same one
//! when it is promoted (`install-vps --build`): nothing is rebuilt per
//! instance. A build is made in `.<commit>.partial` and renamed whole, so a
//! folder named for a commit is always complete.
//!
//! A commit may be a pull request's, so its build runs none of its code on
//! the host: its dependencies are fetched by its lockfiles into the
//! builds' own caches (`~/.local/share/ren/build-cache`), and each step that runs
//! its code (compiling, the UI's build, the binary's version) runs in a
//! [`BuildJob`] container, with no network and no home.

use crate::{
    build_job::BuildJob,
    file::{FileError, FileMode},
    invocation::{Invocation, InvocationError},
    pod::Pod,
    progress::Progress,
    ui::{UiBuild, UiBuildError},
};
use ren_gateway::release::Attempt;
use ren_git::Commit;
use ren_harness::host::HostEnv;
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// The builds folder.
pub struct Builds {
    root: PathBuf,
}

/// One complete build.
pub struct StoredBuild {
    dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildsError {
    #[error("no build of {0} is kept; the canary makes one (`ren deployer canary run`)")]
    NotBuilt(Commit),
    #[error("the binary built for {expected} reports {found:?}")]
    Revision { expected: Commit, found: String },
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Ui(#[from] UiBuildError),
    #[error(transparent)]
    Engine(#[from] ren_container::EngineError),
    #[error("{0} has no folder above it to mount")]
    Toolchain(PathBuf),
}

impl Builds {
    /// How many builds are kept besides the ones named to keep.
    const NEWEST: usize = 3;

    pub fn new(home: &Path) -> Self {
        Self {
            root: home.join(".local/share/ren/builds"),
        }
    }

    /// The build of `commit`, when one is kept whole.
    pub fn get(&self, commit: &Commit) -> Option<StoredBuild> {
        let build = StoredBuild {
            dir: self.root.join(commit.to_string()),
        };
        (build.ren().is_file() && build.ui().is_dir()).then_some(build)
    }

    /// The build of `commit`, kept or made from `checkout`, which must have
    /// `commit` checked out clean.
    pub fn make(
        &self,
        env: &HostEnv,
        checkout: &Path,
        commit: &Commit,
    ) -> Result<StoredBuild, BuildsError> {
        // One build at a time: the canary's and the factory service's
        // passes build apart, and the same commit is built once.
        let _lock = self.lock()?;
        if let Some(build) = self.get(commit) {
            Progress(&format!(
                "Using the build of {} already kept",
                commit.short()
            ))
            .show();
            return Ok(build);
        }
        Progress(&format!(
            "Building {} once, for the canary and production",
            commit.short()
        ))
        .show();
        let cargo = Self::cargo(checkout)?;
        let cargo = cargo.as_path();
        let ui = UiBuild::new(env, checkout)?;
        let job = self.job(cargo, checkout, &ui)?;
        Pod::ensure_image(checkout)?;
        Progress("Fetching its dependencies by its lockfiles").show();
        Invocation::new(cargo)
            .args(job.fetch_args())
            .env("CARGO_HOME", job.cargo_home())
            .run()?;
        ui.fetch(&self.caches().join("bun"))?;
        Progress("Building it in a container with no network and no home").show();
        job.run(
            checkout,
            &[cargo.display().to_string()]
                .into_iter()
                .chain(
                    [
                        "build",
                        "--release",
                        "--locked",
                        "--offline",
                        "--package",
                        "ren-rs",
                        "--bin",
                        "ren",
                    ]
                    .map(str::to_owned),
                )
                .collect::<Vec<_>>(),
        )?;
        let built = checkout.join("target/release/ren");
        let version = job.capture(
            checkout,
            &[built.display().to_string(), "--version".to_owned()],
        )?;
        if !version.contains(&format!("({commit})")) {
            return Err(BuildsError::Revision {
                expected: commit.clone(),
                found: version.trim().to_owned(),
            });
        }
        let partial = self.root.join(format!(".{commit}.partial"));
        if partial.exists() {
            fs::remove_dir_all(&partial).map_err(FileError::at(&partial))?;
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&partial)
            .map_err(FileError::at(&partial))?;
        let binary = fs::read(&built).map_err(FileError::at(&built))?;
        FileMode::Executable.write(&partial.join("ren"), &binary)?;
        job.run(ui.source(), &ui.build_argv())?;
        ui.stage(&partial.join("ui"))?;
        let dir = self.root.join(commit.to_string());
        fs::rename(&partial, &dir).map_err(FileError::at(&dir))?;
        Ok(StoredBuild { dir })
    }

    /// The store's lock, held while a build is made; released when dropped.
    fn lock(&self) -> Result<fs::File, BuildsError> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)
            .map_err(FileError::at(&self.root))?;
        let path = self.root.join(".lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(FileError::at(&path))?;
        file.lock().map_err(FileError::at(&path))?;
        Ok(file)
    }

    /// The builds' dependency caches, never the operator's: beside the
    /// builds, apart from the factory's worktrees.
    fn caches(&self) -> PathBuf {
        self.root.with_file_name("build-cache")
    }

    /// The cargo that builds `checkout`: its toolchain's own, as rustup
    /// picks it there (the checkout's `rust-toolchain.toml` decides), not
    /// rustup's proxy, so the toolchain's folder is what a build sees.
    fn cargo(checkout: &Path) -> Result<PathBuf, BuildsError> {
        let found = Invocation::new("rustup")
            .current_dir(checkout)
            .args(["which", "cargo"])
            .capture()?;
        Ok(PathBuf::from(found.trim()))
    }

    /// The container `checkout`'s build steps run in: the host's programs
    /// and each toolchain's folder read-only, found from the programs that
    /// build (`cargo`, the UI's Bun and Node) as the host resolves them.
    fn job(&self, cargo: &Path, checkout: &Path, ui: &UiBuild) -> Result<BuildJob, BuildsError> {
        let programs = [cargo, ui.bun(), ui.node()];
        // `/etc/alternatives`: where the host's `cc` and friends point.
        let mut toolchains = vec![PathBuf::from("/usr"), PathBuf::from("/etc/alternatives")];
        let mut path = Vec::new();
        for program in programs {
            let resolved = fs::canonicalize(program).map_err(FileError::at(program))?;
            let bin = resolved
                .parent()
                .ok_or_else(|| BuildsError::Toolchain(resolved.clone()))?;
            // A program's folder is a toolchain's `bin`: the toolchain is
            // the folder above it.
            let toolchain = bin
                .parent()
                .ok_or_else(|| BuildsError::Toolchain(bin.to_path_buf()))?;
            if !toolchains.iter().any(|seen| toolchain.starts_with(seen)) {
                toolchains.push(toolchain.to_path_buf());
            }
            if !path.contains(&bin.to_path_buf()) {
                path.push(bin.to_path_buf());
            }
        }
        path.extend(["/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
        // The repository's git folder, read-only: a worktree's is the
        // repository's, which the build reads to name its commit.
        let git = Invocation::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .capture()?;
        toolchains.push(PathBuf::from(git.trim()));
        Ok(BuildJob::new(
            Pod::image(),
            checkout.to_path_buf(),
            self.caches().join("cargo"),
            toolchains,
            path,
        ))
    }

    /// Removes every build but `keep` and the newest few.
    pub fn prune(&self, keep: &[&Commit]) -> Result<(), BuildsError> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Ok(());
        };
        let mut builds: Vec<(SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            // Builds alone: folders named for a commit, not the store's
            // lock or a build being made.
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.parse::<Commit>().is_ok()
                    && entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && !keep.iter().any(|commit| commit.to_string() == name)
            })
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.path()))
            })
            .collect();
        builds.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, dir) in builds.into_iter().skip(Self::NEWEST) {
            fs::remove_dir_all(&dir).map_err(FileError::at(&dir))?;
        }
        Ok(())
    }
}

impl StoredBuild {
    /// Marks a build production failed to take, so it is not tried again.
    const PROMOTION_FAILED: &str = "promotion-failed";

    /// The release binary.
    pub fn ren(&self) -> PathBuf {
        self.dir.join("ren")
    }

    /// The UI's deployable copy, as `UiBuild::build_into` stages it.
    pub fn ui(&self) -> PathBuf {
        self.dir.join("ui")
    }

    /// Whether production already tried to take this build and failed.
    pub fn attempt(&self) -> Attempt {
        if self.dir.join(Self::PROMOTION_FAILED).exists() {
            Attempt::Failed
        } else {
            Attempt::Untried
        }
    }

    /// Records that production tried to take this build and was rolled back.
    pub fn failed_promotion(&self) -> Result<(), BuildsError> {
        Ok(FileMode::Private.write(&self.dir.join(Self::PROMOTION_FAILED), b"")?)
    }

    /// Forgets a failed promotion, when the operator promotes it by hand.
    pub fn retry_promotion(&self) -> Result<(), BuildsError> {
        let mark = self.dir.join(Self::PROMOTION_FAILED);
        match fs::remove_file(&mark) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(FileError::at(&mark)(error).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Builds;
    use crate::{invocation::Invocation, ui::UiBuild};
    use ren_harness::host::HostEnv;
    use std::path::PathBuf;

    fn host() -> (HostEnv, PathBuf) {
        let env = HostEnv::capture();
        let cargo = env
            .absolute_path("CARGO")
            .unwrap_or_else(|| PathBuf::from("cargo"));
        (env, cargo)
    }

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .to_path_buf()
    }

    /// A build step runs the commit's own code: here, code that tries to
    /// read the operator's home and keys and to reach the internet.
    #[test]
    #[ignore = "runs a container; by hand, with the instance image built"]
    fn a_build_step_cannot_read_the_home_or_reach_out() {
        let (env, cargo) = host();
        let home = env.absolute_path("HOME").unwrap();
        let scratch = tempfile::tempdir_in(home.join(".local/share")).unwrap();
        let builds = Builds {
            root: scratch.path().join("builds"),
        };
        // A checkout is a repository, whose git folder a build reads.
        Invocation::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(scratch.path())
            .run()
            .unwrap();
        let ui = UiBuild::new(&env, &repo()).unwrap();
        let job = builds.job(&cargo, scratch.path(), &ui).unwrap();
        crate::pod::Pod::ensure_image(&repo()).unwrap();
        // The home holds only the folders above what it mounts, and nothing
        // of the operator's own.
        let listed = job
            .capture(
                scratch.path(),
                &[
                    "/bin/ls".to_owned(),
                    "-A".to_owned(),
                    home.display().to_string(),
                ],
            )
            .unwrap();
        let mut entries: Vec<&str> = listed.lines().collect();
        entries.sort_unstable();
        assert_eq!(entries, [".bun", ".local", ".rustup"], "{listed}");
        for (what, step) in [
            ("the keys", format!("ls {}/.ssh", home.display())),
            (
                "the Git config",
                format!("cat {}/.gitconfig", home.display()),
            ),
            ("the SSH agent", "test -n \"$SSH_AUTH_SOCK\"".to_owned()),
            ("the internet", "getent hosts example.com".to_owned()),
            (
                "an address",
                "timeout 5 bash -c 'echo > /dev/tcp/1.1.1.1/443'".to_owned(),
            ),
        ] {
            let argv = ["/bin/sh".to_owned(), "-c".to_owned(), step];
            assert!(
                job.run(scratch.path(), &argv).is_err(),
                "a build step reached {what}"
            );
        }
        // While what it was given, it uses.
        let argv = [cargo.display().to_string(), "--version".to_owned()];
        assert!(
            job.capture(scratch.path(), &argv)
                .unwrap()
                .starts_with("cargo ")
        );
    }

    /// The commit at `HEAD`, built into a store of its own as the factory
    /// builds: fetched on the host, built in containers.
    #[test]
    #[ignore = "builds Ren in release; by hand, minutes"]
    fn a_commit_is_built_in_containers() {
        let (env, _) = host();
        let home = env.absolute_path("HOME").unwrap();
        let scratch = tempfile::tempdir_in(home.join(".local/share")).unwrap();
        let checkout = scratch.path().join("checkout");
        Invocation::new("git")
            .arg("-C")
            .arg(repo())
            .args(["worktree", "add", "--detach"])
            .arg(&checkout)
            .arg("HEAD")
            .run()
            .unwrap();
        let commit = Invocation::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "HEAD"])
            .capture()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let builds = Builds {
            root: scratch.path().join("builds"),
        };
        let made = builds.make(&env, &checkout, &commit);
        Invocation::new("git")
            .arg("-C")
            .arg(repo())
            .args(["worktree", "remove", "--force"])
            .arg(&checkout)
            .run()
            .unwrap();
        let build = made.unwrap();
        assert!(build.ren().is_file());
        assert!(build.ui().join(UiBuild::entry()).is_dir());
    }
}
