//! Where an operator's session stands: a draft until its first prompt goes
//! out, active after, and archived when the operator shelves it. Pure: the
//! supervisor applies each input and carries out what it returns.
//!
//! A session is not its process. Its client starts with its first prompt,
//! idles, and ends any number of times (`run`); the session goes on until
//! it is archived, and comes back when it is written to again. A draft runs
//! no client until it is asked something, and its client and folder can
//! change until then.

use crate::attachments::Attached;
use crate::files::FileListing;
use crate::workdir::Workdir;
use crate::{ImageTaking, SessionError, SessionSeed, SessionSnapshot};
use ren_harness::Harness;
use ren_harness::models::ModelId;
use ren_harness::prompt::Prompt;
use ren_harness::settings::RunSettings;
use ren_protocol::native::NativeThreadId;
use ren_recorder::batch::SessionFact;
use ren_recorder::id::{RunId, SessionId};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Whether a prompt has gone out in the session.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum SessionPhase {
    /// Nothing sent yet: its client can still be changed.
    Draft,
    /// A prompt went out: its client is fixed.
    Active,
}

/// Where a session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Listed with the current sessions.
    Open(SessionPhase),
    /// Shelved by the operator, in the phase it had.
    Archived(SessionPhase),
}

/// What happens to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionInput {
    /// The operator chose the client it works with.
    ChooseClient(Harness),
    /// The operator chose the folder it works in.
    ChooseFolder,
    /// A prompt went out to its client.
    Prompted,
    Archive,
    Unarchive,
}

/// The operator shelving a session, or taking it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelving {
    Archive,
    Unarchive,
}

impl Shelving {
    pub fn input(self) -> SessionInput {
        match self {
            Self::Archive => SessionInput::Archive,
            Self::Unarchive => SessionInput::Unarchive,
        }
    }
}

/// What the supervisor does for a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    /// Run with this client from its first process on.
    UseClient(Harness),
    /// Work in the folder chosen from its first process on.
    UseFolder,
    /// End the client process serving the session, if one runs: nothing is
    /// served while it is shelved.
    StopProcess,
}

/// Why an input was refused in the session's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionRefused {
    #[error("the session's client is fixed once a prompt went out")]
    ClientFixed,
    #[error("the session's folder is fixed once a prompt went out")]
    FolderFixed,
    #[error("the session is archived")]
    Archived,
    #[error("the session is not archived")]
    NotArchived,
}

impl SessionState {
    /// A session as it is created: an open draft.
    pub const NEW: Self = Self::Open(SessionPhase::Draft);

    pub fn phase(self) -> SessionPhase {
        match self {
            Self::Open(phase) | Self::Archived(phase) => phase,
        }
    }

    /// A prompt went out. Every session takes one: written to, a session is
    /// current again, as a recording with new traffic is.
    pub fn prompted(self) -> Self {
        Self::Open(SessionPhase::Active)
    }

    /// The state after `input`, and what to do for it.
    pub fn apply(self, input: SessionInput) -> Result<(Self, Vec<SessionAction>), SessionRefused> {
        match (self, input) {
            (Self::Open(SessionPhase::Draft), SessionInput::ChooseClient(harness)) => {
                Ok((self, vec![SessionAction::UseClient(harness)]))
            }
            (Self::Open(SessionPhase::Active), SessionInput::ChooseClient(_)) => {
                Err(SessionRefused::ClientFixed)
            }
            (Self::Archived(_), SessionInput::ChooseClient(_)) => Err(SessionRefused::Archived),
            (Self::Open(SessionPhase::Draft), SessionInput::ChooseFolder) => {
                Ok((self, vec![SessionAction::UseFolder]))
            }
            (Self::Open(SessionPhase::Active), SessionInput::ChooseFolder) => {
                Err(SessionRefused::FolderFixed)
            }
            (Self::Archived(_), SessionInput::ChooseFolder) => Err(SessionRefused::Archived),
            // Written to, a session is current again, as a recording with
            // new traffic is.
            (Self::Open(_) | Self::Archived(_), SessionInput::Prompted) => {
                Ok((self.prompted(), Vec::new()))
            }
            (Self::Open(phase), SessionInput::Archive) => {
                Ok((Self::Archived(phase), vec![SessionAction::StopProcess]))
            }
            (Self::Archived(_), SessionInput::Archive) => Err(SessionRefused::Archived),
            (Self::Archived(phase), SessionInput::Unarchive) => Ok((Self::Open(phase), Vec::new())),
            (Self::Open(_), SessionInput::Unarchive) => Err(SessionRefused::NotArchived),
        }
    }
}

/// Whether the recorder has room for a session's fact. A change whose fact
/// cannot be recorded is not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Room {
    Recordable,
    Full,
}

/// What the task does for a session.
#[derive(Debug)]
pub(crate) enum SessionEffect {
    /// Record this fact (room for it was checked).
    Record(SessionFact),
    /// End the process serving it, if one runs.
    StopProcess,
    /// Start a process for it, sent `first` once its conversation opens.
    StartProcess { first: Option<Prompt> },
}

/// How many turns of a session ended: a folder walk begun before a turn
/// ended is stale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TurnsEnded(u64);

/// What a session's next process starts with.
#[derive(Debug, Clone)]
pub(crate) struct Launch {
    pub harness: Harness,
    pub workdir: Workdir,
    pub settings: RunSettings,
    /// The conversation it resumes, when one ran a turn.
    pub resume: Option<NativeThreadId>,
}

/// A session as the supervisor serves it, pure: where it stands, its
/// client and folder, what its next turn runs on, the conversation it goes
/// on with, its newest process, its folder as last walked, and its
/// attachments. Each change is checked first and refused unchanged; what
/// to do for it comes back as [`SessionEffect`]s.
#[derive(Debug)]
pub(crate) struct Session {
    state: SessionState,
    harness: Harness,
    workdir: Workdir,
    settings: RunSettings,
    /// The client's conversation, once a turn ran in it: the next process
    /// resumes it.
    thread: Option<NativeThreadId>,
    process: Option<RunId>,
    /// Its folder as walked for the last file search, until a turn ends:
    /// turns change files.
    files: Option<FileListing>,
    turns_ended: TurnsEnded,
    attached: Attached,
}

impl Session {
    /// A new draft.
    pub(crate) fn new(
        harness: Harness,
        workdir: Workdir,
        settings: RunSettings,
        resume: Option<NativeThreadId>,
    ) -> Self {
        Self {
            state: SessionState::NEW,
            harness,
            workdir,
            settings,
            thread: resume,
            process: None,
            files: None,
            turns_ended: TurnsEnded::default(),
            attached: Attached::default(),
        }
    }

    /// A session the store holds, served again.
    pub(crate) fn restore(seed: SessionSeed, workdir: Workdir) -> Self {
        Self {
            state: seed.state,
            harness: seed.harness,
            workdir,
            settings: seed.settings,
            thread: seed.thread,
            process: None,
            files: None,
            turns_ended: TurnsEnded::default(),
            attached: Attached::from_stored(seed.attachments),
        }
    }

    pub(crate) fn snapshot(&self, session: SessionId) -> SessionSnapshot {
        SessionSnapshot {
            session,
            harness: self.harness,
            workdir: self.workdir.as_path().to_path_buf(),
            settings: self.settings.clone(),
            state: self.state,
            process: self.process,
            images: ImageTaking::of(self.harness),
        }
    }

    pub(crate) fn harness(&self) -> Harness {
        self.harness
    }

    pub(crate) fn workdir(&self) -> &Workdir {
        &self.workdir
    }

    pub(crate) fn process(&self) -> Option<RunId> {
        self.process
    }

    pub(crate) fn attached(&self) -> &Attached {
        &self.attached
    }

    pub(crate) fn attached_mut(&mut self) -> &mut Attached {
        &mut self.attached
    }

    /// What its next process starts with. A client that changes its own
    /// default when launched starts on the one it `listed`, by name, when
    /// the session chose none; the session keeps that choice.
    pub(crate) fn launch(&mut self, listed: Option<&ModelId>) -> Launch {
        if self.settings.model.is_none() && self.harness.names_its_default() {
            self.settings.model = listed.cloned();
        }
        Launch {
            harness: self.harness,
            workdir: self.workdir.clone(),
            settings: self.settings.clone(),
            resume: self.thread,
        }
    }

    /// `run` is its newest process.
    pub(crate) fn started(&mut self, run: RunId) {
        self.process = Some(run);
    }

    /// Its process took these settings.
    pub(crate) fn took(&mut self, settings: RunSettings) {
        self.settings = settings;
    }

    /// A prompt went out to its running process.
    pub(crate) fn prompted(&mut self, room: Room) -> Result<Vec<SessionEffect>, SessionError> {
        let fact = match self.state {
            SessionState::Archived(_) => Some(SessionFact::Unarchived),
            SessionState::Open(_) => None,
        };
        let effects = Self::recording(fact, room)?;
        self.state = self.state.prompted();
        Ok(effects)
    }

    /// A prompt for a session with no process: a new one resumes its
    /// conversation with it.
    pub(crate) fn prompt_unserved(
        &mut self,
        prompt: Prompt,
        room: Room,
    ) -> Result<Vec<SessionEffect>, SessionError> {
        let mut effects = self.prompted(room)?;
        effects.push(SessionEffect::StartProcess {
            first: Some(prompt),
        });
        Ok(effects)
    }

    /// Settings for a session with no process, from its next one on.
    pub(crate) fn configure_unserved(
        &mut self,
        settings: RunSettings,
        room: Room,
    ) -> Result<Vec<SessionEffect>, SessionError> {
        let effects = Self::recording(
            Some(SessionFact::Configured {
                settings: settings.clone(),
            }),
            room,
        )?;
        self.settings = settings;
        Ok(effects)
    }

    /// The operator chose a draft's client: any process of the old one
    /// goes, with its conversation and settings; the new client starts with
    /// the first prompt.
    pub(crate) fn choose_client(
        &mut self,
        harness: Harness,
        room: Room,
    ) -> Result<Vec<SessionEffect>, SessionError> {
        let (state, actions) = self
            .state
            .apply(SessionInput::ChooseClient(harness))
            .map_err(SessionError::Refused)?;
        let mut effects = Self::recording(Some(SessionFact::ClientChosen { harness }), room)?;
        self.state = state;
        effects.extend(self.act(actions));
        Ok(effects)
    }

    /// The operator chose a draft's folder: any process goes, with its
    /// conversation and its folder's listing; the next starts there.
    pub(crate) fn choose_folder(
        &mut self,
        workdir: Workdir,
        room: Room,
    ) -> Result<Vec<SessionEffect>, SessionError> {
        let (state, actions) = self
            .state
            .apply(SessionInput::ChooseFolder)
            .map_err(SessionError::Refused)?;
        let mut effects = Self::recording(
            Some(SessionFact::FolderChosen {
                workdir: workdir.as_str().to_owned(),
            }),
            room,
        )?;
        self.state = state;
        self.workdir = workdir;
        effects.extend(self.act(actions));
        Ok(effects)
    }

    /// The operator archived or unarchived it.
    pub(crate) fn shelve(
        &mut self,
        shelving: Shelving,
        room: Room,
    ) -> Result<Vec<SessionEffect>, SessionError> {
        let (state, actions) = self
            .state
            .apply(shelving.input())
            .map_err(SessionError::Refused)?;
        let fact = match shelving {
            Shelving::Archive => SessionFact::Archived,
            Shelving::Unarchive => SessionFact::Unarchived,
        };
        let mut effects = Self::recording(Some(fact), room)?;
        self.state = state;
        effects.extend(self.act(actions));
        Ok(effects)
    }

    /// A turn started in its process: the conversation that process
    /// opened is kept from now on.
    pub(crate) fn turn_started(&mut self, conversation: Option<NativeThreadId>) {
        if let Some(thread) = conversation {
            self.thread = Some(thread);
        }
    }

    /// A turn ended: its folder's listing is stale.
    pub(crate) fn turn_ended(&mut self) {
        self.files = None;
        self.turns_ended = TurnsEnded(self.turns_ended.0.wrapping_add(1));
    }

    /// Its folder as last walked, if it is still current.
    pub(crate) fn files(&self) -> Option<&FileListing> {
        self.files.as_ref()
    }

    /// The folder to walk, and when the walk began.
    pub(crate) fn walk(&self) -> (&Workdir, TurnsEnded) {
        (&self.workdir, self.turns_ended)
    }

    /// A walk begun at `since` finished: kept, unless a turn ended since.
    pub(crate) fn walked(&mut self, listing: FileListing, since: TurnsEnded) {
        if since == self.turns_ended {
            self.files = Some(listing);
        }
    }

    /// The facts to record, or `Unrecorded` when there is no room.
    fn recording(
        fact: Option<SessionFact>,
        room: Room,
    ) -> Result<Vec<SessionEffect>, SessionError> {
        match (fact, room) {
            (None, Room::Recordable | Room::Full) => Ok(Vec::new()),
            (Some(fact), Room::Recordable) => Ok(vec![SessionEffect::Record(fact)]),
            (Some(_), Room::Full) => Err(SessionError::Unrecorded),
        }
    }

    /// What the session machine's actions mean here.
    fn act(&mut self, actions: Vec<SessionAction>) -> Vec<SessionEffect> {
        actions
            .into_iter()
            .flat_map(|action| match action {
                SessionAction::UseClient(harness) => {
                    // Any process of the old client goes, with its
                    // conversation and settings.
                    self.harness = harness;
                    self.settings = RunSettings::default();
                    self.thread = None;
                    vec![SessionEffect::StopProcess]
                }
                SessionAction::UseFolder => {
                    // The folder is set by the caller; any process and
                    // conversation of the old one go.
                    self.thread = None;
                    self.files = None;
                    vec![SessionEffect::StopProcess]
                }
                SessionAction::StopProcess => vec![SessionEffect::StopProcess],
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(state: SessionState, inputs: &[SessionInput]) -> SessionState {
        inputs.iter().fold(state, |state, input| {
            state.apply(*input).map(|(next, _)| next).unwrap_or(state)
        })
    }

    #[test]
    fn a_draft_takes_a_client_until_its_first_prompt_fixes_it() {
        let draft = SessionState::NEW;
        assert_eq!(
            draft.apply(SessionInput::ChooseClient(Harness::Pi)),
            Ok((draft, vec![SessionAction::UseClient(Harness::Pi)]))
        );
        let (active, actions) = draft.apply(SessionInput::Prompted).unwrap();
        assert_eq!(active, SessionState::Open(SessionPhase::Active));
        assert!(actions.is_empty());
        assert_eq!(
            active.apply(SessionInput::ChooseClient(Harness::Codex)),
            Err(SessionRefused::ClientFixed)
        );
        // Prompting again keeps it active.
        assert_eq!(
            active.apply(SessionInput::Prompted),
            Ok((active, Vec::new()))
        );
    }

    #[test]
    fn archiving_stops_the_process_and_keeps_the_phase() {
        for phase in [SessionPhase::Draft, SessionPhase::Active] {
            let open = SessionState::Open(phase);
            assert_eq!(
                open.apply(SessionInput::Archive),
                Ok((
                    SessionState::Archived(phase),
                    vec![SessionAction::StopProcess]
                ))
            );
            let archived = SessionState::Archived(phase);
            assert_eq!(
                archived.apply(SessionInput::Unarchive),
                Ok((open, Vec::new()))
            );
            assert_eq!(archived.phase(), phase);
        }
    }

    #[test]
    fn illegal_inputs_are_refused_with_why() {
        let archived = SessionState::Archived(SessionPhase::Draft);
        assert_eq!(
            archived.apply(SessionInput::Archive),
            Err(SessionRefused::Archived)
        );
        assert_eq!(
            archived.apply(SessionInput::ChooseClient(Harness::Pi)),
            Err(SessionRefused::Archived)
        );
        assert_eq!(
            SessionState::NEW.apply(SessionInput::Unarchive),
            Err(SessionRefused::NotArchived)
        );
    }

    #[test]
    fn writing_to_an_archived_session_brings_it_back_active() {
        let state = after(
            SessionState::NEW,
            &[SessionInput::Archive, SessionInput::Prompted],
        );
        assert_eq!(state, SessionState::Open(SessionPhase::Active));
    }
}

#[cfg(test)]
mod served {
    use super::*;
    use crate::files::{FoundPath, PathKind};
    use crate::workdir::{Bounds, Roots};
    use std::path::PathBuf;

    fn draft() -> Session {
        let bounds = Bounds::new(
            Roots::new(vec![PathBuf::from("/work")]).unwrap(),
            Vec::new(),
            None,
            None,
        )
        .unwrap();
        let workdir = Workdir::confirm(&bounds, PathBuf::from("/work/acme")).unwrap();
        Session::new(Harness::Codex, workdir, RunSettings::default(), None)
    }

    fn prompt() -> Prompt {
        Prompt::try_from("fix it".to_owned()).unwrap()
    }

    #[test]
    fn a_change_without_room_for_its_fact_is_refused_unchanged() {
        let mut session = draft();
        assert!(matches!(
            session.shelve(Shelving::Archive, Room::Full),
            Err(SessionError::Unrecorded)
        ));
        assert_eq!(session.state, SessionState::NEW);
        assert!(matches!(
            session.choose_client(Harness::Pi, Room::Full),
            Err(SessionError::Unrecorded)
        ));
        assert_eq!(session.harness(), Harness::Codex);
        // A prompt to an open session records nothing, so needs no room.
        assert!(session.prompted(Room::Full).unwrap().is_empty());
        assert_eq!(session.state, SessionState::Open(SessionPhase::Active));
    }

    #[test]
    fn prompting_an_archived_session_records_its_return() {
        let mut session = draft();
        session.shelve(Shelving::Archive, Room::Recordable).unwrap();
        assert!(matches!(
            session.prompted(Room::Full),
            Err(SessionError::Unrecorded)
        ));
        let effects = session.prompt_unserved(prompt(), Room::Recordable).unwrap();
        assert!(matches!(
            effects.as_slice(),
            [
                SessionEffect::Record(SessionFact::Unarchived),
                SessionEffect::StartProcess { first: Some(_) }
            ]
        ));
        assert_eq!(session.state, SessionState::Open(SessionPhase::Active));
    }

    #[test]
    fn choosing_a_draft_folder_moves_it_and_starts_nothing() {
        let mut session = draft();
        session.turn_started(Some(NativeThreadId::new(uuid::Uuid::from_u128(7)).unwrap()));
        let bounds = Bounds::new(
            Roots::new(vec![PathBuf::from("/work")]).unwrap(),
            Vec::new(),
            None,
            None,
        )
        .unwrap();
        let elsewhere = Workdir::confirm(&bounds, PathBuf::from("/work/other")).unwrap();
        let effects = session
            .choose_folder(elsewhere.clone(), Room::Recordable)
            .unwrap();
        assert!(matches!(
            effects.as_slice(),
            [
                SessionEffect::Record(SessionFact::FolderChosen { workdir }),
                SessionEffect::StopProcess,
            ] if workdir == "/work/other"
        ));
        let launch = session.launch(None);
        assert_eq!(launch.workdir, elsewhere);
        assert_eq!(
            launch.resume, None,
            "the old folder's conversation stays there"
        );
        assert!(matches!(
            session.choose_folder(elsewhere.clone(), Room::Full),
            Err(SessionError::Unrecorded)
        ));
        // Once asked something, its folder is fixed.
        session.prompted(Room::Recordable).unwrap();
        assert!(matches!(
            session.choose_folder(elsewhere, Room::Recordable),
            Err(SessionError::Refused(SessionRefused::FolderFixed))
        ));
    }

    #[test]
    fn choosing_a_client_resets_what_the_old_one_had_and_starts_nothing() {
        let mut session = draft();
        session.turn_started(Some(NativeThreadId::new(uuid::Uuid::from_u128(7)).unwrap()));
        session.took(RunSettings {
            model: Some("gpt-fast".parse().unwrap()),
            effort: None,
            approvals: None,
        });
        let effects = session
            .choose_client(Harness::Pi, Room::Recordable)
            .unwrap();
        assert!(matches!(
            effects.as_slice(),
            [
                SessionEffect::Record(SessionFact::ClientChosen {
                    harness: Harness::Pi
                }),
                SessionEffect::StopProcess,
            ]
        ));
        let launch = session.launch(None);
        assert_eq!(launch.harness, Harness::Pi);
        assert_eq!(launch.settings, RunSettings::default());
        assert_eq!(launch.resume, None);
        // Once asked something, its client is fixed.
        session.prompted(Room::Recordable).unwrap();
        assert!(matches!(
            session.choose_client(Harness::Codex, Room::Recordable),
            Err(SessionError::Refused(SessionRefused::ClientFixed))
        ));
    }

    #[test]
    fn a_walk_begun_before_a_turn_ended_is_not_kept() {
        let listing = || {
            FileListing::new(vec![FoundPath {
                path: "src/lib.rs".into(),
                kind: PathKind::File,
            }])
        };
        let mut session = draft();
        let (_, since) = session.walk();
        session.turn_ended();
        session.walked(listing(), since);
        assert!(session.files().is_none());
        let (_, since) = session.walk();
        session.walked(listing(), since);
        assert!(session.files().is_some());
        session.turn_ended();
        assert!(session.files().is_none());
    }

    #[test]
    fn a_client_that_names_its_default_starts_on_the_one_it_listed() {
        let listed: ModelId = "free-model".parse().unwrap();
        let mut session = draft();
        let codex = session.launch(Some(&listed));
        assert_eq!(
            codex.settings.model.is_some(),
            Harness::Codex.names_its_default()
        );
        let mut chosen = draft();
        chosen.took(RunSettings {
            model: Some("gpt-fast".parse().unwrap()),
            effort: None,
            approvals: None,
        });
        assert_eq!(
            chosen.launch(Some(&listed)).settings.model,
            Some("gpt-fast".parse().unwrap())
        );
    }
}
