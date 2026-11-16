use std::io;

/// Why a git command did not answer.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("`git {args}` could not start: {source}")]
    NotStarted {
        args: String,
        #[source]
        source: io::Error,
    },
    #[error("`git {args}` exited with {exit}{detail}")]
    Failed {
        args: String,
        exit: Exit,
        /// Its standard error, after a colon, when it printed any.
        detail: String,
    },
    #[error("unexpected output from `git {args}`: {output:?}")]
    Unexpected { args: String, output: String },
}

/// How a git command exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
    /// The host could not say how it exited.
    Unknown,
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Code(code) => write!(f, "code {code}"),
            Self::Signal(signal) => write!(f, "signal {signal}"),
            Self::Unknown => f.write_str("an unknown status"),
        }
    }
}

impl From<std::process::ExitStatus> for Exit {
    fn from(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return Self::Signal(signal);
            }
        }
        status.code().map_or(Self::Unknown, Self::Code)
    }
}
