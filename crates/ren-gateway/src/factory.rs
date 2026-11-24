//! The factory, as the gateway asks it (docs/plans/instances.md, phase 3):
//! the process on the host that builds commits and runs instances of them,
//! asked over a Unix socket to bring the previews up to date now rather than
//! at its next timed pass. The gateway is handed a [`Factory`]; the
//! factory's side reads a [`FactoryRequest`] and writes a [`FactoryAnswer`],
//! each one line, pure here.

use futures_util::future::BoxFuture;

/// The factory, asked to bring the instances up to date.
pub trait Factory: Send + Sync + 'static {
    /// Asks for a pass now, and answers once it has planned which
    /// instances run (each one asked for holding its slot, building), or
    /// failed to.
    fn plan(&self) -> BoxFuture<'static, Result<FactoryAnswer, FactoryError>>;
}

/// What the factory is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactoryRequest {
    /// Bring the previews up to date now.
    Previews,
}

/// What the factory answers a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryAnswer {
    /// The pass planned which instances run, and goes on building them.
    Planned,
    /// The pass failed before it planned.
    Failed(String),
}

/// Why the factory could not be asked: not listening, or no answer.
#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    #[error("the factory is not listening: {0}")]
    Unreachable(#[source] std::io::Error),
    #[error("the factory did not answer in time")]
    TimedOut,
    #[error("the factory answered {0:?}, which is not an answer")]
    Unread(String),
}

impl FactoryRequest {
    /// The request as its line on the socket.
    pub fn line(self) -> &'static str {
        match self {
            Self::Previews => "previews\n",
        }
    }

    /// The request a line asks, if it is one.
    pub fn read(line: &str) -> Option<Self> {
        match line.trim_end() {
            "previews" => Some(Self::Previews),
            _ => None,
        }
    }
}

impl FactoryAnswer {
    /// The answer as its line on the socket.
    pub fn line(&self) -> String {
        match self {
            Self::Planned => "planned\n".to_owned(),
            // One line: a failure's own line breaks are not kept.
            Self::Failed(why) => format!("failed {}\n", why.replace('\n', " ")),
        }
    }

    /// The answer a line gives.
    pub fn read(line: &str) -> Result<Self, FactoryError> {
        let line = line.trim_end();
        match line.split_once(' ') {
            None if line == "planned" => Ok(Self::Planned),
            Some(("failed", why)) => Ok(Self::Failed(why.to_owned())),
            None | Some(_) => Err(FactoryError::Unread(line.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FactoryAnswer, FactoryRequest};

    #[test]
    fn requests_and_answers_read_back_as_written() {
        let request = FactoryRequest::Previews;
        assert_eq!(FactoryRequest::read(request.line()), Some(request));
        assert_eq!(FactoryRequest::read("everything\n"), None);
        for answer in [
            FactoryAnswer::Planned,
            FactoryAnswer::Failed("the backup failed".into()),
        ] {
            assert_eq!(FactoryAnswer::read(&answer.line()).unwrap(), answer);
        }
        assert_eq!(
            FactoryAnswer::read(&FactoryAnswer::Failed("two\nlines".into()).line()).unwrap(),
            FactoryAnswer::Failed("two lines".into())
        );
        assert!(FactoryAnswer::read("maybe\n").is_err());
    }
}
