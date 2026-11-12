//! Everything the terminal has read from its gateway, in the one place every
//! screen reads it from: the sessions and their model calls (`sessions`),
//! the daemon's runs with their live conversations (`desk`, `transcript`),
//! and the projects and clients a run can start with.
//!
//! [`Model::apply`] takes one [`Input`] and answers with what it calls for:
//! reads, commands (a prompt kept for a run that now waits for one), and
//! news the screen shown may act on.

pub(crate) mod call;
pub(crate) mod desk;
pub(crate) mod sessions;
pub(crate) mod transcript;

pub use call::CallEnd;
pub use sessions::{Snapshot, SnapshotQuery};

use crate::Size;
use desk::{Attention, Desk, Run};
use ren_harness::{
    Harness,
    drive::{ApprovalId, Decision, Prompt},
    settings::{ApprovalPolicy, RunSettings},
};
use ren_protocol::label::{Labeled, Stored};
use ren_protocol::native::NativeThreadId;
use ren_recorder::Observation;
use ren_recorder::batch::{HistoryChange, HistorySubject};
use ren_recorder::id::{CallId, RunId, TurnId};
use ren_recorder::progress::Progress;
use ren_store::activity::{CallActivity, TurnActivity};
use ren_store::view::{ConversationView, RunView};
use ren_supervisor::{Client, ClientStatus, Places, Project, RunSnapshot, RunState, RunUpdate};
use sessions::Sessions;
use std::collections::HashMap;

/// What was read, or what happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Snapshot(Snapshot),
    /// A turn's activity, as asked for with [`Read::Turn`].
    Turn(Box<TurnActivity>),
    /// A call's activity, as asked for with [`Read::Call`].
    Call(Box<CallActivity>),
    /// An update on a streaming call.
    Progress(Progress),
    Observation(Observation),
    /// A read of sessions failed: why. What was read last stays, said to be
    /// stale until a snapshot succeeds.
    ReadFailed(String),
    /// The daemon's live runs, or why they cannot be read.
    Runs(Result<Vec<RunSnapshot>, String>),
    /// Recorded runs, newest first.
    Recorded(Vec<RunView>),
    /// Where runs can start, and the scratch folder, or why not read.
    Projects(Result<Places, String>),
    /// The clients and whether a run can start with each, or why they
    /// cannot be read.
    Clients(Result<Vec<Client>, String>),
    /// The models a client offers, as it lists them, or why it did not.
    Models {
        harness: Harness,
        models: Result<Vec<ren_harness::models::Model>, String>,
    },
    /// One update from the run feed.
    Update(RunUpdate),
    /// The run feed skipped updates: the runs are read again.
    Lagged,
    /// A run's conversation, as the daemon stored it.
    Conversation {
        run: RunId,
        conversation: ConversationView,
    },
    /// How a command was answered.
    Answered(Result<Answered, String>),
    Resize(Size),
}

/// How the daemon took a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    Sent,
    Started(RunId),
}

/// A read the terminal needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// A new [`Snapshot`], from [`Model::snapshot_query`].
    Snapshot,
    Turn(TurnId),
    /// One call's activity, for a session whose client sends no turns.
    Call(CallId),
    /// The daemon's live runs, through its control API.
    Runs,
    Recorded,
    Projects,
    Clients,
    /// The models a client offers, through the control API.
    Models(Harness),
    /// A run's conversation, whole, as the daemon stored it.
    Conversation(RunId),
}

/// A command for the control API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start {
        harness: Harness,
        /// None: a new scratch folder of the run's own.
        dir: Option<String>,
        resume: Option<NativeThreadId>,
        prompt: Option<Prompt>,
        /// Each left out is the client's own.
        settings: RunSettings,
    },
    Prompt {
        run: RunId,
        prompt: Prompt,
    },
    Answer {
        run: RunId,
        approval: ApprovalId,
        decision: Decision,
    },
    Interrupt(RunId),
    Stop(RunId),
    /// Archive, unarchive, or delete what was recorded of a session or run.
    Edit {
        subject: HistorySubject,
        change: HistoryChange,
    },
}

/// What an input calls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    Read(Read),
    Send(Command),
    /// A run asked for here started.
    Started(RunId),
    /// The daemon refused a command, in words.
    Refused(String),
}

/// The projects runs start in, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Projects {
    Unread,
    Read(Vec<Project>),
    Failed(String),
}

/// A client's models, as far as they are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Offered {
    Asked,
    Listed(Vec<ren_harness::models::Model>),
    Failed(String),
}

/// Whether runs can be driven from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Control {
    Unread,
    Ready,
    /// Why not: no control access, or runs are off on the daemon.
    Unavailable(String),
}

/// A run asked for, to hold as soon as the daemon names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Starting {
    harness: Harness,
    /// None: a scratch folder, named once the daemon lists the run.
    dir: Option<String>,
    resumes: bool,
    prompt: Option<Prompt>,
}

/// What was read, kept for every screen.
#[derive(Debug)]
pub(crate) struct Model {
    pub sessions: Sessions,
    pub desk: Desk,
    pub projects: Projects,
    /// The clients as the daemon has them; empty until read.
    clients: Vec<Client>,
    pub control: Control,
    /// The client a new run starts with: the last one started.
    pub client: Harness,
    /// The folder scratch runs get theirs in, once read; none: no scratch.
    pub scratch: Option<String>,
    /// Each client's models, once asked for.
    models: HashMap<Harness, Offered>,
    /// The settings each client last started a run with, while this app runs.
    pub chosen: HashMap<Harness, RunSettings>,
    /// The run asked to start here, until the daemon answers.
    starting: Option<Starting>,
}

impl Model {
    pub fn new() -> Self {
        Self {
            sessions: Sessions::default(),
            desk: Desk::default(),
            projects: Projects::Unread,
            clients: Vec::new(),
            control: Control::Unread,
            client: Harness::ClaudeCode,
            scratch: None,
            models: HashMap::new(),
            chosen: HashMap::new(),
            starting: None,
        }
    }

    /// The reads for a snapshot at `now`.
    pub fn snapshot_query(now: ren_recorder::time::UnixMillis) -> SnapshotQuery {
        Sessions::query(now)
    }

    pub fn projects(&self) -> &[Project] {
        match &self.projects {
            Projects::Read(projects) => projects,
            Projects::Unread | Projects::Failed(_) => &[],
        }
    }

    /// The clients a run can be started with, ready or not: as the daemon
    /// said, or, until it has, every client Ren drives.
    pub fn clients(&self) -> Vec<Client> {
        if self.clients.is_empty() {
            Harness::ALL
                .iter()
                .filter(|harness| harness.driven_program().is_ok())
                .map(|&harness| Client {
                    harness,
                    status: ClientStatus::Ready,
                    approvals: ApprovalPolicy::offered(harness).to_vec(),
                })
                .collect()
        } else {
            self.clients
                .iter()
                .filter(|client| match client.status {
                    ClientStatus::Ready | ClientStatus::NotInstalled => true,
                    ClientStatus::NotDrivable => false,
                })
                .cloned()
                .collect()
        }
    }

    /// Where a run works, as the operator would say it: Scratch for a
    /// scratch run, else its folder as `~/…` where a place read shows the
    /// home.
    pub fn place(&self, workdir: &str) -> String {
        if let Some(scratch) = &self.scratch
            && workdir
                .strip_prefix(scratch.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        {
            return "Scratch".to_owned();
        }
        if let Some(known) = self.projects().iter().find(|place| place.path == workdir) {
            return known.shown.clone();
        }
        // The home, as a place read under it shows it.
        let home = self.projects().iter().find_map(|place| {
            let rest = place.shown.strip_prefix('~')?;
            place.path.strip_suffix(rest)
        });
        match home.and_then(|home| workdir.strip_prefix(home)) {
            Some(rest) if rest.starts_with('/') => format!("~{rest}"),
            Some(_) | None => workdir.to_owned(),
        }
    }

    /// What is known of `harness`'s models.
    pub fn models(&self, harness: Harness) -> Option<&Offered> {
        self.models.get(&harness)
    }

    /// The read for `harness`'s models, the first time they are wanted, or
    /// again after a listing that failed or was empty.
    pub fn ask_models(&mut self, harness: Harness) -> Option<Read> {
        match self.models.get(&harness) {
            Some(Offered::Asked) => None,
            Some(Offered::Listed(models)) if !models.is_empty() => None,
            Some(Offered::Listed(_) | Offered::Failed(_)) | None => {
                self.models.insert(harness, Offered::Asked);
                Some(Read::Models(harness))
            }
        }
    }

    pub fn apply(&mut self, input: Input) -> Vec<Effect> {
        let reads = |reads: Vec<Read>| reads.into_iter().map(Effect::Read).collect();
        match input {
            Input::Snapshot(snapshot) => {
                self.sessions.take(snapshot);
                Vec::new()
            }
            Input::Turn(activity) => {
                self.sessions.take_turn(*activity);
                Vec::new()
            }
            Input::Call(activity) => {
                self.sessions.take_call(*activity);
                Vec::new()
            }
            Input::Progress(progress) => {
                self.sessions.take_progress(progress);
                Vec::new()
            }
            Input::Observation(observation) => reads(self.sessions.observe(&observation)),
            Input::ReadFailed(why) => {
                self.sessions.failed(why);
                Vec::new()
            }
            Input::Runs(Ok(snapshots)) => {
                self.control = Control::Ready;
                self.desk.live(&snapshots);
                let mut effects = self.release_queued();
                effects.extend(self.name_runs());
                effects
            }
            Input::Runs(Err(why)) => {
                self.control = Control::Unavailable(why);
                Vec::new()
            }
            Input::Recorded(recorded) => {
                self.desk.recorded(recorded);
                self.name_runs()
            }
            Input::Projects(projects) => {
                self.projects = match projects {
                    Ok(places) => {
                        self.scratch = places.scratch;
                        Projects::Read(places.projects)
                    }
                    Err(why) => Projects::Failed(why),
                };
                Vec::new()
            }
            Input::Clients(clients) => {
                // Unreadable, every client Ren drives is offered; starting
                // one says whether it is there.
                self.clients = clients.unwrap_or_default();
                Vec::new()
            }
            Input::Models { harness, models } => {
                self.models.insert(
                    harness,
                    match models {
                        Ok(models) => Offered::Listed(models),
                        Err(why) => Offered::Failed(why),
                    },
                );
                Vec::new()
            }
            Input::Lagged => vec![Effect::Read(Read::Runs)],
            Input::Update(update) => self.update(update),
            Input::Conversation { run, conversation } => {
                if let Some(held) = self.desk.get_mut(run) {
                    held.transcript.load(&conversation);
                }
                Vec::new()
            }
            Input::Answered(Ok(Answered::Started(run))) => {
                if let Some(starting) = self.starting.take() {
                    let held = self.desk.entry(run, |seen| {
                        Run::new(
                            run,
                            Stored::Known(starting.harness),
                            starting
                                .dir
                                .clone()
                                .or_else(|| self.scratch.clone())
                                .unwrap_or_default(),
                            seen,
                        )
                    });
                    if let Some(prompt) = &starting.prompt {
                        held.transcript.prompted(prompt.as_str());
                    }
                }
                vec![
                    Effect::Read(Read::Runs),
                    Effect::Read(Read::Recorded),
                    Effect::Started(run),
                ]
            }
            Input::Answered(Ok(Answered::Sent)) => Vec::new(),
            Input::Answered(Err(why)) => {
                self.starting = None;
                vec![
                    Effect::Refused(Self::refusal(&why)),
                    Effect::Read(Read::Runs),
                ]
            }
            // The size is the screens'.
            Input::Resize(_) => Vec::new(),
        }
    }

    /// Ask the daemon to start a run; it is held once the daemon names it.
    pub fn start(
        &mut self,
        harness: Harness,
        dir: Option<String>,
        resume: Option<NativeThreadId>,
        prompt: Option<Prompt>,
        settings: RunSettings,
    ) -> Command {
        self.starting = Some(Starting {
            harness,
            dir: dir.clone(),
            resumes: resume.is_some(),
            prompt: prompt.clone(),
        });
        Command::Start {
            harness,
            dir,
            resume,
            prompt,
            settings,
        }
    }

    /// Keep `prompt` until the run waits for one.
    pub fn queue(&mut self, run: RunId, prompt: Prompt) {
        if let Some(run) = self.desk.get_mut(run) {
            run.queued = Some(match run.queued.take() {
                Some(earlier) => {
                    Prompt::parse(format!("{}\n\n{}", earlier.as_str(), prompt.as_str()))
                        .unwrap_or(prompt)
                }
                None => prompt,
            });
        }
    }

    /// Send `prompt` to `run` now.
    pub fn prompt(&mut self, run: RunId, prompt: Prompt) -> Command {
        if let Some(held) = self.desk.get_mut(run) {
            held.transcript.prompted(prompt.as_str());
        }
        Command::Prompt { run, prompt }
    }

    /// Read the conversations of live runs not read yet: what each is
    /// doing, and what it was first asked, which names it.
    fn name_runs(&mut self) -> Vec<Effect> {
        let unread: Vec<RunId> = self
            .desk
            .ordered()
            .iter()
            .filter(|run| !run.asked)
            .map(|run| run.id)
            .collect();
        unread
            .into_iter()
            .map(|id| {
                if let Some(run) = self.desk.get_mut(id) {
                    run.asked = true;
                }
                Effect::Read(Read::Conversation(id))
            })
            .collect()
    }

    /// Prompts kept for runs that now wait for one.
    fn release_queued(&mut self) -> Vec<Effect> {
        let ready: Vec<RunId> = self
            .desk
            .ordered()
            .iter()
            .filter(|run| run.queued.is_some() && run.attention() == Attention::YourTurn)
            .map(|run| run.id)
            .collect();
        let mut effects = Vec::new();
        for id in ready {
            if let Some(prompt) = self.desk.get_mut(id).and_then(|run| run.queued.take()) {
                effects.push(Effect::Send(self.prompt(id, prompt)));
            }
        }
        effects
    }

    fn update(&mut self, update: RunUpdate) -> Vec<Effect> {
        match update {
            RunUpdate::State { run, state } => {
                let Some(held) = self.desk.get_mut(run) else {
                    // A run started elsewhere: its folder and client come
                    // with the next listing.
                    return vec![Effect::Read(Read::Runs), Effect::Read(Read::Recorded)];
                };
                held.live = Some(state);
                let mut effects = self.release_queued();
                // A run that ended is read again to learn how.
                if let Some(RunState::Ended { .. }) =
                    self.desk.get(run).and_then(|run| run.live.as_ref())
                {
                    effects.push(Effect::Read(Read::Recorded));
                }
                effects
            }
            RunUpdate::Event { run, event } => {
                let Some(held) = self.desk.get_mut(run) else {
                    return vec![Effect::Read(Read::Runs)];
                };
                held.transcript.apply(&event);
                Vec::new()
            }
        }
    }

    /// The daemon's error code for a refused command, in words.
    fn refusal(code: &str) -> String {
        match code {
            "runs_disabled" => "runs are off on the daemon: add a folder to [runs] roots",
            "invalid_directory" => "that folder is not under one of the daemon's run roots",
            "directory_not_found" => "that folder does not exist on the daemon's host",
            "not_drivable" => "Ren cannot drive that client yet",
            "spawn_failed" => "the client did not start: is it installed on the daemon's host?",
            "unrecorded" => "the recorder has no room to record a new run right now",
            "busy" => "a turn is already running",
            "starting" => "the run is still opening its conversation",
            "not_working" | "turn_not_started" => "no turn is running",
            "no_such_approval" => "that approval no longer waits",
            "stopping" | "ended" => "the run is stopping or has ended",
            "not_found" => "the daemon does not know that run",
            "call_open" => "a call of it is still open: delete it once the call ends",
            "run_running" => "a run of it is still running: stop it before deleting",
            "unauthorized" | "forbidden" => "the daemon refused this terminal's control access",
            other => return other.to_owned(),
        }
        .to_owned()
    }
}
