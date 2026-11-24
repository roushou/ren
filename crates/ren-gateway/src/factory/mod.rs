//! The factory, as the gateway asks it (docs/plans/instances.md, phases 3,
//! 4, and 6): the process on the host that builds commits and runs
//! instances of them, asked over a Unix socket to act now rather than at
//! its next timed pass: to bring the previews up to date, to check a
//! project's `main` as its canary, or to carry out a promotion the operator
//! asked for. The gateway is handed a [`Factory`]; the factory's side reads
//! a [`FactoryRequest`] and writes a [`FactoryAnswer`], each one line, pure
//! here. What the GitHub watch sees asks the factory through the
//! [`Follower`].

mod follow;

pub use follow::Follower;

use futures_util::future::BoxFuture;
use ren_recorder::project::ProjectName;

/// The factory, asked to act now.
pub trait Factory: Send + Sync + 'static {
    /// Asks `request`, and answers once the factory has taken it up (for
    /// the previews, once it has planned which run), or failed to.
    fn ask(
        &self,
        request: FactoryRequest,
    ) -> BoxFuture<'static, Result<FactoryAnswer, FactoryError>>;
}

/// What the factory is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryRequest {
    /// Bring the previews up to date.
    Previews,
    /// Check this project's `main`, as the canary, and promote it as the
    /// policy says: the factory does for the project it builds.
    Canary(ProjectName),
    /// Carry out the promotion the operator asked for.
    Promote,
}

/// What the factory answers a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryAnswer {
    /// The previews pass planned which instances run, and goes on building
    /// them.
    Planned,
    /// The canary's or the promotion's pass started.
    Started,
    /// The factory does not build that project; nothing was done.
    NotBuilt,
    /// The pass failed before it planned or started.
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
    pub fn line(&self) -> String {
        match self {
            Self::Previews => "previews\n".to_owned(),
            Self::Canary(project) => format!("canary {project}\n"),
            Self::Promote => "promote\n".to_owned(),
        }
    }

    /// The request a line asks, if it is one.
    pub fn read(line: &str) -> Option<Self> {
        let line = line.trim_end();
        match line.split_once(' ') {
            None if line == "previews" => Some(Self::Previews),
            None if line == "promote" => Some(Self::Promote),
            Some(("canary", project)) => project.parse().ok().map(Self::Canary),
            None | Some(_) => None,
        }
    }
}

impl FactoryAnswer {
    /// The answer as its line on the socket.
    pub fn line(&self) -> String {
        match self {
            Self::Planned => "planned\n".to_owned(),
            Self::Started => "started\n".to_owned(),
            Self::NotBuilt => "not_built\n".to_owned(),
            // One line: a failure's own line breaks are not kept.
            Self::Failed(why) => format!("failed {}\n", why.replace('\n', " ")),
        }
    }

    /// The answer a line gives.
    pub fn read(line: &str) -> Result<Self, FactoryError> {
        let line = line.trim_end();
        match line.split_once(' ') {
            None if line == "planned" => Ok(Self::Planned),
            None if line == "started" => Ok(Self::Started),
            None if line == "not_built" => Ok(Self::NotBuilt),
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
        for request in [
            FactoryRequest::Previews,
            FactoryRequest::Canary("ren".parse().unwrap()),
            FactoryRequest::Promote,
        ] {
            assert_eq!(FactoryRequest::read(&request.line()), Some(request));
        }
        // The wire is pinned: a factory and a daemon are different builds.
        assert_eq!(FactoryRequest::Previews.line(), "previews\n");
        assert_eq!(
            FactoryRequest::Canary("ren".parse().unwrap()).line(),
            "canary ren\n"
        );
        assert_eq!(FactoryRequest::read("everything\n"), None);
        assert_eq!(FactoryRequest::read("canary\n"), None);
        for answer in [
            FactoryAnswer::Planned,
            FactoryAnswer::Started,
            FactoryAnswer::NotBuilt,
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
