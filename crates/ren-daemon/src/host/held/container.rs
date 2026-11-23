//! A run host in a container of its own (self-build plan, phase 11): the
//! run's client sees only what the run works on and what its client keeps,
//! not the operator's home, Ren's data, or other runs.
//!
//! The container is rootless Podman's, as the operator (`--userns=keep-id`),
//! on the host's network so the client reaches the gateway on loopback.
//! Every path it sees is mounted at its host path, so the run folder, the
//! workdir, and the client's files keep the names the daemon gave them.
//! This part is pure: what a run's container is, and the `podman run`
//! that starts it. Finding what a run needs on the host is
//! [`super::HeldRuns`]'s.

use ren_recorder::id::RunId;
use std::path::{Path, PathBuf};

/// What a run's container may do with a path it sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

/// A host path a run's container sees, at the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub path: PathBuf,
    pub access: Access,
}

impl Mount {
    pub fn read_only(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            access: Access::ReadOnly,
        }
    }

    pub fn read_write(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            access: Access::ReadWrite,
        }
    }

    fn arg(&self) -> String {
        let access = match self.access {
            Access::ReadOnly => "ro",
            Access::ReadWrite => "rw",
        };
        let path = self.path.display();
        format!("--volume={path}:{path}:{access}")
    }
}

/// How the daemon reaches Podman to start a run's container, and so what
/// watches the container until its host exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodmanReach {
    /// `podman` here, from a transient systemd scope of the run's own
    /// (`ren-run-<run>.scope`): for a daemon on the host. The container's
    /// monitor is a child of the `podman` that starts it, so in the
    /// daemon's unit it would stop with the daemon, leaving the run going
    /// with nothing to see its host exit and remove its container.
    Scope,
    /// The operator's Podman API socket: for a daemon in a container, which
    /// has no systemd of its own. The API service starts and watches the
    /// run's container, apart from the daemon's.
    Socket(PathBuf),
}

/// How run hosts start in containers: the image, how Podman is reached,
/// and what every run's container sees besides what the run itself needs
/// (`[runs.container]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContainers {
    image: String,
    reach: PodmanReach,
    /// The operator's home, the container's `HOME`.
    home: PathBuf,
    /// What every run sees: the host's programs and toolchains.
    shared: Vec<Mount>,
    /// The SSH agent's socket, when the daemon has one.
    ssh_agent: Option<PathBuf>,
}

/// What one run needs to see, found on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunNeeds {
    /// The run's folder: its spec, socket, spool, exit, and journal.
    pub(super) folder: PathBuf,
    /// Where the run works.
    pub(super) workdir: PathBuf,
    /// The repository's git folder, where it is not under the workdir: a
    /// worktree's common folder, or a repository's above a subfolder.
    pub(super) git: Option<PathBuf>,
    /// The `ren` the run host is.
    pub(super) host: PathBuf,
    /// Files the client's command reads: its program, as found on the
    /// path, and any file its arguments name.
    pub(super) reads: Vec<PathBuf>,
    /// What the client keeps under the home, that exists.
    pub(super) keeps: Vec<PathBuf>,
}

/// One run's container.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub(super) struct RunContainer {
    name: String,
    image: String,
    reach: PodmanReach,
    mounts: Vec<Mount>,
    env: Vec<(String, String)>,
    /// The `ren` it runs as its entrypoint: never the image's own, which a
    /// mount of the host's programs may hide.
    host: PathBuf,
    /// The run folder its host serves.
    folder: PathBuf,
}

impl RunContainers {
    pub fn new(
        image: String,
        reach: PodmanReach,
        home: PathBuf,
        shared: Vec<Mount>,
        ssh_agent: Option<PathBuf>,
    ) -> Self {
        Self {
            image,
            reach,
            home,
            shared,
            ssh_agent,
        }
    }

    /// The container `run`'s host is named: the systemd unit's name, so
    /// one name finds a run's host however it was started.
    pub fn name(run: RunId) -> String {
        super::HeldRuns::unit(run)
    }

    /// The home a run's container takes as its own.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `run`'s container: the shared mounts, then what the run needs; the
    /// run host serving `needs.folder`.
    pub(super) fn container(&self, run: RunId, needs: &RunNeeds) -> RunContainer {
        let mut mounts = self.shared.clone();
        mounts.push(Mount::read_only(&needs.host));
        mounts.extend(needs.reads.iter().map(Mount::read_only));
        mounts.extend(needs.keeps.iter().map(Mount::read_write));
        mounts.push(Mount::read_write(&needs.workdir));
        mounts.extend(needs.git.iter().map(Mount::read_write));
        mounts.push(Mount::read_write(&needs.folder));
        let mut env = vec![("HOME".to_owned(), self.home.display().to_string())];
        if let Some(agent) = &self.ssh_agent {
            mounts.push(Mount::read_write(agent));
            env.push(("SSH_AUTH_SOCK".to_owned(), agent.display().to_string()));
        }
        RunContainer {
            name: Self::name(run),
            image: self.image.clone(),
            reach: self.reach.clone(),
            mounts,
            env,
            host: needs.host.clone(),
            folder: needs.folder.clone(),
        }
    }
}

impl RunContainer {
    /// Seconds Podman waits after SIGTERM before it kills the run host: the
    /// time it takes to stop its client and write its exit.
    const STOP_TIMEOUT: u32 = 30;

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// Whether it is started through the API socket, by a daemon in a
    /// container.
    pub(super) fn through_socket(&self) -> bool {
        match self.reach {
            PodmanReach::Socket(_) => true,
            PodmanReach::Scope => false,
        }
    }

    /// The program that starts it, and its arguments, as Podman is
    /// reached ([`PodmanReach`]).
    pub(super) fn start(&self) -> (&'static str, Vec<String>) {
        match &self.reach {
            PodmanReach::Scope => {
                let mut args = vec![
                    "--user".to_owned(),
                    "--scope".to_owned(),
                    "--quiet".to_owned(),
                    "--collect".to_owned(),
                    format!("--unit={}", self.name),
                    "--".to_owned(),
                    "podman".to_owned(),
                ];
                args.extend(self.run_args());
                ("systemd-run", args)
            }
            PodmanReach::Socket(socket) => {
                let mut args = vec![format!("--url=unix://{}", socket.display())];
                args.extend(self.run_args());
                ("podman", args)
            }
        }
    }

    /// `podman` arguments that start it, detached and removed once its run
    /// host exits.
    pub(super) fn run_args(&self) -> Vec<String> {
        let mut args = vec![
            "run".to_owned(),
            "--detach".to_owned(),
            "--rm".to_owned(),
            format!("--name={}", self.name),
            "--network=host".to_owned(),
            "--userns=keep-id".to_owned(),
            format!("--stop-timeout={}", Self::STOP_TIMEOUT),
        ];
        args.extend(self.mounts.iter().map(Mount::arg));
        args.extend(
            self.env
                .iter()
                .map(|(key, value)| format!("--env={key}={value}")),
        );
        args.push(format!("--entrypoint={}", self.host.display()));
        args.push(self.image.clone());
        args.push("run-host".to_owned());
        args.push(self.folder.display().to_string());
        args
    }
}

#[cfg(test)]
mod tests {
    use super::{Mount, PodmanReach, RunContainers, RunNeeds};
    use ren_recorder::id::RunId;

    fn needs() -> RunNeeds {
        RunNeeds {
            folder: "/home/op/.local/state/ren/runs/r".into(),
            workdir: "/home/op/dev/app/.worktrees/fix".into(),
            git: Some("/home/op/dev/app/.git".into()),
            host: "/home/op/.local/bin/ren".into(),
            reads: vec!["/home/op/.local/share/claude/versions/2.1.0".into()],
            keeps: vec!["/home/op/.claude".into(), "/home/op/.claude.json".into()],
        }
    }

    fn containers(ssh_agent: Option<&str>) -> RunContainers {
        RunContainers::new(
            "localhost/ren-instance:abc".into(),
            PodmanReach::Scope,
            "/home/op".into(),
            vec![
                Mount::read_only("/usr"),
                Mount::read_write("/home/op/.cargo"),
            ],
            ssh_agent.map(Into::into),
        )
    }

    #[test]
    fn a_run_sees_only_what_it_works_on_and_what_its_client_keeps() {
        let run = RunId::restore(uuid::Uuid::nil());
        let args = containers(None).container(run, &needs()).run_args();
        let volumes: Vec<&str> = args
            .iter()
            .filter_map(|arg| arg.strip_prefix("--volume="))
            .collect();
        assert_eq!(
            volumes,
            [
                "/usr:/usr:ro",
                "/home/op/.cargo:/home/op/.cargo:rw",
                "/home/op/.local/bin/ren:/home/op/.local/bin/ren:ro",
                "/home/op/.local/share/claude/versions/2.1.0:/home/op/.local/share/claude/versions/2.1.0:ro",
                "/home/op/.claude:/home/op/.claude:rw",
                "/home/op/.claude.json:/home/op/.claude.json:rw",
                "/home/op/dev/app/.worktrees/fix:/home/op/dev/app/.worktrees/fix:rw",
                "/home/op/dev/app/.git:/home/op/dev/app/.git:rw",
                "/home/op/.local/state/ren/runs/r:/home/op/.local/state/ren/runs/r:rw",
            ]
        );
        // Nothing else of the home: not its whole folder, not Ren's
        // configuration.
        assert!(!volumes.iter().any(|volume| volume.starts_with("/home/op:")));
    }

    #[test]
    fn its_host_runs_as_the_operator_on_the_hosts_network_and_goes_when_done() {
        let run = RunId::restore(uuid::Uuid::nil());
        let container = containers(None).container(run, &needs());
        let args = container.run_args();
        assert_eq!(container.name(), format!("ren-run-{run}"));
        for flag in ["--detach", "--rm", "--network=host", "--userns=keep-id"] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}");
        }
        assert!(args.iter().any(|arg| arg == "--env=HOME=/home/op"));
        assert_eq!(
            &args[args.len() - 4..],
            [
                "--entrypoint=/home/op/.local/bin/ren",
                "localhost/ren-instance:abc",
                "run-host",
                "/home/op/.local/state/ren/runs/r",
            ]
        );
    }

    #[test]
    fn from_the_host_its_monitor_lives_in_a_scope_of_its_own_not_the_daemons() {
        let run = RunId::restore(uuid::Uuid::nil());
        let container = containers(None).container(run, &needs());
        let (program, args) = container.start();
        assert_eq!(program, "systemd-run");
        let split = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(
            &args[..split],
            [
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                &format!("--unit=ren-run-{run}"),
            ]
        );
        assert_eq!(args[split + 1], "podman");
        assert_eq!(&args[split + 2..], container.run_args());
    }

    #[test]
    fn from_a_container_the_api_service_starts_and_watches_it() {
        let run = RunId::restore(uuid::Uuid::nil());
        let mut through_socket = containers(None);
        through_socket.reach = PodmanReach::Socket("/run/user/1000/podman/podman.sock".into());
        let container = through_socket.container(run, &needs());
        let (program, args) = container.start();
        assert_eq!(program, "podman");
        assert_eq!(args[0], "--url=unix:///run/user/1000/podman/podman.sock");
        assert_eq!(&args[1..], container.run_args());
    }

    #[test]
    fn the_ssh_agent_is_passed_when_the_daemon_has_one() {
        let run = RunId::restore(uuid::Uuid::nil());
        let without = containers(None).container(run, &needs()).run_args();
        assert!(!without.iter().any(|arg| arg.contains("SSH_AUTH_SOCK")));
        let with = containers(Some("/run/user/1000/agent"))
            .container(run, &needs())
            .run_args();
        assert!(
            with.iter()
                .any(|arg| arg == "--volume=/run/user/1000/agent:/run/user/1000/agent:rw")
        );
        assert!(
            with.iter()
                .any(|arg| arg == "--env=SSH_AUTH_SOCK=/run/user/1000/agent")
        );
    }
}
