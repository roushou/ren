//! The dev slot's live traffic settings, as pure data: how often the
//! generator sends a call, how fast the fake provider streams, and the mix
//! of replies, harnesses, and callers. They live in one TOML file that the
//! generator and the fake provider read again on every call, so a change
//! takes effect at once, with no restart.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr, time::Duration};

/// Whether the generator sends calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    #[default]
    Running,
    Paused,
}

/// How fast the fake provider streams a reply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Pace {
    /// Every piece at once.
    Instant,
    /// A piece every 15 ms.
    #[default]
    Normal,
    /// A piece every 150 ms: calls stay in progress for a few seconds.
    Slow,
}

impl Pace {
    /// The wait before each streamed piece.
    pub fn gap(self) -> Duration {
        match self {
            Self::Instant => Duration::ZERO,
            Self::Normal => Duration::from_millis(15),
            Self::Slow => Duration::from_millis(150),
        }
    }
}

/// Relative weights over a fixed set of choices, each 0 to 100, written as
/// a table of `label = weight`; a missing label weighs 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weights<K: Choice>(Vec<(K, u8)>);

/// A set of choices a mix weighs.
pub trait Choice: Copy + Eq + fmt::Debug + 'static {
    const ALL: &'static [Self];
    fn label(self) -> &'static str;
}

/// What the fake provider replies, by the model it is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Text,
    Tools,
    Long,
    Slow,
    Cut,
    Error,
    RateLimited,
}

/// Which harness's headers a call carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Pi,
    Opencode,
    Deepseek,
}

/// Who sends a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    /// The named user `alice`, by Tailscale login.
    Alice,
    /// The named user `ci`, by token.
    Ci,
    /// A Tailscale login no user lists.
    Unlisted,
    Unidentified,
}

impl Choice for Reply {
    const ALL: &'static [Self] = &[
        Self::Text,
        Self::Tools,
        Self::Long,
        Self::Slow,
        Self::Cut,
        Self::Error,
        Self::RateLimited,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Tools => "tools",
            Self::Long => "long",
            Self::Slow => "slow",
            Self::Cut => "cut",
            Self::Error => "error",
            Self::RateLimited => "rate_limited",
        }
    }
}

impl Choice for Harness {
    const ALL: &'static [Self] = &[
        Self::ClaudeCode,
        Self::Codex,
        Self::Pi,
        Self::Opencode,
        Self::Deepseek,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Opencode => "opencode",
            Self::Deepseek => "deepseek",
        }
    }
}

impl Choice for Sender {
    const ALL: &'static [Self] = &[Self::Alice, Self::Ci, Self::Unlisted, Self::Unidentified];
    fn label(self) -> &'static str {
        match self {
            Self::Alice => "alice",
            Self::Ci => "ci",
            Self::Unlisted => "unlisted",
            Self::Unidentified => "unidentified",
        }
    }
}

/// Why a mix was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("`{0}` is not `name=weight`")]
    Pair(String),
    #[error("{0} is not one of: {1}")]
    Unknown(String, String),
    #[error("weights are 0 to 100")]
    Weight,
    #[error("at most {} calls a minute", Settings::MAX_RATE)]
    Rate,
}

impl<K: Choice> Weights<K> {
    fn of(pairs: &[(K, u8)]) -> Self {
        Self(
            K::ALL
                .iter()
                .map(|&key| {
                    (
                        key,
                        pairs.iter().find(|(k, _)| *k == key).map_or(0, |(_, w)| *w),
                    )
                })
                .collect(),
        )
    }

    #[cfg(test)]
    pub fn get(&self, key: K) -> u8 {
        self.0
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, weight)| *weight)
    }

    /// The choice for the `n`th call: deterministic, but spread so that
    /// consecutive calls do not run through one choice at a time. `None`
    /// when every weight is 0.
    pub fn pick(&self, n: u64) -> Option<K> {
        let total: u64 = self.0.iter().map(|(_, w)| u64::from(*w)).sum();
        if total == 0 {
            return None;
        }
        // A Fibonacci hash scatters n over [0, total).
        let mut point = n.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29) % total;
        for (key, weight) in &self.0 {
            let weight = u64::from(*weight);
            if point < weight {
                return Some(*key);
            }
            point -= weight;
        }
        None
    }

    /// Parse `name=weight,…` over these choices; unnamed choices keep
    /// their weight.
    pub fn with(&self, text: &str) -> Result<Self, SettingsError> {
        let mut next = self.clone();
        for pair in text
            .split(',')
            .map(str::trim)
            .filter(|pair| !pair.is_empty())
        {
            let (name, weight) = pair
                .split_once('=')
                .ok_or_else(|| SettingsError::Pair(pair.to_owned()))?;
            let key = K::ALL
                .iter()
                .copied()
                .find(|key| key.label() == name.trim())
                .ok_or_else(|| {
                    SettingsError::Unknown(
                        name.trim().to_owned(),
                        K::ALL
                            .iter()
                            .map(|key| key.label())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                })?;
            let weight: u8 = weight.trim().parse().map_err(|_| SettingsError::Weight)?;
            if weight > 100 {
                return Err(SettingsError::Weight);
            }
            for entry in &mut next.0 {
                if entry.0 == key {
                    entry.1 = weight;
                }
            }
        }
        Ok(next)
    }
}

impl<K: Choice> Serialize for Weights<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(key, weight)| (key.label(), weight)))
    }
}

impl<'de, K: Choice> Deserialize<'de> for Weights<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let table = std::collections::BTreeMap::<String, u8>::deserialize(deserializer)?;
        let text: Vec<String> = table
            .iter()
            .map(|(label, weight)| format!("{label}={weight}"))
            .collect();
        Self::of(&[])
            .with(&text.join(","))
            .map_err(serde::de::Error::custom)
    }
}

impl<K: Choice> fmt::Display for Weights<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: Vec<String> = self
            .0
            .iter()
            .map(|(key, weight)| format!("{}={weight}", key.label()))
            .collect();
        f.write_str(&text.join(","))
    }
}

/// The live traffic settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub flow: Flow,
    /// Calls a minute, 0 to [`Settings::MAX_RATE`].
    pub rate: u16,
    pub pace: Pace,
    pub replies: Weights<Reply>,
    pub harnesses: Weights<Harness>,
    pub senders: Weights<Sender>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            flow: Flow::Running,
            rate: 12,
            pace: Pace::Normal,
            replies: Weights::of(&[
                (Reply::Text, 60),
                (Reply::Tools, 25),
                (Reply::Long, 3),
                (Reply::Slow, 5),
                (Reply::Cut, 2),
                (Reply::Error, 3),
                (Reply::RateLimited, 2),
            ]),
            harnesses: Weights::of(&[
                (Harness::ClaudeCode, 45),
                (Harness::Codex, 30),
                (Harness::Pi, 10),
                (Harness::Opencode, 10),
                (Harness::Deepseek, 5),
            ]),
            senders: Weights::of(&[
                (Sender::Alice, 30),
                (Sender::Ci, 20),
                (Sender::Unlisted, 10),
                (Sender::Unidentified, 40),
            ]),
        }
    }
}

impl Settings {
    pub const MAX_RATE: u16 = 600;

    /// Settings read from TOML, checked; anything else is refused.
    pub fn parse(text: &str) -> Result<Self, String> {
        let settings: Self = toml::from_str(text).map_err(|error| error.message().to_owned())?;
        settings.check().map_err(|error| error.to_string())?;
        Ok(settings)
    }

    pub fn check(&self) -> Result<(), SettingsError> {
        // Weights are checked as they are read.
        if self.rate > Self::MAX_RATE {
            return Err(SettingsError::Rate);
        }
        Ok(())
    }

    /// The wait between calls, or `None` when no call is due (paused, or a
    /// rate of 0).
    pub fn interval(&self) -> Option<Duration> {
        match self.flow {
            Flow::Paused => None,
            Flow::Running if self.rate == 0 => None,
            Flow::Running => Some(Duration::from_millis(60_000 / u64::from(self.rate))),
        }
    }

    /// The settings in `path`; the defaults when it is missing or invalid,
    /// so a bad edit never stops the slot.
    pub fn read(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::parse(&text).ok())
            .unwrap_or_default()
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }
}

impl FromStr for Pace {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        <Self as clap::ValueEnum>::from_str(text, true)
    }
}

#[cfg(test)]
mod tests {
    use super::{Flow, Harness, Reply, Sender, Settings, Weights};
    use std::time::Duration;

    #[test]
    fn the_defaults_round_trip_through_toml() {
        let settings = Settings::default();
        let text = settings.to_toml();
        assert!(text.contains("[replies]"), "{text}");
        assert_eq!(Settings::parse(&text), Ok(settings));
        assert!(Settings::parse("rate = 5").is_err());
    }

    #[test]
    fn picks_follow_the_weights_and_are_spread() {
        let weights = Weights::of(&[(Reply::Text, 3), (Reply::Error, 1)]);
        let picks: Vec<Reply> = (0..400).filter_map(|n| weights.pick(n)).collect();
        let errors = picks.iter().filter(|reply| **reply == Reply::Error).count();
        assert!((80..=120).contains(&errors), "{errors}");
        // Not one long run of the same choice.
        assert!(picks[..8].iter().any(|reply| *reply != picks[0]));
        assert_eq!(Weights::<Sender>::of(&[]).pick(1), None);
    }

    #[test]
    fn a_mix_edits_only_the_named_weights_and_refuses_bad_ones() {
        let harnesses = Settings::default().harnesses;
        let edited = harnesses.with("pi=0, codex=90").unwrap();
        assert_eq!(
            (edited.get(Harness::Pi), edited.get(Harness::Codex)),
            (0, 90)
        );
        assert_eq!(
            edited.get(Harness::ClaudeCode),
            harnesses.get(Harness::ClaudeCode)
        );
        assert!(harnesses.with("vim=1").is_err());
        assert!(harnesses.with("pi=101").is_err());
        assert!(harnesses.with("pi").is_err());
    }

    #[test]
    fn the_interval_follows_the_rate_and_the_flow() {
        let mut settings = Settings {
            rate: 30,
            ..Settings::default()
        };
        assert_eq!(settings.interval(), Some(Duration::from_secs(2)));
        settings.flow = Flow::Paused;
        assert_eq!(settings.interval(), None);
        settings = Settings {
            rate: 0,
            ..Settings::default()
        };
        assert_eq!(settings.interval(), None);
    }
}
