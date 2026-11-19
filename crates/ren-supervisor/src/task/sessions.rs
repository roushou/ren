//! What the task does for a session's requests: creating, prompting,
//! configuring, shelving, adopting, and forgetting sessions, and their
//! composer's files and attachments.

use super::{Composing, Reading, Served, SupervisorTask, Walked};
use crate::attachments::AttachmentUse;
use crate::host::Scratch;
use crate::workspace::{Slot, Workspace};
use crate::{
    Create, Reply, SessionError, SessionRequest, SessionSeed, StartError, StartPlace,
    files::{FileListing, FileMatch, FileQuery},
    host::AgentHost,
    run::{Delivery, RunCommand},
    session::{Session, SessionEffect, Shelving},
    workdir::{RequestedDir, Workdir},
};
use futures_util::{FutureExt, future::BoxFuture};
use ren_harness::{
    Harness,
    attachment::{Attachment, NotAccepted, Offer},
    prompt::{Prompt, PromptImage},
    settings::RunSettings,
};
use ren_recorder::{
    RunSession, SessionSlot,
    batch::SessionFact,
    id::{AttachmentId, RunId, SessionId},
};

impl<H: AgentHost> SupervisorTask<H> {
    /// Answer a session request, each in its own reply. A folder walk and
    /// an attachment read answer later, off the task.
    pub(super) async fn session(&self, request: SessionRequest, served: &mut Served) {
        match request {
            SessionRequest::Create { create, reply } => {
                let _ = reply.send(self.create(create, served).await);
            }
            SessionRequest::Adopt { seed, reply } => {
                let _ = reply.send(self.adopt(seed, served));
            }
            SessionRequest::Send {
                session,
                command,
                reply,
            } => {
                let _ = reply.send(self.send(session, command, served).await);
            }
            SessionRequest::ChooseClient {
                session,
                harness,
                reply,
            } => {
                let _ = reply.send(self.choose_client(session, harness, served).await);
            }
            SessionRequest::ChooseFolder {
                session,
                place,
                reply,
            } => {
                let _ = reply.send(self.choose_folder(session, place, served).await);
            }
            SessionRequest::Shelve {
                session,
                shelving,
                reply,
            } => {
                let _ = reply.send(self.shelve(session, shelving, served).await);
            }
            SessionRequest::Forget { session, reply } => {
                let _ = reply.send(self.forget(session, served).await);
            }
            SessionRequest::Attach {
                session,
                name,
                bytes,
                reply,
            } => {
                let _ = reply.send(self.attach(session, &name, bytes, served).await);
            }
            SessionRequest::Attachments { session, reply } => {
                let _ = reply.send(
                    served
                        .sessions
                        .get(&session)
                        .ok_or(SessionError::NoSuchSession(session))
                        .map(|serving| serving.attached().list()),
                );
            }
            SessionRequest::Prompt {
                session,
                text,
                attachments,
                delivery,
                reply,
            } => {
                let prompted = self
                    .prompt_with(session, text, attachments, delivery, served)
                    .await;
                let _ = reply.send(prompted);
            }
            SessionRequest::ReadAttachment {
                session,
                attachment,
                reply,
            } => match self.read_attachment(session, attachment, served) {
                Ok(reading) => served.replies.push(
                    async move {
                        let _ = reply.send(reading.await);
                    }
                    .boxed(),
                ),
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            SessionRequest::Detach {
                session,
                attachment,
                reply,
            } => {
                let _ = reply.send(self.detach(session, attachment, served).await);
            }
            SessionRequest::List { reply } => {
                let _ = reply.send(Ok(served
                    .sessions
                    .iter()
                    .map(|(session, serving)| serving.snapshot(*session))
                    .collect()));
            }
            SessionRequest::Files {
                session,
                query,
                limit,
                reply,
            } => {
                if let Some(walk) = self.search_files(session, query, limit, reply, served) {
                    served.walks.push(walk);
                }
            }
        }
    }

    /// Stop serving `session`, which is being deleted, with its attachments
    /// and ended runs; refused while a process serves it.
    pub(super) async fn forget(
        &self,
        session: SessionId,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        if served.process(session).is_some() {
            return Err(SessionError::Running);
        }
        // Its workspace goes with it, unless it holds changes not committed.
        let workspace = served.sessions.get(&session).and_then(|serving| {
            Workspace::of(self.bounds.worktrees()?, serving.workdir().as_path())
        });
        if let Some(workspace) = workspace
            && let Err(error) = self
                .off_task(move |host| host.remove_worktree(&workspace, &host.tools()))
                .await
        {
            tracing::warn!(%session, %error, "a deleted session's workspace was kept");
        }
        // Its attachments go with it, served or not.
        if let Some(folders) = &self.attachments {
            let folder = folders.session(session);
            self.off_task(move |host| host.remove_tree(&folder))
                .await
                .map_err(SessionError::AttachmentStorage)?;
        }
        if served.sessions.remove(&session).is_none() {
            return Err(SessionError::NoSuchSession(session));
        }
        served
            .runs
            .retain(|_, live| live.process.session() != session);
        Ok(())
    }

    /// Prompt `session` with `text` and its `attachments`; sent, they stay.
    pub(super) async fn prompt_with(
        &self,
        session: SessionId,
        text: String,
        attachments: Vec<AttachmentId>,
        delivery: Delivery,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let composing = self.compose(session, text, &attachments, served)?;
        let prompt = self.prompt(composing).await?;
        self.send(session, delivery.command(prompt), served).await?;
        if !attachments.is_empty() {
            self.record(
                session,
                SessionFact::AttachmentsSent {
                    attachments: attachments.clone(),
                },
            )?;
            if let Some(serving) = served.sessions.get_mut(&session) {
                serving.attached_mut().sent(&attachments);
            }
        }
        Ok(())
    }

    /// Remove one of `session`'s unsent attachments, its record, then its
    /// file.
    pub(super) async fn detach(
        &self,
        session: SessionId,
        attachment: AttachmentId,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let serving = served
            .sessions
            .get_mut(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        match serving.attached().used(attachment) {
            None => return Err(SessionError::NoSuchAttachment(attachment)),
            Some(AttachmentUse::Sent) => {
                return Err(SessionError::AttachmentSent(attachment));
            }
            Some(AttachmentUse::Unsent) => {}
        }
        self.record(session, SessionFact::Detached { attachment })?;
        serving.attached_mut().remove(attachment);
        // Detached as recorded; a file left behind is only space.
        let folder = folders.attachment(session, attachment);
        if let Err(error) = self.off_task(move |host| host.remove_tree(&folder)).await {
            tracing::warn!(%session, %attachment, %error, "a detached attachment's file stays");
        }
        Ok(())
    }

    pub(super) async fn create(
        &self,
        Create {
            harness,
            place,
            settings,
            resume,
        }: Create,
        served: &mut Served,
    ) -> Result<SessionId, SessionError> {
        let workdir = self.place(place).await.map_err(SessionError::Start)?;
        Self::drivable(harness, &settings).map_err(SessionError::Start)?;
        let session = SessionId::generate(self.ids.as_ref());
        self.record(
            session,
            SessionFact::Created {
                harness,
                workdir: workdir.as_str().to_owned(),
                settings: settings.clone(),
            },
        )?;
        // Its client starts with its first prompt; until then its commands
        // are read from the command book.
        let new = Session::new(harness, workdir, settings, resume);
        let _ = self
            .carry_out_session(session, new, Vec::new(), None, served)
            .await;
        Ok(session)
    }

    /// The prompt `text` and `attachments` make for `session`: each one the
    /// session's, together within the limits, of a kind its client takes,
    /// and read from its file.
    pub(super) fn compose(
        &self,
        session: SessionId,
        text: String,
        attachments: &[AttachmentId],
        served: &Served,
    ) -> Result<Composing, SessionError> {
        let held = served
            .sessions
            .get(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        if attachments.is_empty() {
            return Prompt::try_from(text)
                .map(Composing::Ready)
                .map_err(|_| SessionError::BlankPrompt);
        }
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let reads = attachments
            .iter()
            .map(|attachment| {
                held.attached()
                    .get(*attachment)
                    .cloned()
                    .ok_or(SessionError::NoSuchAttachment(*attachment))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.attachment_limits
            .check_prompt(&reads)
            .map_err(SessionError::PromptAttachments)?;
        // What the running client said; with none running, the next one says
        // as it opens, and its codec refuses what it was not offered.
        let offer = served
            .process(session)
            .and_then(|run| served.runs.get(&run))
            .map_or(Offer::Unknown, |live| live.process.image_offer());
        let mut files = Vec::with_capacity(reads.len());
        for (attachment, read) in attachments.iter().zip(reads) {
            match held.harness().acceptance().of(read.kind()).resolve(offer) {
                Ok(_) | Err(NotAccepted::Unknown) => {}
                Err(refused @ (NotAccepted::Never | NotAccepted::NotOffered)) => {
                    return Err(SessionError::ImagesNotTaken(refused));
                }
            }
            let path = folders.file(session, *attachment, read.name());
            files.push((read, path));
        }
        Ok(Composing::Images { text, files })
    }

    /// The prompt, its images read off the task.
    pub(super) async fn prompt(&self, composing: Composing) -> Result<Prompt, SessionError> {
        let (text, files) = match composing {
            Composing::Ready(prompt) => return Ok(prompt),
            Composing::Images { text, files } => (text, files),
        };
        let paths: Vec<_> = files.iter().map(|(_, path)| path.clone()).collect();
        let bytes = self
            .off_task(move |host| {
                paths
                    .iter()
                    .map(|path| host.read_file(path))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
            .map_err(SessionError::AttachmentStorage)?;
        let images = files
            .into_iter()
            .zip(bytes)
            .map(|((read, path), bytes)| PromptImage::new(read, path, bytes.into()))
            .collect::<Vec<_>>();
        Prompt::with_images(text, images).map_err(|_| SessionError::BlankPrompt)
    }

    /// Read `bytes`, uploaded as `name`, as an attachment of `session`,
    /// write it to the session's folder, and record it.
    pub(super) async fn attach(
        &self,
        session: SessionId,
        name: &str,
        bytes: Vec<u8>,
        served: &mut Served,
    ) -> Result<(AttachmentId, Attachment), SessionError> {
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let held = served
            .sessions
            .get_mut(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        let read = Attachment::parse(name, &bytes, &self.attachment_limits)
            .map_err(SessionError::Attachment)?;
        let attachment = AttachmentId::generate(self.ids.as_ref());
        let file = folders.file(session, attachment, read.name());
        let incoming = folders.incoming(attachment);
        let placing = file.clone();
        self.off_task(move |host| host.place_file(&incoming, &placing, &bytes))
            .await
            .map_err(SessionError::AttachmentStorage)?;
        // One that cannot be recorded is not kept.
        if let Err(error) = self.record(
            session,
            SessionFact::Attached {
                attachment,
                read: read.clone(),
            },
        ) {
            let folder = folders.attachment(session, attachment);
            let _ = self.off_task(move |host| host.remove_tree(&folder)).await;
            return Err(error);
        }
        held.attached_mut().insert(attachment, read.clone());
        Ok((attachment, read))
    }

    pub(super) fn adopt(&self, seed: SessionSeed, served: &mut Served) -> Result<(), SessionError> {
        if served.sessions.contains_key(&seed.session) {
            return Ok(());
        }
        let requested = seed
            .workdir
            .parse::<RequestedDir>()
            .map_err(|error| SessionError::Start(StartError::Workdir(error)))?;
        let workdir = self.confirm(&requested).map_err(SessionError::Start)?;
        let session = seed.session;
        let mut restored = Session::restore(seed, workdir);
        // A process taken up from its run host serves it again.
        if let Some(run) = served.rejoined.remove(&session) {
            restored.started(run);
        }
        self.publish_session(session, &restored);
        served.sessions.insert(session, restored);
        Ok(())
    }

    pub(super) async fn send(
        &self,
        session: SessionId,
        command: RunCommand,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let prompt = match &command {
            RunCommand::Prompt(prompt) | RunCommand::Queue(prompt) | RunCommand::Steer(prompt) => {
                Some(prompt.clone())
            }
            RunCommand::Unqueue
            | RunCommand::Configure(_)
            | RunCommand::Answer { .. }
            | RunCommand::Interrupt
            | RunCommand::Stop => None,
        };
        let (slot, room) = self.room();
        let outcome = match served.serving(&serving) {
            Some(run) => match served.runs.get_mut(&run) {
                None => Err(SessionError::NoProcess),
                Some(live) => match Self::command(live, command) {
                    Err(error) => Err(SessionError::Command(error)),
                    Ok(effects) => {
                        self.carry_out(live, effects, &mut served.idles);
                        serving.took(live.process.settings().clone());
                        match prompt {
                            Some(_) => serving.prompted(room),
                            None => Ok(Vec::new()),
                        }
                    }
                },
            },
            // No process: settings wait for the next one, and a prompt
            // starts one that resumes the conversation with it.
            None => match (command, prompt) {
                (RunCommand::Configure(settings), _) => serving.configure_unserved(settings, room),
                (_, Some(prompt)) => serving.prompt_unserved(prompt, room),
                (
                    RunCommand::Prompt(_)
                    | RunCommand::Queue(_)
                    | RunCommand::Steer(_)
                    | RunCommand::Unqueue
                    | RunCommand::Answer { .. }
                    | RunCommand::Interrupt
                    | RunCommand::Stop,
                    None,
                ) => Err(SessionError::NoProcess),
            },
        };
        match self
            .settle_session(session, serving, outcome, slot, served)
            .await?
        {
            Some(Err(error)) => Err(SessionError::Start(error)),
            Some(Ok(_)) | None => Ok(()),
        }
    }

    pub(super) async fn choose_client(
        &self,
        session: SessionId,
        harness: Harness,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        if !served.sessions.contains_key(&session) {
            return Err(SessionError::NoSuchSession(session));
        }
        Self::drivable(harness, &RunSettings::default()).map_err(SessionError::Start)?;
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let (slot, room) = self.room();
        let outcome = serving.choose_client(harness, room);
        self.settle_session(session, serving, outcome, slot, served)
            .await
            .map(|started| {
                if let Some(Err(error)) = started {
                    tracing::debug!(%session, %error, "the chosen client did not start");
                }
            })
    }

    pub(super) async fn choose_folder(
        &self,
        session: SessionId,
        place: StartPlace,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        if !served.sessions.contains_key(&session) {
            return Err(SessionError::NoSuchSession(session));
        }
        let workdir = self.place(place).await.map_err(SessionError::Start)?;
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let (slot, room) = self.room();
        let outcome = serving.choose_folder(workdir, room);
        self.settle_session(session, serving, outcome, slot, served)
            .await
            .map(|_| ())
    }

    pub(super) async fn shelve(
        &self,
        session: SessionId,
        shelving: Shelving,
        served: &mut Served,
    ) -> Result<(), SessionError> {
        let Some(mut serving) = served.sessions.remove(&session) else {
            return Err(SessionError::NoSuchSession(session));
        };
        let (slot, room) = self.room();
        let outcome = serving.shelve(shelving, room);
        self.settle_session(session, serving, outcome, slot, served)
            .await
            .map(|_| ())
    }

    /// Carry out a session change's effects, or serve it again unchanged
    /// when it was refused.
    pub(super) async fn settle_session(
        &self,
        session: SessionId,
        serving: Session,
        outcome: Result<Vec<SessionEffect>, SessionError>,
        slot: Option<SessionSlot>,
        served: &mut Served,
    ) -> Result<Option<Result<RunId, StartError>>, SessionError> {
        match outcome {
            Ok(effects) => Ok(self
                .carry_out_session(session, serving, effects, slot, served)
                .await),
            Err(refused) => {
                served.sessions.insert(session, serving);
                Err(refused)
            }
        }
    }

    /// Search the session's folder as last walked, answering now, or the
    /// walk off the task that answers once [`Self::walked`] takes it.
    /// Bounded by `FileListing::LIMITS`.
    #[must_use = "a walk answers its search only once it is taken"]
    pub(super) fn search_files(
        &self,
        session: SessionId,
        query: FileQuery,
        limit: usize,
        reply: Reply<Vec<FileMatch>>,
        served: &Served,
    ) -> Option<BoxFuture<'static, Walked>> {
        let Some(served_session) = served.sessions.get(&session) else {
            let _ = reply.send(Err(SessionError::NoSuchSession(session)));
            return None;
        };
        if let Some(files) = served_session.files() {
            let _ = reply.send(Ok(files.search(&query, limit)));
            return None;
        }
        let (workdir, turns_ended) = served_session.walk();
        let dir = workdir.as_path().to_path_buf();
        let walk = self.off_task(move |host| host.files(&dir, FileListing::LIMITS));
        Some(
            async move {
                Walked {
                    session,
                    turns_ended,
                    found: walk.await,
                    query,
                    limit,
                    reply,
                }
            }
            .boxed(),
        )
    }

    /// A walk finished: kept for later searches, unless a turn ended since
    /// it began, and the search that asked answered.
    pub(super) fn walked(walked: Walked, served: &mut Served) {
        let Walked {
            session,
            turns_ended,
            found,
            query,
            limit,
            reply,
        } = walked;
        let answer = found.map_err(SessionError::Unreadable).map(|found| {
            let listing = FileListing::new(found);
            let matches = listing.search(&query, limit);
            if let Some(served_session) = served.sessions.get_mut(&session) {
                served_session.walked(listing, turns_ended);
            }
            matches
        });
        let _ = reply.send(answer);
    }

    /// An attachment's bytes, read off the task.
    pub(super) fn read_attachment(
        &self,
        session: SessionId,
        attachment: AttachmentId,
        served: &Served,
    ) -> Result<Reading, SessionError> {
        let folders = self
            .attachments
            .as_ref()
            .ok_or(SessionError::NoAttachments)?;
        let held = served
            .sessions
            .get(&session)
            .ok_or(SessionError::NoSuchSession(session))?;
        let read = held
            .attached()
            .get(attachment)
            .ok_or(SessionError::NoSuchAttachment(attachment))?
            .clone();
        let path = folders.file(session, attachment, read.name());
        let reading = self.off_task(move |host| host.read_file(&path));
        Ok(async move {
            reading
                .await
                .map(|bytes| (read, bytes))
                .map_err(SessionError::AttachmentStorage)
        }
        .boxed())
    }

    /// The folder a session works in: a folder inside the bounds, or a new
    /// scratch folder of its own.
    pub(super) async fn place(&self, place: StartPlace) -> Result<Workdir, StartError> {
        // A registered project's repository is trusted; any other folder
        // must lie inside the bounds.
        let place_kind = match place {
            StartPlace::Project(_) => PlaceKind::Registered,
            StartPlace::Scratch | StartPlace::Folder(_) | StartPlace::Worktree(_) => {
                PlaceKind::Bounded
            }
        };
        let requested = match place {
            StartPlace::Folder(dir) => dir.parse::<RequestedDir>().map_err(StartError::Workdir)?,
            StartPlace::Scratch => {
                let scratch = self.bounds.scratch().ok_or(StartError::NoScratch)?;
                // Its last eight hex digits: random for real ids, and
                // distinct for sequential ones.
                let id = self.ids.next_uuid().simple().to_string();
                let folder = scratch.join(format!(
                    "scratch-{}",
                    id.get(id.len().saturating_sub(8)..).unwrap_or(&id)
                ));
                let making = folder.clone();
                let made = self
                    .off_task(move |host| host.make_scratch(&making, &host.tools()))
                    .await
                    .map_err(StartError::Scratch)?;
                match made {
                    Scratch::Repository => {}
                    Scratch::Untracked(why) => {
                        tracing::warn!(?folder, ?why, "a scratch folder is not a git repository");
                    }
                }
                folder
                    .to_str()
                    .ok_or(StartError::NoScratch)?
                    .parse::<RequestedDir>()
                    .map_err(StartError::Workdir)?
            }
            StartPlace::Worktree(repo) | StartPlace::Project(repo) => {
                let requested = repo.parse::<RequestedDir>().map_err(StartError::Workdir)?;
                let repo = match place_kind {
                    PlaceKind::Bounded => self.confirm(&requested)?,
                    PlaceKind::Registered => Workdir::registered(
                        self.host
                            .resolve(requested.as_path())
                            .map_err(StartError::Resolve)?,
                    )
                    .map_err(StartError::Workdir)?,
                };
                let root = self
                    .bounds
                    .worktrees()
                    .ok_or(StartError::NoWorktrees)?
                    .to_path_buf();
                let looking = root.clone();
                let look = self
                    .off_task(move |host| host.look(&looking))
                    .await
                    .map_err(StartError::Worktree)?;
                let slot = Slot::free(&Workspace::taken(look.folders.iter().map(String::as_str)))
                    .ok_or(StartError::NoSlot)?;
                let workspace =
                    Workspace::new(&root, repo.as_path(), slot).ok_or(StartError::NoWorktrees)?;
                let id = self.ids.next_uuid().simple().to_string();
                let branch = workspace.branch(id.get(id.len().saturating_sub(8)..).unwrap_or(&id));
                let (folder, data) = (workspace.folder(), workspace.data());
                let making = folder.clone();
                self.off_task(move |host| {
                    host.make_worktree(repo.as_path(), &making, &data, &branch, &host.tools())
                })
                .await
                .map_err(StartError::Worktree)?;
                folder
                    .to_str()
                    .ok_or(StartError::NoWorktrees)?
                    .parse::<RequestedDir>()
                    .map_err(StartError::Workdir)?
            }
        };
        self.confirm(&requested)
    }

    /// Carry out `serving`'s effects, with the slot its fact records in
    /// (each change records at most one), and serve it again. A process it
    /// started, or why it did not start.
    pub(super) async fn carry_out_session(
        &self,
        session: SessionId,
        mut serving: Session,
        effects: Vec<SessionEffect>,
        mut slot: Option<SessionSlot>,
        served: &mut Served,
    ) -> Option<Result<RunId, StartError>> {
        let mut started = None;
        for effect in effects {
            match effect {
                SessionEffect::Record(fact) => match slot.take() {
                    Some(slot) => slot.record(session, fact),
                    None => tracing::warn!(%session, "a session fact had no slot"),
                },
                SessionEffect::StopProcess => {
                    let run = served.serving(&serving);
                    self.stop_process(session, run, served);
                }
                SessionEffect::StartProcess { first } => {
                    started = Some(
                        self.spawn(
                            session,
                            &mut serving,
                            RunSession::Existing(session),
                            first,
                            served,
                        )
                        .await,
                    );
                }
            }
        }
        self.publish_session(session, &serving);
        served.sessions.insert(session, serving);
        started
    }
}

/// Whether a place's repository must lie inside the bounds.
#[derive(Debug, Clone, Copy)]
enum PlaceKind {
    Bounded,
    Registered,
}
