//! `/control/v1/history`: archiving, unarchiving, and deleting what was
//! recorded: a recording or a run.
//!
//! - `POST /control/v1/history/recordings/{recording}/{change}`
//! - `POST /control/v1/history/runs/{run}/{change}`
//!
//! `change` is `archive`, `unarchive`, or `delete`. An edit is recorded with
//! who asked and applied by the recorder, after everything queued before it;
//! the answer (`202`) says it was accepted. A delete is refused while the
//! subject's calls are open or its runs still running, since the recorder
//! keeps both.

use super::{ControlApi, ControlRefusal};
use axum::{
    Json, Router,
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::post,
};
use http::{HeaderMap, StatusCode};
use ren_recorder::batch::{HistoryChange, HistorySubject};
use ren_store::Reader;
use std::sync::Arc;

pub(super) struct HistoryApi {
    pub control: Arc<ControlApi>,
    pub reader: Reader,
}

impl HistoryApi {
    pub fn router(self) -> Router {
        Router::new()
            .route(
                "/control/v1/history/recordings/{recording}/{change}",
                post(Self::recording),
            )
            .route("/control/v1/history/runs/{run}/{change}", post(Self::run))
            .with_state(Arc::new(self))
    }

    async fn recording(
        State(api): State<Arc<Self>>,
        Path((recording, change)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Response {
        let subject = recording.parse().map(HistorySubject::Recording);
        api.edit(&headers, subject.ok(), &change).await
    }

    async fn run(
        State(api): State<Arc<Self>>,
        Path((run, change)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Response {
        let subject = run.parse().map(HistorySubject::Run);
        api.edit(&headers, subject.ok(), &change).await
    }

    async fn edit(
        &self,
        headers: &HeaderMap,
        subject: Option<HistorySubject>,
        change: &str,
    ) -> Response {
        let actor = match self.control.authorize(headers) {
            Ok(actor) => actor,
            Err(refusal) => return refusal.into_response(),
        };
        let (Some(subject), Ok(change)) = (subject, change.parse::<HistoryChange>()) else {
            return ControlRefusal::NotFound.into_response();
        };
        let target = match self.reader.history_target(subject).await {
            Ok(Some(target)) => target,
            Ok(None) => return ControlRefusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "history target unreadable");
                return ControlRefusal::Unreadable.into_response();
            }
        };
        match change {
            HistoryChange::Delete if target.running_runs > 0 => {
                return ControlRefusal::RunRunning.into_response();
            }
            HistoryChange::Delete if target.open_calls > 0 => {
                return ControlRefusal::CallOpen.into_response();
            }
            HistoryChange::Archive | HistoryChange::Unarchive | HistoryChange::Delete => {}
        }
        let Some(slot) = self.control.recorder.history_slot() else {
            return ControlRefusal::Unrecorded.into_response();
        };
        slot.record(actor, subject, change);
        (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "api_version": ControlApi::API_VERSION,
                "subject": subject,
                "change": change,
            })),
        )
            .into_response()
    }
}
