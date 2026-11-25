//! The deployer, as the gateway asks it (docs/plans/instances.md, phases 3,
//! 4, and 6): the process on the host that builds commits and runs
//! instances of them, asked over a Unix socket to act now rather than at
//! its next timed pass: to bring the previews up to date, to check a
//! project's `main` as its canary, or to carry out a promotion the operator
//! asked for. The gateway is handed a [`Deployer`]; the deployer's side reads
//! a [`DeployerRequest`] and writes a [`DeployerAnswer`], each one line, pure
//! here. What the GitHub watch sees asks the deployer through the
//! [`Follower`].

mod follow;

pub use follow::Follower;

use futures_util::future::BoxFuture;
use ren_recorder::project::ProjectName;

/// The deployer, asked to act now.
pub trait Deployer: Send + Sync + 'static {
    /// Asks `request`, and answers once the deployer has taken it up (for
    /// the previews, once it has planned which run), or failed to.
    fn ask(
        &self,
        request: DeployerRequest,
    ) -> BoxFuture<'static, Result<DeployerAnswer, DeployerError>>;
}

/// What the deployer is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployerRequest {
    /// Bring the previews up to date.
    Previews,
    /// Check this project's `main`, as the canary, and promote it as the
    /// policy says: the deployer does for the project it builds.
    Canary(ProjectName),
    /// Carry out the promotion the operator asked for.
    Promote,
}

/// What the deployer answers a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployerAnswer {
    /// The previews pass planned which instances run, and goes on building
    /// them.
    Planned,
    /// The canary's or the promotion's pass started.
    Started,
    /// The deployer does not build that project; nothing was done.
    NotBuilt,
    /// The pass failed before it planned or started.
    Failed(String),
}

/// Why the deployer could not be asked: not listening, or no answer.
#[derive(Debug, thiserror::Error)]
pub enum DeployerError {
    #[error("the deployer is not listening: {0}")]
    Unreachable(#[source] std::io::Error),
    #[error("the deployer did not answer in time")]
    TimedOut,
    #[error("the deployer answered {0:?}, which is not an answer")]
    Unread(String),
}

impl DeployerRequest {
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

impl DeployerAnswer {
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
    pub fn read(line: &str) -> Result<Self, DeployerError> {
        let line = line.trim_end();
        match line.split_once(' ') {
            None if line == "planned" => Ok(Self::Planned),
            None if line == "started" => Ok(Self::Started),
            None if line == "not_built" => Ok(Self::NotBuilt),
            Some(("failed", why)) => Ok(Self::Failed(why.to_owned())),
            None | Some(_) => Err(DeployerError::Unread(line.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeployerAnswer, DeployerRequest};

    #[test]
    fn requests_and_answers_read_back_as_written() {
        for request in [
            DeployerRequest::Previews,
            DeployerRequest::Canary("ren".parse().unwrap()),
            DeployerRequest::Promote,
        ] {
            assert_eq!(DeployerRequest::read(&request.line()), Some(request));
        }
        // The wire is pinned: a deployer and a daemon are different builds.
        assert_eq!(DeployerRequest::Previews.line(), "previews\n");
        assert_eq!(
            DeployerRequest::Canary("ren".parse().unwrap()).line(),
            "canary ren\n"
        );
        assert_eq!(DeployerRequest::read("everything\n"), None);
        assert_eq!(DeployerRequest::read("canary\n"), None);
        for answer in [
            DeployerAnswer::Planned,
            DeployerAnswer::Started,
            DeployerAnswer::NotBuilt,
            DeployerAnswer::Failed("the backup failed".into()),
        ] {
            assert_eq!(DeployerAnswer::read(&answer.line()).unwrap(), answer);
        }
        assert_eq!(
            DeployerAnswer::read(&DeployerAnswer::Failed("two\nlines".into()).line()).unwrap(),
            DeployerAnswer::Failed("two lines".into())
        );
        assert!(DeployerAnswer::read("maybe\n").is_err());
    }
}
