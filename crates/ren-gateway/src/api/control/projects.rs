//! `/control/v1/projects`: registering the repositories Ren builds
//! (docs/plans/hierarchy.md).
//!
//! - `POST /control/v1/projects`: register a project
//!   (`RegisterProjectRequest`); `201` with its id. A name or a repository
//!   another project holds, archived or not, is `409` `project_taken`; text
//!   that is not a name, an absolute repository path, or a factory is `422`
//!   `invalid_project`.
//! - `POST /control/v1/projects/{name}/archive` and `/unarchive`: `202`.
//!
//! Each is recorded with the recorder, after everything queued before it.
//! The store keeps the first project to take a name or a repository, so two
//! registered at once never both hold one.

use super::{ControlApi, ControlRefusal};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::post,
};
use http::{HeaderMap, StatusCode};
use ren_recorder::{
    batch::ProjectFact,
    id::ProjectId,
    project::{Factory, ProjectName, ProjectRegistration, RepositoryPath},
};
use ren_store::Reader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

pub(super) struct ProjectsApi {
    pub control: Arc<ControlApi>,
    pub reader: Reader,
}

/// A project to register. The repository is its folder's canonical absolute
/// path; the caller resolves it on the daemon's host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct RegisterProjectRequest {
    pub name: String,
    pub repository: String,
    /// `ren` or `contract`.
    pub factory: String,
}

/// What a project request answers: the project, by id and name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProjectAnswer {
    #[ts(as = "String")]
    pub id: ProjectId,
    #[ts(as = "String")]
    pub name: ProjectName,
}

impl RegisterProjectRequest {
    fn registration(self) -> Result<ProjectRegistration, String> {
        Ok(ProjectRegistration {
            name: self.name.parse().map_err(|error| format!("{error}"))?,
            repository: self
                .repository
                .parse::<RepositoryPath>()
                .map_err(|error| format!("{error}"))?,
            factory: self
                .factory
                .parse::<Factory>()
                .map_err(|_| format!("a factory is `ren` or `contract`: {:?}", self.factory))?,
        })
    }
}

/// An archive or unarchive.
#[derive(Debug, Clone, Copy)]
enum Shelving {
    Archive,
    Unarchive,
}

impl ProjectsApi {
    pub fn router(self) -> Router {
        Router::new()
            .route("/control/v1/projects", post(Self::register))
            .route("/control/v1/projects/{name}/archive", post(Self::archive))
            .route(
                "/control/v1/projects/{name}/unarchive",
                post(Self::unarchive),
            )
            .with_state(Arc::new(self))
    }

    async fn register(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<RegisterProjectRequest>, JsonRejection>,
    ) -> Response {
        if let Err(refusal) = api.control.authorize(&headers) {
            return refusal.into_response();
        }
        let Ok(Json(body)) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        let registration = match body.registration() {
            Ok(registration) => registration,
            Err(message) => return ControlRefusal::InvalidProject.explained(Some(message)),
        };
        let taken = match (
            api.reader.project_named(&registration.name).await,
            api.reader.project_at(&registration.repository).await,
        ) {
            (Ok(named), Ok(at)) => named.or(at),
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!(%error, "projects unreadable");
                return ControlRefusal::Unreadable.into_response();
            }
        };
        if let Some(taken) = taken {
            return ControlRefusal::ProjectTaken.explained(Some(format!(
                "project {} is {}{}",
                taken.name,
                taken.repository,
                if taken.archived_at.is_some() {
                    ", archived"
                } else {
                    ""
                }
            )));
        }
        let Some(slot) = api.control.recorder.project_slot() else {
            return ControlRefusal::Unrecorded.into_response();
        };
        let name = registration.name.clone();
        let id = slot.register(registration);
        (
            StatusCode::CREATED,
            ControlApi::answer(ProjectAnswer { id, name }),
        )
            .into_response()
    }

    async fn archive(
        State(api): State<Arc<Self>>,
        Path(name): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        api.shelve(&headers, &name, Shelving::Archive).await
    }

    async fn unarchive(
        State(api): State<Arc<Self>>,
        Path(name): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        api.shelve(&headers, &name, Shelving::Unarchive).await
    }

    async fn shelve(&self, headers: &HeaderMap, name: &str, shelving: Shelving) -> Response {
        if let Err(refusal) = self.control.authorize(headers) {
            return refusal.into_response();
        }
        let Ok(name) = name.parse::<ProjectName>() else {
            return ControlRefusal::NotFound.into_response();
        };
        let project = match self.reader.project_named(&name).await {
            Ok(Some(project)) => project,
            Ok(None) => return ControlRefusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "project unreadable");
                return ControlRefusal::Unreadable.into_response();
            }
        };
        let fact = match (shelving, project.archived_at) {
            (Shelving::Archive, None) => ProjectFact::Archived,
            (Shelving::Unarchive, Some(_)) => ProjectFact::Unarchived,
            (Shelving::Archive, Some(_)) => return ControlRefusal::Archived.into_response(),
            (Shelving::Unarchive, None) => return ControlRefusal::NotArchived.into_response(),
        };
        let Some(slot) = self.control.recorder.project_slot() else {
            return ControlRefusal::Unrecorded.into_response();
        };
        slot.record(project.id, fact);
        (
            StatusCode::ACCEPTED,
            ControlApi::answer(ProjectAnswer {
                id: project.id,
                name: project.name,
            }),
        )
            .into_response()
    }
}
