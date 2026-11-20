//! The factory's previews of a project (docs/plans/self-build.md, phase
//! 9): `GET /api/projects/{name}/previews`, each preview in its slot (of a
//! pull request or a commit asked for, the commit it runs, where it stands,
//! and the ports its UI answers on) and the commits asked for, as the
//! factory last wrote them; `404` for a name no project has. Only Ren
//! itself is previewed so; any other project has none.

use super::{ApiError, ApiState};
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use ren_preview::{Preview, PreviewKey, PreviewState, Reach};
use ren_protocol::label::Stored;
use ren_recorder::project::Factory;
use ren_release::Commit;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// `/api/projects/{name}/previews`' answer.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PreviewsBody {
    pub previews: Vec<PreviewBody>,
    /// The commits the operator asked to preview, the newest ask first.
    #[ts(as = "Vec<String>")]
    pub requested: Vec<Commit>,
}

/// One preview in its slot.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PreviewBody {
    pub slot: u64,
    pub of: PreviewOfBody,
    #[ts(as = "String")]
    pub commit: Commit,
    pub state: PreviewStateBody,
    /// The loopback port its UI answers on.
    pub ui_port: u16,
    /// The tailnet port its UI is published on, when the factory publishes.
    pub published_ui_port: u16,
}

/// What a preview is of.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewOfBody {
    /// An open pull request, at its head.
    Pull { number: u64 },
    /// A commit the operator asked for.
    Commit,
}

/// Where a preview stands.
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PreviewStateBody {
    Building,
    /// Reached on loopback alone (`loopback`), or on the tailnet too
    /// (`tailnet`), on `published_ui_port`.
    Running {
        #[ts(as = "String")]
        reach: Reach,
    },
    /// Not tried again for this commit.
    Failed {
        detail: String,
    },
}

impl ApiState {
    /// A project's previews and the commits asked for.
    pub(super) async fn project_previews(
        State(state): State<Arc<Self>>,
        Path(name): Path<String>,
    ) -> Result<Response, ApiError> {
        let project = state.named(&name).await?;
        let body = match project.factory {
            Stored::Known(Factory::Ren) => PreviewsBody {
                previews: state
                    .release
                    .previews()
                    .await?
                    .previews()
                    .iter()
                    .map(PreviewBody::of)
                    .collect(),
                requested: state.release.preview_requests().await?.commits().to_vec(),
            },
            Stored::Known(Factory::Contract) | Stored::Unknown(_) => PreviewsBody {
                previews: Vec::new(),
                requested: Vec::new(),
            },
        };
        Ok(Self::versioned(body).into_response())
    }
}

impl PreviewBody {
    fn of(preview: &Preview) -> Self {
        Self {
            slot: u64::try_from(preview.slot.index()).unwrap_or(u64::MAX),
            of: match &preview.key {
                PreviewKey::Pull(number) => PreviewOfBody::Pull {
                    number: number.get(),
                },
                PreviewKey::Commit(_) => PreviewOfBody::Commit,
            },
            commit: preview.commit.clone(),
            state: match &preview.state {
                PreviewState::Building => PreviewStateBody::Building,
                PreviewState::Running { reach } => PreviewStateBody::Running { reach: *reach },
                PreviewState::Failed { detail } => PreviewStateBody::Failed {
                    detail: detail.clone(),
                },
            },
            ui_port: preview.slot.ports().1,
            published_ui_port: preview.slot.published().1,
        }
    }
}
