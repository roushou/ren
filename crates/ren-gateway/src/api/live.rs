//! `GET /api/live`: committed observations as server-sent events.
//!
//! A feed subscribes before it reads the store, catches up from `after`, then
//! follows the recorder's broadcast. A subscriber that falls behind the
//! broadcast gets a `lagged` event and catches up from the store again. If
//! `after` was already pruned, the feed sends `gone` and ends: the client must
//! reload. Feeds end when the daemon shuts down.
//!
//! With `progress=true`, a following feed also sends `progress` events:
//! updates on streaming calls (the reply so far, redacted, the tools
//! begun, and whether the model is thinking). They carry no id, are never replayed, and a feed that falls
//! behind skips some; the finished call is the record.

use axum::response::sse::Event;
use ren_recorder::{Observation, progress::Progress};
use ren_store::{Reader, query::PageLimit};
use serde::Serialize;
use std::{collections::VecDeque, convert::Infallible};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

pub(super) struct LiveFeed {
    reader: Reader,
    receiver: broadcast::Receiver<Observation>,
    /// Updates on streaming calls, when the client asked for them.
    progress: Option<broadcast::Receiver<Progress>>,
    shutdown: CancellationToken,
    last: u64,
    pending: VecDeque<Observation>,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Check that `after` is still retained.
    Start,
    CatchUp,
    Follow,
    Done,
}

#[derive(Serialize)]
struct Gone {
    oldest_retained: u64,
}

#[derive(Serialize)]
struct Lagged {
    resume_from: u64,
}

impl LiveFeed {
    const CATCH_UP_PAGE: u32 = 256;

    pub fn new(
        reader: Reader,
        receiver: broadcast::Receiver<Observation>,
        progress: Option<broadcast::Receiver<Progress>>,
        shutdown: CancellationToken,
        after: Option<u64>,
    ) -> Self {
        Self {
            reader,
            receiver,
            progress,
            shutdown,
            last: after.unwrap_or(0),
            pending: VecDeque::new(),
            phase: match after {
                Some(_) => Phase::Start,
                None => Phase::Follow,
            },
        }
    }

    /// The feed as a stream of SSE events.
    pub fn stream(self) -> impl futures_util::Stream<Item = Result<Event, Infallible>> {
        futures_util::stream::unfold(self, |mut feed| async move {
            feed.next().await.map(|event| (Ok(event), feed))
        })
    }

    async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(observation) = self.pending.pop_front() {
                if observation.seq <= self.last {
                    continue;
                }
                self.last = observation.seq;
                if let Ok(event) = Event::default()
                    .event("observation")
                    .id(observation.seq.to_string())
                    .json_data(&observation)
                {
                    return Some(event);
                }
                continue;
            }
            match self.phase {
                Phase::Done => return None,
                Phase::Start => {
                    self.phase = Phase::CatchUp;
                    match self.reader.oldest_seq().await {
                        Ok(Some(oldest)) if self.last.saturating_add(1) < oldest => {
                            self.phase = Phase::Done;
                            return Event::default()
                                .event("gone")
                                .json_data(Gone {
                                    oldest_retained: oldest,
                                })
                                .ok();
                        }
                        Ok(Some(_) | None) => {}
                        Err(_) => return self.fail(),
                    }
                }
                Phase::CatchUp => {
                    let page = PageLimit::new(Self::CATCH_UP_PAGE).unwrap_or(PageLimit::DEFAULT);
                    match self.reader.observations_after(self.last, page).await {
                        Ok(observations) if observations.is_empty() => self.phase = Phase::Follow,
                        Ok(observations) => self.pending.extend(observations),
                        Err(_) => return self.fail(),
                    }
                }
                Phase::Follow => {
                    let Self {
                        receiver,
                        progress,
                        shutdown,
                        ..
                    } = self;
                    let update = async {
                        match progress.as_mut() {
                            Some(progress) => progress.recv().await,
                            None => std::future::pending().await,
                        }
                    };
                    tokio::select! {
                        () = shutdown.cancelled() => self.phase = Phase::Done,
                        received = receiver.recv() => match received {
                            Ok(observation) => self.pending.push_back(observation),
                            Err(RecvError::Lagged(_)) => {
                                self.phase = Phase::CatchUp;
                                return Event::default()
                                    .event("lagged")
                                    .json_data(Lagged { resume_from: self.last })
                                    .ok();
                            }
                            Err(RecvError::Closed) => self.phase = Phase::Done,
                        },
                        received = update => match received {
                            Ok(update) => {
                                if let Ok(event) = Event::default().event("progress").json_data(&update) {
                                    return Some(event);
                                }
                            }
                            // Stale updates are worth skipping, not catching up.
                            Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => self.progress = None,
                        },
                    }
                }
            }
        }
    }

    fn fail(&mut self) -> Option<Event> {
        self.phase = Phase::Done;
        Some(Event::default().event("error").data("store unavailable"))
    }
}
