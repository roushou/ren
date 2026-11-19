//! `/control/v1/projects`: registering the repositories Ren builds
//! (docs/plans/hierarchy.md).
//!
//! - `POST /control/v1/projects`: register a project
//!   (`RegisterProjectRequest`); `201` with its id. A name or a repository
//!   another project holds, archived or not, is `409` `project_taken`; text
//!   that is not a name, an absolute repository path, or a factory is `422`
//!   `invalid_project`.
//! - `POST /control/v1/projects/{name}/archive` and `/unarchive`: `202`.
//! - `POST /control/v1/projects/{name}/adopt`: assign it the sessions no
//!   project owns whose folder is in its repository, or is one of its
//!   worktrees; `202` with the sessions it took.
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
use ren_protocol::label::Stored;
use ren_recorder::{
    batch::{ProjectFact, SessionFact},
    id::{ProjectId, SessionId},
    project::{Factory, ProjectName, ProjectRegistration, RepositoryPath},
};
use ren_store::{Reader, view::ProjectView};
use ren_supervisor::{Supervisor, Worktree};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

pub(super) struct ProjectsApi {
    pub control: Arc<ControlApi>,
    pub reader: Reader,
    /// Where worktrees are, to tell a project's worktrees apart.
    pub supervisor: Option<Supervisor>,
}

/// What an adoption answers: the project, and the sessions it took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProjectAdoption {
    #[ts(as = "String")]
    pub id: ProjectId,
    #[ts(as = "String")]
    pub name: ProjectName,
    #[ts(as = "Vec<String>")]
    pub sessions: Vec<SessionId>,
}

/// The registered projects, read once for one request: what owns a
/// folder, and which project a name or Ren itself is.
pub(super) struct ProjectRoster(Vec<ProjectView>);

impl ProjectRoster {
    pub(super) async fn read(reader: &Reader) -> Result<Self, ControlRefusal> {
        reader.projects().await.map(Self).map_err(|error| {
            tracing::warn!(%error, "projects unreadable");
            ControlRefusal::Unreadable
        })
    }

    /// The project a session in `folder` works for, as the host resolves
    /// the folder (a link or another spelling of a registered repository
    /// still names it); the folder as given when it does not resolve.
    pub(super) async fn owning_folder(&self, folder: &str) -> Option<ProjectId> {
        let resolved = tokio::fs::canonicalize(folder)
            .await
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned));
        self.owning(resolved.as_deref().unwrap_or(folder))
    }

    /// The project a session in `folder` works for: the unarchived one
    /// whose repository holds it, the innermost when repositories nest.
    pub(super) fn owning(&self, folder: &str) -> Option<ProjectId> {
        self.0
            .iter()
            .filter(|project| project.archived_at.is_none() && project.repository.holds(folder))
            .max_by_key(|project| project.repository.to_string().len())
            .map(|project| project.id)
    }

    /// The project `name` names, to work in: none is `not_found`, and an
    /// archived one `archived`.
    pub(super) fn named(&self, name: &str) -> Result<&ProjectView, ControlRefusal> {
        let project = self
            .0
            .iter()
            .find(|project| project.name.as_str() == name)
            .ok_or(ControlRefusal::NotFound)?;
        match project.archived_at {
            None => Ok(project),
            Some(_) => Err(ControlRefusal::Archived),
        }
    }

    /// Ren itself: the one unarchived project built as Ren, when there is
    /// exactly one.
    pub(super) fn ren(&self) -> Option<ProjectId> {
        let mut ren = self.0.iter().filter(|project| {
            project.archived_at.is_none() && project.factory == Stored::Known(Factory::Ren)
        });
        match (ren.next(), ren.next()) {
            (Some(project), None) => Some(project.id),
            (Some(_), Some(_)) | (None, _) => None,
        }
    }

    /// Whether `workdir` is a worktree of `project`'s repository, in the
    /// worktrees folder `root`. Worktrees are named for their repository's
    /// folder, so one shared with another project's repository says
    /// nothing.
    fn worktree_of(&self, project: &ProjectView, root: &std::path::Path, workdir: &str) -> bool {
        let repository = project.repository.as_path();
        let shared = self.0.iter().any(|other| {
            other.id != project.id
                && other.repository.as_path().file_name() == repository.file_name()
        });
        !shared
            && Worktree::of(root, std::path::Path::new(workdir))
                .is_some_and(|worktree| worktree.is_of(repository))
    }
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
            .route("/control/v1/projects/{name}/adopt", post(Self::adopt))
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

    async fn adopt(
        State(api): State<Arc<Self>>,
        Path(name): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(refusal) = api.control.authorize(&headers) {
            return refusal.into_response();
        }
        let roster = match ProjectRoster::read(&api.reader).await {
            Ok(roster) => roster,
            Err(refusal) => return refusal.into_response(),
        };
        let project = match roster.named(&name) {
            Ok(project) => project,
            Err(refusal) => return refusal.into_response(),
        };
        let unassigned = match api.reader.unassigned_sessions().await {
            Ok(unassigned) => unassigned,
            Err(error) => {
                tracing::warn!(%error, "sessions unreadable");
                return ControlRefusal::Unreadable.into_response();
            }
        };
        let root = api.supervisor.as_ref().and_then(Supervisor::worktrees);
        let mut sessions = Vec::new();
        for (session, workdir) in unassigned {
            let ours = project.repository.holds(&workdir)
                || root.is_some_and(|root| roster.worktree_of(project, root, &workdir));
            if !ours {
                continue;
            }
            let Some(slot) = api.control.recorder.session_slot() else {
                return ControlRefusal::Unrecorded.into_response();
            };
            slot.record(
                session,
                SessionFact::Assigned {
                    project: Some(project.id),
                },
            );
            sessions.push(session);
        }
        (
            StatusCode::ACCEPTED,
            ControlApi::answer(ProjectAdoption {
                id: project.id,
                name: project.name.clone(),
                sessions,
            }),
        )
            .into_response()
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
