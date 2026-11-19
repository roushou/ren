//! Builds of Ren made once per commit, kept in `~/.local/share/ren/builds`:
//! each `<commit>/` holds the release binary (`ren`) and the UI's deployable
//! copy (`ui/`). The canary runs one, and production installs that same one
//! when it is promoted (`install-vps --build`): nothing is rebuilt per
//! instance. A build is made in `.<commit>.partial` and renamed whole, so a
//! folder named for a commit is always complete.

use crate::{
    file::{FileError, FileMode},
    invocation::{Invocation, InvocationError},
    progress::Progress,
    ui::{UiBuild, UiBuildError},
};
use ren_harness::host::HostEnv;
use ren_release::{Attempt, Commit};
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
    #[error("no build of {0} is kept; the canary makes one (`cargo xtask canary run`)")]
    NotBuilt(Commit),
    #[error("the binary built for {expected} reports {found:?}")]
    Revision { expected: Commit, found: String },
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Ui(#[from] UiBuildError),
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
        cargo: &Path,
        checkout: &Path,
        commit: &Commit,
    ) -> Result<StoredBuild, BuildsError> {
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
        Invocation::new(cargo)
            .args(["build", "--release", "--locked", "--package", "ren-rs"])
            .args(["--bin", "ren", "--manifest-path"])
            .arg(checkout.join("Cargo.toml"))
            .run()?;
        let built = checkout.join("target/release/ren");
        let version = Invocation::new(&built).arg("--version").capture()?;
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
        UiBuild::new(env, checkout)?.build_into(&partial.join("ui"))?;
        let dir = self.root.join(commit.to_string());
        fs::rename(&partial, &dir).map_err(FileError::at(&dir))?;
        Ok(StoredBuild { dir })
    }

    /// Removes every build but `keep` and the newest few.
    pub fn prune(&self, keep: &[&Commit]) -> Result<(), BuildsError> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Ok(());
        };
        let mut builds: Vec<(SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                !keep.iter().any(|commit| commit.to_string() == name)
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
