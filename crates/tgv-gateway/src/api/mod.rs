//! The management API: health, metrics, calls, sessions, body previews, the
//! live observation feed, and authenticated controls under `/control/v1`.
//!
//! Every JSON response carries `api_version`, which the UI checks at load.
//! The one non-JSON success is a complete body export, sent as raw bytes.
//! Query strings are parsed once into store query types; unknown parameters
//! are rejected. Read handlers use only the store's `Reader`; control handlers
//! apply commands through `Controls` and run backups through `Backup`.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

mod control;
mod live;
mod params;
pub mod preview;

pub(crate) use control::ControlApi;
pub use control::{ControlOptions, ControlOptionsError, ControlToken};

use crate::build::Build;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::{IntoResponse, Response, Sse, sse::KeepAlive},
    routing::get,
};
use http::{HeaderValue, StatusCode, header};
use live::LiveFeed;
use params::{RawCallQuery, RawLiveQuery, RawPageQuery, RawUsageQuery};
use preview::Previewer;
use serde::Serialize;
use std::sync::Arc;
use tgv_protocol::direction::Direction;
use tgv_recorder::{
    Recorder, RecorderMetrics,
    id::{CallId, SessionId},
    retention::RetentionPolicy,
};
use tgv_store::{
    Reader, StoreError,
    backup::Backup,
    query::{CallQuery, QueryError, TimeRange},
    view::{SessionView, StorageView},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Resources and settings for the gateway's management and control routes.
/// The host opens storage and supplies the token; the gateway owns HTTP handling.
pub struct ManagementSetup {
    pub reader: Reader,
    pub backup: Backup,
    pub retention: RetentionPolicy,
    pub control: ControlOptions,
    pub token: ControlToken,
    /// What `/health` reports as the running build.
    pub build: Build,
}

/// What the read API reads from. It never writes.
pub(crate) struct ApiState {
    reader: Reader,
    recorder: Recorder,
    retention: RetentionPolicy,
    build: Build,
    previews: Arc<Semaphore>,
    exports: Arc<Semaphore>,
    shutdown: CancellationToken,
}

/// A response body tagged with the API version.
#[derive(Serialize)]
struct Versioned<T> {
    api_version: u32,
    #[serde(flatten)]
    body: T,
}

/// Whether persistence is working.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum HealthStatus {
    Ok,
    Degraded,
}

/// Machine-readable API error codes.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ErrorCode {
    NotFound,
    InvalidQuery,
    Busy,
    Unavailable,
}

#[derive(Serialize)]
struct Health<'a> {
    status: HealthStatus,
    build: &'a Build,
    recorder: RecorderMetrics,
}

#[derive(Serialize)]
struct Metrics {
    recorder: RecorderMetrics,
    storage: StorageView,
}

#[derive(Serialize)]
struct Storage<'a> {
    policy: &'a RetentionPolicy,
    usage: StorageView,
}

#[derive(Serialize)]
struct Sessions {
    sessions: Vec<SessionView>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorCode,
}

/// A failed API request.
#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error("too many concurrent reads")]
    Busy,
    #[error("store unavailable: {0}")]
    Store(StoreError),
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound | StoreError::IncompleteBody => Self::NotFound,
            StoreError::Io(_)
            | StoreError::Locked
            | StoreError::Database(_)
            | StoreError::Migration(_)
            | StoreError::Json(_)
            | StoreError::Worker(_)
            | StoreError::NonUtf8Path
            | StoreError::Corrupt(_) => Self::Store(error),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = match &self {
            Self::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            Self::Query(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery),
            Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Busy),
            Self::Store(error) => {
                tracing::warn!(%error, "management query failed");
                (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable)
            }
        };
        (status, ApiState::versioned(ErrorBody { error })).into_response()
    }
}

impl ApiState {
    /// Clients reject responses whose version they do not understand.
    pub const API_VERSION: u32 = 1;
    /// Body previews decode off the async runtime; at most this many at once.
    const PREVIEW_PERMITS: usize = 2;
    /// A body export is held in memory whole; at most this many at once.
    const EXPORT_PERMITS: usize = 2;

    pub fn new(
        reader: Reader,
        recorder: Recorder,
        retention: RetentionPolicy,
        build: Build,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            reader,
            recorder,
            retention,
            build,
            previews: Arc::new(Semaphore::new(Self::PREVIEW_PERMITS)),
            exports: Arc::new(Semaphore::new(Self::EXPORT_PERMITS)),
            shutdown,
        }
    }

    /// The API routes, with private response headers on every one.
    pub fn router(self) -> Router {
        Router::new()
            .route("/api/metrics", get(Self::metrics))
            .route("/api/storage", get(Self::storage))
            .route("/api/calls", get(Self::calls))
            .route("/api/calls/{id}", get(Self::call))
            .route("/api/calls/{id}/body/{direction}", get(Self::body))
            .route("/api/calls/{id}/body/{direction}/export", get(Self::export))
            .route("/api/sessions", get(Self::sessions))
            .route("/api/usage", get(Self::usage))
            .route("/api/sessions/{id}/calls", get(Self::session_calls))
            .route("/api/live", get(Self::live))
            .route("/health", get(Self::health))
            .layer(axum::middleware::map_response(Self::private))
            .with_state(Arc::new(self))
    }

    fn versioned<T: Serialize>(body: T) -> Json<Versioned<T>> {
        Json(Versioned {
            api_version: Self::API_VERSION,
            body,
        })
    }

    async fn private(mut response: Response) -> Response {
        let headers = response.headers_mut();
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }

    async fn health(State(state): State<Arc<Self>>) -> Response {
        let recorder = state.recorder.metrics();
        let (code, status) = if recorder.healthy() {
            (StatusCode::OK, HealthStatus::Ok)
        } else {
            (StatusCode::SERVICE_UNAVAILABLE, HealthStatus::Degraded)
        };
        (
            code,
            Self::versioned(Health {
                status,
                build: &state.build,
                recorder,
            }),
        )
            .into_response()
    }

    async fn metrics(State(state): State<Arc<Self>>) -> Result<Response, ApiError> {
        let storage = state.reader.storage().await?;
        Ok(Self::versioned(Metrics {
            recorder: state.recorder.metrics(),
            storage,
        })
        .into_response())
    }

    async fn storage(State(state): State<Arc<Self>>) -> Result<Response, ApiError> {
        let usage = state.reader.storage().await?;
        Ok(Self::versioned(Storage {
            policy: &state.retention,
            usage,
        })
        .into_response())
    }

    async fn calls(
        State(state): State<Arc<Self>>,
        Query(raw): Query<RawCallQuery>,
    ) -> Result<Response, ApiError> {
        let query = CallQuery::try_from(raw)?;
        Ok(Self::versioned(state.reader.calls(&query).await?).into_response())
    }

    async fn call(
        State(state): State<Arc<Self>>,
        Path(id): Path<CallId>,
    ) -> Result<Response, ApiError> {
        Ok(Self::versioned(state.reader.call(id).await?).into_response())
    }

    async fn body(
        State(state): State<Arc<Self>>,
        Path((id, direction)): Path<(CallId, Direction)>,
    ) -> Result<Response, ApiError> {
        let permit = state
            .previews
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::Busy)?;
        let previewer = Previewer::default();
        let key = state.reader.body_key(id, direction).await?;
        let prefix = state
            .reader
            .body_prefix(key, previewer.read_limit())
            .await?;
        // The permit moves into the task, so it is held even if the client
        // disconnects while decoding runs.
        let preview = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            previewer.preview(prefix)
        })
        .await
        .map_err(|error| ApiError::Store(StoreError::Worker(error)))?;
        Ok(Self::versioned(preview).into_response())
    }

    /// A complete captured body, byte for byte as `tgv inspect --body`
    /// exports it. Only bodies stored whole are exported, so one never
    /// exceeds the capture body limit; it is buffered to send its length.
    async fn export(
        State(state): State<Arc<Self>>,
        Path((id, direction)): Path<(CallId, Direction)>,
    ) -> Result<Response, ApiError> {
        let _permit = state
            .exports
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::Busy)?;
        let key = state.reader.body_key(id, direction).await?;
        let mut body = Vec::new();
        state
            .reader
            .export(key, |bytes| {
                body.extend_from_slice(bytes);
                Ok(())
            })
            .await?;
        Ok((
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            )],
            body,
        )
            .into_response())
    }

    async fn usage(
        State(state): State<Arc<Self>>,
        Query(raw): Query<RawUsageQuery>,
    ) -> Result<Response, ApiError> {
        let range = TimeRange::try_from(raw)?;
        Ok(Self::versioned(state.reader.usage(range).await?).into_response())
    }

    async fn sessions(
        State(state): State<Arc<Self>>,
        Query(raw): Query<RawPageQuery>,
    ) -> Result<Response, ApiError> {
        let sessions = state.reader.sessions(raw.limit()?).await?;
        Ok(Self::versioned(Sessions { sessions }).into_response())
    }

    async fn session_calls(
        State(state): State<Arc<Self>>,
        Path(id): Path<SessionId>,
        Query(raw): Query<RawPageQuery>,
    ) -> Result<Response, ApiError> {
        let page = state
            .reader
            .session_calls(id, raw.limit()?, raw.after()?)
            .await?;
        Ok(Self::versioned(page).into_response())
    }

    /// `after` comes from the query, or from `Last-Event-ID` when a browser
    /// `EventSource` reconnects on its own.
    async fn live(
        State(state): State<Arc<Self>>,
        headers: http::HeaderMap,
        Query(raw): Query<RawLiveQuery>,
    ) -> Response {
        let resume = headers
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let feed = LiveFeed::new(
            state.reader.clone(),
            state.recorder.subscribe(),
            state.shutdown.clone(),
            raw.after.or(resume),
        );
        Sse::new(feed.stream())
            .keep_alive(KeepAlive::default())
            .into_response()
    }
}
