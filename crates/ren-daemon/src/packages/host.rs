//! The depot's view of this host: programs on a search path, and their
//! versions, read by running them briefly.

use ren_depot::{ProgramHost, package::ProgramName, source::Platform, version::ProbeFailure};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The depot's view of this host: programs on a search path, and their
/// versions, read by running them briefly. The search path is passed in
/// (from `HostEnv`), never read here.
#[derive(Debug, Clone)]
pub struct HostPrograms {
    search_path: Vec<PathBuf>,
}

impl HostPrograms {
    /// How often, and how many times, a probe checks whether its program
    /// has exited: 60 checks 50 ms apart, about three seconds in all.
    const POLL: std::time::Duration = std::time::Duration::from_millis(50);
    const POLLS: u32 = 60;
    /// The most of a probe's output that is read.
    const OUTPUT_LIMIT: u64 = 64 * 1024;

    pub fn new(search_path: Vec<PathBuf>) -> Self {
        Self { search_path }
    }

    /// The platform this binary was built for, if Ren installs packages there.
    pub fn platform() -> Option<Platform> {
        Platform::of(std::env::consts::OS, std::env::consts::ARCH)
    }

    fn executable(path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|metadata| {
            if !metadata.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
    }

    fn output(child: &mut std::process::Child) -> String {
        use std::io::Read as _;
        let mut bytes = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            let _ = stdout.take(Self::OUTPUT_LIMIT).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl ProgramHost for HostPrograms {
    fn locate(&self, program: &ProgramName) -> Option<PathBuf> {
        self.search_path.iter().find_map(|dir| {
            #[cfg(windows)]
            let file = dir.join(format!("{program}.exe"));
            #[cfg(not(windows))]
            let file = dir.join(program.as_str());
            Self::executable(&file).then_some(file)
        })
    }

    fn probe(&self, path: &Path, args: &[&str]) -> Result<String, ProbeFailure> {
        let mut command = std::process::Command::new(path);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[expect(
            clippy::disallowed_methods,
            reason = "the host runs a program it found to read its version"
        )]
        let mut child = command.spawn().map_err(|_| ProbeFailure::NotStarted)?;
        for _ in 0..Self::POLLS {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(Self::output(&mut child)),
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Self::POLL),
            }
        }
        let timed_out = matches!(child.try_wait(), Ok(None));
        let _ = child.kill();
        let _ = child.wait();
        Err(if timed_out {
            ProbeFailure::TimedOut
        } else {
            ProbeFailure::Failed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::HostPrograms;

    use ren_depot::{
        Depot, ProgramHost,
        package::PackageId,
        provision::{Found, Provision},
        version::{ObservedVersion, ProbeFailure},
    };

    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str, mode: u32) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn host_programs_find_executables_on_their_path_and_read_versions() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        // Not executable here, so the next folder's `node` is the one found.
        script(first.path(), "node", "echo v1.0.0", 0o644);
        let node = script(second.path(), "node", "echo v22.17.1", 0o755);
        script(second.path(), "jq", "exit 3", 0o755);
        let host = HostPrograms::new(vec![first.path().into(), second.path().into()]);
        let depot = Depot::builtin().unwrap();
        let provision = |id: PackageId| depot.provision(&host, depot.package(&id).unwrap());
        assert_eq!(
            provision(PackageId::NODE),
            Provision::System(Found {
                path: node,
                version: ObservedVersion::Read("22.17.1".parse().unwrap()),
            })
        );
        assert!(matches!(
            provision(PackageId::JQ),
            Provision::System(Found {
                version: ObservedVersion::Unread(ProbeFailure::Failed),
                ..
            })
        ));
        assert_eq!(provision(PackageId::BUN), Provision::Missing);
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_does_not_answer_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let slow = script(dir.path(), "slow", "exec sleep 30", 0o755);
        let host = HostPrograms::new(vec![dir.path().into()]);
        assert_eq!(
            host.probe(&slow, &["--version"]),
            Err(ProbeFailure::TimedOut)
        );
    }
}
