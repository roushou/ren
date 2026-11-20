//! When production takes a build that passed the canary: the operator's
//! [`Policy`], written as `manual`, `automatic`, or a daily window in UTC
//! such as `02:00-05:00`.

use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};

/// When production takes a build that passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, SerializeDisplay, DeserializeFromStr)]
pub enum Policy {
    /// Only when the operator promotes it. The default.
    #[default]
    Manual,
    /// As soon as it passes.
    Automatic,
    /// As soon as it passes within the window; else when the window opens.
    Window(Window),
}

/// A daily window in UTC, from `opens` until `closes`. One that closes
/// before it opens runs past midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    opens: MinuteOfDay,
    closes: MinuteOfDay,
}

/// A minute of a day in UTC, `00:00` to `23:59`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct MinuteOfDay(u16);

/// The release settings, `~/.config/ren/release.toml`: Ren's own policy,
/// and each project's, by its repository's folder name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReleaseOptions {
    pub promote: Policy,
    pub projects: std::collections::BTreeMap<String, ProjectRelease>,
    /// How many previews of pull requests and commits run at once.
    pub previews: PreviewCapacity,
}

/// How many previews run at once: 1 to 8. Each is a daemon, a UI, and a
/// copy of production's history. Read from configuration as a number, so
/// one out of range is refused as it loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct PreviewCapacity(u8);

/// A preview capacity outside 1 to 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("previews runs 1 to 8 previews at once")]
pub struct PreviewCapacityOutOfRange;

impl PreviewCapacity {
    pub const DEFAULT: Self = Self(2);
    pub const MOST: Self = Self(8);

    pub fn get(self) -> usize {
        usize::from(self.0)
    }
}

impl Default for PreviewCapacity {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u64> for PreviewCapacity {
    type Error = PreviewCapacityOutOfRange;

    fn try_from(count: u64) -> Result<Self, PreviewCapacityOutOfRange> {
        u8::try_from(count)
            .ok()
            .filter(|count| (1..=Self::MOST.0).contains(count))
            .map(Self)
            .ok_or(PreviewCapacityOutOfRange)
    }
}

impl From<PreviewCapacity> for u64 {
    fn from(capacity: PreviewCapacity) -> Self {
        capacity.0.into()
    }
}

/// One project's release settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectRelease {
    pub promote: Policy,
}

impl ReleaseOptions {
    /// `project`'s policy; manual for one not named.
    pub fn project(&self, project: &str) -> Policy {
        self.projects
            .get(project)
            .map_or(Policy::Manual, |release| release.promote)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("not a policy (`manual`, `automatic`, or a window such as `02:00-05:00`): {0:?}")]
    Policy(String),
    #[error("not a time of day (`HH:MM`, UTC): {0:?}")]
    Minute(String),
    #[error("a window that opens as it closes: {0:?}")]
    Empty(String),
}

impl MinuteOfDay {
    const DAY: u16 = 24 * 60;

    /// The minute of the day `unix_ms` falls in.
    pub fn of_unix_ms(unix_ms: u64) -> Self {
        let minutes = unix_ms / 60_000 % u64::from(Self::DAY);
        Self(u16::try_from(minutes).unwrap_or(0))
    }
}

impl FromStr for MinuteOfDay {
    type Err = PolicyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || PolicyError::Minute(text.to_owned());
        let (hours, minutes) = text.split_once(':').ok_or_else(error)?;
        let two_digits = |part: &str| part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit());
        if !two_digits(hours) || !two_digits(minutes) {
            return Err(error());
        }
        let (hours, minutes): (u16, u16) = (
            hours.parse().map_err(|_| error())?,
            minutes.parse().map_err(|_| error())?,
        );
        if hours >= 24 || minutes >= 60 {
            return Err(error());
        }
        Ok(Self(hours * 60 + minutes))
    }
}

impl fmt::Display for MinuteOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

impl Window {
    /// Whether `now` is within the window.
    pub fn holds(self, now: MinuteOfDay) -> bool {
        if self.opens < self.closes {
            self.opens <= now && now < self.closes
        } else {
            now >= self.opens || now < self.closes
        }
    }

    pub fn opens(self) -> MinuteOfDay {
        self.opens
    }
}

impl FromStr for Policy {
    type Err = PolicyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "manual" => Ok(Self::Manual),
            "automatic" => Ok(Self::Automatic),
            _ => {
                let (opens, closes) = text
                    .split_once('-')
                    .ok_or_else(|| PolicyError::Policy(text.to_owned()))?;
                let (opens, closes) = (opens.parse()?, closes.parse()?);
                if opens == closes {
                    return Err(PolicyError::Empty(text.to_owned()));
                }
                Ok(Self::Window(Window { opens, closes }))
            }
        }
    }
}

/// `02:00-05:00`.
impl fmt::Display for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.opens, self.closes)
    }
}

impl fmt::Display for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => f.write_str("manual"),
            Self::Automatic => f.write_str("automatic"),
            Self::Window(window) => write!(f, "{window}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MinuteOfDay, Policy, PolicyError, PreviewCapacity, ReleaseOptions};

    fn minute(text: &str) -> MinuteOfDay {
        text.parse().unwrap()
    }

    #[test]
    fn policies_read_back_as_written_and_malformed_ones_are_refused() {
        for text in ["manual", "automatic", "02:00-05:00", "22:30-01:15"] {
            assert_eq!(text.parse::<Policy>().unwrap().to_string(), text);
        }
        for text in [
            "",
            "auto",
            "2:00-05:00",
            "02:00-24:00",
            "02:60-03:00",
            "02:00",
            "05:00-05:00",
        ] {
            assert!(text.parse::<Policy>().is_err(), "{text:?}");
        }
        assert_eq!(
            "05:00-05:00".parse::<Policy>(),
            Err(PolicyError::Empty("05:00-05:00".into()))
        );
    }

    #[test]
    fn a_window_holds_its_minutes_and_may_run_past_midnight() {
        let Ok(Policy::Window(night)) = "22:30-01:15".parse() else {
            panic!("a window");
        };
        for inside in ["22:30", "23:59", "00:00", "01:14"] {
            assert!(night.holds(minute(inside)), "{inside}");
        }
        for outside in ["01:15", "12:00", "22:29"] {
            assert!(!night.holds(minute(outside)), "{outside}");
        }
        let Ok(Policy::Window(early)) = "02:00-05:00".parse() else {
            panic!("a window");
        };
        assert!(early.holds(minute("02:00")) && !early.holds(minute("05:00")));
    }

    #[test]
    fn a_minute_is_read_from_unix_time_in_utc() {
        // 2026-11-19T03:07:42Z.
        assert_eq!(MinuteOfDay::of_unix_ms(1_795_057_662_000), minute("03:07"));
        assert_eq!(MinuteOfDay::of_unix_ms(0), minute("00:00"));
    }

    #[test]
    fn the_settings_default_to_manual() {
        let options: ReleaseOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(options.promote, Policy::Manual);
        let options: ReleaseOptions = serde_json::from_str(
            r#"{ "promote": "02:00-05:00", "projects": { "acme-web": { "promote": "automatic" } } }"#,
        )
        .unwrap();
        assert_eq!(options.promote.to_string(), "02:00-05:00");
        assert_eq!(options.project("acme-web"), Policy::Automatic);
        assert_eq!(options.project("acme-api"), Policy::Manual);
    }

    #[test]
    fn previews_run_one_to_eight_at_once() {
        let read = |json: &str| serde_json::from_str::<ReleaseOptions>(json);
        assert_eq!(read("{}").unwrap().previews, PreviewCapacity::DEFAULT);
        assert_eq!(read(r#"{"previews": 3}"#).unwrap().previews.get(), 3);
        assert!(read(r#"{"previews": 0}"#).is_err());
        assert!(read(r#"{"previews": 9}"#).is_err());
    }
}
