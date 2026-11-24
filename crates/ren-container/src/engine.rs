//! The program that runs containers, and the arguments it takes for one.

use crate::{Access, Container, Environment, Lifetime, Placement, Privileges, Runtime};

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

    /// The arguments that run a container under `runtime`, its network
    /// `network` as Podman's `--network` names it.
    fn podman_runtime(runtime: Runtime, network: &str) -> Vec<String> {
        match runtime {
            Runtime::HostKernel => Vec::new(),
            // Rootless, gVisor cannot make the cgroup Podman asks for, nor
            // set up a network in a namespace it does not own: it takes
            // the container's network as its own (`host`) or keeps only its
            // loopback (`none`). Its kernel connects to a socket on the host,
            // or makes one the host sees, only when told to: a run host
            // serves the daemon on a socket in its folder, and reaches out
            // through its doors and the SSH agent, in folders mounted for
            // them.
            Runtime::Gvisor => vec![
                "--runtime=runsc".to_owned(),
                "--runtime-flag=ignore-cgroups".to_owned(),
                format!("--runtime-flag=network={network}"),
                "--runtime-flag=host-uds=all".to_owned(),
            ],
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
            Placement::Host(runtime) => {
                args.extend(["--network=host".to_owned(), "--userns=keep-id".to_owned()]);
                args.extend(Self::podman_runtime(*runtime, "host"));
            }
            Placement::Isolated(runtime) => {
                args.extend(["--network=none".to_owned(), "--userns=keep-id".to_owned()]);
                args.extend(Self::podman_runtime(*runtime, "none"));
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
    use crate::{Container, Engine, Environment, Lifetime, Mount, Placement, Privileges, Runtime};

    #[test]
    fn a_removed_container_on_the_host_runs_as_the_operator() {
        let container = Container::new(
            "ren-run-1",
            "localhost/ren-instance:abc",
            Lifetime::Removed,
            Placement::Host(Runtime::HostKernel),
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
        let container = Container::new(
            "ren-daemon",
            "image",
            Lifetime::Foreground,
            Placement::Host(Runtime::HostKernel),
        )
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

    #[test]
    fn an_isolated_container_has_no_network_but_its_loopback() {
        let container = Container::new(
            "ren-run-2",
            "image",
            Lifetime::Removed,
            Placement::Isolated(Runtime::HostKernel),
        );
        let args = Engine::Podman.run_args(&container);
        assert!(args.iter().any(|arg| arg == "--network=none"), "{args:?}");
        assert!(args.iter().any(|arg| arg == "--userns=keep-id"), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "--network=host"), "{args:?}");
    }

    #[test]
    fn a_gvisor_container_keeps_its_network_and_shares_sockets_with_the_host() {
        let isolated = Engine::Podman.run_args(&Container::new(
            "ren-run-3",
            "image",
            Lifetime::Removed,
            Placement::Isolated(Runtime::Gvisor),
        ));
        assert_eq!(
            &isolated[4..10],
            [
                "--network=none",
                "--userns=keep-id",
                "--runtime=runsc",
                "--runtime-flag=ignore-cgroups",
                "--runtime-flag=network=none",
                "--runtime-flag=host-uds=all",
            ]
        );
        let host = Engine::Podman.run_args(&Container::new(
            "ren-run-4",
            "image",
            Lifetime::Removed,
            Placement::Host(Runtime::Gvisor),
        ));
        assert!(
            host.iter().any(|arg| arg == "--runtime-flag=network=host"),
            "{host:?}"
        );
        // The host's kernel is the engine's default, named nowhere.
        let kernel = Engine::Podman.run_args(&Container::new(
            "ren-run-5",
            "image",
            Lifetime::Removed,
            Placement::Host(Runtime::HostKernel),
        ));
        assert!(
            !kernel.iter().any(|arg| arg.starts_with("--runtime")),
            "{kernel:?}"
        );
    }
}
