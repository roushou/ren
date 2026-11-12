//! Runs under `/control/v1/runs`: start an agent in a folder, prompt it,
//! answer its approvals, interrupt or stop it, list the live runs, and follow
//! their updates as server-sent events. Access is the control API's; with no
//! run roots configured, every route answers `runs_disabled`.
//!
//! Live updates carry what the agent reported (its reply as it streams, its
//! commands, and its approvals). They are not stored: the run's facts and
//! its model traffic are the record.

use super::{ControlApi, Refusal};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use http::{HeaderMap, StatusCode};
use ren_harness::{
    Harness,
    drive::{ApprovalId, Decision, Prompt},
};
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::RunId;
use ren_supervisor::{
    CommandError, Conversation, RunCommand, RunRefused, RunSnapshot, StartError, Supervisor,
    WorkdirError,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StartRequest {
    harness: Harness,
    dir: String,
    /// The client's thread to continue, instead of a new conversation.
    #[serde(default)]
    resume: Option<uuid::Uuid>,
    /// The first prompt, sent once the conversation opens.
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptRequest {
    text: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerRequest {
    decision: Decision,
}

#[derive(serde::Serialize)]
struct Started {
    api_version: u32,
    run: RunId,
}

#[derive(serde::Serialize)]
struct Runs {
    api_version: u32,
    runs: Vec<RunSnapshot>,
}

/// A refused request: the control API's access check or a run refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Denied {
    Control(Refusal),
    Run(RunRefusal),
}

impl IntoResponse for Denied {
    fn into_response(self) -> Response {
        match self {
            Self::Control(refusal) => refusal.into_response(),
            Self::Run(refusal) => refusal.into_response(),
        }
    }
}

/// Why a run request was refused, with the label clients match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunRefusal {
    Disabled,
    InvalidDirectory,
    DirectoryNotFound,
    NotDrivable,
    Unrecorded,
    SpawnFailed,
    NotFound,
    Refused(&'static str),
    Unavailable,
}

impl IntoResponse for RunRefusal {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Disabled => (StatusCode::NOT_FOUND, "runs_disabled"),
            Self::InvalidDirectory => (StatusCode::BAD_REQUEST, "invalid_directory"),
            Self::DirectoryNotFound => (StatusCode::BAD_REQUEST, "directory_not_found"),
            Self::NotDrivable => (StatusCode::BAD_REQUEST, "not_drivable"),
            Self::Unrecorded => (StatusCode::SERVICE_UNAVAILABLE, "unrecorded"),
            Self::SpawnFailed => (StatusCode::BAD_GATEWAY, "spawn_failed"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Refused(reason) => (StatusCode::CONFLICT, reason),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        };
        (
            status,
            Json(serde_json::json!({ "api_version": ControlApi::API_VERSION, "error": code })),
        )
            .into_response()
    }
}

impl From<StartError> for RunRefusal {
    fn from(error: StartError) -> Self {
        match error {
            StartError::Workdir(
                WorkdirError::Relative
                | WorkdirError::Unnormalized
                | WorkdirError::OutsideRoots
                | WorkdirError::NotUtf8,
            ) => Self::InvalidDirectory,
            StartError::Workdir(WorkdirError::NoRoots) => Self::Disabled,
            StartError::Resolve(_) => Self::DirectoryNotFound,
            StartError::NotDrivable(_) => Self::NotDrivable,
            StartError::Unrecorded(_) => Self::Unrecorded,
            StartError::Spawn { .. } => Self::SpawnFailed,
            StartError::Gone | StartError::NilId => Self::Unavailable,
        }
    }
}

impl From<CommandError> for RunRefusal {
    fn from(error: CommandError) -> Self {
        match error {
            CommandError::NoSuchRun(_) => Self::NotFound,
            CommandError::Refused(refused) => Self::Refused(match refused {
                RunRefused::Starting => "starting",
                RunRefused::Busy => "busy",
                RunRefused::NotWorking => "not_working",
                RunRefused::TurnNotStarted => "turn_not_started",
                RunRefused::NoSuchApproval(_) => "no_such_approval",
                RunRefused::Stopping => "stopping",
                RunRefused::Ended => "ended",
            }),
            CommandError::Protocol(_) => Self::Refused("protocol"),
            CommandError::Gone => Self::Unavailable,
        }
    }
}

/// The run routes' state: the control API's access check, the supervisor
/// when runs are enabled, and the token that ends update feeds at shutdown.
pub(crate) struct RunsApi {
    pub(super) control: Arc<ControlApi>,
    pub(super) supervisor: Option<Supervisor>,
    pub(super) shutdown: CancellationToken,
}

type Api = State<Arc<RunsApi>>;

impl RunsApi {
    pub(super) fn router(self) -> Router {
        Router::new()
            .route("/control/v1/runs", get(Self::list).post(Self::start))
            .route("/control/v1/runs/events", get(Self::events))
            .route("/control/v1/runs/projects", get(Self::projects))
            .route("/control/v1/runs/clients", get(Self::clients))
            .route("/control/v1/runs/{run}/prompt", post(Self::prompt))
            .route(
                "/control/v1/runs/{run}/approvals/{approval}",
                post(Self::answer),
            )
            .route("/control/v1/runs/{run}/interrupt", post(Self::interrupt))
            .route("/control/v1/runs/{run}/stop", post(Self::stop))
            .with_state(Arc::new(self))
    }

    /// The supervisor, once the request is authorized.
    fn admit(&self, headers: &HeaderMap) -> Result<&Supervisor, Denied> {
        self.control.authorize(headers).map_err(Denied::Control)?;
        self.supervisor
            .as_ref()
            .ok_or(Denied::Run(RunRefusal::Disabled))
    }

    fn run(text: &str) -> Result<RunId, Denied> {
        text.parse().map_err(|_| Denied::Run(RunRefusal::NotFound))
    }

    fn body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Denied> {
        body.map(|Json(body)| body)
            .map_err(|_| Denied::Control(Refusal::Invalid))
    }

    async fn command(
        supervisor: &Supervisor,
        run: RunId,
        command: RunCommand,
    ) -> Result<Response, Denied> {
        supervisor
            .command(run, command)
            .await
            .map_err(|error| Denied::Run(error.into()))?;
        // Sent to the agent; its reply arrives as run updates.
        Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "api_version": ControlApi::API_VERSION })),
        )
            .into_response())
    }

    async fn list(State(api): Api, headers: HeaderMap) -> Result<Response, Denied> {
        let runs = api
            .admit(&headers)?
            .runs()
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        Ok(Json(Runs {
            api_version: ControlApi::API_VERSION,
            runs,
        })
        .into_response())
    }

    async fn projects(State(api): Api, headers: HeaderMap) -> Result<Response, Denied> {
        let projects = api
            .admit(&headers)?
            .projects()
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        Ok(Json(serde_json::json!({
            "api_version": ControlApi::API_VERSION,
            "projects": projects,
        }))
        .into_response())
    }

    async fn clients(State(api): Api, headers: HeaderMap) -> Result<Response, Denied> {
        let clients = api
            .admit(&headers)?
            .clients()
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        Ok(Json(serde_json::json!({
            "api_version": ControlApi::API_VERSION,
            "clients": clients,
        }))
        .into_response())
    }

    async fn start(
        State(api): Api,
        headers: HeaderMap,
        body: Result<Json<StartRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let request = Self::body(body)?;
        let conversation = match request.resume {
            None => Conversation::New,
            Some(thread) => Conversation::Resume(
                NativeThreadId::new(thread).ok_or(Denied::Control(Refusal::Invalid))?,
            ),
        };
        let first = request
            .prompt
            .map(Prompt::parse)
            .transpose()
            .map_err(|_| Denied::Control(Refusal::Invalid))?;
        let run = supervisor
            .start(request.harness, &request.dir, conversation, first)
            .await
            .map_err(|error| Denied::Run(error.into()))?;
        Ok((
            StatusCode::CREATED,
            Json(Started {
                api_version: ControlApi::API_VERSION,
                run,
            }),
        )
            .into_response())
    }

    async fn prompt(
        State(api): Api,
        headers: HeaderMap,
        Path(run): Path<String>,
        body: Result<Json<PromptRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let run = Self::run(&run)?;
        let prompt =
            Prompt::parse(Self::body(body)?.text).map_err(|_| Denied::Control(Refusal::Invalid))?;
        Self::command(supervisor, run, RunCommand::Prompt(prompt)).await
    }

    async fn answer(
        State(api): Api,
        headers: HeaderMap,
        Path((run, approval)): Path<(String, String)>,
        body: Result<Json<AnswerRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let run = Self::run(&run)?;
        let approval = approval
            .parse::<ApprovalId>()
            .map_err(|_| Denied::Run(RunRefusal::NotFound))?;
        let decision = Self::body(body)?.decision;
        Self::command(supervisor, run, RunCommand::Answer { approval, decision }).await
    }

    async fn interrupt(
        State(api): Api,
        headers: HeaderMap,
        Path(run): Path<String>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        Self::command(supervisor, Self::run(&run)?, RunCommand::Interrupt).await
    }

    async fn stop(
        State(api): Api,
        headers: HeaderMap,
        Path(run): Path<String>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        Self::command(supervisor, Self::run(&run)?, RunCommand::Stop).await
    }

    /// Updates from now on, as `run` events; `lagged` when the feed fell
    /// behind and skipped some (list the runs again to catch up).
    async fn events(State(api): Api, headers: HeaderMap) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let receiver = supervisor.subscribe();
        let shutdown = api.shutdown.clone();
        let stream = futures_util::stream::unfold(
            (receiver, shutdown),
            |(mut receiver, shutdown)| async move {
                let event = tokio::select! {
                    () = shutdown.cancelled() => return None,
                    received = receiver.recv() => match received {
                        Ok(update) => Event::default().event("run").json_data(&update).ok()?,
                        Err(RecvError::Lagged(skipped)) => Event::default()
                            .event("lagged")
                            .json_data(serde_json::json!({ "skipped": skipped }))
                            .ok()?,
                        Err(RecvError::Closed) => return None,
                    },
                };
                Some((Ok::<_, Infallible>(event), (receiver, shutdown)))
            },
        );
        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
            .into_response())
    }
}
