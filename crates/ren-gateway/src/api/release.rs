//! `/api/release`'s answer: where the release stands, as the factory
//! (`cargo xtask canary`) last wrote it to this instance's data folder
//! ([`ren_release::Release::FILE`]). The gateway only reads and reports it;
//! the CLI reads these same views back.

use ren_protocol::label::Stored;
use ren_release::{CheckKind, Commit, Decision, Outcome, Policy, Release, Revision};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};
use ts_rs::TS;

/// The release file in a data folder.
#[derive(Debug, Clone)]
pub struct ReleaseFile {
    path: PathBuf,
}

/// Why the release file does not read.
#[derive(Debug, thiserror::Error)]
pub(super) enum ReleaseFileError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not a release: {source}")]
    Shape {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl ReleaseFile {
    /// The release file in `data_dir`.
    pub fn in_data(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(Release::FILE),
        }
    }

    /// The release, or none while the factory has written none. The file is
    /// small and written whole, so it is read in one go.
    pub(super) fn read(&self) -> Result<Option<Release>, ReleaseFileError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ReleaseFileError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| ReleaseFileError::Shape {
                path: self.path.clone(),
                source,
            })
    }
}

/// `/api/release`'s answer.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReleaseBody {
    /// None until the factory has run against this instance.
    pub release: Option<ReleaseView>,
}

/// Where the release stands.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReleaseView {
    /// `main`'s tip as the factory last fetched it.
    #[ts(type = "string")]
    pub main: Commit,
    /// The build production ran then: `<commit>`, `<commit>-dirty`, or
    /// `unknown`.
    #[ts(type = "string")]
    pub production: Revision,
    /// Commits `main` has that production does not; null when production's
    /// revision is not a commit of `main`'s history.
    pub behind: Option<u64>,
    pub canary: Option<CanaryView>,
    /// `manual`, `automatic`, or a daily window in UTC (`02:00-05:00`).
    #[ts(type = "string")]
    pub policy: Policy,
    #[ts(type = "string")]
    pub decision: Stored<Decision>,
}

/// The build on the canary and its checks.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CanaryView {
    #[ts(type = "string")]
    pub commit: Commit,
    /// Whether every check ran and passed.
    pub passed: bool,
    pub checks: Vec<CheckView>,
}

/// One check of the canary's build.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CheckView {
    #[ts(type = "string")]
    pub check: Stored<CheckKind>,
    pub passed: bool,
    /// What failed, for a failed check.
    pub detail: Option<String>,
}

impl From<&Release> for ReleaseView {
    fn from(release: &Release) -> Self {
        Self {
            main: release.main().clone(),
            production: release.production().clone(),
            behind: release.behind(),
            canary: release.canary().map(|canary| CanaryView {
                commit: canary.commit().clone(),
                passed: canary.verdict().passed(),
                checks: canary
                    .verdict()
                    .checks()
                    .iter()
                    .map(|check| {
                        let (passed, detail) = match check.outcome() {
                            Outcome::Passed => (true, None),
                            Outcome::Failed { detail } => (false, Some(detail.clone())),
                        };
                        CheckView {
                            check: check.check().clone(),
                            passed,
                            detail,
                        }
                    })
                    .collect(),
            }),
            policy: release.policy(),
            decision: release.decision().clone(),
        }
    }
}
