//! External commands, reported by their argument list.
use crate::install::InstallError;
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Whether a command's output reaches the terminal.
#[derive(Clone, Copy)]
pub enum Echo {
    Show,
    Hide,
}

/// What an unsuccessful exit means for a captured command.
#[derive(Clone, Copy)]
enum Refusal {
    /// It failed.
    Fail,
    /// What it printed is still its answer.
    Answer,
}

/// A program and its arguments, run without a shell.
#[must_use]
pub struct Invocation {
    program: OsString,
    args: Vec<OsString>,
    dir: Option<PathBuf>,
}

impl Invocation {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            dir: None,
        }
    }

    /// Runs in `dir` instead of the current directory.
    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.dir = Some(dir.as_ref().to_owned());
        self
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I>(self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        args.into_iter().fold(self, Self::arg)
    }

    fn describe(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|part| part.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).stdin(Stdio::null());
        if let Some(dir) = &self.dir {
            command.current_dir(dir);
        }
        command
    }

    fn spawn_error(&self, source: std::io::Error) -> InstallError {
        InstallError::Spawn {
            command: self.describe(),
            source,
        }
    }

    /// Runs with the terminal attached; fails unless it exits successfully.
    pub fn run(&self) -> Result<(), InstallError> {
        let status = self
            .command()
            .status()
            .map_err(|source| self.spawn_error(source))?;
        if status.success() {
            Ok(())
        } else {
            Err(InstallError::Failed {
                command: self.describe(),
                status,
                detail: String::new(),
            })
        }
    }

    /// Whether it exits successfully. Failing to start is still an error.
    pub fn succeeds(&self, echo: Echo) -> Result<bool, InstallError> {
        let mut command = self.command();
        match echo {
            Echo::Show => {}
            Echo::Hide => {
                command.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        command
            .status()
            .map(|status| status.success())
            .map_err(|source| self.spawn_error(source))
    }

    /// Its standard output; fails, with its standard error, unless it exits
    /// successfully.
    pub fn capture(&self) -> Result<String, InstallError> {
        self.output(Refusal::Fail)
    }

    /// Its standard output, also when it exits unsuccessfully but printed an
    /// answer (`ren runs ls` prints a refusal as JSON); fails, with its
    /// standard error, only when it printed nothing.
    pub fn answer(&self) -> Result<String, InstallError> {
        self.output(Refusal::Answer)
    }

    fn output(&self, refusal: Refusal) -> Result<String, InstallError> {
        let output = self
            .command()
            .output()
            .map_err(|source| self.spawn_error(source))?;
        let answered = match refusal {
            Refusal::Fail => false,
            Refusal::Answer => !output.stdout.trim_ascii().is_empty(),
        };
        if output.status.success() || answered {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            Err(InstallError::Failed {
                command: self.describe(),
                status: output.status,
                detail: if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                },
            })
        }
    }
}
