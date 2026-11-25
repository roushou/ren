//! The management API: health, metrics, calls, recordings, body previews, the
//! live observation feed, and authenticated controls under `/control/v1`.
//!
//! Every JSON response carries `api_version`, which the UI checks at load.
//! The one non-JSON success is a complete body export, sent as raw bytes.
//! Query strings are parsed once into store query types; unknown parameters
//! are rejected. Read handlers use only the store's `Reader`; control handlers
//! apply commands through `Controls` and run backups through `Backup`.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

pub mod control;
pub mod health;
pub mod live;
mod params;
pub mod preview;
pub mod previews;
pub mod projects;
pub mod pulls;
pub mod read;
pub mod release;
mod route;
mod storage;

pub(crate) use control::ControlApi;
pub use control::{ControlOptions, ControlRefusal, ControlToken};
pub use release::ReleaseFolder;
pub use route::ApiRead;

use crate::{build::InstanceOptions, control::Controls, github::PullBoard, user::Users};
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    response::{IntoResponse, Response, Sse, sse::KeepAlive},
    routing::get,
};
use health::{Health, OperatorSummary, RecorderMetrics};
use http::{HeaderValue, StatusCode, Uri, header};
use live::LiveFeed;
use params::RawLiveQuery;
use preview::Previewer;
use release::ReleaseFileError;
use ren_protocol::direction::Direction;
use ren_recorder::time::Clock;
use ren_recorder::{Recorder, id::CallId, retention::RetentionPolicy};
use ren_release::Build;
use ren_store::{Reader, StoreError, backup::Backup, query::QueryError, search::SearchError};
use serde::Serialize;
use std::sync::Arc;
use storage::{CaptureLimits, Storage, StorageView};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

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
    /// What `/health` reports this instance to be.
    pub instance: InstanceOptions,
    /// Agent runs, when run roots are configured; the run routes answer
    /// `runs_disabled` without it.
    pub runs: Option<ren_supervisor::Supervisor>,
    /// The host's packages; the packages routes answer `packages_disabled` without.
    pub packages: Option<std::sync::Arc<dyn crate::packages::Packages>>,
    /// Where the factory writes the release, which `/api/release` reports.
    pub release: ReleaseFolder,
    /// What the daemon's poller last saw of each watched GitHub repository.
    pub pulls: PullBoard,
    /// The factory on the host, asked to plan the previews when one is
    /// asked for; without it, the factory's next timed pass does.
    pub factory: Option<std::sync::Arc<dyn crate::factory::Factory>>,
}

/// What the read API reads and reports, from the management setup.
pub(crate) struct ReadSources {
    pub reader: Reader,
    pub retention: RetentionPolicy,
    pub build: Build,
    pub instance: InstanceOptions,
    pub release: ReleaseFolder,
    pub pulls: PullBoard,
}

/// What the read API reads from. It never writes.
pub(crate) struct ApiState {
    reader: Reader,
    recorder: Recorder,
    controls: Arc<Controls>,
    clock: Arc<dyn Clock>,
    retention: RetentionPolicy,
    build: Build,
    instance: InstanceOptions,
    release: ReleaseFolder,
    pulls: PullBoard,
    previews: Arc<Semaphore>,
    exports: Arc<Semaphore>,
    shutdown: CancellationToken,
    /// The named users `/api/caller` identifies callers among.
    users: Users,
}

impl ApiState {
    /// One of a bounded pool's permits, or `Busy` when all are taken.
    fn permit(pool: &Arc<Semaphore>) -> Result<OwnedSemaphorePermit, ApiError> {
        pool.clone().try_acquire_owned().map_err(|_| ApiError::Busy)
    }
}

/// `/api/caller`'s answer: how the gateway identifies the request's caller.
#[derive(Serialize, TS)]
#[ts(export)]
struct CallerBody {
    caller: read::calls::CallerView,
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
    /// The request's user token or Tailscale identity was refused.
    IdentityRefused,
}

/// `/api/metrics`'s answer.
#[derive(Serialize, TS)]
#[ts(export)]
struct Metrics {
    recorder: RecorderMetrics,
    storage: StorageView,
}

/// A failed read's body: its code; a client keeps codes it does not know.
#[derive(Serialize, TS)]
#[ts(export)]
struct ErrorBody {
    #[ts(as = "String")]
    error: ErrorCode,
}

/// A failed API request.
#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Search(#[from] SearchError),
    /// Query parameters the route does not take, or values it cannot read.
    #[error("invalid parameters")]
    Parameters,
    #[error("too many concurrent reads")]
    Busy,
    #[error("identity refused")]
    IdentityRefused,
    #[error("store unavailable: {0}")]
    Store(StoreError),
    #[error(transparent)]
    Release(#[from] ReleaseFileError),
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
            Self::Query(_) | Self::Search(_) | Self::Parameters => {
                (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery)
            }
            Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Busy),
            Self::IdentityRefused => (StatusCode::UNAUTHORIZED, ErrorCode::IdentityRefused),
            Self::Store(error) => {
                tracing::warn!(%error, "management query failed");
                (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable)
            }
            Self::Release(error) => {
                tracing::warn!(%error, "release file unreadable");
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
        sources: ReadSources,
        recorder: Recorder,
        controls: Arc<Controls>,
        clock: Arc<dyn Clock>,
        shutdown: CancellationToken,
        users: Users,
    ) -> Self {
        let ReadSources {
            reader,
            retention,
            build,
            instance,
            release,
            pulls,
        } = sources;
        Self {
            reader,
            recorder,
            controls,
            clock,
            retention,
            build,
            instance,
            release,
            pulls,
            previews: Arc::new(Semaphore::new(Self::PREVIEW_PERMITS)),
            exports: Arc::new(Semaphore::new(Self::EXPORT_PERMITS)),
            shutdown,
            users,
        }
    }

    /// The API routes, with private response headers on every one.
    pub fn router(self) -> Router {
        let mut router = Router::new();
        for route in ApiRead::ROUTES {
            router = router.route(route, get(Self::read));
        }
        router
            .route("/api/metrics", get(Self::metrics))
            .route("/api/storage", get(Self::storage))
            .route("/api/caller", get(Self::caller))
            .route("/api/projects", get(Self::projects))
            .route("/api/projects/{name}", get(Self::project))
            .route("/api/projects/{name}/release", get(Self::project_release))
            .route("/api/projects/{name}/pulls", get(Self::project_pulls))
            .route("/api/projects/{name}/previews", get(Self::project_previews))
            .route(
                "/api/projects/{name}/release/shots/{shot}",
                get(Self::project_shot),
            )
            .route("/api/calls/{id}/body/{direction}", get(Self::body))
            .route("/api/calls/{id}/body/{direction}/export", get(Self::export))
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
                build: (&state.build).into(),
                instance: state.instance.into(),
                operator: {
                    let snapshot = state.controls.snapshot();
                    let now = state.clock.now().wall;
                    OperatorSummary {
                        pause: snapshot.pause().into(),
                        blocked_sessions: snapshot.blocked_sessions(now),
                        blocked_users: snapshot.blocked_users(now),
                    }
                },
                recorder: recorder.into(),
            }),
        )
            .into_response()
    }

    async fn metrics(State(state): State<Arc<Self>>) -> Result<Response, ApiError> {
        let storage = state.reader.storage().await?;
        Ok(Self::versioned(Metrics {
            recorder: state.recorder.metrics().into(),
            storage: storage.into(),
        })
        .into_response())
    }

    /// How the gateway identifies the request's caller, as it would a
    /// forwarded call: for a client checking its token or login.
    async fn caller(
        State(state): State<Arc<Self>>,
        headers: http::HeaderMap,
    ) -> Result<Response, ApiError> {
        let caller = state
            .users
            .identify(&headers)
            .map_err(|_| ApiError::IdentityRefused)?;
        Ok(Self::versioned(CallerBody {
            caller: (&caller).into(),
        })
        .into_response())
    }

    async fn storage(State(state): State<Arc<Self>>) -> Result<Response, ApiError> {
        let usage = state.reader.storage().await?;
        Ok(Self::versioned(Storage {
            policy: (&state.retention).into(),
            usage: usage.into(),
            capture: CaptureLimits {
                body_limit_bytes: state.recorder.body_limit_bytes(),
            },
        })
        .into_response())
    }

    /// Any history read: the URI says which, the reader answers.
    async fn read(State(state): State<Arc<Self>>, uri: Uri) -> Result<Response, ApiError> {
        let ApiRead(request) = ApiRead::parse(&uri)?;
        Ok(
            Self::versioned(read::ReadBody::from(state.reader.serve(&request).await?))
                .into_response(),
        )
    }

    async fn body(
        State(state): State<Arc<Self>>,
        path: Result<Path<(CallId, Direction)>, PathRejection>,
    ) -> Result<Response, ApiError> {
        let Path((id, direction)) = path.map_err(|_| ApiError::NotFound)?;
        let permit = ApiState::permit(&state.previews)?;
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

    /// A complete captured body, byte for byte as `ren inspect --body`
    /// exports it. Only bodies stored whole are exported, so one never
    /// exceeds the capture body limit; it is buffered to send its length.
    async fn export(
        State(state): State<Arc<Self>>,
        path: Result<Path<(CallId, Direction)>, PathRejection>,
    ) -> Result<Response, ApiError> {
        let Path((id, direction)) = path.map_err(|_| ApiError::NotFound)?;
        let _permit = ApiState::permit(&state.exports)?;
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

    /// `after` comes from the query, or from `Last-Event-ID` when a browser
    /// `EventSource` reconnects on its own.
    async fn live(
        State(state): State<Arc<Self>>,
        headers: http::HeaderMap,
        query: Result<Query<RawLiveQuery>, QueryRejection>,
    ) -> Result<Response, ApiError> {
        let Query(raw) = query.map_err(|_| ApiError::Parameters)?;
        let resume = headers
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let feed = LiveFeed::new(
            state.reader.clone(),
            state.recorder.subscribe(),
            raw.progress
                .unwrap_or(false)
                .then(|| state.recorder.subscribe_progress()),
            state.shutdown.clone(),
            raw.after.or(resume),
        );
        Ok(Sse::new(feed.stream())
            .keep_alive(KeepAlive::default())
            .into_response())
    }
}
