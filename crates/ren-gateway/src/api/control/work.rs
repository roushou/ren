//! `/control/v1/work`: queuing work items and moving them on.
//!
//! - `POST /control/v1/work`: queue an order (`WorkOrderBody`); `201` with
//!   the new item's id.
//! - `POST /control/v1/work/{item}/send`, `land`, `ship`, or `dismiss`:
//!   one transition of the item's state, checked against the state the
//!   store holds by the pure `WorkState::apply`; `202` with the state it
//!   moves to. A change its state does not take is `409` `work_state`.
//!
//! Each is recorded with the recorder, after everything queued before it.

use super::{
    ControlApi, ControlRefusal,
    request::{LandWorkRequest, SendWorkRequest, ShipWorkRequest},
};
use crate::api::read::work::WorkOrderBody;
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::post,
};
use http::{HeaderMap, StatusCode};
use ren_protocol::label::Stored;
use ren_recorder::{
    batch::WorkFact,
    id::WorkItemId,
    work::{WorkEvent, WorkOrder, WorkStateKind},
};
use ren_store::Reader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

pub(super) struct WorkApi {
    pub control: Arc<ControlApi>,
    pub reader: Reader,
}

/// What a work request answers: the item, and the state it is in now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WorkAnswer {
    #[ts(as = "String")]
    pub id: WorkItemId,
    #[ts(as = "String")]
    pub state: Stored<WorkStateKind>,
}

impl WorkApi {
    pub fn router(self) -> Router {
        Router::new()
            .route("/control/v1/work", post(Self::queue))
            .route("/control/v1/work/{item}/send", post(Self::send))
            .route("/control/v1/work/{item}/land", post(Self::land))
            .route("/control/v1/work/{item}/ship", post(Self::ship))
            .route("/control/v1/work/{item}/dismiss", post(Self::dismiss))
            .with_state(Arc::new(self))
    }

    async fn queue(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<WorkOrderBody>, JsonRejection>,
    ) -> Response {
        if let Err(refusal) = api.control.authorize(&headers) {
            return refusal.into_response();
        }
        let Ok(Json(body)) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        let order = match WorkOrder::from(body).checked() {
            Ok(order) => order,
            Err(error) => return ControlRefusal::InvalidOrder.explained(Some(error.to_string())),
        };
        let Some(slot) = api.control.recorder.work_slot() else {
            return ControlRefusal::Unrecorded.into_response();
        };
        let id = slot.queue(order);
        (
            StatusCode::CREATED,
            ControlApi::answer(WorkAnswer {
                id,
                state: Stored::Known(WorkStateKind::Queued),
            }),
        )
            .into_response()
    }

    async fn send(
        State(api): State<Arc<Self>>,
        Path(item): Path<String>,
        headers: HeaderMap,
        body: Result<Json<SendWorkRequest>, JsonRejection>,
    ) -> Response {
        let Ok(Json(SendWorkRequest { session })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        match api.reader.session(session).await {
            Ok(Some(_)) => {}
            Ok(None) => return ControlRefusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "session unreadable");
                return ControlRefusal::Unreadable.into_response();
            }
        }
        api.change(&headers, &item, WorkEvent::Send { session })
            .await
    }

    async fn land(
        State(api): State<Arc<Self>>,
        Path(item): Path<String>,
        headers: HeaderMap,
        body: Result<Json<LandWorkRequest>, JsonRejection>,
    ) -> Response {
        let Ok(Json(LandWorkRequest { commit })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        api.change(&headers, &item, WorkEvent::Land { commit })
            .await
    }

    async fn ship(
        State(api): State<Arc<Self>>,
        Path(item): Path<String>,
        headers: HeaderMap,
        body: Result<Json<ShipWorkRequest>, JsonRejection>,
    ) -> Response {
        let Ok(Json(ShipWorkRequest { revision })) = body else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        api.change(&headers, &item, WorkEvent::Ship { revision })
            .await
    }

    async fn dismiss(
        State(api): State<Arc<Self>>,
        Path(item): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        api.change(&headers, &item, WorkEvent::Dismiss).await
    }

    /// Applies `event` to the item's stored state, and records it.
    async fn change(&self, headers: &HeaderMap, item: &str, event: WorkEvent) -> Response {
        if let Err(refusal) = self.control.authorize(headers) {
            return refusal.into_response();
        }
        let Ok(item) = item.parse::<WorkItemId>() else {
            return ControlRefusal::NotFound.into_response();
        };
        let stored = match self.reader.work_item(item).await {
            Ok(Some(stored)) => stored,
            Ok(None) => return ControlRefusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "work item unreadable");
                return ControlRefusal::Unreadable.into_response();
            }
        };
        // A state this build does not know takes no change it could check.
        let Some(state) = stored.state() else {
            return ControlRefusal::WorkState.into_response();
        };
        let next = match state.apply(event.clone()) {
            Ok(next) => next,
            Err(error) => return ControlRefusal::WorkState.explained(Some(error.to_string())),
        };
        let fact = match event {
            WorkEvent::Send { session } => WorkFact::Sent { session },
            WorkEvent::Land { commit } => WorkFact::Landed { commit },
            WorkEvent::Ship { revision } => WorkFact::Shipped { revision },
            WorkEvent::Dismiss => WorkFact::Dismissed,
        };
        let Some(slot) = self.control.recorder.work_slot() else {
            return ControlRefusal::Unrecorded.into_response();
        };
        slot.record(item, fact);
        (
            StatusCode::ACCEPTED,
            ControlApi::answer(WorkAnswer {
                id: item,
                state: Stored::Known(next.kind()),
            }),
        )
            .into_response()
    }
}
