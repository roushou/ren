//! The host's clocks, randomness, and the observation plane's thread, as the
//! daemon hands them to components.
//!
//! These are the production implementations of the recorder's `Clock` and
//! `IdSource` ports, and the only code that reads the host clocks or mints
//! random identities. Components receive them as values; nothing else calls
//! `SystemTime::now`, `Instant::now`, or `Uuid::new_v4`.

use ren_recorder::{
    id::IdSource,
    time::{Clock, Moment, Monotonic, UnixMillis},
};
use std::future::Future;
use std::io;
use std::time::{Instant, SystemTime};
use tokio::sync::oneshot;
use uuid::Uuid;

/// The host's clocks. Monotonic time counts from [`SystemClock::start`].
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned reader of the host's monotonic clock"
    )]
    pub fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    /// A host clock set before 1970 reads as the epoch: wall time is for
    /// display only, while ordering and durations use monotonic time.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned reader of the host's wall clock"
    )]
    fn now(&self) -> Moment {
        Moment {
            wall: UnixMillis::try_from(SystemTime::now()).unwrap_or(UnixMillis::EPOCH),
            monotonic: Monotonic::from_elapsed(self.origin.elapsed()),
        }
    }
}

/// Random version 4 UUIDs.
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomIds;

impl IdSource for RandomIds {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned source of random identities"
    )]
    fn next_uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}

/// The observation plane's own thread and runtime. The recorder runs there,
/// so cutting chunks, inspecting, deriving digests, and writing storage never
/// occupy a worker that forwards traffic, however much work they take.
#[must_use = "the observation task stops when this handle is aborted"]
pub struct ObservationThread<T> {
    output: oneshot::Receiver<T>,
    stop: Option<oneshot::Sender<()>>,
}

impl<T: Send + 'static> ObservationThread<T> {
    /// Run `task` on a current-thread runtime of a new thread.
    pub fn start<F>(task: F) -> io::Result<Self>
    where
        F: Future<Output = T> + Send + 'static,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("ren-observation")
            .build()?;
        let (output_sender, output) = oneshot::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("ren-observation".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    tokio::select! {
                        result = task => {
                            let _ = output_sender.send(result);
                        }
                        // Only an explicit stop ends it early; a dropped
                        // handle lets the task finish on its own.
                        Ok(()) = stopped => {}
                    }
                });
            })?;
        Ok(Self {
            output,
            stop: Some(stop),
        })
    }

    /// The task's output, once it finishes. `None` when it stopped without
    /// one: it panicked, or it was stopped.
    pub async fn join(&mut self) -> Option<T> {
        (&mut self.output).await.ok()
    }

    /// Stop the task at its next await point; [`Self::join`] then returns
    /// `None`.
    pub fn abort(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RandomIds, SystemClock};
    use ren_recorder::{
        id::{AttemptId, IdSource},
        time::{Clock, UnixMillis},
    };

    #[test]
    fn system_clock_is_monotonic() {
        let clock = SystemClock::start();
        let first = clock.now();
        let second = clock.now();
        assert!(second.monotonic >= first.monotonic);
        assert!(first.wall > UnixMillis::EPOCH);
    }

    #[test]
    fn random_ids_are_distinct_and_round_trip_through_text() {
        let source: &dyn IdSource = &RandomIds;
        let first = AttemptId::generate(source);
        let second = AttemptId::generate(source);
        assert_ne!(first, second);
        assert_eq!(first.to_string().parse::<AttemptId>(), Ok(first));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn work_on_the_observation_thread_never_stalls_the_callers_runtime() {
        // A task that blocks its thread, as heavy parsing would.
        let mut thread = super::ObservationThread::start(async {
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::thread::current().name().map(str::to_owned)
        })
        .unwrap();
        // Meanwhile this single-threaded runtime keeps running timers.
        let mut ticks = 0;
        let output = loop {
            tokio::select! {
                output = thread.join() => break output,
                () = tokio::time::sleep(std::time::Duration::from_millis(10)) => ticks += 1,
            }
        };
        assert_eq!(output, Some(Some("ren-observation".to_owned())));
        assert!(ticks >= 10, "the caller's runtime stalled: {ticks} ticks");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_stopped_task_ends_without_output() {
        let mut stopped = super::ObservationThread::start(std::future::pending::<()>()).unwrap();
        stopped.abort();
        assert_eq!(stopped.join().await, None);
        let mut finished = super::ObservationThread::start(async { 7 }).unwrap();
        assert_eq!(finished.join().await, Some(7));
    }
}
