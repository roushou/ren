//! A transient systemd user unit the deployer starts (`systemd-run --user`)
//! and stops by its exact name, never by a pattern that could match the
//! installed service: a deployment's processes. And an
//! installed unit made to run a container in the foreground.

use crate::invocation::{Echo, Invocation, InvocationError};
use std::{
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpStream},
    path::Path,
    thread,
    time::Duration,
};

/// One transient unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransientUnit {
    /// Its name, without `.service`.
    pub name: String,
    /// What `systemctl status` describes it as.
    pub description: String,
    pub kill: KillMode,
}

/// Which of the unit's processes systemd signals when it stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillMode {
    /// The main process alone, as the installed daemon's unit does, so it
    /// drains its runs itself before the final kill covers what is left.
    Mixed,
    /// Every process of the unit.
    ControlGroup,
}

impl KillMode {
    fn property(self) -> &'static str {
        match self {
            Self::Mixed => "mixed",
            Self::ControlGroup => "control-group",
        }
    }
}

impl TransientUnit {
    fn service(&self) -> String {
        format!("{}.service", self.name)
    }

    pub fn active(&self) -> Result<bool, InvocationError> {
        Invocation::new("systemctl")
            .args(["--user", "is-active", "--quiet"])
            .arg(self.service())
            .succeeds(Echo::Hide)
    }

    pub fn stop(&self) -> Result<(), InvocationError> {
        if self.active()? {
            Invocation::new("systemctl")
                .args(["--user", "stop"])
                .arg(self.service())
                .run()?;
        }
        // A failed transient unit stays loaded until reset.
        Invocation::new("systemctl")
            .args(["--user", "reset-failed"])
            .arg(self.service())
            .succeeds(Echo::Hide)?;
        Ok(())
    }

    /// Run `command` as this unit, with `environment`, until stopped.
    pub fn start(
        &self,
        dir: Option<&Path>,
        command: &[&Path],
        args: &[String],
        environment: &[(&str, String)],
    ) -> Result<(), InvocationError> {
        let mut run = Invocation::new("systemd-run")
            .args(["--user", "--collect", "--quiet"])
            .arg(format!("--unit={}", self.name))
            .arg(format!("--description={}", self.description))
            .arg(format!("--property=KillMode={}", self.kill.property()));
        if let Some(dir) = dir {
            run = run.arg(format!("--working-directory={}", dir.display()));
        }
        for (name, value) in environment {
            run = run.arg(format!("--setenv={name}={value}"));
        }
        run.arg("--").args(command).args(args).run()
    }
}

/// An installed unit's main process as a container run in the foreground:
/// `podman run` with these arguments, as systemd starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct ContainerUnit {
    run_args: Vec<String>,
}

/// A rendered unit with no `ExecStart` line to run a container in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the rendered unit has no ExecStart line to run the container")]
pub struct NoExecStart;

impl ContainerUnit {
    pub fn new(run_args: Vec<String>) -> Self {
        Self { run_args }
    }

    /// `rendered`, a unit written for a process on the host, made to run
    /// the container in the foreground instead: its `ExecStart` is `podman
    /// run`. `NoNewPrivileges` goes, since rootless Podman maps the
    /// operator's user through a setuid helper; the container itself runs
    /// with no new privileges.
    pub fn render(&self, rendered: &str) -> Result<String, NoExecStart> {
        let quoted = |arg: &str| {
            format!(
                "\"{}\"",
                arg.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('%', "%%")
            )
        };
        let exec = std::iter::once(quoted("/usr/bin/podman"))
            .chain(self.run_args.iter().map(|arg| quoted(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        let mut replaced = false;
        let lines: Vec<String> = rendered
            .lines()
            .filter(|line| !line.starts_with("NoNewPrivileges="))
            .map(|line| {
                if line.starts_with("ExecStart=") {
                    replaced = true;
                    format!("ExecStart=:{exec}")
                } else {
                    line.to_owned()
                }
            })
            .collect();
        if replaced {
            Ok(lines.join("\n") + "\n")
        } else {
            Err(NoExecStart)
        }
    }
}

/// An address a unit's process listens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listener(pub SocketAddr);

impl Listener {
    /// Whether it answers `GET path` with `200` or `503` (a degraded
    /// recorder) within ten seconds, polled; polls are counted, not timed.
    pub fn answers_soon(self, path: &str) -> bool {
        for _ in 0..100 {
            if self.answers(path) {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }

    fn answers(self, path: &str) -> bool {
        let address = self.0;
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500))
        else {
            return false;
        };
        let request =
            format!("GET {path} HTTP/1.1\r\nhost: {address}\r\nconnection: close\r\n\r\n");
        let mut head = [0_u8; 12];
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .is_ok()
            && stream.write_all(request.as_bytes()).is_ok()
            && stream.read_exact(&mut head).is_ok()
            && (head.ends_with(b"200") || head.ends_with(b"503"))
    }
}
