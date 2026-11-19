//! A transient systemd user unit xtask starts (`systemd-run --user`) and
//! stops by its exact name, never by a pattern that could match the
//! installed service: the dev slot's and the canary's processes.

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
