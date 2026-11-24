//! The Podman API service whose containers run under gVisor
//! (`ren-gvisor`): a daemon that starts its runs through Podman's API socket
//! cannot choose their runtime, so the socket it is given must be one whose
//! service runs gVisor. Socket-activated, as Podman's own `podman.socket`,
//! and installed beside it.

use crate::{
    file::{FileError, FileMode},
    invocation::{Invocation, InvocationError},
    progress::Progress,
};
use ren_container::{Engine, Runtime};
use ren_harness::host::HostEnv;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The installed `ren-gvisor` units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GvisorService {
    /// The user's unit folder.
    units: PathBuf,
    /// Podman's program, found on the search path.
    podman: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum GvisorServiceError {
    #[error("HOME is not set")]
    NoHome,
    #[error("podman is not on the search path")]
    NoPodman,
    #[error(
        "runsc is not on the search path: install gVisor (Ubuntu's runsc package) for [runs.container] runtime = \"gvisor\""
    )]
    NoRunsc,
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
}

impl GvisorService {
    /// The units' name.
    pub const NAME: &'static str = "ren-gvisor";

    pub fn new(env: &HostEnv) -> Result<Self, GvisorServiceError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(GvisorServiceError::NoHome)?;
        let found = |program: &str| {
            env.search_path()
                .into_iter()
                .map(|dir| dir.join(program))
                .find(|path| path.is_file())
        };
        let podman = found(Engine::Podman.program()).ok_or(GvisorServiceError::NoPodman)?;
        found("runsc").ok_or(GvisorServiceError::NoRunsc)?;
        Ok(Self {
            units: home.join(".config/systemd/user"),
            podman,
        })
    }

    /// Where the service listens, under the user's runtime folder
    /// (`XDG_RUNTIME_DIR`): what `[runs.container] podman_socket` names.
    pub fn socket(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join(Self::NAME).join("podman.sock")
    }

    /// The socket unit.
    fn socket_unit() -> String {
        format!(
            "[Unit]\nDescription=Podman API socket for Ren's runs under gVisor\n\n[Socket]\nListenStream=%t/{name}/podman.sock\nSocketMode=0660\n\n[Install]\nWantedBy=sockets.target\n",
            name = Self::NAME,
        )
    }

    /// The service unit: Podman's own `podman.service`, its containers run
    /// under gVisor. Stopping it leaves its containers running: each has
    /// its own monitor.
    fn service_unit(&self) -> String {
        let args = Engine::Podman.service_args(Runtime::Gvisor).join(" ");
        format!(
            "[Unit]\nDescription=Podman API service for Ren's runs under gVisor\nRequires={name}.socket\nAfter={name}.socket\nStartLimitIntervalSec=0\n\n[Service]\nDelegate=true\nType=exec\nKillMode=process\nExecStart={podman} {args}\n\n[Install]\nWantedBy=default.target\n",
            name = Self::NAME,
            podman = self.podman.display(),
        )
    }

    /// Writes the units if they differ and listens on the socket; a
    /// changed service is restarted, its containers left running.
    pub fn ensure(&self) -> Result<(), GvisorServiceError> {
        fs::create_dir_all(&self.units).map_err(FileError::at(&self.units))?;
        let mut changed = Vec::new();
        for (suffix, rendered) in [
            ("socket", Self::socket_unit()),
            ("service", self.service_unit()),
        ] {
            let unit = self.units.join(format!("{}.{suffix}", Self::NAME));
            if fs::read_to_string(&unit).ok().as_deref() != Some(rendered.as_str()) {
                Progress(&format!("Writing {}", unit.display())).show();
                FileMode::Readable.write(&unit, rendered.as_bytes())?;
                changed.push(suffix);
            }
        }
        Invocation::new("systemctl")
            .args(["--user", "daemon-reload"])
            .run()?;
        Invocation::new("systemctl")
            .args(["--user", "enable", "--now"])
            .arg(format!("{}.socket", Self::NAME))
            .run()?;
        if changed.contains(&"service") {
            Invocation::new("systemctl")
                .args(["--user", "try-restart"])
                .arg(format!("{}.service", Self::NAME))
                .run()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::GvisorService;
    use std::path::Path;

    #[test]
    fn its_service_runs_podman_under_gvisor_on_its_own_socket() {
        let service = GvisorService {
            units: "/home/op/.config/systemd/user".into(),
            podman: "/usr/bin/podman".into(),
        };
        let unit = service.service_unit();
        assert!(
            unit.contains("\nExecStart=/usr/bin/podman --runtime=runsc --runtime-flag=ignore-cgroups --runtime-flag=network=none --runtime-flag=host-uds=all system service\n"),
            "{unit}"
        );
        // Its containers outlive it, as Podman's own service's do.
        assert!(unit.contains("\nKillMode=process\n"), "{unit}");
        assert!(
            GvisorService::socket_unit().contains("\nListenStream=%t/ren-gvisor/podman.sock\n")
        );
        assert_eq!(
            GvisorService::socket(Path::new("/run/user/1000")),
            Path::new("/run/user/1000/ren-gvisor/podman.sock")
        );
    }
}
