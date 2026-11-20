//! Runs under `/control/v1/runs`: the client processes serving sessions,
//! listed live and followed as server-sent events, and what a session can
//! start with (the projects, the clients, and each client's models), with
//! each client's sign-in (`sign_in`). A run
//! is driven through its session (`sessions`), whose routes share the
//! refusals here. Access is the control API's; with no run
//! roots configured, every route answers `runs_disabled`.
//!
//! Live updates carry what the agent reported (its reply as it streams, its
//! commands, and its approvals). The supervisor stores every event but the
//! streamed pieces as the run's conversation; its model traffic is recorded
//! like any other client's.

use super::body::runs::{
    ClientsBody, CommandsBody, ModelsBody, Places, RunUpdate, RunsBody, Skipped,
};
use super::{ControlApi, ControlRefusal};
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use http::HeaderMap;
use ren_harness::{Harness, commands::ClientCommand, drive::CommandRefused};
use ren_supervisor::{
    CommandError, CommandsError, ListingError, RunRefused, StartError, StartPlace, Supervisor,
    WorkdirError,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

/// Which folder's commands to list: `dir`, or the scratch folder.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
pub(super) struct CommandsRequest {
    #[serde(default)]
    pub(super) dir: Option<String>,
}

impl From<StartError> for ControlRefusal {
    fn from(error: StartError) -> Self {
        match error {
            StartError::Workdir(
                WorkdirError::Relative
                | WorkdirError::Unnormalized
                | WorkdirError::OutsideRoots
                | WorkdirError::NotUtf8,
            ) => Self::InvalidDirectory,
            StartError::Workdir(WorkdirError::NoRoots) => Self::RunsDisabled,
            StartError::Resolve(_) => Self::DirectoryNotFound,
            StartError::NoApprovals(_) => Self::NoApprovals,
            StartError::NoScratch | StartError::Scratch(_) => Self::NoScratch,
            StartError::NoWorktrees | StartError::Worktree(_) => Self::NoWorktree,
            StartError::NoSlot => Self::WorktreesFull,
            StartError::Unrecorded(_) => Self::Unrecorded,
            StartError::Spawn { .. } => Self::SpawnFailed,
            StartError::Gone | StartError::NilId => Self::Unavailable,
        }
    }
}

impl From<CommandError> for ControlRefusal {
    fn from(error: CommandError) -> Self {
        match error {
            CommandError::NoSuchRun(_) => Self::NotFound,
            CommandError::Refused(refused) => match refused {
                RunRefused::Starting => Self::Starting,
                RunRefused::Busy => Self::Busy,
                RunRefused::NotWorking => Self::NotWorking,
                RunRefused::TurnNotStarted => Self::TurnNotStarted,
                RunRefused::NoSuchApproval(_) => Self::NoSuchApproval,
                RunRefused::Unanswered(_) => Self::Unanswered,
                RunRefused::NotQuestions(_) => Self::NotQuestions,
                RunRefused::Unfit(_) => Self::UnfitReply,
                RunRefused::Stopping => Self::Stopping,
                RunRefused::Ended => Self::Ended,
                RunRefused::NothingQueued => Self::NothingQueued,
            },
            // What the client has no way to do mid-run.
            CommandError::Protocol(CommandRefused::NotOffered(_)) => Self::NotOffered,
            CommandError::Protocol(CommandRefused::NoImages) => Self::NoImages,
            CommandError::Protocol(CommandRefused::Unanswered(_)) => Self::Unanswered,
            CommandError::Protocol(CommandRefused::NotQuestions(_)) => Self::NotQuestions,
            CommandError::Protocol(
                CommandRefused::NotReady
                | CommandRefused::TurnRunning
                | CommandRefused::NoTurn
                | CommandRefused::TurnNotStarted
                | CommandRefused::UnknownApproval(_),
            ) => Self::Protocol,
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
            .route("/control/v1/runs", get(Self::list))
            .route("/control/v1/runs/events", get(Self::events))
            .route("/control/v1/runs/places", get(Self::places))
            .route("/control/v1/runs/clients", get(Self::clients))
            .route(
                "/control/v1/runs/clients/{harness}/commands",
                get(Self::client_commands),
            )
            .route(
                "/control/v1/runs/clients/{harness}/models",
                get(Self::models),
            )
            .route(
                "/control/v1/runs/clients/{harness}/sign-in",
                post(Self::start_sign_in),
            )
            .route(
                "/control/v1/runs/clients/{harness}/sign-in/answer",
                post(Self::answer_sign_in),
            )
            .route(
                "/control/v1/runs/clients/{harness}/sign-in/check",
                post(Self::check_sign_in),
            )
            .route(
                "/control/v1/runs/clients/{harness}/sign-in/cancel",
                post(Self::cancel_sign_in),
            )
            .route(
                "/control/v1/runs/clients/{harness}/sign-in/wait",
                post(Self::wait_sign_in),
            )
            .with_state(Arc::new(self))
    }

    /// The supervisor, once the request is authorized.
    pub(super) fn admit(&self, headers: &HeaderMap) -> Result<&Supervisor, ControlRefusal> {
        self.control.authorize(headers)?;
        self.supervisor.as_ref().ok_or(ControlRefusal::RunsDisabled)
    }

    pub(super) fn body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, ControlRefusal> {
        body.map(|Json(body)| body)
            .map_err(|_| ControlRefusal::InvalidRequest)
    }

    async fn list(State(api): Api, headers: HeaderMap) -> Result<Response, ControlRefusal> {
        let runs = api
            .admit(&headers)?
            .runs()
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        Ok(ControlApi::answer(RunsBody {
            runs: runs.into_iter().map(Into::into).collect(),
        }))
    }

    async fn places(State(api): Api, headers: HeaderMap) -> Result<Response, ControlRefusal> {
        let places = api
            .admit(&headers)?
            .places()
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        Ok(ControlApi::answer(Places::from(places)))
    }

    async fn clients(State(api): Api, headers: HeaderMap) -> Result<Response, ControlRefusal> {
        let clients = api
            .admit(&headers)?
            .clients()
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        Ok(ControlApi::answer(ClientsBody {
            clients: clients.into_iter().map(Into::into).collect(),
        }))
    }

    async fn models(
        State(api): Api,
        headers: HeaderMap,
        Path(harness): Path<String>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let harness = harness
            .parse::<Harness>()
            .map_err(|_| ControlRefusal::NotFound)?;
        let unlisted = |error: ListingError| {
            ControlRefusal::ModelsUnavailable.explained(Some(error.to_string()))
        };
        Ok(match supervisor.models(harness).await {
            Ok(models) => ControlApi::answer(ModelsBody {
                harness,
                models: models.into_iter().map(Into::into).collect(),
            }),
            Err(ListingError::NoRoot) => ControlRefusal::RunsDisabled.into_response(),
            Err(ListingError::Gone) => ControlRefusal::Unavailable.into_response(),
            Err(
                error @ (ListingError::Spawn
                | ListingError::Refused(_)
                | ListingError::Exited
                | ListingError::TimedOut),
            ) => unlisted(error),
        })
    }

    /// What `harness` offers a prompt in a folder (`dir`, or scratch),
    /// with no session: as the installed client last listed it there, or
    /// listed now by starting it briefly in that folder.
    async fn client_commands(
        State(api): Api,
        headers: HeaderMap,
        Path(harness): Path<String>,
        query: Result<Query<CommandsRequest>, QueryRejection>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let harness = harness
            .parse::<Harness>()
            .map_err(|_| ControlRefusal::NotFound)?;
        let Query(request) = query.map_err(|_| ControlRefusal::InvalidRequest)?;
        let place = match request.dir {
            Some(dir) => StartPlace::Folder(dir),
            None => StartPlace::Scratch,
        };
        Ok(match supervisor.client_commands(harness, place).await {
            Ok(commands) => Self::commands_body(commands),
            Err(error) => Self::commands_refused(error),
        })
    }

    /// Why commands could not be read, as the API answers it.
    pub(super) fn commands_refused(error: CommandsError) -> Response {
        match error {
            CommandsError::NoSuchSession(_) => ControlRefusal::NotFound.into_response(),
            CommandsError::Place(error) => ControlRefusal::from(error).into_response(),
            CommandsError::NotInstalled(_) => ControlRefusal::NotInstalled.into_response(),
            CommandsError::Listing(ListingError::NoRoot) => {
                ControlRefusal::RunsDisabled.into_response()
            }
            CommandsError::Listing(ListingError::Gone) | CommandsError::Gone => {
                ControlRefusal::Unavailable.into_response()
            }
            CommandsError::Listing(
                error @ (ListingError::Spawn
                | ListingError::Refused(_)
                | ListingError::Exited
                | ListingError::TimedOut),
            ) => ControlRefusal::CommandsUnavailable.explained(Some(error.to_string())),
        }
    }

    /// The answer listing `commands`, each with what a prompt writes to run it.
    pub(super) fn commands_body(commands: Vec<ClientCommand>) -> Response {
        ControlApi::answer(CommandsBody {
            commands: commands.into_iter().map(Into::into).collect(),
        })
    }

    /// Updates from now on, as `run` events; `lagged` when the feed fell
    /// behind and skipped some (list the runs again to catch up).
    async fn events(State(api): Api, headers: HeaderMap) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let receiver = supervisor.subscribe();
        let shutdown = api.shutdown.clone();
        let stream = futures_util::stream::unfold(
            (receiver, shutdown),
            |(mut receiver, shutdown)| async move {
                let event = tokio::select! {
                    () = shutdown.cancelled() => return None,
                    received = receiver.recv() => match received {
                        Ok(update) => Event::default()
                            .event("run")
                            .json_data(RunUpdate::from(update))
                            .ok()?,
                        Err(RecvError::Lagged(skipped)) => Event::default()
                            .event("lagged")
                            .json_data(Skipped { skipped })
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
