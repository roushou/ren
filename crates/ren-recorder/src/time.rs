//! Time as a capability.
//!
//! Components receive a [`Clock`] instead of calling `SystemTime::now` or
//! `Instant::now`. Every reading is a [`Moment`]: wall time for display and
//! correlation with the outside world, and monotonic time for durations and
//! ordering within one daemon run. Wall time can jump; monotonic time cannot.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

/// Milliseconds since the Unix epoch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct UnixMillis(u64);

impl UnixMillis {
    pub const EPOCH: Self = Self(0);

    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }

    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(
            self.0
                .saturating_add(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)),
        )
    }
}

/// Why a system time has no `UnixMillis` representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnixMillisError {
    #[error("time is before the Unix epoch")]
    BeforeEpoch,
    #[error("time is too far in the future")]
    Overflow,
}

impl TryFrom<SystemTime> for UnixMillis {
    type Error = UnixMillisError;

    fn try_from(time: SystemTime) -> Result<Self, Self::Error> {
        let since_epoch = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| UnixMillisError::BeforeEpoch)?;
        u64::try_from(since_epoch.as_millis())
            .map(Self)
            .map_err(|_| UnixMillisError::Overflow)
    }
}

/// Elapsed time since the clock's origin (for the daemon's system clock,
/// daemon start). Comparable only between readings of the same clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Monotonic(Duration);

impl Monotonic {
    pub const ORIGIN: Self = Self(Duration::ZERO);

    /// A reading `elapsed` after the clock's origin, for [`Clock`]
    /// implementations outside this crate.
    pub const fn from_elapsed(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    pub const fn since_origin(self) -> Duration {
        self.0
    }

    /// Time from `earlier` to `self`, or zero if `earlier` is later.
    pub fn saturating_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

/// One clock reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    pub wall: UnixMillis,
    pub monotonic: Monotonic,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Moment;
}

/// A clock that moves only when told to. Wall time can be set independently
/// to simulate host clock jumps.
#[derive(Debug)]
pub struct ManualClock {
    wall_millis: AtomicU64,
    monotonic_nanos: AtomicU64,
}

impl ManualClock {
    pub fn at(wall: UnixMillis) -> Self {
        Self {
            wall_millis: AtomicU64::new(wall.as_millis()),
            monotonic_nanos: AtomicU64::new(0),
        }
    }

    /// Advance both wall and monotonic time.
    pub fn advance(&self, by: Duration) {
        let millis = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        let nanos = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        self.wall_millis.fetch_add(millis, Ordering::SeqCst);
        self.monotonic_nanos.fetch_add(nanos, Ordering::SeqCst);
    }

    /// Move only wall time, as a host clock adjustment would.
    pub fn set_wall(&self, wall: UnixMillis) {
        self.wall_millis.store(wall.as_millis(), Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Moment {
        Moment {
            wall: UnixMillis(self.wall_millis.load(Ordering::SeqCst)),
            monotonic: Monotonic(Duration::from_nanos(
                self.monotonic_nanos.load(Ordering::SeqCst),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, ManualClock, Monotonic, UnixMillis, UnixMillisError};
    use std::time::{Duration, SystemTime};

    #[test]
    fn manual_clock_separates_wall_jumps_from_monotonic_time() {
        let clock = ManualClock::at(UnixMillis::from_millis(1_000));
        let start = clock.now();
        assert_eq!(start.monotonic, Monotonic::ORIGIN);
        clock.advance(Duration::from_millis(250));
        clock.set_wall(UnixMillis::from_millis(10));
        let later = clock.now();
        assert_eq!(later.wall, UnixMillis::from_millis(10));
        assert_eq!(
            later.monotonic.saturating_since(start.monotonic),
            Duration::from_millis(250)
        );
        assert_eq!(
            start.monotonic.saturating_since(later.monotonic),
            Duration::ZERO
        );
    }

    #[test]
    fn system_time_conversion_rejects_unrepresentable_times() {
        let before = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(
            UnixMillis::try_from(before),
            Err(UnixMillisError::BeforeEpoch)
        );
        let after = SystemTime::UNIX_EPOCH + Duration::from_millis(42);
        assert_eq!(UnixMillis::try_from(after), Ok(UnixMillis::from_millis(42)));
    }
}
