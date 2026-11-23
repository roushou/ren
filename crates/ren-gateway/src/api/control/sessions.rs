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

use super::body::runs::Skipped;
use super::body::sessions::{
    Accepted, Attached, AttachmentEntry, AttachmentsBody, Created, FilesBody, OneSession,
    SessionBody, SessionsBody,
};
use super::projects::ProjectRoster;
use super::request::{AnswerRequest, Checkout, CreateRequest, PromptRequest, ReplyRequest};
use super::runs::RunsApi;
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
    settings::{ApprovalPolicy, Effort, RunSettings},
};
use ren_protocol::label::Stored;
use ren_protocol::native::NativeThreadId;
use ren_recorder::{
    batch::{HistoryChange, HistorySubject, SessionFact, WorkFact},
    id::{AttachmentId, ProjectId, SessionId},
    project::ProjectName,
};
use ren_store::{
    Reader,
    query::{PageLimit, Shelf},
    view::{AttachmentView, SessionView},
};
use ren_supervisor::{
    AttachmentUse, CommandError, Create, FileQuery, KeptAttachment, RunCommand, RunRefused,
    SessionError, SessionPhase, SessionRefused, SessionSeed, SessionState, Shelving, StartPlace,
    Supervisor,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

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

/// A draft's new client.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
struct ClientRequest {
    #[ts(as = "String")]
    harness: Harness,
}

/// A draft's new folder: `dir`, or a new scratch folder of its own.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields = nullable)]
struct FolderRequest {
    #[serde(default)]
    dir: Option<String>,
}

/// Which sessions to list: the current ones (the default) or the archived,
/// of every project or of the one named.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
struct ListRequest {
    #[serde(default)]
    #[ts(as = "Option<String>")]
    shelf: Option<Shelf>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    project: Option<String>,
}

/// An upload's name, as `?name=`.
#[derive(serde::Deserialize, TS)]
#[ts(export, optional_fields)]
struct AttachRequest {
    name: String,
}

/// A search of a run's folder for a prompt to name a file: `q`, what was
/// typed after `@`, and how many to answer.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
struct FilesRequest {
    #[serde(default)]
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

impl FilesRequest {
    const LIMIT: usize = 20;
    const MOST: usize = 100;
}

impl SettingsRequest {
    /// The settings asked for: a model must be one a client could name.
    fn settings(self) -> Result<RunSettings, ControlRefusal> {
        let model = self
            .model
            .map(|model| model.parse::<ModelId>())
            .transpose()
            .map_err(|_| ControlRefusal::InvalidRequest)?;
        Ok(RunSettings {
            model,
            effort: self.effort,
            approvals: self.approvals,
        })
    }
}

/// The settings a run moves to; each left out is the client's own.
#[derive(serde::Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields = nullable)]
struct SettingsRequest {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    #[ts(as = "Option<String>")]
    effort: Option<Effort>,
    #[serde(default)]
    #[ts(as = "Option<String>")]
    approvals: Option<ApprovalPolicy>,
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
                "/control/v1/sessions/{session}/approvals/{approval}/replies",
                post(Self::reply),
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
        (StatusCode::ACCEPTED, ControlApi::answer(Accepted {})).into_response()
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
        let project = request
            .project
            .map(|name| name.parse::<ProjectName>())
            .transpose()
            .map_err(|_| ControlRefusal::InvalidRequest)?;
        let stored = api
            .reader
            .sessions(limit, shelf, project.as_ref())
            .await
            .map_err(|_| ControlRefusal::Unavailable)?;
        let mut served = supervisor.sessions().await.map_err(Self::refused)?;
        let mut sessions: Vec<SessionBody> = stored
            .into_iter()
            .map(|stored| {
                let at = served
                    .iter()
                    .position(|snapshot| snapshot.session == stored.id);
                SessionBody::new(
                    stored.id,
                    Some(stored.into()),
                    at.map(|at| served.remove(at).into()),
                    supervisor.worktrees(),
                )
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
                fresh.push(SessionBody::new(
                    snapshot.session,
                    None,
                    Some(snapshot.into()),
                    supervisor.worktrees(),
                ));
            }
        }
        fresh.append(&mut sessions);
        Ok(ControlApi::answer(SessionsBody { sessions: fresh }))
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
        Ok(ControlApi::answer(OneSession {
            session: SessionBody::new(
                session,
                stored.map(Into::into),
                served.map(Into::into),
                supervisor.worktrees(),
            ),
        }))
    }

    async fn create(
        State(api): Api,
        headers: HeaderMap,
        body: Result<Json<CreateRequest>, JsonRejection>,
    ) -> Result<Response, ControlRefusal> {
        let supervisor = api.admit(&headers)?;
        let request = RunsApi::body(body)?;
        let roster = ProjectRoster::read(&api.reader).await?;
        // Asked for by project, it works in the project's repository;
        // asked for by folder, for the project that holds the folder.
        let (place, project) = match (&request.project, &request.dir) {
            (Some(_), Some(_)) => return Err(ControlRefusal::InvalidRequest),
            (Some(name), None) => {
                let project = roster.named(name)?;
                let dir = project.repository.to_string();
                let place = match request.checkout.unwrap_or(Checkout::Worktree) {
                    Checkout::Worktree => StartPlace::Project(dir),
                    Checkout::Folder => StartPlace::Folder(dir),
                };
                (place, Some(project.id))
            }
            (None, dir) => (
                request.place().ok_or(ControlRefusal::InvalidRequest)?,
                match dir {
                    Some(dir) => roster.owning_folder(dir).await,
                    None => None,
                },
            ),
        };
        let settings = SettingsRequest {
            model: request.model,
            effort: request.effort,
            approvals: request.approvals,
        }
        .settings()?;
        let session = supervisor
            .create(Create {
                harness: request.harness,
                place,
                settings,
                resume: request.resume.and_then(NativeThreadId::new),
            })
            .await
            .map_err(Self::refused)?;
        if project.is_some() {
            api.assign(session, project);
        }
        Ok((StatusCode::CREATED, ControlApi::answer(Created { session })).into_response())
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

    async fn reply(
        State(api): Api,
        headers: HeaderMap,
        Path((session, approval)): Path<(String, String)>,
        body: Result<Json<ReplyRequest>, JsonRejection>,
    ) -> Result<Response, ControlRefusal> {
        let approval = approval
            .parse::<ApprovalId>()
            .map_err(|_| ControlRefusal::NotFound)?;
        let replies = RunsApi::body(body)?.into();
        api.send(&headers, &session, RunCommand::Reply { approval, replies })
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
        let dir = RunsApi::body(body)?.dir;
        let project = match &dir {
            Some(dir) => {
                ProjectRoster::read(&api.reader)
                    .await?
                    .owning_folder(dir)
                    .await
            }
            None => None,
        };
        let place = match dir {
            Some(dir) => StartPlace::Folder(dir),
            None => StartPlace::Scratch,
        };
        api.load(supervisor, session).await?;
        supervisor
            .choose_folder(session, place)
            .await
            .map_err(Self::refused)?;
        api.assign(session, project);
        Ok(Self::done())
    }

    /// Records that `session` works for `project`, or for none. Recorded
    /// after its creation or its move, which the supervisor recorded first;
    /// without room, it stays as it was, and a project can adopt it later.
    fn assign(&self, session: SessionId, project: Option<ProjectId>) {
        match self.control.recorder.session_slot() {
            Some(slot) => slot.record(session, SessionFact::Assigned { project }),
            None => tracing::warn!(%session, "no room to record the session's project"),
        }
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

    /// Delete a session for good: its runs, what they recorded, and the
    /// session, even one with no run. A client still running is stopped
    /// first, and the delete waits until its run's end and its calls are
    /// recorded and the supervisor has let its process go, so nothing it
    /// was doing lands after the session is gone; refused only when they
    /// are not within the wait. The supervisor stops serving it at once;
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
        let slot = api
            .control
            .recorder
            .history_slot()
            .ok_or(ControlRefusal::Unrecorded)?;
        let served = api.stop_to_delete(supervisor, session, subject).await?;
        if target.is_none() && !served {
            return Err(ControlRefusal::NotFound);
        }
        slot.record(actor, subject, HistoryChange::Delete);
        api.forget_previews(session).await;
        Ok(Self::done())
    }

    /// How long a delete waits for a stopped client's run to end and its
    /// calls to close: this many reads, this far apart.
    const STOP_READS: u32 = 100;
    const STOP_READ_EVERY: Duration = Duration::from_millis(100);

    /// Stop the client serving `session`, if any, wait until the store
    /// holds no run of it running and none of its calls open, and have the
    /// supervisor stop serving it: whether it served it, else why it cannot
    /// be deleted yet. The supervisor, not the store, says whether a client
    /// runs: the store records a run's start and end after the supervisor
    /// sees them, so it can show none running while the process is served.
    async fn stop_to_delete(
        &self,
        supervisor: &Supervisor,
        session: SessionId,
        subject: HistorySubject,
    ) -> Result<bool, ControlRefusal> {
        match supervisor.send(session, RunCommand::Stop).await {
            // Already on its way out, gone, or never started: there is only
            // the wait.
            Ok(())
            | Err(
                SessionError::NoProcess
                | SessionError::NoSuchSession(_)
                | SessionError::Command(CommandError::Refused(
                    RunRefused::Stopping | RunRefused::Ended,
                )),
            ) => {}
            Err(error) => return Err(Self::refused(error)),
        }
        let mut left = ControlRefusal::RunRunning;
        for _ in 0..Self::STOP_READS {
            let target = self
                .reader
                .history_target(subject)
                .await
                .map_err(|_| ControlRefusal::Unavailable)?;
            left = match target {
                Some(target) if target.running_runs > 0 => ControlRefusal::RunRunning,
                Some(target) if target.open_calls > 0 => ControlRefusal::CallOpen,
                Some(_) | None => match supervisor.forget(session).await {
                    Ok(served) => return Ok(served),
                    Err(SessionError::Running) => ControlRefusal::RunRunning,
                    Err(error) => return Err(Self::refused(error)),
                },
            };
            tokio::time::sleep(Self::STOP_READ_EVERY).await;
        }
        Err(left)
    }

    /// Takes the preview off each work item `session` took up: deleting the
    /// session removed its worktree, and its preview with it, so a link to
    /// it would lead nowhere (or, later, to another worktree's).
    async fn forget_previews(&self, session: SessionId) {
        let items = match self
            .reader
            .work_items(PageLimit::LARGEST, Shelf::Current, None)
            .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(%session, %error, "a deleted session's previews were left on its work items");
                return;
            }
        };
        for item in items
            .into_iter()
            .filter(|item| item.session == Some(session) && item.preview.is_some())
        {
            match self.control.recorder.work_slot() {
                Some(slot) => slot.record(item.id, WorkFact::Previewed { preview: None }),
                None => {
                    tracing::warn!(item = %item.id, "no room to take a gone preview off a work item")
                }
            }
        }
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
        Ok(ControlApi::answer(FilesBody {
            files: files.into_iter().map(Into::into).collect(),
        }))
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
            ControlApi::answer(Attached {
                attachment: AttachmentEntry {
                    id: attachment,
                    read: read.into(),
                },
            }),
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
        Ok(ControlApi::answer(AttachmentsBody {
            attachments: kept.into_iter().map(Into::into).collect(),
        }))
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
                        Ok(snapshot) => Event::default()
                            .event("session")
                            .json_data(super::body::sessions::SessionSnapshot::from(snapshot))
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
