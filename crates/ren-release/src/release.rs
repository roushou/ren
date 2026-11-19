//! Where a release stands, as the factory last found it: `main`'s tip, the
//! build production runs and how far behind it is, the build on the canary
//! and its verdict, and what promotion decided under the operator's policy.
//! The factory writes it to production's data folder ([`Release::FILE`]);
//! the gateway reads it back and serves it, so `ren` and the UI show it.

use crate::{
    Commit, Revision, Verdict,
    policy::{MinuteOfDay, Policy},
};
use ren_protocol::label::Stored;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Where a release stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    main: Commit,
    production: Revision,
    /// Commits `main` has that production does not; none when production's
    /// revision is not a commit of `main`'s history.
    behind: Option<u64>,
    canary: Option<Canary>,
    policy: Policy,
    decision: Stored<Decision>,
}

/// The build on the canary, and what its checks found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canary {
    commit: Commit,
    verdict: Verdict,
}

/// How the canary's commit relates to the one production runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lineage {
    /// The same commit.
    Same,
    /// The canary's commit follows production's.
    Ahead,
    /// Production has commits the canary's lacks: taking the canary's
    /// build would lose them.
    Behind,
    /// Production runs a build not of a clean commit the factory knows.
    Unknown,
}

/// Whether production already tried to take the canary's build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// Never tried.
    Untried,
    /// Tried, failed its checks, and was rolled back: never tried again
    /// unless the operator promotes it.
    Failed,
}

/// What promotion decided for the canary's build.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Decision {
    /// Production takes it now.
    Promote,
    /// Production already runs it.
    Current,
    /// It failed its checks; production keeps what it runs.
    Failed,
    /// No build has been checked on the canary.
    Unchecked,
    /// Production runs commits it lacks; it is never taken over them.
    ProductionAhead,
    /// Production's build is not one the factory can place; only the
    /// operator promotes over it.
    Unconfirmed,
    /// It passed; the policy leaves promoting it to the operator.
    AwaitingOperator,
    /// It passed; production takes it when the policy's window opens.
    OutsideWindow,
    /// Production tried to take it and was rolled back; it is not tried
    /// again unless the operator promotes it.
    PromotionFailed,
}

/// What the factory found, before the policy decides.
pub struct Assessment {
    pub main: Commit,
    pub production: Revision,
    pub behind: Option<u64>,
    pub canary: Option<(Canary, Lineage, Attempt)>,
}

impl Canary {
    pub fn new(commit: Commit, verdict: Verdict) -> Self {
        Self { commit, verdict }
    }

    pub fn commit(&self) -> &Commit {
        &self.commit
    }

    pub fn verdict(&self) -> &Verdict {
        &self.verdict
    }
}

impl Lineage {
    /// How `candidate` relates to `running`, the clean commit production
    /// runs (none when it runs no clean commit), given how many commits
    /// `running` has that `candidate` lacks, as git counted them (none when
    /// it could not, as for a commit it does not have).
    pub fn read(
        running: Option<&Commit>,
        candidate: &Commit,
        candidate_lacks: Option<u64>,
    ) -> Self {
        match running {
            None => Self::Unknown,
            Some(running) if running == candidate => Self::Same,
            Some(_) => match candidate_lacks {
                Some(0) => Self::Ahead,
                Some(_) => Self::Behind,
                None => Self::Unknown,
            },
        }
    }
}

impl Decision {
    /// What `policy` decides at `now` for a canary build in `lineage` to
    /// production, after `attempt`.
    pub fn of(
        canary: &Canary,
        lineage: Lineage,
        attempt: Attempt,
        policy: Policy,
        now: MinuteOfDay,
    ) -> Self {
        if !canary.verdict.passed() {
            return Self::Failed;
        }
        match (lineage, attempt) {
            (Lineage::Same, Attempt::Untried | Attempt::Failed) => Self::Current,
            (Lineage::Behind, Attempt::Untried | Attempt::Failed) => Self::ProductionAhead,
            (Lineage::Unknown, Attempt::Untried | Attempt::Failed) => Self::Unconfirmed,
            (Lineage::Ahead, Attempt::Failed) => Self::PromotionFailed,
            (Lineage::Ahead, Attempt::Untried) => match policy {
                Policy::Manual => Self::AwaitingOperator,
                Policy::Automatic => Self::Promote,
                Policy::Window(window) if window.holds(now) => Self::Promote,
                Policy::Window(_) => Self::OutsideWindow,
            },
        }
    }
}

impl Release {
    /// The file in production's data folder the factory writes it to.
    pub const FILE: &str = "release.json";
    /// The folder in Ren's data folder each project's release is kept in:
    /// `projects/<name>/release.json`.
    pub const PROJECTS: &str = "projects";

    /// Where the release stands, with what `policy` decides at `now`.
    pub fn assess(assessment: Assessment, policy: Policy, now: MinuteOfDay) -> Self {
        let Assessment {
            main,
            production,
            behind,
            canary,
        } = assessment;
        let (canary, decision) = match canary {
            Some((canary, lineage, attempt)) => {
                let decision = Decision::of(&canary, lineage, attempt, policy, now);
                (Some(canary), decision)
            }
            None => (None, Decision::Unchecked),
        };
        Self {
            main,
            production,
            behind,
            canary,
            policy,
            decision: decision.into(),
        }
    }

    pub fn main(&self) -> &Commit {
        &self.main
    }

    pub fn production(&self) -> &Revision {
        &self.production
    }

    pub fn behind(&self) -> Option<u64> {
        self.behind
    }

    pub fn canary(&self) -> Option<&Canary> {
        self.canary.as_ref()
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    pub fn decision(&self) -> &Stored<Decision> {
        &self.decision
    }
}

#[cfg(test)]
mod tests {
    use super::{Assessment, Attempt, Canary, Decision, Lineage, Release};
    use crate::{Check, CheckKind, Outcome, Policy, Verdict, policy::MinuteOfDay};
    use ren_protocol::label::Stored;
    use strum::VariantArray;

    const MAIN: &str = "62e9e9e0000000000000000000000000000000ff";
    const PRODUCTION: &str = "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708";

    fn canary(outcome: Outcome) -> Canary {
        let mut checks: Vec<_> = CheckKind::VARIANTS
            .iter()
            .map(|kind| Check::new(*kind, Outcome::Passed))
            .collect();
        checks.push(Check::new(CheckKind::Pages, outcome));
        Canary::new(MAIN.parse().unwrap(), Verdict::ren(checks))
    }

    fn at(text: &str) -> MinuteOfDay {
        text.parse().unwrap()
    }

    #[test]
    fn a_passing_build_ahead_of_production_is_promoted_as_the_policy_says() {
        let passed = canary(Outcome::Passed);
        let window: Policy = "02:00-05:00".parse().unwrap();
        for (policy, now, decision) in [
            (Policy::Manual, "03:00", Decision::AwaitingOperator),
            (Policy::Automatic, "12:00", Decision::Promote),
            (window, "03:00", Decision::Promote),
            (window, "12:00", Decision::OutsideWindow),
        ] {
            assert_eq!(
                Decision::of(&passed, Lineage::Ahead, Attempt::Untried, policy, at(now)),
                decision,
                "{policy} at {now}"
            );
        }
    }

    #[test]
    fn nothing_is_promoted_that_failed_is_current_or_would_lose_commits() {
        let failed = canary(Outcome::Failed {
            detail: "the sessions page did not load".into(),
        });
        let passed = canary(Outcome::Passed);
        let now = at("03:00");
        for lineage in [
            Lineage::Same,
            Lineage::Ahead,
            Lineage::Behind,
            Lineage::Unknown,
        ] {
            assert_eq!(
                Decision::of(&failed, lineage, Attempt::Untried, Policy::Automatic, now),
                Decision::Failed
            );
        }
        for (lineage, decision) in [
            (Lineage::Same, Decision::Current),
            (Lineage::Behind, Decision::ProductionAhead),
            (Lineage::Unknown, Decision::Unconfirmed),
        ] {
            assert_eq!(
                Decision::of(&passed, lineage, Attempt::Untried, Policy::Automatic, now),
                decision
            );
        }
        // A build production failed to take is not tried again.
        assert_eq!(
            Decision::of(
                &passed,
                Lineage::Ahead,
                Attempt::Failed,
                Policy::Automatic,
                now
            ),
            Decision::PromotionFailed
        );
    }

    #[test]
    fn a_candidate_follows_production_only_when_it_lacks_none_of_its_commits() {
        let (production, candidate): (crate::Commit, crate::Commit) =
            (PRODUCTION.parse().unwrap(), MAIN.parse().unwrap());
        assert_eq!(Lineage::read(None, &candidate, Some(0)), Lineage::Unknown);
        assert_eq!(
            Lineage::read(Some(&candidate), &candidate, None),
            Lineage::Same
        );
        assert_eq!(
            Lineage::read(Some(&production), &candidate, Some(0)),
            Lineage::Ahead
        );
        assert_eq!(
            Lineage::read(Some(&production), &candidate, Some(2)),
            Lineage::Behind
        );
        assert_eq!(
            Lineage::read(Some(&production), &candidate, None),
            Lineage::Unknown
        );
    }

    #[test]
    fn a_release_reads_back_and_keeps_a_decision_it_does_not_know() {
        let release = Release::assess(
            Assessment {
                main: MAIN.parse().unwrap(),
                production: PRODUCTION.parse().unwrap(),
                behind: Some(3),
                canary: Some((canary(Outcome::Passed), Lineage::Ahead, Attempt::Untried)),
            },
            Policy::Manual,
            at("03:00"),
        );
        assert_eq!(
            release.decision(),
            &Stored::Known(Decision::AwaitingOperator)
        );
        let json = serde_json::to_value(&release).unwrap();
        assert_eq!(json["decision"], "awaiting_operator");
        assert_eq!(json["policy"], "manual");
        assert_eq!(json["production"], PRODUCTION);
        assert_eq!(
            serde_json::from_value::<Release>(json.clone()).unwrap(),
            release
        );

        let mut later = json;
        later["decision"] = "held_for_review".into();
        let read: Release = serde_json::from_value(later).unwrap();
        assert_eq!(read.decision().label(), "held_for_review");

        let unchecked = Release::assess(
            Assessment {
                main: MAIN.parse().unwrap(),
                production: "unknown".parse().unwrap(),
                behind: None,
                canary: None,
            },
            Policy::Automatic,
            at("03:00"),
        );
        assert_eq!(unchecked.decision(), &Stored::Known(Decision::Unchecked));
    }
}
