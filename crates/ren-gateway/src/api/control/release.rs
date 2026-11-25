//! `POST /control/v1/release/promote`: the operator asks that production
//! take the canary's build. The gateway records the request
//! (`crate::release::PromotionRequest`); the deployer carries it out with the
//! install's steps, since an install restarts the daemon that would
//! otherwise run it: at once with a [`Deployer`] on the host, which is asked
//! to ([`DeployerRequest::Promote`]), else at its next timed pass. Refused,
//! `409` `not_promotable`, unless the commit is the canary's and its build
//! passed.
//!
//! `POST /control/v1/previews/ask` and `/previews/withdraw`: the operator
//! asks for a preview of a commit, or withdraws the ask
//! (`crate::deployment::ChosenCommits`). With a [`Deployer`] on the host, either
//! is planned at once: the answer comes once the deployer has given the
//! commit its slot or taken it back (or after a few seconds, while it is
//! busy); without one, the deployer's next timed pass starts or stops it. Both answer the
//! commits asked for, the newest first; a withdrawal of a commit not asked
//! for is `404` `not_found`.

use super::{
    ControlApi, ControlRefusal,
    request::{PreviewAskRequest, PreviewWithdrawRequest, PromoteRequest},
};
use crate::api::release::ReleaseFolder;
use crate::deployer::{Deployer, DeployerAnswer, DeployerRequest};
use crate::deployment::{Chosen, ChosenCommits, Seed};
use crate::release::PromotionRequest;
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::post,
};
use http::{HeaderMap, StatusCode};
use std::sync::Arc;

pub(super) struct ReleaseApi {
    pub control: Arc<ControlApi>,
    pub folder: ReleaseFolder,
    pub deployer: Option<Arc<dyn Deployer>>,
}

impl ReleaseApi {
    /// How long an ask waits for the deployer to take it up: a pass already
    /// building takes it only once it is done.
    const PLAN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    pub fn router(self) -> Router {
        Router::new()
            .route("/control/v1/release/promote", post(Self::promote))
            .route("/control/v1/previews/ask", post(Self::ask_preview))
            .route(
                "/control/v1/previews/withdraw",
                post(Self::withdraw_preview),
            )
            .with_state(Arc::new(self))
    }

    async fn promote(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<PromoteRequest>, JsonRejection>,
    ) -> Response {
        if let Err(refusal) = api.control.authorize(&headers) {
            return refusal.into_response();
        }
        let Ok(Json(PromoteRequest { commit })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        let release = match api.folder.read().await {
            Ok(Some(release)) => release,
            Ok(None) => return ControlRefusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "the release does not read");
                return ControlRefusal::Unreadable.into_response();
            }
        };
        let checked = release
            .canary()
            .is_some_and(|canary| *canary.commit() == commit && canary.verdict().passed());
        if !checked {
            return ControlRefusal::NotPromotable.into_response();
        }
        let request = PromotionRequest { commit };
        if let Err(error) = api.folder.request(&request).await {
            tracing::warn!(%error, "the promotion request was not recorded");
            return ControlRefusal::Unavailable.into_response();
        }
        if let Some(deployer) = &api.deployer {
            api.ask(deployer.as_ref(), DeployerRequest::Promote).await;
        }
        (StatusCode::ACCEPTED, ControlApi::answer(request)).into_response()
    }

    async fn ask_preview(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<PreviewAskRequest>, JsonRejection>,
    ) -> Response {
        let Ok(Json(PreviewAskRequest { commit, seed, work })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        let chosen = Chosen {
            commit,
            seed: seed.map_or(Seed::Copy, Seed::Invented),
            work,
        };
        api.change_asks(&headers, |requests| {
            requests.choose(chosen);
            true
        })
        .await
    }

    /// Asks the deployer `request` now; what it answers is for the log
    /// alone: the request is recorded either way, and a later pass takes it
    /// up.
    async fn ask(&self, deployer: &dyn Deployer, request: DeployerRequest) {
        match tokio::time::timeout(Self::PLAN_WAIT, deployer.ask(request)).await {
            Ok(Ok(DeployerAnswer::Planned | DeployerAnswer::Started)) => {}
            Ok(Ok(DeployerAnswer::NotBuilt)) => {
                tracing::warn!("the deployer does not build what was asked of it");
            }
            Ok(Ok(DeployerAnswer::Failed(why))) => {
                tracing::warn!(%why, "the deployer could not take up the request");
            }
            Ok(Err(error)) => tracing::warn!(%error, "the deployer was not asked"),
            // Busy with a pass: the request waits for the next.
            Err(_) => {}
        }
    }

    async fn withdraw_preview(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<PreviewWithdrawRequest>, JsonRejection>,
    ) -> Response {
        let Ok(Json(PreviewWithdrawRequest { commit })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        api.change_asks(&headers, |requests| requests.withdraw(&commit))
            .await
    }

    /// Changes the preview requests with `change`, which says whether the
    /// commit was one it could change; asks the deployer to plan the change
    /// now, then answers the requests.
    async fn change_asks(
        &self,
        headers: &HeaderMap,
        change: impl FnOnce(&mut ChosenCommits) -> bool,
    ) -> Response {
        if let Err(refusal) = self.control.authorize(headers) {
            return refusal.into_response();
        }
        match self.folder.change_preview_requests(change).await {
            Ok((true, requests)) => {
                if let Some(deployer) = &self.deployer {
                    self.ask(deployer.as_ref(), DeployerRequest::Previews).await;
                }
                (StatusCode::ACCEPTED, ControlApi::answer(requests)).into_response()
            }
            Ok((false, _)) => ControlRefusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "the preview requests were not recorded");
                ControlRefusal::Unavailable.into_response()
            }
        }
    }
}
