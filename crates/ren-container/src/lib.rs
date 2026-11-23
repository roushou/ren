//! A container as Ren runs one: what it sees of the host ([`Mount`]), how
//! it lives ([`Lifetime`], [`Placement`]), what it runs, and the
//! [`Engine`]'s arguments that start it, all pure; and running the engine
//! (`launch`): starting a container as a [`Launch`] says, and the pods,
//! containers, and images it keeps.

mod engine;
mod launch;
mod mount;

pub use engine::Engine;
pub use launch::{EngineError, Launch, Reach};
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
    /// On the host's network, as the operator's own user.
    Host,
    /// With no network but its own loopback, as the operator's own user:
    /// it reaches out only through what it is given to see (a socket in a
    /// mounted folder).
    Isolated,
    /// In the named pod, which sets its network and user.
    Pod(String),
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
