//! The control API, under `/control/v1`: the only mutating endpoints.
//!
//! Access:
//! - a request carrying `Origin` or `Sec-Fetch-Site` came from a browser and
//!   is refused, so a web page cannot drive the API (CSRF);
//! - a request with `Tailscale-User-Login` came through Tailscale Serve, which
//!   strips client-supplied identity headers; its login must be listed in
//!   `control.tailscale_logins`;
//! - any other request must carry the daemon's control token as a bearer
//!   token. The token is written to the data directory (mode 0600) at start;
//!   the CLI reads it there. Loopback alone is not trusted, because Serve
//!   forwards tagged devices and Funnel traffic to loopback without identity.

pub mod body;
mod history;
pub mod projects;
mod refusal;
mod release;
pub mod request;
mod runs;
mod sessions;
mod sign_in;
mod tools;
pub mod work;

use crate::api::Versioned;
use crate::control::Controls;
use crate::operator::{OperatorCommand, OperatorState, RetryAfter};
use crate::user::Users;
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use history::HistoryApi;
use http::{HeaderMap, HeaderValue, StatusCode, header};
pub use refusal::ControlRefusal;
use ren_protocol::native::NativeSessionId;
use ren_recorder::{
    Recorder,
    batch::Actor,
    caller::{TailscaleLogin, UserId},
    id::{CallId, IdSource},
    time::Clock,
};
use ren_store::backup::{Backup, BackupError};
use ren_supervisor::Supervisor;
use request::{BlockRequest, CancelRequest, PauseRequest};
use runs::RunsApi;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// The `[control]` configuration section.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ControlOptions {
    /// Tailscale logins (as Tailscale Serve reports them in
    /// `Tailscale-User-Login`) allowed to use the control API. Empty means
    /// the control API answers loopback requests only.
    pub tailscale_logins: Vec<TailscaleLogin>,
}

/// A per-run secret that authorizes control requests without a Tailscale
/// identity. Never shown in `Debug`.
#[derive(Clone)]
pub struct ControlToken(Box<str>);

impl ControlToken {
    pub const FILE: &'static str = "control.token";

    pub fn generate<S: IdSource + ?Sized>(ids: &S) -> Self {
        Self(format!("{}{}", ids.next_uuid().simple(), ids.next_uuid().simple()).into())
    }

    /// A token read back from the token file.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .then(|| Self(text.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Constant-time comparison.
    fn matches(&self, candidate: &[u8]) -> bool {
        let expected = self.0.as_bytes();
        expected.len() == candidate.len()
            && expected
                .iter()
                .zip(candidate)
                .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }
}

impl std::fmt::Debug for ControlToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ControlToken(<redacted>)")
    }
}

/// The control API's state and capabilities.
pub(crate) struct ControlApi {
    controls: Arc<Controls>,
    recorder: Recorder,
    clock: Arc<dyn Clock>,
    token: ControlToken,
    logins: Vec<TailscaleLogin>,
    /// The named users an operator can block.
    users: Users,
    backup: Backup,
}

impl ControlApi {
    pub const API_VERSION: u32 = 1;
    /// The longest session block a request may set.
    const MAX_BLOCK: Duration = Duration::from_secs(7 * 24 * 3600);

    pub fn new(
        controls: Arc<Controls>,
        recorder: Recorder,
        clock: Arc<dyn Clock>,
        token: ControlToken,
        logins: Vec<TailscaleLogin>,
        users: Users,
        backup: Backup,
    ) -> Self {
        Self {
            controls,
            recorder,
            clock,
            token,
            logins,
            users,
            backup,
        }
    }

    /// The control routes, with the run routes when `supervisor` is given.
    /// `reader` tells what a history edit would touch; `shutdown` ends run
    /// update feeds so the server can drain.
    pub fn router(
        self,
        reader: ren_store::Reader,
        supervisor: Option<Supervisor>,
        toolbox: Option<Arc<dyn crate::tools::Toolbox>>,
        release: crate::api::release::ReleaseFolder,
        shutdown: CancellationToken,
    ) -> Router {
        let control = Arc::new(self);
        let session_routes = sessions::SessionsApi {
            control: control.clone(),
            supervisor: supervisor.clone(),
            reader: reader.clone(),
            shutdown: shutdown.clone(),
        }
        .router();
        let runs = RunsApi {
            control: control.clone(),
            supervisor: supervisor.clone(),
            shutdown,
        };
        let run_routes = runs.router();
        let history_routes = HistoryApi {
            control: control.clone(),
            reader: reader.clone(),
        }
        .router();
        let work_routes = work::WorkApi {
            control: control.clone(),
            reader: reader.clone(),
        }
        .router();
        let project_routes = projects::ProjectsApi {
            control: control.clone(),
            reader,
            supervisor: supervisor.clone(),
        }
        .router();
        let release_routes = release::ReleaseApi {
            control: control.clone(),
            folder: release,
        }
        .router();
        let tools_routes = tools::ToolsApi {
            control: control.clone(),
            toolbox,
        }
        .router();
        Router::new()
            .route("/control/v1/state", get(Self::state))
            .route("/control/v1/pause", post(Self::pause))
            .route("/control/v1/resume", post(Self::resume))
            .route(
                "/control/v1/client-sessions/{session}/block",
                post(Self::block),
            )
            .route(
                "/control/v1/client-sessions/{session}/unblock",
                post(Self::unblock),
            )
            .route("/control/v1/users/{user}/block", post(Self::block_user))
            .route("/control/v1/users/{user}/unblock", post(Self::unblock_user))
            .route("/control/v1/calls/{call}/cancel", post(Self::cancel))
            .route("/control/v1/backup", post(Self::backup))
            .with_state(control)
            .merge(run_routes)
            .merge(tools_routes)
            .merge(session_routes)
            .merge(history_routes)
            .merge(work_routes)
            .merge(project_routes)
            .merge(release_routes)
            .layer(axum::middleware::map_response(Self::private))
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

    fn authorize(&self, headers: &HeaderMap) -> Result<Actor, ControlRefusal> {
        if headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site") {
            return Err(ControlRefusal::CrossSite);
        }
        if let Some(login) = headers.get(Users::LOGIN_HEADER) {
            let login = login
                .to_str()
                .ok()
                .and_then(|login| login.parse::<TailscaleLogin>().ok())
                .ok_or(ControlRefusal::Forbidden)?;
            return if self.logins.contains(&login) {
                Ok(Actor::Tailscale(login))
            } else {
                Err(ControlRefusal::Forbidden)
            };
        }
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "))
            .ok_or(ControlRefusal::Unauthorized)?;
        if self.token.matches(bearer) {
            Ok(Actor::Loopback)
        } else {
            Err(ControlRefusal::Unauthorized)
        }
    }

    fn respond(&self, state: &OperatorState) -> Response {
        Self::answer(body::operator::StateBody {
            state: state.into(),
            running: self
                .controls
                .running()
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// A body, tagged with the control API's version.
    pub(super) fn answer<T: serde::Serialize>(body: T) -> Response {
        Json(Versioned {
            api_version: Self::API_VERSION,
            body,
        })
        .into_response()
    }

    fn command(&self, actor: Actor, command: OperatorCommand) -> Response {
        match self.controls.apply(&self.recorder, actor, command) {
            Ok(state) => self.respond(&state),
            Err(error) => ControlRefusal::from(error).into_response(),
        }
    }

    fn session(text: &str) -> Result<NativeSessionId, ControlRefusal> {
        Uuid::try_parse(text)
            .ok()
            .and_then(NativeSessionId::new)
            .ok_or(ControlRefusal::NotFound)
    }

    /// A configured user; any other name is not found.
    fn user(&self, text: &str) -> Result<UserId, ControlRefusal> {
        text.parse::<UserId>()
            .ok()
            .filter(|user| self.users.knows(user))
            .ok_or(ControlRefusal::NotFound)
    }

    fn duration(seconds: Option<u64>) -> Result<Option<Duration>, ControlRefusal> {
        seconds
            .map(Duration::from_secs)
            .map(|duration| {
                if duration.is_zero() || duration > Self::MAX_BLOCK {
                    Err(ControlRefusal::InvalidRequest)
                } else {
                    Ok(duration)
                }
            })
            .transpose()
    }

    async fn state(State(api): State<Arc<Self>>, headers: HeaderMap) -> Response {
        match api.authorize(&headers) {
            Ok(_) => api.respond(&api.controls.snapshot()),
            Err(refusal) => refusal.into_response(),
        }
    }

    async fn pause(
        State(api): State<Arc<Self>>,
        headers: HeaderMap,
        body: Result<Json<PauseRequest>, JsonRejection>,
    ) -> Response {
        let actor = match api.authorize(&headers) {
            Ok(actor) => actor,
            Err(refusal) => return refusal.into_response(),
        };
        let Ok(retry_after) = body
            .map_err(drop)
            .and_then(|Json(body)| RetryAfter::seconds(body.retry_after_seconds).map_err(drop))
        else {
            return ControlRefusal::InvalidRequest.into_response();
        };
        api.command(actor, OperatorCommand::Pause { retry_after })
    }

    async fn resume(State(api): State<Arc<Self>>, headers: HeaderMap) -> Response {
        match api.authorize(&headers) {
            Ok(actor) => api.command(actor, OperatorCommand::Resume),
            Err(refusal) => refusal.into_response(),
        }
    }

    async fn block(
        State(api): State<Arc<Self>>,
        Path(session): Path<String>,
        headers: HeaderMap,
        body: Option<Json<BlockRequest>>,
    ) -> Response {
        let blocked = api.authorize(&headers).and_then(|actor| {
            let session = Self::session(&session)?;
            let duration = Self::duration(body.unwrap_or_default().0.duration_seconds)?;
            Ok((actor, session, duration))
        });
        match blocked {
            Ok((actor, session, duration)) => api.command(
                actor,
                OperatorCommand::block_session(session, duration, api.clock.now().wall),
            ),
            Err(refusal) => refusal.into_response(),
        }
    }

    async fn unblock(
        State(api): State<Arc<Self>>,
        Path(session): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        let unblocked = api
            .authorize(&headers)
            .and_then(|actor| Ok((actor, Self::session(&session)?)));
        match unblocked {
            Ok((actor, session)) => api.command(actor, OperatorCommand::UnblockSession { session }),
            Err(refusal) => refusal.into_response(),
        }
    }

    async fn block_user(
        State(api): State<Arc<Self>>,
        Path(user): Path<String>,
        headers: HeaderMap,
        body: Option<Json<BlockRequest>>,
    ) -> Response {
        let blocked = api.authorize(&headers).and_then(|actor| {
            let user = api.user(&user)?;
            let duration = Self::duration(body.unwrap_or_default().0.duration_seconds)?;
            Ok((actor, user, duration))
        });
        match blocked {
            Ok((actor, user, duration)) => api.command(
                actor,
                OperatorCommand::block_user(user, duration, api.clock.now().wall),
            ),
            Err(refusal) => refusal.into_response(),
        }
    }

    async fn unblock_user(
        State(api): State<Arc<Self>>,
        Path(user): Path<String>,
        headers: HeaderMap,
    ) -> Response {
        let unblocked = api
            .authorize(&headers)
            .and_then(|actor| Ok((actor, api.user(&user)?)));
        match unblocked {
            Ok((actor, user)) => api.command(actor, OperatorCommand::UnblockUser { user }),
            Err(refusal) => refusal.into_response(),
        }
    }

    /// Back up the history into the data directory's `backup` subdirectory.
    /// It changes no operator state, so it is logged, not recorded. A
    /// request dropped mid-way leaves the previous backup restorable.
    async fn backup(State(api): State<Arc<Self>>, headers: HeaderMap) -> Response {
        let actor = match api.authorize(&headers) {
            Ok(actor) => actor,
            Err(refusal) => return refusal.into_response(),
        };
        match api.backup.run().await {
            Ok(report) => {
                tracing::info!(
                    ?actor,
                    chunks_copied = report.chunks_copied,
                    "backup written"
                );
                Self::answer(body::history::BackupBody {
                    backup: report.into(),
                })
            }
            Err(BackupError::Running) => ControlRefusal::BackupRunning.into_response(),
            Err(error @ BackupError::Incomplete(_)) => {
                tracing::warn!(%error, "backup incomplete");
                ControlRefusal::BackupIncomplete.into_response()
            }
            Err(
                error @ (BackupError::Io(_)
                | BackupError::Database(_)
                | BackupError::Store(_)
                | BackupError::NonUtf8Path),
            ) => {
                tracing::warn!(%error, "backup failed");
                ControlRefusal::BackupFailed.into_response()
            }
        }
    }

    /// Cancel a running call. With `block_session_seconds`, its session is
    /// blocked first, so a client retry is refused.
    async fn cancel(
        State(api): State<Arc<Self>>,
        Path(call): Path<String>,
        headers: HeaderMap,
        body: Option<Json<CancelRequest>>,
    ) -> Response {
        let request = api.authorize(&headers).and_then(|actor| {
            let call: CallId = call.parse().map_err(|_| ControlRefusal::NotFound)?;
            let block = Self::duration(body.unwrap_or_default().0.block_session_seconds)?;
            Ok((actor, call, block))
        });
        let (actor, call, block) = match request {
            Ok(request) => request,
            Err(refusal) => return refusal.into_response(),
        };
        // Recorded first: a cancel the recorder cannot record is not made.
        if let Err(error) = api.controls.running_call(call).and_then(|()| {
            api.controls.apply(
                &api.recorder,
                actor.clone(),
                OperatorCommand::CancelCall { call },
            )
        }) {
            return ControlRefusal::from(error).into_response();
        }
        let session = match api.controls.cancel(call) {
            Ok(session) => session,
            Err(error) => return ControlRefusal::from(error).into_response(),
        };
        match (block, session) {
            (Some(duration), Some(session)) => api.command(
                actor,
                OperatorCommand::block_session(session, Some(duration), api.clock.now().wall),
            ),
            (Some(_) | None, None) | (None, Some(_)) => {
                (StatusCode::ACCEPTED, api.respond(&api.controls.snapshot())).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ControlOptions;

    #[test]
    fn logins_are_parsed_as_the_configuration_loads() {
        let options = |login: &str| {
            toml::from_str::<ControlOptions>(&format!("tailscale_logins = [{login:?}]"))
        };
        assert_eq!(
            options("me@example.com").unwrap().tailscale_logins[0].as_str(),
            "me@example.com"
        );
        assert!(options("").is_err());
        assert!(options("two words").is_err());
    }
}
