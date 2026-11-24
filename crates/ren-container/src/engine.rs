//! The program that runs containers, and the arguments it takes for one.

use crate::{Access, Container, Environment, Lifetime, Placement, Privileges, Runtime};

/// Where the engine that starts a container runs: here, or behind its API
/// service, which takes no runtime of a container's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Local,
    Remote,
}

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

    /// The arguments, after the program, that start `container` with the
    /// engine here.
    pub fn run_args(self, container: &Container) -> Vec<String> {
        self.args_for(container, Side::Local)
    }

    /// The arguments, after the program and its service's address, that
    /// start `container` through the engine's API service: what the
    /// service runs is its own, set as it started ([`Self::service_args`]).
    pub(crate) fn remote_run_args(self, container: &Container) -> Vec<String> {
        self.args_for(container, Side::Remote)
    }

    /// The arguments, after the program, that serve the engine's API on
    /// the socket its service manager hands it, running every container
    /// under `runtime`.
    pub fn service_args(self, runtime: Runtime) -> Vec<String> {
        match self {
            Self::Podman => runtime
                .podman_args()
                .iter()
                .map(|&arg| arg.to_owned())
                .chain(["system".to_owned(), "service".to_owned()])
                .collect(),
        }
    }

    fn args_for(self, container: &Container, side: Side) -> Vec<String> {
        match self {
            Self::Podman => Self::podman_run(container, side),
        }
    }

    /// `runtime`'s own arguments when the engine is run here; through its
    /// API service, none.
    fn podman_runtime(runtime: Runtime, side: Side) -> Vec<String> {
        match side {
            Side::Local => runtime
                .podman_args()
                .iter()
                .map(|&arg| arg.to_owned())
                .collect(),
            Side::Remote => Vec::new(),
        }
    }

    fn podman_run(container: &Container, side: Side) -> Vec<String> {
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
            Placement::Isolated(runtime) => {
                args.extend(["--network=none".to_owned(), "--userns=keep-id".to_owned()]);
                args.extend(Self::podman_runtime(*runtime, side));
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
    use crate::{
        Container, Engine, Environment, Lifetime, Mount, Placement, Privileges, ReportedRuntime,
        Runtime,
    };

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
    fn an_isolated_gvisor_container_shares_sockets_with_the_host() {
        let gvisor = Container::new(
            "ren-run-3",
            "image",
            Lifetime::Removed,
            Placement::Isolated(Runtime::Gvisor),
        );
        assert_eq!(
            &Engine::Podman.run_args(&gvisor)[4..10],
            [
                "--network=none",
                "--userns=keep-id",
                "--runtime=runsc",
                "--runtime-flag=ignore-cgroups",
                "--runtime-flag=network=none",
                "--runtime-flag=host-uds=all",
            ]
        );
        // Through the API service, which runs it as it was started.
        let remote = Engine::Podman.remote_run_args(&gvisor);
        assert!(
            !remote.iter().any(|arg| arg.starts_with("--runtime")),
            "{remote:?}"
        );
        // The host's kernel is the engine's default, named nowhere.
        let kernel = Engine::Podman.run_args(&Container::new(
            "ren-run-5",
            "image",
            Lifetime::Removed,
            Placement::Isolated(Runtime::HostKernel),
        ));
        assert!(
            !kernel.iter().any(|arg| arg.starts_with("--runtime")),
            "{kernel:?}"
        );
    }

    #[test]
    fn the_runtime_is_read_from_what_the_engine_reports() {
        assert_eq!(
            Runtime::reported("runsc\n"),
            ReportedRuntime::Known(Runtime::Gvisor)
        );
        assert_eq!(
            Runtime::reported("crun\n"),
            ReportedRuntime::Known(Runtime::HostKernel)
        );
        // Not known to be either: a microVM, say, is not the host's kernel.
        assert_eq!(
            Runtime::reported("krun\n"),
            ReportedRuntime::Unknown("krun".to_owned())
        );
    }

    #[test]
    fn a_service_runs_its_containers_under_its_runtime() {
        assert_eq!(
            Engine::Podman.service_args(Runtime::Gvisor),
            [
                "--runtime=runsc",
                "--runtime-flag=ignore-cgroups",
                "--runtime-flag=network=none",
                "--runtime-flag=host-uds=all",
                "system",
                "service",
            ]
        );
        assert_eq!(
            Engine::Podman.service_args(Runtime::HostKernel),
            ["system", "service"]
        );
    }
}
