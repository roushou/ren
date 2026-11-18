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

use super::request::{AnswerRequest, CreateRequest, PromptRequest};
use super::runs::{FilesRequest, RunsApi, SettingsRequest};
use super::{ControlApi, ControlRefusal};
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
    attachment::{
        Attachment, AttachmentError, AttachmentKind, ByteSize, Dimensions, Image, PromptLimitError,
    },
    drive::ApprovalId,
    models::ModelId,
    settings::RunSettings,
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
    AttachmentUse, Create, FileQuery, KeptAttachment, RunCommand, SessionError, SessionPhase,
    SessionRefused, SessionSeed, SessionSnapshot, SessionState, Shelving, StartPlace, Supervisor,
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
struct ClientRequest {
    harness: Harness,
}

/// A draft's new folder: `dir`, or a new scratch folder of its own.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderRequest {
    #[serde(default)]
    dir: Option<String>,
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
            .route("/control/v1/sessions/{session}/folder", post(Self::folder))
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
                get(Self::attachments).post(Self::attach),
            )
            .route(
                "/control/v1/sessions/{session}/attachments/{attachment}",
                get(Self::attachment).delete(Self::detach),
            )
            .with_state(Arc::new(self))
    }

    /// The supervisor, once the request is authorized.
    fn admit(&self, headers: &HeaderMap) -> Result<&Supervisor, ControlRefusal> {
        self.control.authorize(headers)?;
        self.supervisor.as_ref().ok_or(ControlRefusal::RunsDisabled)
    }

    fn session(text: &str) -> Result<SessionId, ControlRefusal> {
        text.parse().map_err(|_| ControlRefusal::NotFound)
    }

    fn refused(error: SessionError) -> ControlRefusal {
        match error {
            SessionError::NoSuchSession(_) => ControlRefusal::NotFound,
            SessionError::Refused(SessionRefused::ClientFixed) => ControlRefusal::ClientFixed,
            SessionError::Refused(SessionRefused::FolderFixed) => ControlRefusal::FolderFixed,
            SessionError::Refused(SessionRefused::Archived) => ControlRefusal::Archived,
            SessionError::Refused(SessionRefused::NotArchived) => ControlRefusal::NotArchived,
            SessionError::NoProcess => ControlRefusal::NoProcess,
            SessionError::Running => ControlRefusal::RunRunning,
            SessionError::Unreadable(_) => ControlRefusal::Unreadable,
            SessionError::Unrecorded => ControlRefusal::Unrecorded,
            SessionError::Start(error) => error.into(),
            SessionError::Command(error) => error.into(),
            SessionError::Attachment(error) => Self::unattachable(error),
            SessionError::NoAttachments => ControlRefusal::NoAttachments,
            SessionError::NoSuchAttachment(_) => ControlRefusal::NotFound,
            SessionError::AttachmentSent(_) => ControlRefusal::AttachmentSent,
            SessionError::BlankPrompt => ControlRefusal::BlankPrompt,
            SessionError::PromptAttachments(PromptLimitError::TooMany { .. }) => {
                ControlRefusal::TooManyAttachments
            }
            SessionError::PromptAttachments(PromptLimitError::TooLarge { .. }) => {
                ControlRefusal::AttachmentTooLarge
            }
            SessionError::ImagesNotTaken(_) => ControlRefusal::NoImages,
            SessionError::AttachmentStorage(error) => {
                tracing::warn!(%error, "an attachment could not be kept or read");
                ControlRefusal::AttachmentStorage
            }
            SessionError::Gone => ControlRefusal::Unavailable,
        }
    }

    /// Why an upload is not an attachment, as the API names it.
    fn unattachable(error: AttachmentError) -> ControlRefusal {
        match error {
            AttachmentError::TooLarge { .. } => ControlRefusal::AttachmentTooLarge,
            AttachmentError::Name(_) => ControlRefusal::AttachmentName,
            AttachmentError::Empty => ControlRefusal::AttachmentEmpty,
            AttachmentError::Unrecognized => ControlRefusal::AttachmentUnrecognized,
            AttachmentError::Unreadable(_) => ControlRefusal::AttachmentUnreadable,
            AttachmentError::TooWide { .. } => ControlRefusal::AttachmentTooWide,
        }
    }

    /// A stored attachment as the supervisor holds it; `None` for one this
    /// daemon cannot read back (a format it does not know).
    fn restore(stored: AttachmentView) -> Option<KeptAttachment> {
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
        Some(KeptAttachment {
            id: stored.id,
            read,
            used: match stored.sent_at {
                Some(_) => AttachmentUse::Sent,
                None => AttachmentUse::Unsent,
            },
        })
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
    async fn load(
        &self,
        supervisor: &Supervisor,
        session: SessionId,
    ) -> Result<(), ControlRefusal> {
        let stored = self
            .reader
            .session(session)
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        let Some(stored) = stored else {
            return Ok(());
        };
        let mut seed = Self::seed(&stored).ok_or(ControlRefusal::NotDrivable)?;
        seed.attachments = self
            .reader
            .attachments(session)
            .await
            .map_err(|_| ControlRefusal::Unavailable)?
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
    ) -> Result<Response, ControlRefusal> {
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
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let Query(request) = query.map_err(|_| ControlRefusal::InvalidRequest)?;
        let shelf = request.shelf.unwrap_or_default();
        let limit = request
            .limit
            .map(PageLimit::new)
            .transpose()
            .map_err(|_| ControlRefusal::InvalidRequest)?
            .unwrap_or(PageLimit::DEFAULT);
        let stored = api
            .reader
            .sessions(limit, shelf)
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
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
                .map_err(|_| ControlRefusal::Unavailable)?
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
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let stored = api
            .reader
            .session(session)
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        let served = supervisor
            .sessions()
            .await
            .map_err(Self::refused)?
            .into_iter()
            .find(|snapshot| snapshot.session == session);
        if stored.is_none() && served.is_none() {
            return Err(ControlRefusal::NotFound);
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
    ) -> Result<Response, ControlRefusal> {
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
    ) -> Result<Response, ControlRefusal> {
        let request = RunsApi::body(body)?;
        // Blank with nothing attached is a malformed request, as it always was.
        if request.text.trim().is_empty() && request.attachments.is_empty() {
            return Err(ControlRefusal::InvalidRequest);
        }
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        api.load(supervisor, session).await?;
        supervisor
            .prompt(session, request.text, request.attachments, request.delivery)
            .await
            .map_err(Self::refused)?;
        Ok(Self::done())
    }

    async fn unqueue(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, ControlRefusal> {
        api.send(&headers, &session, RunCommand::Unqueue).await
    }

    async fn settings(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        body: Result<Json<SettingsRequest>, JsonRejection>,
    ) -> Result<Response, ControlRefusal> {
        let settings = RunsApi::body(body)?.settings()?;
        api.send(&headers, &session, RunCommand::Configure(settings))
            .await
    }

    async fn answer(
        State(api): Api,
        headers: HeaderMap,
        Path((session, approval)): Path<(String, String)>,
        body: Result<Json<AnswerRequest>, JsonRejection>,
    ) -> Result<Response, ControlRefusal> {
        let approval = approval
            .parse::<ApprovalId>()
            .map_err(|_| ControlRefusal::NotFound)?;
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
    ) -> Result<Response, ControlRefusal> {
        api.send(&headers, &session, RunCommand::Interrupt).await
    }

    async fn stop(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, ControlRefusal> {
        api.send(&headers, &session, RunCommand::Stop).await
    }

    async fn client(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        body: Result<Json<ClientRequest>, JsonRejection>,
    ) -> Result<Response, ControlRefusal> {
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

    /// Move a draft to another folder, or a new scratch folder of its own.
    async fn folder(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        body: Result<Json<FolderRequest>, JsonRejection>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let place = match RunsApi::body(body)?.dir {
            Some(dir) => StartPlace::Folder(dir),
            None => StartPlace::Scratch,
        };
        api.load(supervisor, session).await?;
        supervisor
            .choose_folder(session, place)
            .await
            .map_err(Self::refused)?;
        Ok(Self::done())
    }

    async fn shelve(
        &self,
        headers: &HeaderMap,
        session: &str,
        shelving: Shelving,
    ) -> Result<Response, ControlRefusal> {
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
    ) -> Result<Response, ControlRefusal> {
        api.shelve(&headers, &session, Shelving::Archive).await
    }

    async fn unarchive(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, ControlRefusal> {
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
    ) -> Result<Response, ControlRefusal> {
        let actor = api.control.authorize(&headers)?;
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let subject = HistorySubject::OperatorSession(session);
        // None: not stored yet, a session made a moment ago.
        let target = api
            .reader
            .history_target(subject)
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        if let Some(target) = target {
            if target.running_runs > 0 {
                return Err(ControlRefusal::RunRunning);
            }
            if target.open_calls > 0 {
                return Err(ControlRefusal::CallOpen);
            }
        }
        let slot = api
            .control
            .recorder
            .history_slot()
            .ok_or(ControlRefusal::Unrecorded)?;
        let served = supervisor.forget(session).await.map_err(Self::refused)?;
        if target.is_none() && !served {
            return Err(ControlRefusal::NotFound);
        }
        slot.record(actor, subject, HistoryChange::Delete);
        Ok(Self::done())
    }

    /// What the session's client offers a prompt: its running client's
    /// list, else what the installed client lists in its folder.
    async fn commands(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        api.load(supervisor, session).await?;
        Ok(match supervisor.session_commands(session).await {
            Ok(commands) => RunsApi::commands_body(commands),
            Err(error) => RunsApi::commands_refused(error),
        })
    }

    /// The paths of the session's folder that best match what was typed.
    async fn files(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
        query: Result<Query<FilesRequest>, QueryRejection>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let Query(request) = query.map_err(|_| ControlRefusal::InvalidRequest)?;
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
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        let Query(request) = query.map_err(|_| ControlRefusal::AttachmentName)?;
        let limits = supervisor
            .attachment_limits()
            .ok_or(ControlRefusal::NoAttachments)?;
        let cap = usize::try_from(limits.image_bytes().get()).unwrap_or(usize::MAX);
        let bytes = axum::body::to_bytes(body, cap)
            .await
            .map_err(|_| ControlRefusal::AttachmentTooLarge)?;
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

    /// The session's attachments: each one's id, what it is, and whether it
    /// went out with a prompt.
    async fn attachments(
        State(api): Api,
        headers: HeaderMap,
        Path(session): Path<String>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let session = Self::session(&session)?;
        api.load(supervisor, session).await?;
        let kept = supervisor
            .attachments(session)
            .await
            .map_err(Self::refused)?;
        let attachments: Vec<serde_json::Value> = kept
            .into_iter()
            .map(|kept| {
                serde_json::json!({
                    "id": kept.id,
                    "read": kept.read,
                    "sent": matches!(kept.used, AttachmentUse::Sent),
                })
            })
            .collect();
        Ok(Json(serde_json::json!({
            "api_version": ControlApi::API_VERSION,
            "attachments": attachments,
        }))
        .into_response())
    }

    /// One of the session's attachments: its bytes, as the image it is.
    /// Never cached, and never read as anything but its own type.
    async fn attachment(
        State(api): Api,
        headers: HeaderMap,
        Path((session, attachment)): Path<(String, String)>,
    ) -> Result<Response, ControlRefusal> {
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
    ) -> Result<Response, ControlRefusal> {
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

    fn attachment_id(text: &str) -> Result<AttachmentId, ControlRefusal> {
        text.parse().map_err(|_| ControlRefusal::NotFound)
    }

    /// Session changes from now on, as `session` events (each the session
    /// as served); `lagged` when the feed fell behind (list them again).
    async fn events(State(api): Api, headers: HeaderMap) -> Result<Response, ControlRefusal> {
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
