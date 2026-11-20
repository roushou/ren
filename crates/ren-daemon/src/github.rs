//! The GitHub poller (docs/plans/self-build.md, phase 8): the shell around
//! `ren_github::Watch`. Each period it sends each watched repository's
//! query to GitHub's GraphQL endpoint with the operator's token, reads the
//! answer, and records it, or why it failed, on the gateway's
//! [`PullBoard`]. GitHub never reaches the host: Ren asks.

use ren_gateway::{
    Shutdown,
    github::{PullBoard, Sighting},
};
use ren_github::{ReadError, RepositorySlug, Watch};
use ren_harness::endpoint::{SecureBase, SecureBaseError};
use ren_recorder::{project::ProjectName, time::Clock};
use std::{fmt, sync::Arc, time::Duration};

/// The operator's GitHub token: read from the daemon's environment, sent
/// only to the configured endpoint, and never shown.
#[derive(Clone)]
pub struct GithubToken(String);

/// How often each repository is polled: 10 to 3600 seconds. Read from
/// configuration as its number of seconds, so a value out of range is
/// refused as it loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct PollSecs(u64);

/// A poll period outside 10 to 3600 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a GitHub poll period is 10 to 3600 seconds")]
pub struct PollSecsOutOfRange;

/// GitHub's GraphQL endpoint, or a loopback stand-in for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlUrl(SecureBase);

/// What the daemon watches on GitHub, resolved.
#[derive(Clone)]
pub struct GithubWatch {
    pub token: GithubToken,
    pub endpoint: GraphqlUrl,
    pub period: PollSecs,
    /// Each watched project, and its repository on GitHub.
    pub repositories: Vec<(ProjectName, RepositorySlug)>,
}

/// Polls each watched repository until the daemon drains.
pub struct GithubPoller {
    client: reqwest::Client,
    watch: GithubWatch,
    board: PullBoard,
    clock: Arc<dyn Clock>,
}

/// Why one poll saw nothing.
#[derive(Debug, thiserror::Error)]
enum PollError {
    #[error("GitHub could not be reached: {0}")]
    Transport(#[source] reqwest::Error),
    #[error("GitHub answered {0}")]
    Status(reqwest::StatusCode),
    #[error("GitHub's answer is larger than {} bytes", GithubPoller::ANSWER_LIMIT)]
    TooLarge,
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl GithubToken {
    /// A token as the environment holds it; none when blank or not a
    /// header value.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        let valid = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_graphic());
        valid.then(|| Self(text.to_owned()))
    }
}

impl fmt::Debug for GithubToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GithubToken(<redacted>)")
    }
}

impl PollSecs {
    pub const DEFAULT: Self = Self(60);

    pub fn duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

impl Default for PollSecs {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u64> for PollSecs {
    type Error = PollSecsOutOfRange;

    fn try_from(seconds: u64) -> Result<Self, PollSecsOutOfRange> {
        if (10..=3600).contains(&seconds) {
            Ok(Self(seconds))
        } else {
            Err(PollSecsOutOfRange)
        }
    }
}

impl From<PollSecs> for u64 {
    fn from(period: PollSecs) -> Self {
        period.0
    }
}

impl GraphqlUrl {
    pub const GITHUB: &str = "https://api.github.com/graphql";

    pub fn parse(text: &str) -> Result<Self, SecureBaseError> {
        let url = reqwest::Url::parse(text).map_err(|_| SecureBaseError::Insecure)?;
        SecureBase::new(url).map(Self)
    }

    fn url(&self) -> &reqwest::Url {
        self.0.url()
    }
}

impl GithubWatch {
    /// A board watching these repositories, nothing seen yet.
    pub fn board(&self) -> PullBoard {
        PullBoard::watching(self.repositories.iter().cloned())
    }
}

impl GithubPoller {
    /// The most of an answer read: 50 pull requests with 50 checks each
    /// and their descriptions fit well within it.
    const ANSWER_LIMIT: usize = 16 * 1024 * 1024;

    /// A poller recording on `board`, which must watch `watch`'s
    /// repositories ([`GithubWatch::board`]).
    pub fn new(
        watch: GithubWatch,
        board: PullBoard,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("ren/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            client,
            watch,
            board,
            clock,
        })
    }

    /// Polls every repository, then again each period, until the gateway
    /// drains.
    pub async fn run(self, stop: Shutdown) {
        loop {
            for (project, repository) in self.board.repositories() {
                let result = tokio::select! {
                    result = self.poll(&repository) => result,
                    () = stop.draining() => return,
                };
                let at = self.clock.now().wall;
                match result {
                    Ok(snapshot) => self.board.record_seen(&project, Sighting { snapshot, at }),
                    Err(error) => {
                        tracing::warn!(%repository, %error, "GitHub poll failed");
                        self.board.record_failed(&project, error.to_string(), at);
                    }
                }
            }
            tokio::select! {
                () = tokio::time::sleep(self.watch.period.duration()) => {}
                () = stop.draining() => return,
            }
        }
    }

    async fn poll(&self, repository: &RepositorySlug) -> Result<ren_github::Snapshot, PollError> {
        let watch = Watch::new(repository.clone());
        let mut response = self
            .client
            .post(self.watch.endpoint.url().clone())
            .bearer_auth(&self.watch.token.0)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(watch.query())
            .send()
            .await
            .map_err(PollError::Transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(PollError::Status(status));
        }
        let mut answer = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(PollError::Transport)? {
            if answer.len().saturating_add(chunk.len()) > Self::ANSWER_LIMIT {
                return Err(PollError::TooLarge);
            }
            answer.extend_from_slice(&chunk);
        }
        Ok(watch.read(&answer)?)
    }
}
