//! The read API's projects (docs/plans/hierarchy.md, phase 3): each
//! registered project with what is under way for it and its release, and a
//! project's release in full.
//!
//! - `GET /api/projects`: every project, archived ones too, by name.
//! - `GET /api/projects/{name}`: one; `404` for a name no project has.
//! - `GET /api/projects/{name}/release`: its release, and for Ren itself
//!   the pages its canary captured and any promotion asked for.
//! - `GET /api/projects/{name}/release/shots/{shot}`: one captured page,
//!   as PNG; only Ren's canary captures pages.
//!
//! They join the store's projects with the release files the factory
//! writes, so they are served here rather than as a store read.

use super::{ApiError, ApiState, release::ReleaseBody, release::ReleaseView};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use http::header;
use ren_protocol::label::Stored;
use ren_recorder::{
    id::ProjectId,
    project::{Factory, ProjectName, RepositoryPath},
    time::UnixMillis,
};
use ren_release::ShotName;
use ren_store::view::{ProjectActivity, ProjectView, WorkCounts};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// `/api/projects`' answer.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProjectsBody {
    pub projects: Vec<ProjectBody>,
}

/// One project: what it is, what is under way for it, and where its
/// release stands.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProjectBody {
    #[ts(as = "String")]
    pub id: ProjectId,
    #[ts(as = "String")]
    pub name: ProjectName,
    #[ts(as = "String")]
    pub repository: RepositoryPath,
    /// `ren` (Ren itself) or `contract` (built by its `.ren/project.toml`).
    #[ts(as = "String")]
    pub factory: Stored<Factory>,
    #[ts(as = "u64")]
    pub created_at: UnixMillis,
    #[ts(as = "Option<u64>")]
    pub archived_at: Option<UnixMillis>,
    pub work: WorkCountsBody,
    /// Its sessions that are not archived.
    pub sessions: u64,
    /// Its runs that have not ended.
    pub running: u64,
    /// None until its canary has run.
    pub release: Option<ReleaseView>,
}

/// A project's work items, counted by state.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkCountsBody {
    pub queued: u64,
    pub sent: u64,
    pub landed: u64,
    pub shipped: u64,
    pub dismissed: u64,
    /// In a state this build does not know.
    pub other: u64,
}

impl From<WorkCounts> for WorkCountsBody {
    fn from(counts: WorkCounts) -> Self {
        let WorkCounts {
            queued,
            sent,
            landed,
            shipped,
            dismissed,
            other,
        } = counts;
        Self {
            queued,
            sent,
            landed,
            shipped,
            dismissed,
            other,
        }
    }
}

impl ApiState {
    pub(super) async fn projects(State(state): State<Arc<Self>>) -> Result<Response, ApiError> {
        let mut activity = state.reader.project_activity().await?;
        let mut projects = Vec::new();
        for project in state.reader.projects().await? {
            let under_way = activity.remove(&project.id).unwrap_or_default();
            projects.push(state.project_body(project, under_way).await?);
        }
        Ok(Self::versioned(ProjectsBody { projects }).into_response())
    }

    pub(super) async fn project(
        State(state): State<Arc<Self>>,
        Path(name): Path<String>,
    ) -> Result<Response, ApiError> {
        let project = state.named(&name).await?;
        let under_way = state
            .reader
            .project_activity()
            .await?
            .remove(&project.id)
            .unwrap_or_default();
        Ok(Self::versioned(state.project_body(project, under_way).await?).into_response())
    }

    /// A project's release; for Ren itself, with the pages its canary
    /// captured and any promotion asked for.
    pub(super) async fn project_release(
        State(state): State<Arc<Self>>,
        Path(name): Path<String>,
    ) -> Result<Response, ApiError> {
        let project = state.named(&name).await?;
        let release = state.release.release_of(&project).await?;
        let (shots, promotion_requested) = match project.factory {
            Stored::Known(Factory::Ren) => (
                state.release.shots().await,
                state
                    .release
                    .requested()
                    .await?
                    .map(|request| request.commit),
            ),
            Stored::Known(Factory::Contract) | Stored::Unknown(_) => (Vec::new(), None),
        };
        Ok(Self::versioned(ReleaseBody {
            release: release.as_ref().map(ReleaseView::from),
            shots,
            promotion_requested,
        })
        .into_response())
    }

    /// One page Ren's canary captured, as PNG.
    pub(super) async fn project_shot(
        State(state): State<Arc<Self>>,
        Path((name, shot)): Path<(String, String)>,
    ) -> Result<Response, ApiError> {
        let project = state.named(&name).await?;
        match project.factory {
            Stored::Known(Factory::Ren) => {}
            Stored::Known(Factory::Contract) | Stored::Unknown(_) => {
                return Err(ApiError::NotFound);
            }
        }
        let shot: ShotName = shot
            .strip_suffix(".png")
            .unwrap_or(&shot)
            .parse()
            .map_err(|_| ApiError::NotFound)?;
        let bytes = state.release.shot(&shot).await?.ok_or(ApiError::NotFound)?;
        Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
    }

    pub(super) async fn named(&self, name: &str) -> Result<ProjectView, ApiError> {
        let name: ProjectName = name.parse().map_err(|_| ApiError::NotFound)?;
        self.reader
            .project_named(&name)
            .await?
            .ok_or(ApiError::NotFound)
    }

    async fn project_body(
        &self,
        project: ProjectView,
        under_way: ProjectActivity,
    ) -> Result<ProjectBody, ApiError> {
        let release = self.release.release_of(&project).await?;
        let ProjectView {
            id,
            name,
            repository,
            factory,
            created_at,
            archived_at,
        } = project;
        let ProjectActivity {
            work,
            sessions,
            running,
        } = under_way;
        Ok(ProjectBody {
            id,
            name,
            repository,
            factory,
            created_at,
            archived_at,
            work: work.into(),
            sessions,
            running,
            release: release.as_ref().map(ReleaseView::from),
        })
    }
}
