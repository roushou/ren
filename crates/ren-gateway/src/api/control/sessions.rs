//! Sessions under `/control/v1/sessions`: what an operator works in, read
//! and driven. Creating one starts its client in a place; a prompt goes
//! through its process, or a new one that resumes its conversation; its
//! client changes while it is a draft; it is archived and unarchived; and
//! its composer reads its client's commands and its folder's files. Images
//! are uploaded to it as attachments, read back, and removed. Access
//! is the control API's; with no run roots configured, every route answers
//! `runs_disabled`.
//!
//! A session this daemon has not served since it started (one the store
//! holds from before) is loaded from the store on its first request.
//!
//! `/control/v1/sessions/{id}/block` and `/unblock` are the operator
//! state's: they take a client's own session id, not one of these.

use super::runs::{
    AnswerRequest, Denied, FilesRequest, PromptRequest, RunRefusal, RunsApi, SettingsRequest,
};
use super::{ControlApi, Refusal};
use axum::{
    Json, Router,
    body::Body,
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
use http::{HeaderMap, StatusCode, header};
use ren_harness::{
    Harness,
    attachment::{Attachment, AttachmentError, AttachmentKind, ByteSize, Dimensions, Image},
    drive::{ApprovalId, Prompt},
    models::ModelId,
    settings::{ApprovalPolicy, Effort, RunSettings},
};
use ren_protocol::label::Stored;
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    batch::{HistoryChange, HistorySubject},
    id::{AttachmentId, SessionId},
};
use ren_store::{
    Reader,
    query::{PageLimit, Shelf},
    view::{AttachmentView, SessionView},
};
use ren_supervisor::{
    Create, FileQuery, RunCommand, SessionError, SessionPhase, SessionRefused, SessionSeed,
    SessionSnapshot, SessionState, Shelving, StartPlace, Supervisor,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

/// The session routes' state: the control API's access check, the
/// supervisor when runs are enabled, the store's reader for sessions it
/// holds, and the token that ends update feeds at shutdown.
pub(super) struct SessionsApi {
    pub(super) control: Arc<ControlApi>,
    pub(super) supervisor: Option<Supervisor>,
    pub(super) reader: Reader,
    pub(super) shutdown: CancellationToken,
}

type Api = State<Arc<SessionsApi>>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    harness: Harness,
    /// The folder, inside a root or another place; left out, the session
    /// gets a new folder of its own in scratch.
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<Effort>,
    #[serde(default)]
    approvals: Option<ApprovalPolicy>,
    /// A client conversation to go on with, begun elsewhere.
    #[serde(default)]
    resume: Option<uuid::Uuid>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientRequest {
    harness: Harness,
}

/// Which sessions to list: the current ones (the default) or the archived.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequest {
    #[serde(default)]
    shelf: Option<Shelf>,
    #[serde(default)]
    limit: Option<u32>,
}

/// An upload's name, as `?name=`.
#[derive(serde::Deserialize)]
struct AttachRequest {
    name: String,
}

/// One session: as the store holds it (with its runs), and as the
/// supervisor serves it now. Either may be missing: the store records a
/// new session a moment after it is made, and the supervisor serves a
/// stored one only once asked.
#[derive(serde::Serialize)]
struct SessionBody {
    session: SessionId,
    stored: Option<SessionView>,
    served: Option<SessionSnapshot>,
}

impl SessionsApi {
    pub(super) fn router(self) -> Router {
        Router::new()
            .route("/control/v1/sessions", get(Self::list).post(Self::create))
            .route("/control/v1/sessions/events", get(Self::events))
            .route("/control/v1/sessions/{session}", get(Self::one))
            .route("/control/v1/sessions/{session}/prompt", post(Self::prompt))
            .route(
                "/control/v1/sessions/{session}/unqueue",
                post(Self::unqueue),
            )
            .route(
                "/control/v1/sessions/{session}/settings",
                post(Self::settings),
            )
            .route("/control/v1/sessions/{session}/client", post(Self::client))
            .route(
                "/control/v1/sessions/{session}/approvals/{approval}",
                post(Self::answer),
            )
            .route(
                "/control/v1/sessions/{session}/interrupt",
                post(Self::interrupt),
            )
            .route("/control/v1/sessions/{session}/stop", post(Self::stop))
            .route(
                "/control/v1/sessions/{session}/archive",
                post(Self::archive),
            )
            .route(
                "/control/v1/sessions/{session}/unarchive",
                post(Self::unarchive),
            )
            .route("/control/v1/sessions/{session}/delete", post(Self::delete))
            .route(
                "/control/v1/sessions/{session}/commands",
                get(Self::commands),
            )
            .route("/control/v1/sessions/{session}/files", get(Self::files))
            .route(
                "/control/v1/sessions/{session}/attachments",
                post(Self::attach),
            )
            .route(
                "/control/v1/sessions/{session}/attachments/{attachment}",
                get(Self::attachment).delete(Self::detach),
            )
            .with_state(Arc::new(self))
    }

    /// The supervisor, once the request is authorized.
    fn admit(&self, headers: &HeaderMap) -> Result<&Supervisor, Denied> {
        self.control.authorize(headers).map_err(Denied::Control)?;
        self.supervisor
            .as_ref()
            .ok_or(Denied::Run(RunRefusal::Disabled))
    }

    fn session(text: &str) -> Result<SessionId, Denied> {
        text.parse().map_err(|_| Denied::Run(RunRefusal::NotFound))
    }

    fn refused(error: SessionError) -> Denied {
        Denied::Run(match error {
            SessionError::NoSuchSession(_) => RunRefusal::NotFound,
            SessionError::Refused(SessionRefused::ClientFixed) => {
                RunRefusal::Refused("client_fixed")
            }
            SessionError::Refused(SessionRefused::Archived) => RunRefusal::Refused("archived"),
            SessionError::Refused(SessionRefused::NotArchived) => {
                RunRefusal::Refused("not_archived")
            }
            SessionError::NoProcess => RunRefusal::Refused("no_process"),
            SessionError::Running => RunRefusal::Refused("run_running"),
            SessionError::Unreadable(_) => RunRefusal::Refused("unreadable"),
            SessionError::Unrecorded => RunRefusal::Unrecorded,
            SessionError::Start(error) => error.into(),
            SessionError::Command(error) => error.into(),
            SessionError::Attachment(error) => Self::unattachable(error),
            SessionError::NoAttachments => RunRefusal::NoAttachments,
            SessionError::NoSuchAttachment(_) => RunRefusal::NotFound,
            SessionError::AttachmentStorage(error) => {
                tracing::warn!(%error, "an attachment could not be kept or read");
                RunRefusal::AttachmentStorage
            }
            SessionError::Gone => RunRefusal::Unavailable,
        })
    }

    /// Why an upload is not an attachment, as the API names it.
    fn unattachable(error: AttachmentError) -> RunRefusal {
        match error {
            AttachmentError::TooLarge { .. } => RunRefusal::AttachmentTooLarge,
            AttachmentError::Name(_) => RunRefusal::InvalidAttachment("attachment_name"),
            AttachmentError::Empty => RunRefusal::InvalidAttachment("attachment_empty"),
            AttachmentError::Unrecognized => {
                RunRefusal::InvalidAttachment("attachment_unrecognized")
            }
            AttachmentError::Unreadable(_) => {
                RunRefusal::InvalidAttachment("attachment_unreadable")
            }
            AttachmentError::TooWide { .. } => RunRefusal::InvalidAttachment("attachment_too_wide"),
        }
    }

    /// A stored attachment as the supervisor holds it; `None` for one this
    /// daemon cannot read back (a format it does not know).
    fn restore(stored: AttachmentView) -> Option<(AttachmentId, Attachment)> {
        let Stored::Known(format) = stored.format else {
            return None;
        };
        let dimensions = Dimensions::new(stored.width, stored.height)?;
        let sha256 = stored.sha256.parse().ok()?;
        let read = Attachment::restore(
            &stored.name,
            AttachmentKind::Image(Image::new(format, dimensions)),
            ByteSize::new(stored.size),
            sha256,
        )
        .ok()?;
        Some((stored.id, read))
    }

    /// What a stored session is to the supervisor: where it works, on
    /// what, the conversation its newest process opened (a draft's none: no
    /// turn ran, so its client kept no conversation to resume), and where
    /// it stands. `None` for one whose client this daemon does not know.
    fn seed(stored: &SessionView) -> Option<SessionSeed> {
        let Stored::Known(harness) = stored.harness else {
            return None;
        };

        let phase = match stored.first_turn_at {
            Some(_) => SessionPhase::Active,
            None => SessionPhase::Draft,
        };
        Some(SessionSeed {
            session: stored.id,
            harness,
            workdir: stored.workdir.clone(),
            settings: RunSettings {
                model: stored
                    .model
                    .as_deref()
                    .and_then(|model| model.parse::<ModelId>().ok()),
                effort: match stored.effort {
                    Some(Stored::Known(effort)) => Some(effort),
                    Some(Stored::Unknown(_)) | None => None,
                },
                approvals: match stored.approvals {
                    Some(Stored::Known(policy)) => Some(policy),
                    Some(Stored::Unknown(_)) | None => None,
                },
            },
            thread: match phase {
                SessionPhase::Draft => None,
                SessionPhase::Active => stored
                    .runs
                    .iter()
                    .find_map(|run| run.thread.as_deref())
                    .and_then(|thread| thread.parse::<uuid::Uuid>().ok())
                    .and_then(NativeThreadId::new),
            },
            state: match stored.archived_at {
                Some(_) => SessionState::Archived(phase),
                None => SessionState::Open(phase),
            },
            // Read separately, by `load`.
            attachments: Vec::new(),
        })
    }

    /// Serve `session` from the store, if it holds it and the supervisor
    /// does not serve it yet. A session the store does not hold may still
    /// be served: the store records a new one a moment after it is made.
    async fn load(&self, supervisor: &Supervisor, session: SessionId) -> Result<(), Denied> {
        let stored = self
            .reader
            .session(session)
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        let Some(stored) = stored else {
            return Ok(());
        };
        let mut seed = Self::seed(&stored).ok_or(Denied::Run(RunRefusal::NotDrivable))?;
        seed.attachments = self
            .reader
            .attachments(session)
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?
            .into_iter()
            .filter_map(|stored| {
                let id = stored.id;
                let restored = Self::restore(stored);
                if restored.is_none() {
                    tracing::warn!(%session, attachment = %id, "a stored attachment cannot be read back");
                }
                restored
            })
            .collect();
        supervisor.adopt(seed).await.map_err(Self::refused)
    }

    fn done() -> Response {
        (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "api_version": ControlApi::API_VERSION })),
        )
            .into_response()
    }

    /// Carry out `command` in `session`, loading it from the store first.
    async fn send(
        &self,
        headers: &HeaderMap,
        session: &str,
        command: RunCommand,
    ) -> Result<Response, Denied> {
        let supervisor = self.admit(headers)?;
        let session = Self::session(session)?;
        self.load(supervisor, session).await?;
        supervisor
            .send(session, command)
            .await
            .map_err(Self::refused)?;
        Ok(Self::done())
    }

    async fn list(
        State(api): Api,
        headers: HeaderMap,
        query: Result<Query<ListRequest>, QueryRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let Query(request) = query.map_err(|_| Denied::Control(Refusal::Invalid))?;
        let shelf = request.shelf.unwrap_or_default();
        let limit = request
            .limit
            .map(PageLimit::new)
            .transpose()
            .map_err(|_| Denied::Control(Refusal::Invalid))?
            .unwrap_or(PageLimit::DEFAULT);
        let stored = api
            .reader
            .sessions(limit, shelf)
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        let mut served = supervisor.sessions().await.map_err(Self::refused)?;
        let mut sessions: Vec<SessionBody> = stored
            .into_iter()
            .map(|stored| {
                let at = served
                    .iter()
                    .position(|snapshot| snapshot.session == stored.id);
                SessionBody {
                    session: stored.id,
                    served: at.map(|at| served.remove(at)),
                    stored: Some(stored),
                }
            })
            .collect();
        // Served sessions the store has not recorded yet, on their shelf.
        let unstored = served.into_iter().filter(|snapshot| {
            let archived = matches!(snapshot.state, SessionState::Archived(_));
            match shelf {
                Shelf::Current => !archived,
                Shelf::Archived => archived,
            }
        });
        let mut fresh: Vec<SessionBody> = Vec::new();
        for snapshot in unstored {
            if api
                .reader
                .session(snapshot.session)
                .await
                .map_err(|_| Denied::Run(RunRefusal::Unavailable))?
                .is_none()
            {
                fresh.push(SessionBody {
                    session: snapshot.session,
                    stored: None,
                    served: Some(snapshot),
                });
            }
        }
        fresh.append(&mut sessions);
        Ok(Json(serde_json::json!({
            "api_version": ControlApi::API_VERSION,
            "sessions": fresh,
        }))
        .into_response())
    }

    async fn one(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let stored = api
            .reader
            .session(session)
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        let served = supervisor
            .sessions()
            .await
            .map_err(Self::refused)?
            .into_iter()
            .find(|snapshot| snapshot.session == session);
        if stored.is_none() && served.is_none() {
            return Err(Denied::Run(RunRefusal::NotFound));
        }
        Ok(Json(serde_json::json!({
            "api_version": ControlApi::API_VERSION,
            "session": SessionBody { session, stored, served },
        }))
        .into_response())
    }

    async fn create(
        State(api): Api,
        headers: HeaderMap,
        body: Result<Json<CreateRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let request = RunsApi::body(body)?;
        let settings = SettingsRequest {
            model: request.model,
            effort: request.effort,
            approvals: request.approvals,
        }
        .settings()?;
        let session = supervisor
            .create(Create {
                harness: request.harness,
                place: match request.dir {
                    Some(dir) => StartPlace::Folder(dir),
                    None => StartPlace::Scratch,
                },
                settings,
                resume: request.resume.and_then(NativeThreadId::new),
            })
            .await
            .map_err(Self::refused)?;
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "api_version": ControlApi::API_VERSION,
                "session": session,
            })),
        )
            .into_response())
    }

    async fn prompt(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        body: Result<Json<PromptRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let request = RunsApi::body(body)?;
        let prompt = Prompt::parse(request.text).map_err(|_| Denied::Control(Refusal::Invalid))?;
        api.send(&headers, &session, request.delivery.command(prompt))
            .await
    }

    async fn unqueue(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        api.send(&headers, &session, RunCommand::Unqueue).await
    }

    async fn settings(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        body: Result<Json<SettingsRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let settings = RunsApi::body(body)?.settings()?;
        api.send(&headers, &session, RunCommand::Configure(settings))
            .await
    }

    async fn answer(
        State(api): Api,
        headers: HeaderMap,
        Path((session, approval)): Path<(String, String)>,
        body: Result<Json<AnswerRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let approval = approval
            .parse::<ApprovalId>()
            .map_err(|_| Denied::Run(RunRefusal::NotFound))?;
        let decision = RunsApi::body(body)?.decision;
        api.send(
            &headers,
            &session,
            RunCommand::Answer { approval, decision },
        )
        .await
    }

    async fn interrupt(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        api.send(&headers, &session, RunCommand::Interrupt).await
    }

    async fn stop(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        api.send(&headers, &session, RunCommand::Stop).await
    }

    async fn client(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        body: Result<Json<ClientRequest>, JsonRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let harness = RunsApi::body(body)?.harness;
        api.load(supervisor, session).await?;
        supervisor
            .choose_client(session, harness)
            .await
            .map_err(Self::refused)?;
        Ok(Self::done())
    }

    async fn shelve(
        &self,
        headers: &HeaderMap,
        session: &str,
        shelving: Shelving,
    ) -> Result<Response, Denied> {
        let supervisor = self.admit(headers)?;
        let session = Self::session(session)?;
        self.load(supervisor, session).await?;
        supervisor
            .shelve(session, shelving)
            .await
            .map_err(Self::refused)?;
        Ok(Self::done())
    }

    async fn archive(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        api.shelve(&headers, &session, Shelving::Archive).await
    }

    async fn unarchive(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        api.shelve(&headers, &session, Shelving::Unarchive).await
    }

    /// Delete a session for good: its ended runs, what they recorded, and
    /// the session, even one with no run. Refused while a process serves it
    /// or one of its calls is open. The supervisor stops serving it at once;
    /// the store deletes it when the recorder reaches the edit.
    async fn delete(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        let actor = api.control.authorize(&headers).map_err(Denied::Control)?;
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let subject = HistorySubject::OperatorSession(session);
        // None: not stored yet, a session made a moment ago.
        let target = api
            .reader
            .history_target(subject)
            .await
            .map_err(|_| Denied::Run(RunRefusal::Unavailable))?;
        if let Some(target) = target {
            if target.running_runs > 0 {
                return Err(Denied::Run(RunRefusal::Refused("run_running")));
            }
            if target.open_calls > 0 {
                return Err(Denied::Run(RunRefusal::Refused("call_open")));
            }
        }
        let slot = api
            .control
            .recorder
            .history_slot()
            .ok_or(Denied::Run(RunRefusal::Unrecorded))?;
        let served = supervisor.forget(session).await.map_err(Self::refused)?;
        if target.is_none() && !served {
            return Err(Denied::Run(RunRefusal::NotFound));
        }
        slot.record(actor, subject, HistoryChange::Delete);
        Ok(Self::done())
    }

    /// What the session's client offers a prompt; none until a process
    /// has opened its conversation.
    async fn commands(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        api.load(supervisor, session).await?;
        let commands = supervisor
            .session_commands(session)
            .await
            .map_err(Self::refused)?;
        Ok(RunsApi::commands_body(commands))
    }

    /// The paths of the session's folder that best match what was typed.
    async fn files(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        query: Result<Query<FilesRequest>, QueryRejection>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let Query(request) = query.map_err(|_| Denied::Control(Refusal::Invalid))?;
        api.load(supervisor, session).await?;
        let limit = request
            .limit
            .unwrap_or(FilesRequest::LIMIT)
            .clamp(1, FilesRequest::MOST);
        let files = supervisor
            .session_files(session, FileQuery::parse(&request.q), limit)
            .await
            .map_err(Self::refused)?;
        Ok(Json(serde_json::json!({
            "api_version": ControlApi::API_VERSION,
            "files": files,
        }))
        .into_response())
    }

    /// Keep the request's body, an image named by `name`, with the session,
    /// to go with a prompt. Bodies beyond the attachment limit are refused
    /// as they are read.
    async fn attach(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        query: Result<Query<AttachRequest>, QueryRejection>,
        body: Body,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let Query(request) =
            query.map_err(|_| Denied::Run(RunRefusal::InvalidAttachment("attachment_name")))?;
        let limits = supervisor
            .attachment_limits()
            .ok_or(Denied::Run(RunRefusal::NoAttachments))?;
        let cap = usize::try_from(limits.image_bytes().get()).unwrap_or(usize::MAX);
        let bytes = axum::body::to_bytes(body, cap)
            .await
            .map_err(|_| Denied::Run(RunRefusal::AttachmentTooLarge))?;
        api.load(supervisor, session).await?;
        let (attachment, read) = supervisor
            .attach(session, request.name, bytes.to_vec())
            .await
            .map_err(Self::refused)?;
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "api_version": ControlApi::API_VERSION,
                "attachment": { "id": attachment, "read": read },
            })),
        )
            .into_response())
    }

    /// One of the session's attachments: its bytes, as the image it is.
    /// Never cached, and never read as anything but its own type.
    async fn attachment(
        State(api): Api,
        headers: HeaderMap,
        Path((session, attachment)): Path<(String, String)>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let attachment = Self::attachment_id(&attachment)?;
        api.load(supervisor, session).await?;
        let (read, bytes) = supervisor
            .attachment(session, attachment)
            .await
            .map_err(Self::refused)?;
        let AttachmentKind::Image(image) = read.kind();
        Ok((
            [
                (header::CONTENT_TYPE, image.format().media_type()),
                (header::CACHE_CONTROL, "no-store"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CONTENT_DISPOSITION, "inline"),
            ],
            bytes,
        )
            .into_response())
    }

    /// Remove one of the session's attachments before it is sent.
    async fn detach(
        State(api): Api,
        headers: HeaderMap,
        Path((session, attachment)): Path<(String, String)>,
    ) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let attachment = Self::attachment_id(&attachment)?;
        api.load(supervisor, session).await?;
        supervisor
            .detach(session, attachment)
            .await
            .map_err(Self::refused)?;
        Ok(Self::done())
    }

    fn attachment_id(text: &str) -> Result<AttachmentId, Denied> {
        text.parse().map_err(|_| Denied::Run(RunRefusal::NotFound))
    }

    /// Session changes from now on, as `session` events (each the session
    /// as served); `lagged` when the feed fell behind (list them again).
    async fn events(State(api): Api, headers: HeaderMap) -> Result<Response, Denied> {
        let supervisor = api.admit(&headers)?;
        let receiver = supervisor.subscribe_sessions();
        let shutdown = api.shutdown.clone();
        let stream = futures_util::stream::unfold(
            (receiver, shutdown),
            |(mut receiver, shutdown)| async move {
                let event = tokio::select! {
                    () = shutdown.cancelled() => return None,
                    received = receiver.recv() => match received {
                        Ok(snapshot) => Event::default().event("session").json_data(&snapshot).ok()?,
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
