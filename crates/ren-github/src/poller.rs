//! A poller: reads each repository through a [`Client`] once a period and
//! hands each one's [`Observation`] on after every read, until stopped.

use crate::{
    client::Client,
    observation::{Observation, Sighting},
    repository::RepositorySlug,
};
use std::{future::Future, time::Duration, time::SystemTime};

/// How often each repository is read: 10 to 3600 seconds. Read from
/// configuration as its number of seconds, so one out of range is refused
/// as it loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct PollPeriod(u64);

/// A poll period outside 10 to 3600 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a GitHub poll period is 10 to 3600 seconds")]
pub struct PollPeriodOutOfRange;

/// Reads each repository once a period.
#[derive(Debug)]
pub struct Poller {
    client: Client,
    period: PollPeriod,
    watched: Vec<(RepositorySlug, Observation)>,
}

impl PollPeriod {
    pub const DEFAULT: Self = Self(60);

    pub fn duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

impl Default for PollPeriod {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u64> for PollPeriod {
    type Error = PollPeriodOutOfRange;

    fn try_from(seconds: u64) -> Result<Self, PollPeriodOutOfRange> {
        if (10..=3600).contains(&seconds) {
            Ok(Self(seconds))
        } else {
            Err(PollPeriodOutOfRange)
        }
    }
}

impl From<PollPeriod> for u64 {
    fn from(period: PollPeriod) -> Self {
        period.0
    }
}

impl Poller {
    pub fn new(
        client: Client,
        period: PollPeriod,
        repositories: impl IntoIterator<Item = RepositorySlug>,
    ) -> Self {
        Self {
            client,
            period,
            watched: repositories
                .into_iter()
                .map(|repository| (repository, Observation::Waiting))
                .collect(),
        }
    }

    /// Reads every repository, then again each period, until `stop`
    /// resolves. After each read, `observed` is told the repository's
    /// observation; `now` says when an answer or failure came.
    pub async fn run(
        mut self,
        mut observed: impl FnMut(&RepositorySlug, &Observation),
        now: impl Fn() -> SystemTime,
        stop: impl Future<Output = ()>,
    ) {
        let mut stop = std::pin::pin!(stop);
        loop {
            for (repository, observation) in &mut self.watched {
                let result = tokio::select! {
                    result = self.client.snapshot(repository) => result,
                    () = &mut stop => return,
                };
                let before = std::mem::replace(observation, Observation::Waiting);
                *observation = match result {
                    Ok(snapshot) => before.seen(Sighting {
                        snapshot,
                        at: now(),
                    }),
                    Err(error) => before.failed(error.to_string(), now()),
                };
                observed(repository, observation);
            }
            tokio::select! {
                () = tokio::time::sleep(self.period.duration()) => {}
                () = &mut stop => return,
            }
        }
    }
}
