//! A build's steps in a container of their own (docs/plans/instances.md,
//! phase 2). A build runs the commit's own code (`build.rs`, the UI's build
//! script), which may be a pull request's, so a step sees only the checkout
//! it builds, the toolchains read-only, and the deployer's own dependency
//! cache: no home, no keys, and no network. What a step needs from the
//! network is fetched before, on the host, by commands that run none of the
//! commit's code ([`BuildJob::fetch_args`]).

use crate::invocation::{Invocation, InvocationError};
use ren_container::{Container, Engine, Lifetime, Mount, Placement, Privileges, Runtime};
use std::path::{Path, PathBuf};

/// The container a commit's build steps run in, and where it reads and
/// writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildJob {
    image: String,
    /// The checkout of the commit, built in place.
    checkout: PathBuf,
    /// The deployer's cargo home: the registry it fetched into. Never the
    /// operator's.
    cargo_home: PathBuf,
    /// Seen read-only: the host's programs and toolchains.
    toolchains: Vec<PathBuf>,
    /// The search path the steps run with.
    path: Vec<PathBuf>,
}

impl BuildJob {
    /// The container's name: one build runs at a time, replacing any left.
    const NAME: &str = "ren-build";

    pub fn new(
        image: String,
        checkout: PathBuf,
        cargo_home: PathBuf,
        toolchains: Vec<PathBuf>,
        path: Vec<PathBuf>,
    ) -> Self {
        Self {
            image,
            checkout,
            cargo_home,
            toolchains,
            path,
        }
    }

    /// Where a step's home is: in the checkout's build output, so nothing
    /// the step keeps there outlives the checkout or reaches the
    /// operator's.
    fn home(&self) -> PathBuf {
        self.checkout.join("target/.build-home")
    }

    /// The container one step runs in: `argv` in `workdir`, a folder of the
    /// checkout.
    pub fn container(&self, workdir: &Path, argv: &[String]) -> Container {
        let (program, args) = argv
            .split_first()
            .map_or(("", &[][..]), |(program, args)| (program.as_str(), args));
        let path = self
            .path
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(":");
        Container::new(
            Self::NAME,
            self.image.clone(),
            Lifetime::Foreground,
            Placement::Isolated(Runtime::HostKernel),
        )
        .privileges(Privileges::NoNew)
        .mounts(self.toolchains.iter().map(Mount::read_only))
        .mount(Mount::read_write(&self.checkout))
        .mount(Mount::read_write(&self.cargo_home))
        .workdir(workdir)
        .env("HOME", self.home().display().to_string())
        .env("CARGO_HOME", self.cargo_home.display().to_string())
        .env("CARGO_NET_OFFLINE", "true")
        .env("PATH", path)
        .entrypoint(program)
        .args(args.iter().cloned())
    }

    /// Runs one step in its container; fails when it does.
    pub fn run(&self, workdir: &Path, argv: &[String]) -> Result<(), InvocationError> {
        self.prepare()?;
        Invocation::new(Engine::Podman.program())
            .args(Engine::Podman.run_args(&self.container(workdir, argv)))
            .run()
    }

    /// Runs one step in its container and answers what it printed.
    pub fn capture(&self, workdir: &Path, argv: &[String]) -> Result<String, InvocationError> {
        self.prepare()?;
        Invocation::new(Engine::Podman.program())
            .args(Engine::Podman.run_args(&self.container(workdir, argv)))
            .capture()
    }

    /// The folders a step writes in that it cannot make itself.
    fn prepare(&self) -> Result<(), InvocationError> {
        for dir in [self.home(), self.cargo_home.clone()] {
            std::fs::create_dir_all(&dir)
                .map_err(|error| InvocationError::output("mkdir")(error))?;
        }
        Ok(())
    }

    /// The host's fetch of a commit's Rust dependencies, by its lockfile,
    /// into the deployer's cargo home: a download, which runs none of the
    /// commit's code.
    pub fn fetch_args(&self) -> Vec<String> {
        vec![
            "fetch".to_owned(),
            "--locked".to_owned(),
            "--manifest-path".to_owned(),
            self.checkout.join("Cargo.toml").display().to_string(),
        ]
    }

    pub fn cargo_home(&self) -> &Path {
        &self.cargo_home
    }
}

#[cfg(test)]
mod tests {
    use super::BuildJob;
    use ren_container::Engine;
    use std::path::Path;

    fn job() -> BuildJob {
        BuildJob::new(
            "localhost/ren-instance:abc".into(),
            "/home/op/.local/share/ren/preview-factory".into(),
            "/home/op/.local/share/ren/build-cache/cargo".into(),
            vec![
                "/usr".into(),
                "/opt/node".into(),
                "/home/op/.rustup".into(),
                "/home/op/.bun".into(),
            ],
            vec![
                "/home/op/.rustup/toolchains/1.99.0/bin".into(),
                "/home/op/.bun/bin".into(),
                "/usr/local/bin".into(),
                "/usr/bin".into(),
                "/bin".into(),
            ],
        )
    }

    #[test]
    fn a_build_step_sees_only_its_checkout_toolchains_and_the_deployers_cache() {
        let checkout = Path::new("/home/op/.local/share/ren/preview-factory");
        let args = Engine::Podman.run_args(&job().container(
            checkout,
            &[
                "cargo".to_owned(),
                "build".to_owned(),
                "--offline".to_owned(),
            ],
        ));
        let volumes: Vec<&str> = args
            .iter()
            .filter_map(|arg| arg.strip_prefix("--volume="))
            .collect();
        assert_eq!(
            volumes,
            [
                "/usr:/usr:ro",
                "/opt/node:/opt/node:ro",
                "/home/op/.rustup:/home/op/.rustup:ro",
                "/home/op/.bun:/home/op/.bun:ro",
                "/home/op/.local/share/ren/preview-factory:/home/op/.local/share/ren/preview-factory:rw",
                "/home/op/.local/share/ren/build-cache/cargo:/home/op/.local/share/ren/build-cache/cargo:rw",
            ]
        );
        // No network, no new privileges, the operator's own user.
        for flag in [
            "--network=none",
            "--userns=keep-id",
            "--security-opt=no-new-privileges",
            "--rm",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}: {args:?}");
        }
        // Its home is in the checkout's build output, not the operator's.
        assert!(
            args.iter().any(|arg| arg
                == "--env=HOME=/home/op/.local/share/ren/preview-factory/target/.build-home")
        );
        assert!(args.iter().any(|arg| arg == "--env=CARGO_NET_OFFLINE=true"));
        assert_eq!(
            &args[args.len() - 4..],
            [
                "--entrypoint=cargo",
                "localhost/ren-instance:abc",
                "build",
                "--offline"
            ]
        );
    }
}
