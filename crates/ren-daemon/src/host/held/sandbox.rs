//! A run's envelope (self-build plan, phases 11 and 12): what a run held in
//! a container sees, reaches, holds, and runs on, declared as one
//! [`Sandbox`], and the `ren-container` [`Container`] it starts in.
//!
//! What every run's envelope starts from is [`SandboxSettings`], the operator's
//! setting (`[runs.container]`); what one run needs to see is its
//! [`RunFiles`], found on the host by [`super::HeldRuns`]. A sandbox is made
//! only by [`SandboxSettings::sandbox`], which refuses what a run must never
//! see: Ren's data or configuration. A run reaches the host's network,
//! holding the operator's SSH agent, or is relayed: no network but its
//! loopback, where its doors reach the gateway and the relay, under a
//! [`Runtime`] of the operator's choosing. Every path it sees is mounted
//! at its host path, so the run folder, the workdir, and the client's
//! files keep the names the daemon gave them. Pure: no I/O.

use ren_container::{Container, EngineAccess, Lifetime, Mount, Placement, Runtime};
use ren_recorder::id::RunId;
use ren_relay::RelayPolicy;
use std::path::{Path, PathBuf};

/// What every run's envelope starts from (`[runs.container]`): the image,
/// how Podman is reached, what every run sees besides what it needs, and
/// what runs reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSettings {
    image: String,
    engine: EngineAccess,
    /// The operator's home, the container's `HOME`.
    home: PathBuf,
    /// What every run sees: the host's programs and toolchains, shared
    /// caches.
    shared: Vec<Mount>,
    /// What its runs reach, and with it what they hold.
    network: RunNetwork,
    /// Ren's configuration file, when it was read from one: never seen.
    config: Option<PathBuf>,
}

/// What a run reaches, and so what it holds and runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunNetwork {
    /// The host's network, on the host's kernel. It pushes with the
    /// operator's SSH agent, when the daemon has one: until grants push
    /// for production's runs, the one run that holds a credential that
    /// can go anywhere the operator's can.
    Host { ssh_agent: Option<PathBuf> },
    /// No network but its loopback: only Ren's gateway and the relay, which
    /// lets it out where `policy` and its client list, under `runtime`. It
    /// holds no credential of the operator's; it pushes with a grant.
    Relayed {
        policy: RelayPolicy,
        runtime: Runtime,
    },
}

/// What one run needs to see, found on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RunFiles {
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

/// Where Ren keeps what no run may see whole: its data, with the run
/// folders and doors inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenFolders {
    /// Ren's data folder.
    pub(super) data: PathBuf,
    /// The doors' sockets, which a relayed run sees read-only.
    pub(super) doors: PathBuf,
}

/// One run's envelope: what it sees, reaches, holds, and runs on, and the
/// container that is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub(super) struct Sandbox {
    engine: EngineAccess,
    container: Container,
}

/// What a run's sandbox would expose that no run may see.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    /// A mount is Ren's data folder or holds it.
    #[error("a run may not see {0}: it holds Ren's data")]
    HoldsData(PathBuf),
    /// A mount is Ren's configuration or holds it.
    #[error("a run may not see {0}: it holds Ren's configuration")]
    HoldsConfiguration(PathBuf),
    /// A path every run sees is inside Ren's data, where only what Ren
    /// made for one run (its folder, its worktree, the doors) is seen.
    #[error("[runs.container] may not name {0}: it is inside Ren's data")]
    SharedInData(PathBuf),
}

impl SandboxSettings {
    pub fn new(
        image: String,
        engine: EngineAccess,
        home: PathBuf,
        shared: Vec<Mount>,
        network: RunNetwork,
        config: Option<PathBuf>,
    ) -> Self {
        Self {
            image,
            engine,
            home,
            shared,
            network,
            config,
        }
    }

    /// What its runs reach.
    pub fn network(&self) -> &RunNetwork {
        &self.network
    }

    /// The name `run`'s host goes by, as a container and as a systemd
    /// unit, so one name finds it however it was started.
    pub fn name(run: RunId) -> String {
        format!("ren-run-{run}")
    }

    /// The home a run's container takes as its own.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `run`'s sandbox: the shared mounts, then what it needs to see, then
    /// what its network gives it (the doors for a relayed run, the SSH
    /// agent for one on the host's network); the run host serving
    /// `files.folder`, its entrypoint the `ren` it is, never the image's
    /// own, which a mount of the host's programs may hide. Refused when it
    /// would see Ren's data or configuration whole, or every run would see
    /// inside Ren's data.
    pub(super) fn sandbox(
        &self,
        run: RunId,
        files: &RunFiles,
        ren: &RenFolders,
    ) -> Result<Sandbox, SandboxError> {
        if let Some(inside) = self
            .shared
            .iter()
            .find(|mount| mount.path.starts_with(&ren.data))
        {
            return Err(SandboxError::SharedInData(inside.path.clone()));
        }
        let (placement, given, ssh_agent) = match &self.network {
            RunNetwork::Host { ssh_agent } => (
                Placement::Host,
                ssh_agent.iter().map(Mount::read_write).collect::<Vec<_>>(),
                ssh_agent.as_ref(),
            ),
            RunNetwork::Relayed { runtime, .. } => (
                Placement::Isolated(*runtime),
                vec![Mount::read_only(&ren.doors)],
                None,
            ),
        };
        let mounts: Vec<Mount> = self
            .shared
            .iter()
            .cloned()
            .chain([Mount::read_only(&files.host)])
            .chain(files.reads.iter().map(Mount::read_only))
            .chain(files.keeps.iter().map(Mount::read_write))
            .chain([Mount::read_write(&files.workdir)])
            .chain(files.git.iter().map(Mount::read_write))
            .chain([Mount::read_write(&files.folder)])
            .chain(given)
            .collect();
        for mount in &mounts {
            if ren.data.starts_with(&mount.path) {
                return Err(SandboxError::HoldsData(mount.path.clone()));
            }
            if let Some(config) = &self.config
                && config.starts_with(&mount.path)
            {
                return Err(SandboxError::HoldsConfiguration(mount.path.clone()));
            }
        }
        let mut container = Container::new(
            Self::name(run),
            self.image.clone(),
            Lifetime::Removed,
            placement,
        )
        .stop_timeout(Sandbox::STOP_TIMEOUT)
        .mounts(mounts)
        .env("HOME", self.home.display().to_string());
        if let Some(agent) = ssh_agent {
            container = container.env("SSH_AUTH_SOCK", agent.display().to_string());
        }
        Ok(Sandbox {
            engine: self.engine.clone(),
            container: container
                .entrypoint(&files.host)
                .args(["run-host".to_owned(), files.folder.display().to_string()]),
        })
    }
}

impl Sandbox {
    /// Seconds Podman waits after SIGTERM before it kills the run host: the
    /// time it takes to stop its client and write its exit.
    const STOP_TIMEOUT: u32 = 30;

    pub(super) fn name(&self) -> &str {
        self.container.name()
    }

    /// How the engine is reached to start it.
    pub(super) fn engine(&self) -> &EngineAccess {
        &self.engine
    }

    pub(super) fn spec(&self) -> &Container {
        &self.container
    }
}

#[cfg(test)]
mod tests {
    use super::{RenFolders, RunFiles, RunNetwork, SandboxError, SandboxSettings};
    use ren_container::{Engine, EngineAccess, Mount, Runtime};
    use ren_recorder::id::RunId;

    fn sight() -> RunFiles {
        RunFiles {
            folder: "/home/op/.local/state/ren/runs/r".into(),
            workdir: "/home/op/dev/app/.worktrees/fix".into(),
            git: Some("/home/op/dev/app/.git".into()),
            host: "/home/op/.local/bin/ren".into(),
            reads: vec!["/home/op/.local/share/claude/versions/2.1.0".into()],
            keeps: vec!["/home/op/.claude".into(), "/home/op/.claude.json".into()],
        }
    }

    fn ren() -> RenFolders {
        RenFolders {
            data: "/home/op/.local/state/ren".into(),
            doors: "/home/op/.local/state/ren/runs/.doors".into(),
        }
    }

    fn sandboxes(shared: Vec<Mount>, network: RunNetwork) -> SandboxSettings {
        SandboxSettings::new(
            "localhost/ren-instance:abc".into(),
            EngineAccess::Scope,
            "/home/op".into(),
            shared,
            network,
            Some("/home/op/.config/ren/ren.toml".into()),
        )
    }

    fn host(ssh_agent: Option<&str>) -> SandboxSettings {
        sandboxes(
            vec![
                Mount::read_only("/usr"),
                Mount::read_write("/home/op/.cargo"),
            ],
            RunNetwork::Host {
                ssh_agent: ssh_agent.map(Into::into),
            },
        )
    }

    fn relayed(runtime: Runtime) -> SandboxSettings {
        sandboxes(
            Vec::new(),
            RunNetwork::Relayed {
                policy: ren_relay::RelayPolicy::default(),
                runtime,
            },
        )
    }

    fn args(sandboxes: &SandboxSettings) -> Vec<String> {
        let run = RunId::restore(uuid::Uuid::nil());
        Engine::Podman.run_args(sandboxes.sandbox(run, &sight(), &ren()).unwrap().spec())
    }

    #[test]
    fn a_run_sees_only_what_it_works_on_and_what_its_client_keeps() {
        let args = args(&host(None));
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
        let sandbox = host(None).sandbox(run, &sight(), &ren()).unwrap();
        let args = Engine::Podman.run_args(sandbox.spec());
        assert_eq!(sandbox.name(), format!("ren-run-{run}"));
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
    fn on_the_hosts_network_a_run_holds_the_ssh_agent_when_the_daemon_has_one() {
        let without = args(&host(None));
        assert!(!without.iter().any(|arg| arg.contains("SSH_AUTH_SOCK")));
        let with = args(&host(Some("/run/user/1000/agent")));
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
    fn a_relayed_run_has_no_network_and_sees_the_doors_read_only() {
        let args = args(&relayed(Runtime::HostKernel));
        assert!(args.iter().any(|arg| arg == "--network=none"), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "--network=host"), "{args:?}");
        assert!(
            !args.iter().any(|arg| arg.contains("SSH_AUTH_SOCK")),
            "{args:?}"
        );
        assert!(args.iter().any(|arg| arg
            == "--volume=/home/op/.local/state/ren/runs/.doors:/home/op/.local/state/ren/runs/.doors:ro"));
    }

    #[test]
    fn a_run_under_gvisor_keeps_its_doors() {
        let args = args(&relayed(Runtime::Gvisor));
        for flag in [
            "--network=none",
            "--runtime=runsc",
            "--runtime-flag=network=none",
            "--runtime-flag=host-uds=all",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}: {args:?}");
        }
    }

    #[test]
    fn a_run_never_sees_rens_data_or_configuration() {
        let run = RunId::restore(uuid::Uuid::nil());
        for (shared, refused) in [
            (
                Mount::read_only("/home/op"),
                SandboxError::HoldsData("/home/op".into()),
            ),
            (
                Mount::read_only("/home/op/.local/state"),
                SandboxError::HoldsData("/home/op/.local/state".into()),
            ),
            (
                Mount::read_only("/home/op/.config/ren"),
                SandboxError::HoldsConfiguration("/home/op/.config/ren".into()),
            ),
            (
                Mount::read_write("/home/op/.local/state/ren/blobs.sqlite3"),
                SandboxError::SharedInData("/home/op/.local/state/ren/blobs.sqlite3".into()),
            ),
        ] {
            let sandboxes = sandboxes(vec![shared], RunNetwork::Host { ssh_agent: None });
            assert_eq!(sandboxes.sandbox(run, &sight(), &ren()), Err(refused));
        }
        // What Ren made for the run inside its data is the run's to see.
        let in_worktree = RunFiles {
            workdir: "/home/op/.local/state/ren/worktrees/app/fix".into(),
            ..sight()
        };
        assert!(
            relayed(Runtime::HostKernel)
                .sandbox(run, &in_worktree, &ren())
                .is_ok()
        );
        // A client's file that holds the data whole is refused as well.
        let reads_home = RunFiles {
            reads: vec!["/home/op/.local".into()],
            ..sight()
        };
        assert_eq!(
            relayed(Runtime::HostKernel).sandbox(run, &reads_home, &ren()),
            Err(SandboxError::HoldsData("/home/op/.local".into()))
        );
    }
}
