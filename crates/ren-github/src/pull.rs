//! An open pull request, as GitHub reports it.

use crate::check::{Check, ChecksState};
use crate::oid::Oid;
use crate::reported::Reported;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, num::NonZeroU64, str::FromStr};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A pull request's number in its repository.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct PullNumber(NonZeroU64);

/// A GitHub account's login.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Login(String);

/// Whether a pull request is a draft.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Readiness {
    Draft,
    Ready,
}

/// Whether a pull request can merge into its base now, as GitHub works it
/// out (its `mergeStateStatus`).
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum MergeState {
    /// Mergeable, every requirement met.
    Clean,
    /// Its base has moved on: it must be brought up to date first.
    Behind,
    /// A requirement is not met: a required check or review.
    Blocked,
    /// It conflicts with its base.
    Conflicting,
    /// A draft, which cannot merge.
    Draft,
    /// Mergeable, with failing checks that are not required.
    Unstable,
    /// Mergeable, with pre-receive hooks to pass.
    HasHooks,
    /// GitHub has not worked it out yet.
    Computing,
}

/// What its reviews decided, where the repository asks for reviews.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    /// A review is required and none approved it yet.
    ReviewRequired,
    /// The repository asks for no review.
    NotRequired,
}

/// An open pull request, as GitHub reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: PullNumber,
    pub title: String,
    /// Its description.
    pub body: String,
    pub url: String,
    /// Its author; none for a deleted account.
    pub author: Option<Login>,
    pub readiness: Readiness,
    /// The branch it brings in, and its head.
    pub head_branch: String,
    pub head: Oid,
    /// The branch it merges into.
    pub base_branch: String,
    pub merge: Reported<MergeState>,
    pub review: Reported<ReviewDecision>,
    /// Its head's checks, summed up, and each one.
    pub checks: Reported<ChecksState>,
    pub check_list: Vec<Check>,
}

/// Why a number is not a [`PullNumber`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a pull request number is a positive integer: {0:?}")]
pub struct NotAPullNumber(String);

impl PullNumber {
    pub fn get(self) -> u64 {
        self.0.get()
    }

    pub(crate) fn new(number: u64) -> Option<Self> {
        NonZeroU64::new(number).map(Self)
    }
}

impl fmt::Display for PullNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PullNumber {
    type Err = NotAPullNumber;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse()
            .ok()
            .and_then(Self::new)
            .ok_or_else(|| NotAPullNumber(text.to_owned()))
    }
}

impl Login {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn new(login: String) -> Self {
        Self(login)
    }
}

impl MergeState {
    /// GitHub's `mergeStateStatus`.
    pub(crate) fn of_status(status: &str) -> Reported<Self> {
        match status {
            "CLEAN" => Reported::Known(Self::Clean),
            "BEHIND" => Reported::Known(Self::Behind),
            "BLOCKED" => Reported::Known(Self::Blocked),
            "DIRTY" => Reported::Known(Self::Conflicting),
            "DRAFT" => Reported::Known(Self::Draft),
            "UNSTABLE" => Reported::Known(Self::Unstable),
            "HAS_HOOKS" => Reported::Known(Self::HasHooks),
            "UNKNOWN" => Reported::Known(Self::Computing),
            other => Reported::Unknown(other.to_owned()),
        }
    }
}

impl ReviewDecision {
    /// GitHub's `reviewDecision`, null where no review is required.
    pub(crate) fn of_decision(decision: Option<&str>) -> Reported<Self> {
        match decision {
            None => Reported::Known(Self::NotRequired),
            Some("APPROVED") => Reported::Known(Self::Approved),
            Some("CHANGES_REQUESTED") => Reported::Known(Self::ChangesRequested),
            Some("REVIEW_REQUIRED") => Reported::Known(Self::ReviewRequired),
            Some(other) => Reported::Unknown(other.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MergeState, PullNumber, ReviewDecision};
    use crate::reported::Reported;

    #[test]
    fn a_pull_number_is_positive() {
        assert_eq!("42".parse::<PullNumber>().unwrap().get(), 42);
        assert!("0".parse::<PullNumber>().is_err());
        assert!("-1".parse::<PullNumber>().is_err());
    }

    #[test]
    fn github_values_map_and_unknown_ones_are_kept() {
        assert_eq!(
            MergeState::of_status("DIRTY"),
            Reported::Known(MergeState::Conflicting)
        );
        assert_eq!(
            MergeState::of_status("UNKNOWN"),
            Reported::Known(MergeState::Computing)
        );
        assert_eq!(MergeState::of_status("SOMETHING").label(), "SOMETHING");
        assert_eq!(
            ReviewDecision::of_decision(None),
            Reported::Known(ReviewDecision::NotRequired)
        );
        assert_eq!(ReviewDecision::of_decision(Some("NEW")).known(), None);
    }
}
