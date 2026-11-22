//! `/control/v1/packages`: the packages Ren installs on the daemon's host for
//! its agents, through the daemon's [`Packages`].
//!
//! - `GET /control/v1/packages`: every package, what the host has of it, and
//!   the generations kept.
//! - `POST /control/v1/packages/change` `{"add": [...], "remove": [...]}`:
//!   install what is added and make the next generation current, only if
//!   every install succeeds. Answers when done.
//! - `POST /control/v1/packages/rollback` `{"generation": n}`: make the next
//!   generation hold what a kept generation held, from the store.
//! - `POST /control/v1/packages/gc`: remove generations and installs nothing
//!   current or running uses.
//!
//! Access is the control API's. Without a packages every route answers
//! `packages_disabled`.

use super::body::packages as body;
use super::{ControlApi, ControlRefusal};
use crate::packages::{Packages, PackagesError};
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use http::HeaderMap;
use ren_depot::{
    package::PackageId,
    package_set::{Edit, Generation},
};
use serde::Serialize;
use std::{collections::BTreeSet, sync::Arc};
use ts_rs::TS;

pub(super) struct PackagesApi {
    pub control: Arc<ControlApi>,
    pub packages: Option<Arc<dyn Packages>>,
}

/// Packages to add to and remove from the next generation.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
struct ChangeRequest {
    #[serde(default)]
    #[ts(as = "Vec<String>")]
    add: BTreeSet<PackageId>,
    #[serde(default)]
    #[ts(as = "Vec<String>")]
    remove: BTreeSet<PackageId>,
}

/// The generation to make current again.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
struct RollbackRequest {
    #[ts(as = "u64")]
    generation: Generation,
}

/// Why a packages request was refused, beyond access.
#[derive(Debug)]
enum PackagesRefusal {
    Access(ControlRefusal),
    Disabled,
    Packages(PackagesError),
}

impl IntoResponse for PackagesRefusal {
    fn into_response(self) -> Response {
        let (refusal, message) = match self {
            Self::Access(refusal) => return refusal.into_response(),
            Self::Disabled => (ControlRefusal::PackagesDisabled, None),
            Self::Packages(PackagesError::Busy) => (ControlRefusal::PackagesBusy, None),
            Self::Packages(error @ PackagesError::NoGeneration(_)) => (
                ControlRefusal::PackagesNoGeneration,
                Some(error.to_string()),
            ),
            Self::Packages(error @ PackagesError::Collected(_)) => {
                (ControlRefusal::PackagesCollected, Some(error.to_string()))
            }
            Self::Packages(PackagesError::Unsupported) => {
                (ControlRefusal::PackagesUnsupported, None)
            }
            Self::Packages(error @ PackagesError::Refused(_)) => {
                (ControlRefusal::PackagesRefused, Some(error.to_string()))
            }
            Self::Packages(PackagesError::Failed(reason)) => {
                tracing::warn!(%reason, "the package store failed");
                (ControlRefusal::PackagesFailed, None)
            }
        };
        refusal.explained(message)
    }
}

impl PackagesApi {
    pub fn router(self) -> Router {
        Router::new()
            .route("/control/v1/packages", get(Self::listing))
            .route("/control/v1/packages/change", post(Self::change))
            .route("/control/v1/packages/rollback", post(Self::roll_back))
            .route("/control/v1/packages/gc", post(Self::collect))
            .with_state(Arc::new(self))
    }

    fn packages(&self, headers: &HeaderMap) -> Result<Arc<dyn Packages>, PackagesRefusal> {
        self.control
            .authorize(headers)
            .map_err(PackagesRefusal::Access)?;
        self.packages.clone().ok_or(PackagesRefusal::Disabled)
    }

    /// The packages's answer as its wire type `W`, or why it failed.
    fn answer<T, W: From<T> + Serialize>(result: Result<T, PackagesError>) -> Response {
        match result {
            Ok(body) => ControlApi::answer(W::from(body)),
            Err(error) => PackagesRefusal::Packages(error).into_response(),
        }
    }

    async fn listing(State(api): State<Arc<Self>>, headers: HeaderMap) -> Response {
        match api.packages(&headers) {
            Ok(packages) => Self::answer::<_, body::PackageListing>(packages.listing().await),
            Err(refused) => refused.into_response(),
        }
    }

    async fn change(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<ChangeRequest>, JsonRejection>,
    ) -> Response {
        let packages = match api.packages(&headers) {
            Ok(packages) => packages,
            Err(refused) => return refused.into_response(),
        };
        let Ok(Json(request)) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        let edit = Edit {
            add: request.add,
            remove: request.remove,
        };
        Self::answer::<_, body::PackageSetChange>(packages.change(edit).await)
    }

    async fn roll_back(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<RollbackRequest>, JsonRejection>,
    ) -> Response {
        let packages = match api.packages(&headers) {
            Ok(packages) => packages,
            Err(refused) => return refused.into_response(),
        };
        let Ok(Json(request)) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        Self::answer::<_, body::PackageSetChange>(packages.roll_back(request.generation).await)
    }

    async fn collect(State(api): State<Arc<Self>>, headers: HeaderMap) -> Response {
        match api.packages(&headers) {
            Ok(packages) => Self::answer::<_, body::PackageCollection>(packages.collect().await),
            Err(refused) => refused.into_response(),
        }
    }
}
