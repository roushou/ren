//! Everything the terminal has read from its gateway, in the one place every
//! screen reads it from: the sessions and their model calls (`sessions`),
//! the daemon's runs with their live conversations (`desk`, `transcript`),
//! and the projects a run can start in.
//!
//! [`Model::apply`] takes one [`Input`] and answers with what it calls for:
//! reads, commands (a prompt kept for a run that now waits for one), and
//! news the screen shown may act on.

pub(crate) mod call;
pub(crate) mod desk;
pub(crate) mod sessions;
pub(crate) mod transcript;

pub use call::CallEnd;
pub use sessions::{Reading, Snapshot, SnapshotQuery};

use crate::Size;
use desk::{Attention, Desk, Run};
use ren_harness::{
    Harness,
    drive::{ApprovalId, Decision, DriveEvent, Prompt},
};
use ren_protocol::label::Stored;
use ren_protocol::native::NativeThreadId;
use ren_recorder::Observation;
use ren_recorder::id::{CallId, RunId, SessionId, TurnId};
use ren_recorder::progress::Progress;
use ren_store::activity::{CallActivity, TurnActivity};
use ren_store::view::RunView;
use ren_supervisor::{Project, RunSnapshot, RunState, RunUpdate};
use sessions::Sessions;

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
    Projects(Result<Vec<Project>, String>),
    /// One update from the run feed.
    Update(RunUpdate),
    /// The run feed skipped updates: the runs are read again.
    Lagged,
    /// A run's recorded turns, oldest first.
    History {
        run: RunId,
        turns: Vec<TurnActivity>,
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
    /// A run's recorded turns, from its session, now.
    History {
        run: RunId,
        session: SessionId,
    },
    /// Again after a pause: its newest turn ended and is not recorded yet.
    HistoryLater {
        run: RunId,
        session: SessionId,
    },
}

/// A command for the control API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start {
        harness: Harness,
        dir: String,
        resume: Option<NativeThreadId>,
        prompt: Option<Prompt>,
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
    dir: String,
    resumes: bool,
    prompt: Option<Prompt>,
}

/// What was read, kept for every screen.
#[derive(Debug)]
pub(crate) struct Model {
    pub sessions: Sessions,
    pub desk: Desk,
    pub projects: Projects,
    pub control: Control,
    /// The client `/new` starts when none is named: the last one started.
    pub client: Harness,
    /// The run asked to start here, until the daemon answers.
    starting: Option<Starting>,
}

impl Model {
    /// History reads after a turn ends before its record is given up on.
    const HISTORY_TRIES: u8 = 5;

    pub fn new() -> Self {
        Self {
            sessions: Sessions::default(),
            desk: Desk::default(),
            projects: Projects::Unread,
            control: Control::Unread,
            client: Harness::ClaudeCode,
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
                    Ok(projects) => Projects::Read(projects),
                    Err(why) => Projects::Failed(why),
                };
                Vec::new()
            }
            Input::Lagged => vec![Effect::Read(Read::Runs)],
            Input::Update(update) => self.update(update),
            Input::History { run, turns } => self.history_read(run, turns),
            Input::Answered(Ok(Answered::Started(run))) => {
                if let Some(starting) = self.starting.take() {
                    let held = self.desk.entry(run, |seen| {
                        Run::new(
                            run,
                            Stored::Known(starting.harness),
                            starting.dir.clone(),
                            seen,
                        )
                    });
                    // A new conversation has no recorded turns: every one
                    // recorded from now on is new.
                    if !starting.resumes {
                        held.transcript.knowing(Vec::new());
                    }
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

    /// Read `run`'s recorded turns, now or `later`; a run whose session is
    /// not known yet has the recorded runs read first.
    pub fn history(&self, run: RunId, later: bool) -> Read {
        let session = self
            .desk
            .get(run)
            .and_then(|run| run.recorded.as_ref())
            .and_then(|recorded| recorded.session);
        match (session, later) {
            (Some(session), false) => Read::History { run, session },
            (Some(session), true) => Read::HistoryLater { run, session },
            (None, _) => Read::Recorded,
        }
    }

    /// Ask the daemon to start a run; it is held once the daemon names it.
    pub fn start(
        &mut self,
        harness: Harness,
        dir: String,
        resume: Option<NativeThreadId>,
        prompt: Option<Prompt>,
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

    fn history_read(&mut self, run: RunId, turns: Vec<TurnActivity>) -> Vec<Effect> {
        let Some(held) = self.desk.get_mut(run) else {
            return Vec::new();
        };
        let recorded = held.transcript.recorded_in(&turns);
        let waiting = held.awaiting > 0;
        if recorded {
            held.transcript.clear();
            held.awaiting = 0;
        } else if waiting {
            held.awaiting += 1;
        }
        if held.transcript.is_empty() || !waiting {
            // What was recorded before the next turn starts.
            held.transcript
                .knowing(turns.iter().map(|turn| turn.native_id.clone()).collect());
        }
        held.history = turns;
        if !recorded && waiting && held.awaiting <= Self::HISTORY_TRIES {
            return vec![Effect::Read(self.history(run, true))];
        }
        if !recorded && waiting {
            held.awaiting = 0;
        }
        Vec::new()
    }

    /// Read the history of live runs not read yet, which names them by
    /// their first prompt.
    fn name_runs(&mut self) -> Vec<Effect> {
        let unnamed: Vec<RunId> = self
            .desk
            .ordered()
            .iter()
            .filter(|run| {
                !run.named
                    && run
                        .recorded
                        .as_ref()
                        .is_some_and(|recorded| recorded.session.is_some())
            })
            .map(|run| run.id)
            .collect();
        unnamed
            .into_iter()
            .map(|id| {
                if let Some(run) = self.desk.get_mut(id) {
                    run.named = true;
                }
                Effect::Read(self.history(id, false))
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
                match event {
                    DriveEvent::TurnEnded { .. } => {
                        held.awaiting = 1;
                        vec![Effect::Read(self.history(run, true))]
                    }
                    DriveEvent::Ready { .. }
                    | DriveEvent::TurnStarted { .. }
                    | DriveEvent::Delta { .. }
                    | DriveEvent::Item { .. }
                    | DriveEvent::Approval { .. }
                    | DriveEvent::ApprovalResolved { .. }
                    | DriveEvent::Error { .. }
                    | DriveEvent::Refused { .. }
                    | DriveEvent::Unsupported { .. }
                    | DriveEvent::Unknown { .. }
                    | DriveEvent::Unreadable { .. } => Vec::new(),
                }
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
            "unauthorized" | "forbidden" => "the daemon refused this terminal's control access",
            other => return other.to_owned(),
        }
        .to_owned()
    }
}
