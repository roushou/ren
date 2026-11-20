//! An open pull request, as GitHub reports it.

use crate::check::{Check, ChecksState};
use ren_protocol::label::Stored;
use ren_release::Commit;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, num::NonZeroU64, str::FromStr};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A pull request's number in its repository: GitHub's own identity for
/// it, never one of Ren's.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct PullNumber(NonZeroU64);

/// A GitHub account's login, as GitHub names it: never a Ren user.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GithubLogin(String);

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
    /// Its description, where its `Work-Item:` trailers are.
    pub body: String,
    pub url: String,
    /// Its author; none for a deleted account.
    pub author: Option<GithubLogin>,
    pub readiness: Readiness,
    /// The branch it brings in, and its head.
    pub head_branch: String,
    pub head: Commit,
    /// The branch it merges into.
    pub base_branch: String,
    pub merge: Stored<MergeState>,
    pub review: Stored<ReviewDecision>,
    /// Its head's checks, summed up, and each one.
    pub checks: Stored<ChecksState>,
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

impl GithubLogin {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn new(login: String) -> Self {
        Self(login)
    }
}

impl MergeState {
    /// GitHub's `mergeStateStatus`.
    pub(crate) fn of_status(status: &str) -> Stored<Self> {
        match status {
            "CLEAN" => Stored::Known(Self::Clean),
            "BEHIND" => Stored::Known(Self::Behind),
            "BLOCKED" => Stored::Known(Self::Blocked),
            "DIRTY" => Stored::Known(Self::Conflicting),
            "DRAFT" => Stored::Known(Self::Draft),
            "UNSTABLE" => Stored::Known(Self::Unstable),
            "HAS_HOOKS" => Stored::Known(Self::HasHooks),
            "UNKNOWN" => Stored::Known(Self::Computing),
            other => Stored::from_label(other),
        }
    }
}

impl ReviewDecision {
    /// GitHub's `reviewDecision`, null where no review is required.
    pub(crate) fn of_decision(decision: Option<&str>) -> Stored<Self> {
        match decision {
            None => Stored::Known(Self::NotRequired),
            Some("APPROVED") => Stored::Known(Self::Approved),
            Some("CHANGES_REQUESTED") => Stored::Known(Self::ChangesRequested),
            Some("REVIEW_REQUIRED") => Stored::Known(Self::ReviewRequired),
            Some(other) => Stored::from_label(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MergeState, PullNumber, ReviewDecision};
    use ren_protocol::label::Stored;

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
            Stored::Known(MergeState::Conflicting)
        );
        assert_eq!(
            MergeState::of_status("UNKNOWN"),
            Stored::Known(MergeState::Computing)
        );
        assert_eq!(MergeState::of_status("SOMETHING").label(), "SOMETHING");
        assert_eq!(
            ReviewDecision::of_decision(None),
            Stored::Known(ReviewDecision::NotRequired)
        );
        assert_eq!(ReviewDecision::of_decision(Some("NEW")).known(), None);
    }
}
