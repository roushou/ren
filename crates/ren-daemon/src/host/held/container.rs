//! A run host in a container of its own (self-build plan, phase 11): the
//! run's client sees only what the run works on and what its client keeps,
//! not the operator's home, Ren's data, or other runs.
//!
//! The container is rootless Podman's, as the operator (`--userns=keep-id`),
//! on the host's network so the client reaches the gateway on loopback, or,
//! relayed ([`RunReach::Relayed`], phase 12), with no network but its
//! loopback, where the run host's doorways reach the gateway and the relay
//! through the sockets in the folder it mounts read-only.
//! Every path it sees is mounted at its host path, so the run folder, the
//! workdir, and the client's files keep the names the daemon gave them.
//! This part is pure: what a run's container is (a `ren-container`
//! [`Container`]), and how the engine is reached to start it ([`Reach`]:
//! from a systemd scope of the run's own for a daemon on the host, through
//! the API socket for a daemon in a container). Finding what a run needs on the host is
//! [`super::HeldRuns`]'s.

use ren_container::{Container, Lifetime, Mount, Placement, Reach};
use ren_recorder::id::RunId;
use ren_relay::RelayPolicy;
use std::path::{Path, PathBuf};

/// How run hosts start in containers: the image, how Podman is reached,
/// and what every run's container sees besides what the run itself needs
/// (`[runs.container]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContainers {
    image: String,
    reach: Reach,
    /// The operator's home, the container's `HOME`.
    home: PathBuf,
    /// What every run sees: the host's programs and toolchains.
    shared: Vec<Mount>,
    /// The SSH agent's socket, when the daemon has one.
    ssh_agent: Option<PathBuf>,
    /// What its runs reach.
    network: RunReach,
}

/// What a run's container reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunReach {
    /// The host's network.
    Host,
    /// Only Ren's gateway and the relay, which lets it out where this
    /// policy and its client list.
    Relayed(RelayPolicy),
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
    /// The folder of the doorways' sockets, for a relayed run.
    pub(super) doors: Option<PathBuf>,
}

/// One run's container.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub(super) struct RunContainer {
    reach: Reach,
    container: Container,
}

impl RunContainers {
    pub fn new(
        image: String,
        reach: Reach,
        home: PathBuf,
        shared: Vec<Mount>,
        ssh_agent: Option<PathBuf>,
        network: RunReach,
    ) -> Self {
        Self {
            image,
            reach,
            home,
            shared,
            ssh_agent,
            network,
        }
    }

    /// What its runs reach.
    pub fn network(&self) -> &RunReach {
        &self.network
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
    /// run host serving `needs.folder`, its entrypoint the `ren` it is,
    /// never the image's own, which a mount of the host's programs may
    /// hide.
    pub(super) fn container(&self, run: RunId, needs: &RunNeeds) -> RunContainer {
        let mut container = Container::new(
            Self::name(run),
            self.image.clone(),
            Lifetime::Removed,
            match self.network {
                RunReach::Host => Placement::Host,
                RunReach::Relayed(_) => Placement::Isolated,
            },
        )
        .stop_timeout(RunContainer::STOP_TIMEOUT)
        .mounts(self.shared.iter().cloned())
        .mount(Mount::read_only(&needs.host))
        .mounts(needs.reads.iter().map(Mount::read_only))
        .mounts(needs.keeps.iter().map(Mount::read_write))
        .mount(Mount::read_write(&needs.workdir))
        .mounts(needs.git.iter().map(Mount::read_write))
        .mount(Mount::read_write(&needs.folder))
        .mounts(needs.doors.iter().map(Mount::read_only));
        // A relayed run pushes with a grant, never the operator's agent.
        let agent = match self.network {
            RunReach::Host => self.ssh_agent.as_ref(),
            RunReach::Relayed(_) => None,
        };
        if let Some(agent) = agent {
            container = container.mount(Mount::read_write(agent));
        }
        container = container.env("HOME", self.home.display().to_string());
        if let Some(agent) = agent {
            container = container.env("SSH_AUTH_SOCK", agent.display().to_string());
        }
        RunContainer {
            reach: self.reach.clone(),
            container: container
                .entrypoint(&needs.host)
                .args(["run-host".to_owned(), needs.folder.display().to_string()]),
        }
    }
}

impl RunContainer {
    /// Seconds Podman waits after SIGTERM before it kills the run host: the
    /// time it takes to stop its client and write its exit.
    const STOP_TIMEOUT: u32 = 30;

    pub(super) fn name(&self) -> &str {
        self.container.name()
    }

    pub(super) fn reach(&self) -> &Reach {
        &self.reach
    }

    pub(super) fn spec(&self) -> &Container {
        &self.container
    }
}

#[cfg(test)]
mod tests {
    use super::{RunContainers, RunNeeds, RunReach};
    use ren_container::{Engine, Mount, Reach};
    use ren_recorder::id::RunId;

    fn needs() -> RunNeeds {
        RunNeeds {
            folder: "/home/op/.local/state/ren/runs/r".into(),
            workdir: "/home/op/dev/app/.worktrees/fix".into(),
            git: Some("/home/op/dev/app/.git".into()),
            host: "/home/op/.local/bin/ren".into(),
            reads: vec!["/home/op/.local/share/claude/versions/2.1.0".into()],
            keeps: vec!["/home/op/.claude".into(), "/home/op/.claude.json".into()],
            doors: None,
        }
    }

    fn containers(ssh_agent: Option<&str>) -> RunContainers {
        RunContainers::new(
            "localhost/ren-instance:abc".into(),
            Reach::Scope,
            "/home/op".into(),
            vec![
                Mount::read_only("/usr"),
                Mount::read_write("/home/op/.cargo"),
            ],
            ssh_agent.map(Into::into),
            RunReach::Host,
        )
    }

    #[test]
    fn a_run_sees_only_what_it_works_on_and_what_its_client_keeps() {
        let run = RunId::restore(uuid::Uuid::nil());
        let args = Engine::Podman.run_args(containers(None).container(run, &needs()).spec());
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
        let args = Engine::Podman.run_args(container.spec());
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
    fn the_ssh_agent_is_passed_when_the_daemon_has_one() {
        let run = RunId::restore(uuid::Uuid::nil());
        let without = Engine::Podman.run_args(containers(None).container(run, &needs()).spec());
        assert!(!without.iter().any(|arg| arg.contains("SSH_AUTH_SOCK")));
        let with = Engine::Podman.run_args(
            containers(Some("/run/user/1000/agent"))
                .container(run, &needs())
                .spec(),
        );
        assert!(
            with.iter()
                .any(|arg| arg == "--volume=/run/user/1000/agent:/run/user/1000/agent:rw")
        );
        assert!(
            with.iter()
                .any(|arg| arg == "--env=SSH_AUTH_SOCK=/run/user/1000/agent")
        );
    }

    #[test]
    fn a_relayed_run_never_holds_the_ssh_agent() {
        let run = RunId::restore(uuid::Uuid::nil());
        let relayed = RunContainers::new(
            "localhost/ren-instance:abc".into(),
            Reach::Scope,
            "/home/op".into(),
            Vec::new(),
            Some("/run/user/1000/agent".into()),
            RunReach::Relayed(ren_relay::RelayPolicy::default()),
        );
        let args = Engine::Podman.run_args(relayed.container(run, &needs()).spec());
        assert!(
            !args.iter().any(|arg| arg.contains("/run/user/1000/agent")),
            "{args:?}"
        );
    }

    #[test]
    fn a_relayed_run_has_no_network_and_sees_the_doors_read_only() {
        let run = RunId::restore(uuid::Uuid::nil());
        let relayed = RunContainers::new(
            "localhost/ren-instance:abc".into(),
            Reach::Scope,
            "/home/op".into(),
            Vec::new(),
            None,
            RunReach::Relayed(ren_relay::RelayPolicy::default()),
        );
        let needs = RunNeeds {
            doors: Some("/home/op/.local/state/ren/runs/.reach".into()),
            ..needs()
        };
        let args = Engine::Podman.run_args(relayed.container(run, &needs).spec());
        assert!(args.iter().any(|arg| arg == "--network=none"), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "--network=host"), "{args:?}");
        assert!(args.iter().any(|arg| arg
            == "--volume=/home/op/.local/state/ren/runs/.reach:/home/op/.local/state/ren/runs/.reach:ro"));
    }
}
