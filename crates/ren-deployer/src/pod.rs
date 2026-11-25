//! A rootless Podman pod xtask runs a replica in (docs/plans/self-build.md,
//! phase 7): the canary, or a worktree's preview. Each of its containers
//! sees only what it is given: the kept build it runs, read-only, and the
//! replica's own configuration and data, each mounted at its host path so
//! every path reads the same inside and out. The pod shares the host's
//! network, so its processes listen on the same loopback addresses the
//! tailnet routes point at; `ren` listens only on loopback, which a
//! published port cannot reach. Containers run as the operator's own user
//! (`--userns=keep-id`), so what they write is the operator's.

use crate::progress::Progress;
use ren_container::{Engine, EngineError, Launch, Lifetime, Mount, Placement};
use std::path::{Path, PathBuf};

/// One pod, named `ren-<name>`; its containers `ren-<name>-<role>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pod {
    name: String,
}

/// A container to start in a pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// Its role in the pod: `daemon` or `ui`.
    pub role: &'static str,
    /// Where its command runs; the image's own folder when none.
    pub workdir: Option<PathBuf>,
    pub mounts: Vec<Mount>,
    pub environment: Vec<(String, String)>,
    pub command: Vec<String>,
}

impl Pod {
    /// The image's recipe, which names it.
    const CONTAINERFILE: &str = include_str!("../../../deploy/instance/Containerfile");
    /// Seconds a container has to stop before it is killed: `ren` gives its
    /// requests 10, then drains capture.
    const STOP_SECONDS: u32 = 30;

    pub fn new(name: &str) -> Self {
        Self {
            name: format!("ren-{name}"),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The image, tagged by its recipe's digest.
    pub fn image() -> String {
        let digest = blake3::hash(Self::CONTAINERFILE.as_bytes()).to_hex();
        format!("localhost/ren-instance:{}", &digest[..16])
    }

    /// Builds the image from `repo`'s recipe unless it is built already.
    pub fn ensure_image(repo: &Path) -> Result<(), EngineError> {
        let image = Self::image();
        if Engine::Podman.image_exists(&image)? {
            return Ok(());
        }
        let recipe = repo.join("deploy/instance");
        Progress(&format!("Building the instance image {image}")).show();
        Engine::Podman.build_image(&image, &recipe.join("Containerfile"), &recipe)
    }

    /// Removes the pod and its containers, giving each time to stop.
    pub fn remove(&self) -> Result<(), EngineError> {
        Engine::Podman.remove_pod(&self.name, Some(Self::STOP_SECONDS))
    }

    /// Creates the pod, empty, in place of any earlier one.
    pub fn create(&self) -> Result<(), EngineError> {
        self.remove()?;
        Engine::Podman.create_pod(&self.name)
    }

    /// Starts `container` in the pod, detached, from a transient systemd
    /// scope of its own (`ren-pod-<pod>-<role>`): the container's monitor
    /// is a child of the `podman` that starts it, and in the caller's
    /// cgroup it would end with the caller, as the canary timer's pass
    /// ends with its oneshot service. The pod's first container starts its
    /// infra container too, in the same scope.
    pub fn start(&self, container: &Container) -> Result<(), EngineError> {
        Engine::Podman.start(
            &self.spec(container),
            &Launch::Scope {
                unit: self.scope(container.role),
            },
        )
    }

    /// The scope a container is started from.
    fn scope(&self, role: &str) -> String {
        format!("ren-pod-{}-{role}", self.name.trim_start_matches("ren-"))
    }

    fn spec(&self, container: &Container) -> ren_container::Container {
        let mut spec = ren_container::Container::new(
            format!("{}-{}", self.name, container.role),
            Self::image(),
            Lifetime::Kept,
            Placement::Pod(self.name.clone()),
        )
        .stop_timeout(Self::STOP_SECONDS);
        if let Some(dir) = &container.workdir {
            spec = spec.workdir(dir);
        }
        spec.mounts(container.mounts.iter().cloned())
            .envs(container.environment.iter().cloned())
            .args(container.command.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Container, Pod};
    use ren_container::{Access, Engine, Mount};
    use std::path::PathBuf;

    #[test]
    fn a_container_sees_only_its_mounts_at_their_host_paths() {
        let pod = Pod::new("canary");
        let args = Engine::Podman.run_args(&pod.spec(&Container {
            role: "daemon",
            workdir: None,
            mounts: vec![
                Mount {
                    path: PathBuf::from("/home/op/.local/share/ren/builds/abc/ren"),
                    access: Access::ReadOnly,
                },
                Mount {
                    path: PathBuf::from("/home/op/.local/state/ren-canary"),
                    access: Access::ReadWrite,
                },
            ],
            environment: vec![("ACME_KEY".into(), "placeholder".into())],
            command: vec![
                "/home/op/.local/share/ren/builds/abc/ren".into(),
                "serve".into(),
            ],
        }));
        let image = Pod::image();
        assert_eq!(
            args,
            [
                "run",
                "--detach",
                "--name=ren-canary-daemon",
                "--pod=ren-canary",
                "--stop-timeout=30",
                "--volume=/home/op/.local/share/ren/builds/abc/ren:/home/op/.local/share/ren/builds/abc/ren:ro",
                "--volume=/home/op/.local/state/ren-canary:/home/op/.local/state/ren-canary:rw",
                "--env=ACME_KEY=placeholder",
                image.as_str(),
                "/home/op/.local/share/ren/builds/abc/ren",
                "serve",
            ]
        );
    }

    #[test]
    fn each_container_starts_from_a_scope_of_its_own() {
        let pod = Pod::new("canary");
        assert_eq!(pod.scope("daemon"), "ren-pod-canary-daemon");
        assert_eq!(pod.scope("ui"), "ren-pod-canary-ui");
        assert!(Pod::image().starts_with("localhost/ren-instance:"));
    }
}
