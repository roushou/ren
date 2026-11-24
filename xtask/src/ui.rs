//! Building the UI: `ui/`, built with Bun into a complete copy that Node
//! runs from wherever it is put (ui/README.md): the build, `package.json`,
//! `bun.lock`, the workspace packages, and production dependencies.
//! `install-vps` deploys the copy as `ren-ui.service` (`install::ui`); the
//! dev slot runs it as a transient unit. Either may run it in a container
//! of its own ([`UiContainer`]).
use crate::{
    file::FileError,
    invocation::{Invocation, InvocationError},
    progress::Progress,
};
use ren_container::{Container, Engine, Environment, Lifetime, Mount, Placement, Privileges};
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

/// A deployed copy of the UI in a container of its own, run by the instance
/// image's Node (docs/plans/self-build.md, phase 11). It sees, each at its
/// host path and read-only, only the copy it runs and the daemon's control
/// token file, which the daemon rewrites in place; it shares the host's
/// network and its unit's environment, and runs as the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct UiContainer {
    name: String,
    image: String,
    dir: PathBuf,
    token: Option<PathBuf>,
}

impl UiContainer {
    /// Seconds Podman gives the UI to stop before killing it.
    const STOP_SECONDS: u32 = 30;

    /// `name`'s UI, running the copy deployed in `dir` from `image`, reading
    /// the control token at `token` when it is given one.
    pub fn new(name: &str, image: String, dir: &Path, token: Option<&Path>) -> Self {
        Self {
            name: format!("ren-{name}-ui"),
            image,
            dir: dir.to_path_buf(),
            token: token.map(Path::to_path_buf),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `podman run` arguments that run it in the foreground, as a unit's
    /// main process: replacing a container of its name left behind, and
    /// removed when it exits.
    pub fn run_args(&self) -> Vec<String> {
        Engine::Podman.run_args(
            &Container::new(
                self.name.clone(),
                self.image.clone(),
                Lifetime::Foreground,
                Placement::Host,
            )
            // Its listener, the daemon's address, and the token file's path
            // are its unit's, as they are for a UI on the host.
            .environment(Environment::Host)
            .privileges(Privileges::NoNew)
            .stop_timeout(Self::STOP_SECONDS)
            .workdir(&self.dir)
            .mount(Mount::read_only(&self.dir))
            .mounts(self.token.iter().map(Mount::read_only))
            .args(["node".to_owned(), UiBuild::entry().display().to_string()]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::UiContainer;
    use std::path::Path;

    #[test]
    fn it_sees_its_copy_and_the_token_read_only_and_nothing_else() {
        let container = UiContainer::new(
            "production",
            "localhost/ren-instance:abc".to_owned(),
            Path::new("/home/op/.local/share/ren-ui"),
            Some(Path::new("/home/op/.local/state/ren/control.token")),
        );
        assert_eq!(container.name(), "ren-production-ui");
        let args = container.run_args();
        let volumes: Vec<&str> = args
            .iter()
            .filter_map(|arg| arg.strip_prefix("--volume="))
            .collect();
        assert_eq!(
            volumes,
            [
                "/home/op/.local/share/ren-ui:/home/op/.local/share/ren-ui:ro",
                "/home/op/.local/state/ren/control.token:/home/op/.local/state/ren/control.token:ro",
            ]
        );
        for flag in [
            "--env-host",
            "--network=host",
            "--userns=keep-id",
            "--security-opt=no-new-privileges",
            "--workdir=/home/op/.local/share/ren-ui",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag} in {args:?}");
        }
        assert_eq!(
            args[args.len() - 3..],
            ["localhost/ren-instance:abc", "node", "apps/web/build"]
        );
    }

    #[test]
    fn without_a_token_it_sees_only_its_copy() {
        let container = UiContainer::new(
            "dev",
            "localhost/ren-instance:abc".to_owned(),
            Path::new("/home/op/.local/share/ren-dev/ui"),
            None,
        );
        let volumes = container
            .run_args()
            .iter()
            .filter(|arg| arg.starts_with("--volume="))
            .count();
        assert_eq!(volumes, 1);
    }
}
