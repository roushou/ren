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
//! - `GET /api/projects/{name}/workspaces`: the workspaces its current
//!   sessions work in, each with its slot's ports and data folder, whether
//!   a run is under way in it, the work items it serves, and their preview.
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
    id::{ProjectId, SessionId, WorkItemId},
    project::{Factory, ProjectName, RepositoryPath},
    time::UnixMillis,
    work::{PreviewUrl, WorkStateKind},
};
use ren_release::ShotName;
use ren_store::query::{PageLimit, Shelf};
use ren_store::view::{ProjectActivity, ProjectView, WorkCounts};
use ren_supervisor::Workspace;
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

/// `/api/projects/{name}/workspaces`' answer.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProjectWorkspaces {
    pub workspaces: Vec<WorkspaceBody>,
}

/// One workspace: a worktree of the project's repository a session works
/// in, with the ports and data folder its slot holds.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceBody {
    /// `<repo>-<slot>`, the worktree's folder name.
    pub name: String,
    pub folder: String,
    /// Its data folder, as its runs are told (`REN_WORKSPACE_DATA`).
    pub data: String,
    /// Its ports, first and last, as its runs are told.
    pub ports: (u16, u16),
    #[ts(as = "String")]
    pub session: SessionId,
    /// The first line of the session's first prompt; none until it is asked.
    pub prompt: Option<String>,
    /// Whether one of its runs has not ended.
    pub running: bool,
    /// The work items its session took up.
    pub work: Vec<WorkspaceWork>,
}

/// A work item a workspace's session took up, with its preview, if up.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkspaceWork {
    #[ts(as = "String")]
    pub id: WorkItemId,
    pub title: String,
    #[ts(as = "String")]
    pub state: Stored<WorkStateKind>,
    /// Where the preview of its change is served, while one is up.
    #[ts(as = "Option<String>")]
    pub preview: Option<PreviewUrl>,
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

    /// The workspaces a project's current sessions work in. None when runs
    /// are off, since no workspace is made then.
    pub(super) async fn project_workspaces(
        State(state): State<Arc<Self>>,
        Path(name): Path<String>,
    ) -> Result<Response, ApiError> {
        let project = state.named(&name).await?;
        let Some(root) = state.worktrees.as_deref() else {
            return Ok(Self::versioned(ProjectWorkspaces {
                workspaces: Vec::new(),
            })
            .into_response());
        };
        let sessions = state
            .reader
            .sessions(PageLimit::LARGEST, Shelf::Current, Some(&project.name))
            .await?;
        let work = state
            .reader
            .work_items(PageLimit::LARGEST, Shelf::Current, Some(&project.name))
            .await?;
        let workspaces = sessions
            .into_iter()
            .filter_map(|session| {
                let workspace = Workspace::of(root, std::path::Path::new(&session.workdir))?;
                let folder = workspace.folder();
                Some(WorkspaceBody {
                    name: folder.file_name()?.to_str()?.to_owned(),
                    folder: folder.to_str()?.to_owned(),
                    data: workspace.data().to_str()?.to_owned(),
                    ports: workspace.ports(),
                    session: session.id,
                    prompt: session.first_prompt.clone(),
                    running: session.runs.iter().any(|run| run.ended_at.is_none()),
                    work: work
                        .iter()
                        .filter(|item| item.session == Some(session.id))
                        .map(|item| WorkspaceWork {
                            id: item.id,
                            title: item.title.clone(),
                            state: item.state.clone(),
                            preview: item.preview.as_ref().map(|preview| preview.ui.clone()),
                        })
                        .collect(),
                })
            })
            .collect();
        Ok(Self::versioned(ProjectWorkspaces { workspaces }).into_response())
    }

    async fn named(&self, name: &str) -> Result<ProjectView, ApiError> {
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
