//! `/control/v1/history`: archiving, unarchiving, and deleting what was
//! recorded of a session or a run.
//!
//! - `POST /control/v1/history/sessions/{session}/{change}`
//! - `POST /control/v1/history/runs/{run}/{change}`
//!
//! `change` is `archive`, `unarchive`, or `delete`. An edit is recorded with
//! who asked and applied by the recorder, after everything queued before it;
//! the answer (`202`) says it was accepted. A delete is refused while the
//! subject's calls are open or its runs still running, since the recorder
//! keeps both.

use super::{ControlApi, Refusal};
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

/// Why an edit was refused, beyond access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unedited {
    /// A call of the subject is still open.
    CallOpen,
    /// A run of the subject is still running.
    RunRunning,
    /// What the edit would touch could not be read.
    Unreadable,
}

impl IntoResponse for Unedited {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::CallOpen => (StatusCode::CONFLICT, "call_open"),
            Self::RunRunning => (StatusCode::CONFLICT, "run_running"),
            Self::Unreadable => (StatusCode::INTERNAL_SERVER_ERROR, "unreadable"),
        };
        (
            status,
            Json(serde_json::json!({ "api_version": ControlApi::API_VERSION, "error": code })),
        )
            .into_response()
    }
}

impl HistoryApi {
    pub fn router(self) -> Router {
        Router::new()
            .route(
                "/control/v1/history/sessions/{session}/{change}",
                post(Self::session),
            )
            .route("/control/v1/history/runs/{run}/{change}", post(Self::run))
            .with_state(Arc::new(self))
    }

    async fn session(
        State(api): State<Arc<Self>>,
        Path((session, change)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Response {
        let subject = session.parse().map(HistorySubject::Session);
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
            return Refusal::NotFound.into_response();
        };
        let target = match self.reader.history_target(subject).await {
            Ok(Some(target)) => target,
            Ok(None) => return Refusal::NotFound.into_response(),
            Err(error) => {
                tracing::warn!(%error, "history target unreadable");
                return Unedited::Unreadable.into_response();
            }
        };
        match change {
            HistoryChange::Delete if target.running_runs > 0 => {
                return Unedited::RunRunning.into_response();
            }
            HistoryChange::Delete if target.open_calls > 0 => {
                return Unedited::CallOpen.into_response();
            }
            HistoryChange::Archive | HistoryChange::Unarchive | HistoryChange::Delete => {}
        }
        let Some(slot) = self.control.recorder.history_slot() else {
            return Refusal::Control(crate::control::ControlError::Unrecorded).into_response();
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
