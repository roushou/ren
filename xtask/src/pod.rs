//! A rootless Podman pod xtask runs a replica in (docs/plans/self-build.md,
//! phase 7): the canary, or a worktree's preview. Each of its containers
//! sees only what it is given: the kept build it runs, read-only, and the
//! replica's own configuration and data, each mounted at its host path so
//! every path reads the same inside and out. The pod shares the host's
//! network, so its processes listen on the same loopback addresses the
//! tailnet routes point at; `ren` listens only on loopback, which a
//! published port cannot reach. Containers run as the operator's own user
//! (`--userns=keep-id`), so what they write is the operator's.

use crate::{
    invocation::{Echo, Invocation, InvocationError},
    progress::Progress,
};
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

/// A host path a container sees at the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub path: PathBuf,
    pub access: Access,
}

/// What a container may do with a mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

impl Pod {
    /// The image's recipe, which names it.
    const CONTAINERFILE: &str = include_str!("../../deploy/instance/Containerfile");
    /// Seconds a container has to stop before it is killed: `ren` gives its
    /// requests 10, then drains capture.
    const STOP_SECONDS: &str = "30";

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
    pub fn ensure_image(repo: &Path) -> Result<(), InvocationError> {
        let image = Self::image();
        let built = Invocation::new("podman")
            .args(["image", "exists"])
            .arg(&image)
            .succeeds(Echo::Hide)?;
        if built {
            return Ok(());
        }
        let recipe = repo.join("deploy/instance");
        Progress(&format!("Building the instance image {image}")).show();
        Invocation::new("podman")
            .args(["build", "--quiet", "--tag"])
            .arg(&image)
            .arg("--file")
            .arg(recipe.join("Containerfile"))
            .arg(&recipe)
            .capture()
            .map(drop)
    }

    /// Whether the pod exists, running or not.
    pub fn exists(&self) -> Result<bool, InvocationError> {
        Invocation::new("podman")
            .args(["pod", "exists"])
            .arg(&self.name)
            .succeeds(Echo::Hide)
    }

    /// Removes the pod and its containers, giving each time to stop.
    pub fn remove(&self) -> Result<(), InvocationError> {
        if self.exists()? {
            Invocation::new("podman")
                .args(["pod", "rm", "--force", "--time", Self::STOP_SECONDS])
                .arg(&self.name)
                .succeeds(Echo::Hide)?;
        }
        Ok(())
    }

    /// Creates the pod, empty, in place of any earlier one.
    pub fn create(&self) -> Result<(), InvocationError> {
        self.remove()?;
        Invocation::new("podman")
            .args(self.create_args())
            .capture()
            .map(drop)
    }

    /// Starts `container` in the pod, detached, from a transient systemd
    /// scope of its own (`ren-pod-<pod>-<role>`): the container's monitor
    /// is a child of the `podman` that starts it, and in the caller's
    /// cgroup it would end with the caller, as the canary timer's pass
    /// ends with its oneshot service. The pod's first container starts its
    /// infra container too, in the same scope.
    pub fn start(&self, container: &Container) -> Result<(), InvocationError> {
        Invocation::new("systemd-run")
            .args(Self::scope_args(&self.scope(container.role)))
            .arg("podman")
            .args(self.run_args(container))
            .capture()
            .map(drop)
    }

    /// The scope a container is started from.
    fn scope(&self, role: &str) -> String {
        format!("ren-pod-{}-{role}", self.name.trim_start_matches("ren-"))
    }

    fn scope_args(scope: &str) -> Vec<String> {
        vec![
            "--user".to_owned(),
            "--scope".to_owned(),
            "--quiet".to_owned(),
            "--collect".to_owned(),
            format!("--unit={scope}"),
            "--".to_owned(),
        ]
    }

    fn create_args(&self) -> Vec<String> {
        [
            "pod",
            "create",
            "--name",
            &self.name,
            "--userns=keep-id",
            "--network=host",
        ]
        .map(str::to_owned)
        .into()
    }

    fn run_args(&self, container: &Container) -> Vec<String> {
        let mut args: Vec<String> = [
            "run",
            "--detach",
            "--pod",
            &self.name,
            "--name",
            &format!("{}-{}", self.name, container.role),
            "--stop-timeout",
            Self::STOP_SECONDS,
        ]
        .map(str::to_owned)
        .into();
        if let Some(dir) = &container.workdir {
            args.push(format!("--workdir={}", dir.display()));
        }
        for mount in &container.mounts {
            let access = match mount.access {
                Access::ReadOnly => "ro",
                Access::ReadWrite => "rw",
            };
            let path = mount.path.display();
            args.push(format!("--volume={path}:{path}:{access}"));
        }
        for (name, value) in &container.environment {
            args.push(format!("--env={name}={value}"));
        }
        args.push(Self::image());
        args.extend(container.command.iter().cloned());
        args
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, Container, Mount, Pod};
    use std::path::PathBuf;

    #[test]
    fn a_container_sees_only_its_mounts_at_their_host_paths() {
        let pod = Pod::new("canary");
        let args = pod.run_args(&Container {
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
        });
        let image = Pod::image();
        assert_eq!(
            args,
            [
                "run",
                "--detach",
                "--pod",
                "ren-canary",
                "--name",
                "ren-canary-daemon",
                "--stop-timeout",
                "30",
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
        assert_eq!(
            Pod::scope_args(&pod.scope("ui")),
            [
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "--unit=ren-pod-canary-ui",
                "--"
            ]
        );
    }

    #[test]
    fn the_pod_shares_the_host_network_as_the_operator() {
        assert_eq!(
            Pod::new("preview-ren-3").create_args(),
            [
                "pod",
                "create",
                "--name",
                "ren-preview-ren-3",
                "--userns=keep-id",
                "--network=host",
            ]
        );
        assert!(Pod::image().starts_with("localhost/ren-instance:"));
    }
}
