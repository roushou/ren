//! External commands, reported by their argument list.
use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

/// A command that could not start, failed, or printed what was not
/// understood.
#[derive(Debug, thiserror::Error)]
pub enum InvocationError {
    #[error("`{command}` could not start: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{command}` exited with {status}{detail}")]
    Failed {
        command: String,
        status: ExitStatus,
        detail: String,
    },
    #[error("unexpected output from `{command}`: {detail}")]
    Output { command: String, detail: String },
}

impl InvocationError {
    /// Wraps what was wrong with `command`'s output.
    pub fn output<E: Display>(command: &str) -> impl FnOnce(E) -> Self + '_ {
        move |detail| Self::Output {
            command: command.to_owned(),
            detail: detail.to_string(),
        }
    }
}

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
    /// Variables set for it, over the caller's environment.
    env: Vec<(OsString, OsString)>,
}

impl Invocation {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            dir: None,
            env: Vec::new(),
        }
    }

    /// Runs with `name` set to `value`.
    pub fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((name.as_ref().to_owned(), value.as_ref().to_owned()));
        self
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
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null());
        if let Some(dir) = &self.dir {
            command.current_dir(dir);
        }
        command
    }

    fn spawn_error(&self, source: std::io::Error) -> InvocationError {
        InvocationError::Spawn {
            command: self.describe(),
            source,
        }
    }

    /// Runs with the terminal attached; fails unless it exits successfully.
    #[expect(
        clippy::disallowed_methods,
        reason = "xtask is a shell; Invocation is how it runs external commands"
    )]
    pub fn run(&self) -> Result<(), InvocationError> {
        let status = self
            .command()
            .status()
            .map_err(|source| self.spawn_error(source))?;
        if status.success() {
            Ok(())
        } else {
            Err(InvocationError::Failed {
                command: self.describe(),
                status,
                detail: String::new(),
            })
        }
    }

    /// Whether it exits successfully. Failing to start is still an error.
    #[expect(
        clippy::disallowed_methods,
        reason = "xtask is a shell; Invocation is how it runs external commands"
    )]
    pub fn succeeds(&self, echo: Echo) -> Result<bool, InvocationError> {
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
    pub fn capture(&self) -> Result<String, InvocationError> {
        self.output(Refusal::Fail)
    }

    /// Its standard output read by `parse`; `command` names it in an error.
    pub fn json<T>(
        self,
        command: &str,
        parse: fn(&str) -> Result<T, serde_json::Error>,
    ) -> Result<T, InvocationError> {
        let text = self.capture()?;
        parse(&text).map_err(InvocationError::output(command))
    }

    /// Its standard output, also when it exits unsuccessfully but printed an
    /// answer (`ren runs ls` prints a refusal as JSON); fails, with its
    /// standard error, only when it printed nothing.
    pub fn answer(&self) -> Result<String, InvocationError> {
        self.output(Refusal::Answer)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "xtask is a shell; Invocation is how it runs external commands"
    )]
    fn output(&self, refusal: Refusal) -> Result<String, InvocationError> {
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
            Err(InvocationError::Failed {
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
