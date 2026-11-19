//! Building the UI: `ui/`, built with Bun into a complete copy that Node
//! runs from wherever it is put (ui/README.md): the build, `package.json`,
//! `bun.lock`, the workspace packages, and production dependencies.
//! `install-vps` deploys the copy as `ren-ui.service` (`install::ui`); the
//! dev slot runs it as a transient unit.
use crate::{
    file::FileError,
    invocation::{Invocation, InvocationError},
    progress::Progress,
};
use ren_harness::host::HostEnv;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

/// Why the UI could not be built.
#[derive(Debug, thiserror::Error)]
pub enum UiBuildError {
    #[error("{0} is not on PATH")]
    MissingProgram(&'static str),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    File(#[from] FileError),
}

/// The UI's sources in a checkout, and the programs that build and run it.
pub struct UiBuild {
    source: PathBuf,
    node: PathBuf,
    bun: PathBuf,
}

impl UiBuild {
    /// The SvelteKit app, within `ui/`, the Bun workspace's root.
    const APP: &str = "apps/web";
    /// The folder of the UI's workspace packages (`package.json`'s `workspaces`).
    const PACKAGES: &str = "packages";

    /// What Node runs, from a built copy's root: the app's build.
    pub fn entry() -> PathBuf {
        Path::new(Self::APP).join("build")
    }

    /// The UI of the checkout at `repo`, with the Node and Bun on `PATH`.
    pub fn new(env: &HostEnv, repo: &Path) -> Result<Self, UiBuildError> {
        Ok(Self {
            source: repo.join("ui"),
            node: Self::program(env, "node")?,
            bun: Self::program(env, "bun")?,
        })
    }

    /// The Node on PATH, which runs a built UI.
    pub fn node_on(env: &HostEnv) -> Result<PathBuf, UiBuildError> {
        Self::program(env, "node")
    }

    /// The first executable `name` on PATH, as an absolute path.
    fn program(env: &HostEnv, name: &'static str) -> Result<PathBuf, UiBuildError> {
        env.get("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|dir| Path::new(dir).join(name))
            .find(|path| {
                path.is_absolute()
                    && fs::metadata(path)
                        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
            .ok_or(UiBuildError::MissingProgram(name))
    }

    /// Builds the UI and stages a complete deployable copy in `staged`:
    /// the build, `package.json`, `bun.lock`, the workspace packages, and
    /// production dependencies.
    pub fn build_into(&self, staged: &Path) -> Result<(), UiBuildError> {
        Progress("Building the UI").show();
        Invocation::new(&self.bun)
            .args(["install", "--frozen-lockfile"])
            .current_dir(&self.source)
            .run()?;
        Invocation::new(&self.bun)
            .args(["run", "build"])
            .current_dir(&self.source)
            .run()?;
        Progress(&format!("Staging the UI in {}", staged.display())).show();
        if staged.exists() {
            fs::remove_dir_all(staged).map_err(FileError::at(staged))?;
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(staged)
            .map_err(FileError::at(staged))?;
        // The workspace's shape, so the frozen install below finds every
        // member its lockfile names: the root's manifest and lockfile, the
        // app's manifest and build, and the packages.
        for file in ["package.json", "bun.lock"] {
            let target = staged.join(file);
            fs::copy(self.source.join(file), &target).map_err(FileError::at(&target))?;
        }
        let app = staged.join(Self::APP);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&app)
            .map_err(FileError::at(&app))?;
        let manifest = app.join("package.json");
        fs::copy(self.source.join(Self::APP).join("package.json"), &manifest)
            .map_err(FileError::at(&manifest))?;
        Invocation::new("cp")
            .arg("-R")
            .arg(self.source.join(Self::entry()))
            .arg(staged.join(Self::entry()))
            .run()?;
        // The UI's workspace packages: the lockfile names them, so a
        // frozen install refuses a copy without them.
        let packages = self.source.join(Self::PACKAGES);
        if packages.is_dir() {
            Invocation::new("cp")
                .arg("-R")
                .arg(&packages)
                .arg(staged.join(Self::PACKAGES))
                .run()?;
        }
        Invocation::new(&self.bun)
            .args([
                "install",
                "--production",
                "--frozen-lockfile",
                "--ignore-scripts",
            ])
            .current_dir(staged)
            .run()?;
        Ok(())
    }

    /// The Node that runs the UI.
    pub fn node(&self) -> &Path {
        &self.node
    }
}
