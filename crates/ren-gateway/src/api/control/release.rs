//! `POST /control/v1/release/promote`: the operator asks that production
//! take the canary's build. The gateway records the request
//! (`ren_release::PromotionRequest`); the factory's next pass carries it out
//! with the install's steps, since an install restarts the daemon that
//! would otherwise run it. Refused, `409` `not_promotable`, unless the commit
//! is the canary's and its build passed.
//!
//! `POST /control/v1/previews/ask` and `/previews/withdraw`: the operator
//! asks for a preview of a commit, or withdraws the ask
//! (`crate::preview::PreviewRequests`); the factory's next pass starts or
//! stops it. Both answer the commits asked for, the newest first; a
//! withdrawal of a commit not asked for is `404` `not_found`.

use super::{
    ControlApi, ControlRefusal,
    request::{PreviewAskRequest, PromoteRequest},
};
use crate::api::release::ReleaseFolder;
use crate::preview::PreviewRequests;
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::post,
};
use http::{HeaderMap, StatusCode};
use ren_release::{Commit, PromotionRequest};
use std::sync::Arc;

pub(super) struct ReleaseApi {
    pub control: Arc<ControlApi>,
    pub folder: ReleaseFolder,
}

impl ReleaseApi {
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
        (StatusCode::ACCEPTED, ControlApi::answer(request)).into_response()
    }

    async fn ask_preview(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<PreviewAskRequest>, JsonRejection>,
    ) -> Response {
        api.change_asks(&headers, body, |requests, commit| {
            requests.ask(commit);
            true
        })
        .await
    }

    async fn withdraw_preview(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<PreviewAskRequest>, JsonRejection>,
    ) -> Response {
        api.change_asks(&headers, body, |requests, commit| {
            requests.withdraw(&commit)
        })
        .await
    }

    /// Changes the preview requests with `change`, which says whether the
    /// commit was one it could change; answers the requests after.
    async fn change_asks(
        &self,
        headers: &HeaderMap,
        body: Result<Json<PreviewAskRequest>, JsonRejection>,
        change: impl FnOnce(&mut PreviewRequests, Commit) -> bool,
    ) -> Response {
        if let Err(refusal) = self.control.authorize(headers) {
            return refusal.into_response();
        }
        let Ok(Json(PreviewAskRequest { commit })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        match self
            .folder
            .change_preview_requests(|requests| change(requests, commit))
            .await
        {
            Ok((true, requests)) => {
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
