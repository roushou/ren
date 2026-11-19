//! `POST /control/v1/release/promote`: the operator asks that production
//! take the canary's build. The gateway records the request
//! (`ren_release::PromotionRequest`); the factory's next pass carries it out
//! with the install's steps, since an install restarts the daemon that
//! would otherwise run it. Refused, `409` `not_promotable`, unless the commit
//! is the canary's and its build passed.

use super::{ControlApi, ControlRefusal, request::PromoteRequest};
use crate::api::release::ReleaseFolder;
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::post,
};
use http::{HeaderMap, StatusCode};
use ren_release::PromotionRequest;
use std::sync::Arc;

pub(super) struct ReleaseApi {
    pub control: Arc<ControlApi>,
    pub folder: ReleaseFolder,
}

impl ReleaseApi {
    pub fn router(self) -> Router {
        Router::new()
            .route("/control/v1/release/promote", post(Self::promote))
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
}
