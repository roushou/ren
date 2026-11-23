//! What the factory keeps for the Release view: where each project's
//! release stands, as its canary last wrote it to this instance's data
//! folder (Ren's by `cargo xtask canary`, a contract project's by `ren
//! project canary`), and the pages Ren's canary captured. The gateway reads
//! and reports them (`/api/projects/{name}/release`), and records the
//! operator's promotion request for the factory; the CLI reads these same
//! views back.

use crate::preview::{PreviewRequests, Previews};
use ren_protocol::label::Stored;
use ren_recorder::project::Factory;
use ren_release::{
    CheckKind, Commit, Decision, Outcome, Policy, PromotionRequest, Release, Revision, ShotName,
};
use ren_store::view::ProjectView;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};
use ts_rs::TS;

/// What the factory keeps in a data folder for the Release view: Ren's
/// release ([`Release::FILE`]), the pages its canary captured
/// ([`ShotName::FOLDER`]), the operator's promotion request
/// ([`PromotionRequest::FILE`]), and each project's release
/// ([`Release::PROJECTS`]). Read off the runtime's threads.
#[derive(Debug, Clone)]
pub struct ReleaseFolder {
    data: PathBuf,
    /// Held while the preview requests are read and written back, so two
    /// asks never lose one another.
    asks: std::sync::Arc<tokio::sync::Mutex<()>>,
}

/// Why something of the release folder does not read or write.
#[derive(Debug, thiserror::Error)]
pub(super) enum ReleaseFileError {
    #[error("cannot read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not what it should hold: {source}")]
    Shape {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl ReleaseFolder {
    /// The release folder of the instance whose data is `data_dir`.
    pub fn in_data(data_dir: &Path) -> Self {
        Self {
            data: data_dir.to_path_buf(),
            asks: std::sync::Arc::default(),
        }
    }

    /// Ren's release, or none while the factory has written none.
    pub(super) async fn read(&self) -> Result<Option<Release>, ReleaseFileError> {
        Self::json(&self.data.join(Release::FILE)).await
    }

    /// `project`'s release, or none while its canary has written none:
    /// Ren's where `cargo xtask canary` keeps it, a contract project's in
    /// its folder of [`Release::PROJECTS`], by its registered name.
    pub(super) async fn release_of(
        &self,
        project: &ProjectView,
    ) -> Result<Option<Release>, ReleaseFileError> {
        let path = match project.factory {
            Stored::Known(Factory::Ren) => return self.read().await,
            Stored::Known(Factory::Contract) => self
                .data
                .join(Release::PROJECTS)
                .join(project.name.as_str())
                .join(Release::FILE),
            Stored::Unknown(_) => return Ok(None),
        };
        Self::json(&path).await
    }

    /// The pages the canary captured, by name.
    pub(super) async fn shots(&self) -> Vec<ShotName> {
        let Ok(mut entries) = tokio::fs::read_dir(self.data.join(ShotName::FOLDER)).await else {
            return Vec::new();
        };
        let mut shots = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str().and_then(ShotName::of_file) {
                shots.push(name);
            }
        }
        shots.sort();
        shots
    }

    /// One captured page, as PNG bytes, if the canary kept it.
    pub(super) async fn shot(&self, name: &ShotName) -> Result<Option<Vec<u8>>, ReleaseFileError> {
        let path = self.data.join(ShotName::FOLDER).join(name.file());
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(ReleaseFileError::Io { path, source }),
        }
    }

    /// The build the operator asked production to take, until the factory
    /// takes it up.
    pub(super) async fn requested(&self) -> Result<Option<PromotionRequest>, ReleaseFileError> {
        Self::json(&self.data.join(PromotionRequest::FILE)).await
    }

    /// Records the operator's request, for the factory's next pass.
    pub(super) async fn request(&self, request: &PromotionRequest) -> Result<(), ReleaseFileError> {
        let path = self.data.join(PromotionRequest::FILE);
        let bytes = serde_json::to_vec(request).map_err(|source| ReleaseFileError::Shape {
            path: path.clone(),
            source,
        })?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|source| ReleaseFileError::Io { path, source })
    }

    /// The previews the factory runs; none before its first previews pass.
    pub(super) async fn previews(&self) -> Result<Previews, ReleaseFileError> {
        Ok(Self::json(&self.data.join(Previews::FILE))
            .await?
            .unwrap_or_default())
    }

    /// The commits the operator asked to preview.
    pub(super) async fn preview_requests(&self) -> Result<PreviewRequests, ReleaseFileError> {
        Ok(Self::json(&self.data.join(PreviewRequests::FILE))
            .await?
            .unwrap_or_default())
    }

    /// Changes the preview requests with `change` and writes them back,
    /// one change at a time; answers what `change` answered.
    pub(super) async fn change_preview_requests<T>(
        &self,
        change: impl FnOnce(&mut PreviewRequests) -> T,
    ) -> Result<(T, PreviewRequests), ReleaseFileError> {
        let _held = self.asks.lock().await;
        let mut requests = self.preview_requests().await?;
        let answer = change(&mut requests);
        let path = self.data.join(PreviewRequests::FILE);
        let bytes = serde_json::to_vec(&requests).map_err(|source| ReleaseFileError::Shape {
            path: path.clone(),
            source,
        })?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|source| ReleaseFileError::Io { path, source })?;
        Ok((answer, requests))
    }

    async fn json<T: serde::de::DeserializeOwned>(
        path: &Path,
    ) -> Result<Option<T>, ReleaseFileError> {
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ReleaseFileError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| ReleaseFileError::Shape {
                path: path.to_path_buf(),
                source,
            })
    }
}

/// `/api/projects/{name}/release`'s answer.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReleaseBody {
    /// None until the project's canary has run.
    pub release: Option<ReleaseView>,
    /// The pages Ren's canary captured, by name; each is at
    /// `/api/projects/{name}/release/shots/{shot}`. None for another
    /// project.
    #[serde(default)]
    #[ts(as = "Vec<String>")]
    pub shots: Vec<ShotName>,
    /// The build the operator asked production to take, until the
    /// factory's next pass takes it up.
    #[serde(default)]
    #[ts(as = "Option<String>")]
    pub promotion_requested: Option<Commit>,
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
    /// A project's declared check's name.
    pub name: Option<String>,
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
                            name: check.name().map(str::to_owned),
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
