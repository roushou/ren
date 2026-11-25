//! What the factory keeps beside the release in production's data folder,
//! for the Release view: the pages the canary captured ([`ShotName`]), the
//! operator's request that production take the canary's build
//! ([`PromotionRequest`]), and each project's release
//! ([`crate::release::Release::PROJECTS`]).

use ren_git::Commit;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};

/// The operator asked that production take the canary's build `commit`.
/// The gateway records it; the factory carries it out on its next pass, as
/// an install restarts the daemon that would otherwise run it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionRequest {
    pub commit: Commit,
}

impl PromotionRequest {
    /// Its file in production's data folder.
    pub const FILE: &str = "release-promote.json";
}

/// A page the canary captured, by name (`sessions`): lowercase letters,
/// digits, and dashes, so a name is always a file of the shots folder and
/// never a path out of it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr)]
pub struct ShotName(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a captured page's name: {0:?}")]
pub struct NotAShotName(pub String);

impl ShotName {
    /// The folder in production's data folder the shots are kept in.
    pub const FOLDER: &str = "release-shots";
    const MOST: usize = 40;

    /// Its file in the shots folder.
    pub fn file(&self) -> String {
        format!("{}.png", self.0)
    }

    /// The name a file of the shots folder is, if it is one.
    pub fn of_file(file: &str) -> Option<Self> {
        file.strip_suffix(".png")?.parse().ok()
    }
}

impl FromStr for ShotName {
    type Err = NotAShotName;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let fine = !text.is_empty()
            && text.len() <= Self::MOST
            && !text.starts_with('-')
            && text
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if fine {
            Ok(Self(text.to_owned()))
        } else {
            Err(NotAShotName(text.to_owned()))
        }
    }
}

impl fmt::Display for ShotName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ShotName;

    #[test]
    fn a_shot_name_is_a_file_of_its_folder_and_never_a_path() {
        let name: ShotName = "sessions".parse().unwrap();
        assert_eq!(name.file(), "sessions.png");
        assert_eq!(
            ShotName::of_file("recording-2.png"),
            Some("recording-2".parse().unwrap())
        );
        for text in ["", "../etc", "a/b", "A", "-x", "x.png", &"a".repeat(41)] {
            assert!(text.parse::<ShotName>().is_err(), "{text:?}");
        }
        assert_eq!(ShotName::of_file("notes.txt"), None);
    }
}
