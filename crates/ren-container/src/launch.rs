//! Running the engine: starting a container, and the pods, containers, and
//! images it keeps. What to start is a [`Container`]'s; how it is started,
//! and so what watches it until it exits, is a [`Launch`]'s.

use crate::{Container, Engine, Runtime};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

/// How the engine is reached to start containers: the setting a caller
/// keeps, from which each container's [`Launch`] is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineAccess {
    /// The engine's program here, each container started from a transient
    /// systemd user scope of its own.
    Scope,
    /// The engine's API socket, for a caller in a container, which has no
    /// systemd of its own.
    Socket(PathBuf),
}

/// How one container is started, and so what watches it until it exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// The engine's program here, from the transient systemd user scope
    /// `unit`: the container's monitor is a child of the program that
    /// starts it, and in the caller's own unit it would end with the
    /// caller, leaving the container with nothing to see it exit.
    Scope { unit: String },
    /// The engine's API service at `socket`, which starts and watches the
    /// container apart from the caller. The client keeps its settings and
    /// runtime files under `settings` (and its `run`), which must exist,
    /// when the caller's own folders are not the operator's, which the
    /// client refuses.
    Socket {
        socket: PathBuf,
        settings: Option<PathBuf>,
    },
}

/// An engine command that could not start or failed.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("`{command}` could not start: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` exited with {status}{}", Self::detail(stderr))]
    Failed {
        command: String,
        status: ExitStatus,
        stderr: String,
    },
    /// The API service runs its containers under another runtime than the
    /// container is to run under: the service is not the one it needs.
    #[error("the engine's service at {socket} runs containers under {served:?}, not {wanted:?}")]
    Runtime {
        socket: PathBuf,
        wanted: Runtime,
        served: Runtime,
    },
}

impl EngineError {
    fn detail(stderr: &str) -> String {
        if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        }
    }
}

impl Engine {
    /// The program that starts `container` as `launch` says, and its
    /// arguments.
    pub fn launch_args(
        self,
        container: &Container,
        launch: &Launch,
    ) -> (&'static str, Vec<String>) {
        match launch {
            Launch::Scope { unit } => {
                let mut args = vec![
                    "--user".to_owned(),
                    "--scope".to_owned(),
                    "--quiet".to_owned(),
                    "--collect".to_owned(),
                    format!("--unit={unit}"),
                    "--".to_owned(),
                    self.program().to_owned(),
                ];
                args.extend(self.run_args(container));
                ("systemd-run", args)
            }
            Launch::Socket { socket, .. } => {
                let mut args = vec![Self::url(socket)];
                args.extend(self.remote_run_args(container));
                (self.program(), args)
            }
        }
    }

    /// Starts `container` as `launch` says, and waits for the engine to
    /// have started it. Through the API service, only when the service runs
    /// containers under the container's runtime, which it cannot be told.
    pub fn start(self, container: &Container, launch: &Launch) -> Result<(), EngineError> {
        if let (Launch::Socket { socket, .. }, Some(wanted)) = (launch, container.runtime()) {
            let served = self.served_runtime(launch)?;
            if served != wanted {
                return Err(EngineError::Runtime {
                    socket: socket.clone(),
                    wanted,
                    served,
                });
            }
        }
        let (program, args) = self.launch_args(container, launch);
        let mut command = Command::new(program);
        command.args(&args);
        Self::settle(&mut command, launch);
        let described = Self::describe(program, &args);
        Self::capture(command, described).map(drop)
    }

    /// The runtime the engine at `launch` runs its containers under.
    fn served_runtime(self, launch: &Launch) -> Result<Runtime, EngineError> {
        let mut args = Vec::new();
        if let Launch::Socket { socket, .. } = launch {
            args.push(Self::url(socket));
        }
        args.extend(Self::info_runtime_args().map(str::to_owned));
        let mut command = Command::new(self.program());
        command.args(&args);
        Self::settle(&mut command, launch);
        let described = Self::describe(self.program(), &args);
        Self::capture(command, described).map(|name| Runtime::reported(&name))
    }

    fn info_runtime_args() -> [&'static str; 3] {
        ["info", "--format", "{{.Host.OCIRuntime.Name}}"]
    }

    fn url(socket: &std::path::Path) -> String {
        format!("--url=unix://{}", socket.display())
    }

    /// The environment the engine's client keeps its settings in, for a
    /// caller whose own folders are not the operator's.
    fn settle(command: &mut Command, launch: &Launch) {
        if let Launch::Socket {
            settings: Some(settings),
            ..
        } = launch
        {
            command
                .env("XDG_CONFIG_HOME", settings)
                .env("XDG_RUNTIME_DIR", settings.join("run"));
        }
    }

    /// Creates the empty pod `name`, on the host's network, its containers
    /// running as the operator's own user.
    pub fn create_pod(self, name: &str) -> Result<(), EngineError> {
        self.run(&Self::create_pod_args(name))
    }

    fn create_pod_args(name: &str) -> [&str; 6] {
        [
            "pod",
            "create",
            "--name",
            name,
            "--userns=keep-id",
            "--network=host",
        ]
    }

    /// Removes the pod `name` and its containers, if it exists, giving
    /// each `stop_timeout` seconds to stop, or the engine's default.
    /// Whether the engine managed is not checked: only failing to run it
    /// is an error.
    pub fn remove_pod(self, name: &str, stop_timeout: Option<u32>) -> Result<(), EngineError> {
        let time = stop_timeout.map(|seconds| seconds.to_string());
        let mut args = vec!["pod", "rm", "--force", "--ignore"];
        if let Some(time) = &time {
            args.extend(["--time", time]);
        }
        args.push(name);
        self.succeeds(&args).map(drop)
    }

    /// Removes the container `name`, which stops it, if it exists. Whether
    /// the engine managed is not checked: only failing to run it is an
    /// error.
    pub fn remove_container(self, name: &str) -> Result<(), EngineError> {
        self.succeeds(&["rm", "--force", "--ignore", name])
            .map(drop)
    }

    /// Whether the image `image` is built.
    pub fn image_exists(self, image: &str) -> Result<bool, EngineError> {
        self.succeeds(&["image", "exists", image])
    }

    /// Builds `image` from the recipe `file` with `context`, quietly.
    pub fn build_image(
        self,
        image: &str,
        file: &std::path::Path,
        context: &std::path::Path,
    ) -> Result<(), EngineError> {
        let file = file.display().to_string();
        let context = context.display().to_string();
        self.run(&[
            "build", "--quiet", "--tag", image, "--file", &file, &context,
        ])
    }

    fn describe(program: &str, args: &[impl AsRef<str>]) -> String {
        std::iter::once(program)
            .chain(args.iter().map(AsRef::as_ref))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn command(self, args: &[&str]) -> Command {
        let mut command = Command::new(self.program());
        command.args(args);
        command
    }

    fn run(self, args: &[&str]) -> Result<(), EngineError> {
        Self::capture(self.command(args), Self::describe(self.program(), args)).map(drop)
    }

    /// Its standard output; fails, with its standard error, unless it
    /// exits successfully.
    #[expect(
        clippy::disallowed_methods,
        reason = "ren-container is the one place Ren runs the container engine"
    )]
    fn capture(mut command: Command, described: String) -> Result<String, EngineError> {
        let output =
            command
                .stdin(Stdio::null())
                .output()
                .map_err(|source| EngineError::Spawn {
                    command: described.clone(),
                    source,
                })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(EngineError::Failed {
                command: described,
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// Whether it exits successfully, its output hidden. Failing to start
    /// is still an error.
    #[expect(
        clippy::disallowed_methods,
        reason = "ren-container is the one place Ren runs the container engine"
    )]
    fn succeeds(self, args: &[&str]) -> Result<bool, EngineError> {
        self.command(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .map_err(|source| EngineError::Spawn {
                command: Self::describe(self.program(), args),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Container, Engine, Launch, Lifetime, Placement};

    fn container() -> Container {
        Container::new("ren-run-1", "image", Lifetime::Removed, Placement::Host)
    }

    #[test]
    fn from_a_scope_its_monitor_lives_in_a_unit_of_its_own() {
        let launch = Launch::Scope {
            unit: "ren-run-1".to_owned(),
        };
        let (program, args) = Engine::Podman.launch_args(&container(), &launch);
        assert_eq!(program, "systemd-run");
        let split = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(
            &args[..split],
            [
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "--unit=ren-run-1"
            ]
        );
        assert_eq!(args[split + 1], "podman");
        assert_eq!(&args[split + 2..], Engine::Podman.run_args(&container()));
    }

    #[test]
    fn through_the_socket_the_api_service_starts_and_watches_it() {
        let launch = Launch::Socket {
            socket: "/run/user/1000/podman/podman.sock".into(),
            settings: None,
        };
        let (program, args) = Engine::Podman.launch_args(&container(), &launch);
        assert_eq!(program, "podman");
        assert_eq!(args[0], "--url=unix:///run/user/1000/podman/podman.sock");
        assert_eq!(&args[1..], Engine::Podman.remote_run_args(&container()));
    }

    #[test]
    fn a_pod_shares_the_host_network_as_the_operator() {
        assert_eq!(
            Engine::create_pod_args("ren-preview-ren-3"),
            [
                "pod",
                "create",
                "--name",
                "ren-preview-ren-3",
                "--userns=keep-id",
                "--network=host",
            ]
        );
    }
}
