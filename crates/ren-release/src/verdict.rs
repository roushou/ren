//! What the canary's checks found of a build: one [`Check`] per kind, each
//! passed or failed. A build passes only when every kind this build knows
//! was checked and passed, and nothing checked failed: a check left out is
//! never a pass.

use ren_protocol::label::Stored;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// What the canary checks.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CheckKind {
    /// `ren doctor` against the canary, its build check included.
    Doctor,
    /// Every read of the read API, over the copied history.
    Reads,
    /// Every page of the UI, captured in a headless browser.
    Pages,
}

/// How one check went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    Passed,
    /// What failed, as the check found it.
    Failed {
        detail: String,
    },
}

/// One check of a build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    check: Stored<CheckKind>,
    #[serde(flatten)]
    outcome: Outcome,
}

/// Every check of one build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    checks: Vec<Check>,
}

impl Check {
    pub fn new(check: CheckKind, outcome: Outcome) -> Self {
        Self {
            check: check.into(),
            outcome,
        }
    }

    pub fn check(&self) -> &Stored<CheckKind> {
        &self.check
    }

    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    fn passed(&self) -> bool {
        match self.outcome {
            Outcome::Passed => true,
            Outcome::Failed { .. } => false,
        }
    }
}

impl Verdict {
    pub fn new(checks: Vec<Check>) -> Self {
        Self { checks }
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// Whether the build passed: every kind checked and passed, and no
    /// check (one this build does not know included) failed.
    pub fn passed(&self) -> bool {
        let every_kind = CheckKind::VARIANTS.iter().all(|kind| {
            self.checks
                .iter()
                .any(|check| check.check == Stored::Known(*kind))
        });
        every_kind && self.checks.iter().all(Check::passed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Check, CheckKind, Outcome, Verdict};

    fn passed(kind: CheckKind) -> Check {
        Check::new(kind, Outcome::Passed)
    }

    #[test]
    fn a_build_passes_only_when_every_check_ran_and_passed() {
        let all = Verdict::new(vec![
            passed(CheckKind::Doctor),
            passed(CheckKind::Reads),
            passed(CheckKind::Pages),
        ]);
        assert!(all.passed());
        let missing = Verdict::new(vec![passed(CheckKind::Doctor), passed(CheckKind::Reads)]);
        assert!(!missing.passed());
        assert!(!Verdict::new(Vec::new()).passed());
        let failed = Verdict::new(vec![
            passed(CheckKind::Doctor),
            Check::new(
                CheckKind::Reads,
                Outcome::Failed {
                    detail: "GET /api/usage answered 500".into(),
                },
            ),
            passed(CheckKind::Pages),
        ]);
        assert!(!failed.passed());
    }

    #[test]
    fn a_check_this_build_does_not_know_is_kept_and_still_counts() {
        let json = serde_json::json!({ "checks": [
            { "check": "doctor", "outcome": "passed" },
            { "check": "reads", "outcome": "passed" },
            { "check": "pages", "outcome": "passed" },
            { "check": "real_runs", "outcome": "failed", "detail": "codex did not answer" },
        ]});
        let verdict: Verdict = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(verdict.checks()[3].check().label(), "real_runs");
        assert!(!verdict.passed());
        assert_eq!(serde_json::to_value(&verdict).unwrap(), json);
    }
}
