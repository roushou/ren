//! The program that runs containers, and the arguments it takes for one.

use crate::{Access, Container, Environment, Lifetime, Placement, Privileges};

/// A container engine. Podman is the one Ren runs: rootless, as the
/// operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Podman,
}

impl Engine {
    /// The engine's program, found on the search path.
    pub const fn program(self) -> &'static str {
        match self {
            Self::Podman => "podman",
        }
    }

    /// The arguments, after the program, that start `container`.
    pub fn run_args(self, container: &Container) -> Vec<String> {
        match self {
            Self::Podman => Self::podman_run(container),
        }
    }

    fn podman_run(container: &Container) -> Vec<String> {
        let mut args = vec!["run".to_owned()];
        match container.lifetime {
            Lifetime::Kept => args.push("--detach".to_owned()),
            Lifetime::Removed => args.extend(["--detach".to_owned(), "--rm".to_owned()]),
            Lifetime::Foreground => args.extend(["--rm".to_owned(), "--replace".to_owned()]),
        }
        args.push(format!("--name={}", container.name));
        match &container.placement {
            // As the operator's own user, so what it writes is the
            // operator's, on the host's network.
            Placement::Host => {
                args.extend(["--network=host".to_owned(), "--userns=keep-id".to_owned()])
            }
            // The pod sets the network and the user namespace.
            Placement::Pod(pod) => args.push(format!("--pod={pod}")),
        }
        match container.environment {
            Environment::Own => {}
            Environment::Host => args.push("--env-host".to_owned()),
        }
        match container.privileges {
            Privileges::Image => {}
            Privileges::NoNew => args.push("--security-opt=no-new-privileges".to_owned()),
        }
        if let Some(seconds) = container.stop_timeout {
            args.push(format!("--stop-timeout={seconds}"));
        }
        if let Some(dir) = &container.workdir {
            args.push(format!("--workdir={}", dir.display()));
        }
        args.extend(container.mounts.iter().map(|mount| {
            let access = match mount.access {
                Access::ReadOnly => "ro",
                Access::ReadWrite => "rw",
            };
            let path = mount.path.display();
            format!("--volume={path}:{path}:{access}")
        }));
        args.extend(
            container
                .env
                .iter()
                .map(|(name, value)| format!("--env={name}={value}")),
        );
        if let Some(entrypoint) = &container.entrypoint {
            args.push(format!("--entrypoint={}", entrypoint.display()));
        }
        args.push(container.image.clone());
        args.extend(container.args.iter().cloned());
        args
    }
}

#[cfg(test)]
mod tests {
    use crate::{Container, Engine, Environment, Lifetime, Mount, Placement, Privileges};

    #[test]
    fn a_removed_container_on_the_host_runs_as_the_operator() {
        let container = Container::new(
            "ren-run-1",
            "localhost/ren-instance:abc",
            Lifetime::Removed,
            Placement::Host,
        )
        .stop_timeout(30)
        .mount(Mount::read_only("/usr"))
        .mount(Mount::read_write("/home/op/dev/app"))
        .env("HOME", "/home/op")
        .entrypoint("/home/op/.local/bin/ren")
        .args(["run-host", "/home/op/.local/state/ren/runs/r"]);
        assert_eq!(
            Engine::Podman.run_args(&container),
            [
                "run",
                "--detach",
                "--rm",
                "--name=ren-run-1",
                "--network=host",
                "--userns=keep-id",
                "--stop-timeout=30",
                "--volume=/usr:/usr:ro",
                "--volume=/home/op/dev/app:/home/op/dev/app:rw",
                "--env=HOME=/home/op",
                "--entrypoint=/home/op/.local/bin/ren",
                "localhost/ren-instance:abc",
                "run-host",
                "/home/op/.local/state/ren/runs/r",
            ]
        );
    }

    #[test]
    fn a_foreground_container_replaces_its_name_and_can_be_hardened() {
        let container =
            Container::new("ren-daemon", "image", Lifetime::Foreground, Placement::Host)
                .environment(Environment::Host)
                .privileges(Privileges::NoNew);
        assert_eq!(
            Engine::Podman.run_args(&container),
            [
                "run",
                "--rm",
                "--replace",
                "--name=ren-daemon",
                "--network=host",
                "--userns=keep-id",
                "--env-host",
                "--security-opt=no-new-privileges",
                "image",
            ]
        );
    }

    #[test]
    fn a_kept_container_in_a_pod_takes_the_pods_network() {
        let container = Container::new(
            "ren-canary-ui",
            "image",
            Lifetime::Kept,
            Placement::Pod("ren-canary".to_owned()),
        )
        .workdir("/srv/ui")
        .args(["node", "build"]);
        assert_eq!(
            Engine::Podman.run_args(&container),
            [
                "run",
                "--detach",
                "--name=ren-canary-ui",
                "--pod=ren-canary",
                "--workdir=/srv/ui",
                "image",
                "node",
                "build",
            ]
        );
    }
}
