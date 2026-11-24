//! A container as Ren runs one: what it sees of the host ([`Mount`]), how
//! it lives ([`Lifetime`], [`Placement`], [`Runtime`]), what it runs, and the
//! [`Engine`]'s arguments that start it, all pure; and running the engine
//! (`launch`): starting a container as a [`Launch`] says, and the pods,
//! containers, and images it keeps.

mod engine;
mod launch;
mod mount;

pub use engine::Engine;
pub use launch::{EngineAccess, EngineError, Launch};
pub use mount::{Access, Mount};
use std::path::PathBuf;

/// How long a container lives, and whose process it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    /// Started in the background and kept once it exits: a pod's, which
    /// removing the pod takes with it.
    Kept,
    /// Started in the background and removed once it exits.
    Removed,
    /// The caller's foreground process, replacing a container of its name
    /// left behind, and removed when it exits.
    Foreground,
}

/// Where a container runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// On the host's network, as the operator's own user, on the host's
    /// kernel.
    Host,
    /// With no network but its own loopback, as the operator's own user,
    /// under `Runtime`: it reaches out only through what it is given to see
    /// (a socket in a mounted folder).
    Isolated(Runtime),
    /// In the named pod, which sets its network and user, under the
    /// engine's own runtime, as its pod's other containers.
    Pod(String),
}

/// The runtime an engine says it runs containers under, as it names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportedRuntime {
    Known(Runtime),
    /// A runtime Ren does not know: never taken for one it does.
    Unknown(String),
}

/// What runs a container's processes, and so the boundary between them and
/// the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// The engine's own: the container's processes run on the host's
    /// kernel, apart only by namespaces.
    HostKernel,
    /// gVisor (`runsc`): the container's system calls are served by a
    /// kernel of its own in user space, and only what that kernel needs
    /// reaches the host's. Needs no KVM, so it runs rootless; with no
    /// network but its loopback, as rootless it cannot take a network of
    /// the host's.
    Gvisor,
}

impl Runtime {
    /// Podman's own arguments, before or after its command, that run its
    /// containers under it. Rootless, gVisor cannot make the cgroup Podman
    /// asks for; it keeps only its loopback, or has no network at all; and
    /// it connects to a socket on the host, or makes one the host sees,
    /// only when told to (a run host serves the daemon on a socket in its
    /// folder, and reaches out through its doors).
    pub(crate) fn podman_args(self) -> &'static [&'static str] {
        match self {
            Self::HostKernel => &[],
            Self::Gvisor => &[
                "--runtime=runsc",
                "--runtime-flag=ignore-cgroups",
                "--runtime-flag=network=none",
                "--runtime-flag=host-uds=all",
            ],
        }
    }

    /// The runtime an engine reports by its program's name: gVisor's
    /// `runsc`, or one on the host's kernel (`crun`, `runc`, `youki`); any
    /// other is not known to be either.
    pub fn reported(name: &str) -> ReportedRuntime {
        match name.trim() {
            "runsc" => ReportedRuntime::Known(Self::Gvisor),
            "crun" | "runc" | "youki" => ReportedRuntime::Known(Self::HostKernel),
            other => ReportedRuntime::Unknown(other.to_owned()),
        }
    }
}

/// The environment a container starts with, besides what it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Only what it is given.
    Own,
    /// The starting process's environment too.
    Host,
}

/// What a container's processes may gain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Privileges {
    /// What the image allows.
    Image,
    /// No new privileges, through setuid programs or otherwise.
    NoNew,
}

/// A container to start: its name and image, how it lives, what it sees,
/// and what it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Container {
    name: String,
    image: String,
    lifetime: Lifetime,
    placement: Placement,
    environment: Environment,
    privileges: Privileges,
    /// Seconds the engine waits after SIGTERM before it kills it; the
    /// engine's own default when none.
    stop_timeout: Option<u32>,
    workdir: Option<PathBuf>,
    mounts: Vec<Mount>,
    env: Vec<(String, String)>,
    /// Its program, in place of the image's own.
    entrypoint: Option<PathBuf>,
    args: Vec<String>,
}

impl Container {
    pub fn new(
        name: impl Into<String>,
        image: impl Into<String>,
        lifetime: Lifetime,
        placement: Placement,
    ) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            lifetime,
            placement,
            environment: Environment::Own,
            privileges: Privileges::Image,
            stop_timeout: None,
            workdir: None,
            mounts: Vec::new(),
            env: Vec::new(),
            entrypoint: None,
            args: Vec::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// What runs its processes; a pod's are its pod's.
    pub fn runtime(&self) -> Option<Runtime> {
        match &self.placement {
            Placement::Host => Some(Runtime::HostKernel),
            Placement::Isolated(runtime) => Some(*runtime),
            Placement::Pod(_) => None,
        }
    }

    pub fn environment(self, environment: Environment) -> Self {
        Self {
            environment,
            ..self
        }
    }

    pub fn privileges(self, privileges: Privileges) -> Self {
        Self { privileges, ..self }
    }

    pub fn stop_timeout(self, seconds: u32) -> Self {
        Self {
            stop_timeout: Some(seconds),
            ..self
        }
    }

    pub fn workdir(self, dir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: Some(dir.into()),
            ..self
        }
    }

    pub fn mount(mut self, mount: Mount) -> Self {
        self.mounts.push(mount);
        self
    }

    pub fn mounts(mut self, mounts: impl IntoIterator<Item = Mount>) -> Self {
        self.mounts.extend(mounts);
        self
    }

    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    pub fn envs(mut self, vars: impl IntoIterator<Item = (String, String)>) -> Self {
        self.env.extend(vars);
        self
    }

    pub fn entrypoint(self, program: impl Into<PathBuf>) -> Self {
        Self {
            entrypoint: Some(program.into()),
            ..self
        }
    }

    pub fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}
