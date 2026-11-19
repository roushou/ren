//! What a canary's checks found of a build: each [`Check`] passed or
//! failed. A build passes only when every check its [`Suite`] requires ran
//! and passed, and nothing checked failed: a check left out is never a
//! pass. Ren's suite is its doctor, reads, and pages; a project's is its
//! build, its serve step, and every check its contract declares.

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
    /// A project's build steps.
    Build,
    /// A project's serve step, up and answering its health path.
    Serve,
    /// One check a project's contract declares, by its name.
    Declared,
}

/// Whose checks a verdict holds, and so which it requires.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Suite {
    /// Ren's own canary: doctor, reads, and pages.
    Ren,
    /// A project's: its build, its serve step, and each declared check.
    Project,
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
    /// A declared check's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(flatten)]
    outcome: Outcome,
}

/// Every check of one build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    /// Verdicts written before suites are Ren's.
    #[serde(default = "Verdict::ren_suite")]
    suite: Stored<Suite>,
    /// The checks a project's contract declared, each required.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    declared: Vec<String>,
    checks: Vec<Check>,
}

impl Check {
    pub fn new(check: CheckKind, outcome: Outcome) -> Self {
        Self {
            check: check.into(),
            name: None,
            outcome,
        }
    }

    /// The check a project's contract declares as `name`.
    pub fn declared(name: &str, outcome: Outcome) -> Self {
        Self {
            check: CheckKind::Declared.into(),
            name: Some(name.to_owned()),
            outcome,
        }
    }

    pub fn check(&self) -> &Stored<CheckKind> {
        &self.check
    }

    /// A declared check's name.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
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
    /// Ren's canary's checks.
    pub fn ren(checks: Vec<Check>) -> Self {
        Self {
            suite: Self::ren_suite(),
            declared: Vec::new(),
            checks,
        }
    }

    /// A project's checks, with the names its contract declared.
    pub fn project(declared: Vec<String>, checks: Vec<Check>) -> Self {
        Self {
            suite: Suite::Project.into(),
            declared,
            checks,
        }
    }

    fn ren_suite() -> Stored<Suite> {
        Suite::Ren.into()
    }

    pub fn suite(&self) -> &Stored<Suite> {
        &self.suite
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    /// Whether the build passed: every check its suite requires ran and
    /// passed, and no check (one this build does not know included)
    /// failed. A suite this build does not know never passes.
    pub fn passed(&self) -> bool {
        let ran = |kind: CheckKind, name: Option<&str>| {
            self.checks
                .iter()
                .any(|check| check.check == Stored::Known(kind) && check.name() == name)
        };
        let required = match self.suite {
            Stored::Known(Suite::Ren) => [CheckKind::Doctor, CheckKind::Reads, CheckKind::Pages]
                .into_iter()
                .all(|kind| ran(kind, None)),
            Stored::Known(Suite::Project) => {
                ran(CheckKind::Build, None)
                    && ran(CheckKind::Serve, None)
                    && self
                        .declared
                        .iter()
                        .all(|name| ran(CheckKind::Declared, Some(name)))
            }
            Stored::Unknown(_) => false,
        };
        required && self.checks.iter().all(Check::passed)
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
        let all = Verdict::ren(vec![
            passed(CheckKind::Doctor),
            passed(CheckKind::Reads),
            passed(CheckKind::Pages),
        ]);
        assert!(all.passed());
        let missing = Verdict::ren(vec![passed(CheckKind::Doctor), passed(CheckKind::Reads)]);
        assert!(!missing.passed());
        assert!(!Verdict::ren(Vec::new()).passed());
        let failed = Verdict::ren(vec![
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
    fn a_project_passes_when_its_build_serve_and_every_declared_check_did() {
        let declared = vec!["tests".to_owned(), "lint".to_owned()];
        let checks = |lint: Outcome| {
            vec![
                passed(CheckKind::Build),
                passed(CheckKind::Serve),
                Check::declared("tests", Outcome::Passed),
                Check::declared("lint", lint),
            ]
        };
        assert!(Verdict::project(declared.clone(), checks(Outcome::Passed)).passed());
        let failed = Outcome::Failed {
            detail: "exit 1".into(),
        };
        assert!(!Verdict::project(declared.clone(), checks(failed)).passed());
        // A declared check that did not run is not a pass.
        let mut skipped = checks(Outcome::Passed);
        skipped.pop();
        assert!(!Verdict::project(declared, skipped).passed());
        // Nor are Ren's checks a project's.
        assert!(!Verdict::project(Vec::new(), vec![passed(CheckKind::Doctor)]).passed());
        let json = serde_json::to_value(Verdict::project(
            vec!["tests".into()],
            vec![Check::declared("tests", Outcome::Passed)],
        ))
        .unwrap();
        assert_eq!(json["suite"], "project");
        assert_eq!(json["checks"][0]["name"], "tests");
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
        // Written before suites, it is Ren's, and says so once written again.
        let mut written = json;
        written["suite"] = "ren".into();
        assert_eq!(serde_json::to_value(&verdict).unwrap(), written);
    }
}
