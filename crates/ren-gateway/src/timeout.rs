//! A timeout an operator configures, in whole seconds.

use std::time::Duration;

/// A configured timeout: 1 to 3600 seconds. Read from configuration as its
/// number of seconds, so a value out of range is refused as it loads.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "u64", into = "u64")]
pub struct TimeoutSecs(u16);

/// A timeout outside 1 to 3600 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a timeout must be between 1 and {max} seconds", max = TimeoutSecs::MAX.0)]
pub struct TimeoutOutOfRange;

impl TimeoutSecs {
    pub const MIN: Self = Self(1);
    pub const MAX: Self = Self(3600);

    /// `seconds`, brought into range: for defaults and tests, which name
    /// their value in code.
    pub const fn saturating(seconds: u16) -> Self {
        if seconds < Self::MIN.0 {
            Self::MIN
        } else if seconds > Self::MAX.0 {
            Self::MAX
        } else {
            Self(seconds)
        }
    }

    pub fn duration(self) -> Duration {
        Duration::from_secs(self.0.into())
    }
}

impl TryFrom<u64> for TimeoutSecs {
    type Error = TimeoutOutOfRange;

    fn try_from(seconds: u64) -> Result<Self, TimeoutOutOfRange> {
        u16::try_from(seconds)
            .ok()
            .filter(|seconds| (Self::MIN.0..=Self::MAX.0).contains(seconds))
            .map(Self)
            .ok_or(TimeoutOutOfRange)
    }
}

impl From<TimeoutSecs> for u64 {
    fn from(timeout: TimeoutSecs) -> Self {
        timeout.0.into()
    }
}

#[cfg(test)]
mod tests {
    use super::TimeoutSecs;
    use std::time::Duration;

    #[test]
    fn a_timeout_is_one_second_to_an_hour() {
        assert!(TimeoutSecs::try_from(0).is_err());
        assert!(TimeoutSecs::try_from(3601).is_err());
        assert!(TimeoutSecs::try_from(u64::MAX).is_err());
        assert_eq!(
            TimeoutSecs::try_from(3600).unwrap().duration(),
            Duration::from_secs(3600)
        );
        assert_eq!(TimeoutSecs::saturating(0), TimeoutSecs::MIN);
        assert_eq!(TimeoutSecs::saturating(9999), TimeoutSecs::MAX);
        assert_eq!(u64::from(TimeoutSecs::saturating(45)), 45);
    }
}
