//! One client process as Ren drives it, pure: its run machine (`RunState`),
//! its client's codec, the settings it runs on, the first prompt it holds
//! until its conversation opens, the conversation it opened, what its
//! conversation keeps (`Entries`), and which rest it is on. Frames from the
//! client, the operator's commands, and the process's own events go in;
//! what to do comes out as [`ProcessEffect`]s, which the task carries out
//! with the process's pipes, kill switch, and recording tap.

use crate::entries::Entries;
use crate::run::{ProcessExit, RunAction, RunCommand, RunEvent, RunState, Transition};
use crate::workdir::Workdir;
use crate::{CommandError, RunUpdate};
use ren_harness::Harness;
use ren_harness::attachment::Offer;
use ren_harness::commands::ClientCommand;
use ren_harness::drive::{DriveEvent, Driver, TurnEnd};
use ren_harness::lines::{Framed, Line};
use ren_harness::prompt::Prompt;
use ren_harness::settings::RunSettings;
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use ren_recorder::capture::RunEnd;
use ren_recorder::id::{RunId, SessionId};

/// Which time a process went idle. An idle stop armed for an earlier rest
/// is stale: the process has worked since.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct Rest(u64);

/// What the task does for a process.
#[derive(Debug)]
pub(crate) enum ProcessEffect {
    /// Queue this line for the client's stdin.
    Write(Line),
    /// End the process.
    Kill,
    /// It went idle: stop it after the idle stop, unless this rest is over.
    Rested(Rest),
    /// Tell subscribers.
    Publish(RunUpdate),
    /// Record that its conversation opened.
    Opened(NativeThreadId),
    /// Record the settings it moved to.
    Configured(RunSettings),
    /// Record an entry of its conversation.
    Entry(Option<NativeTurnId>, DriveEvent),
    /// Record how it ended. The last effect a process makes.
    Ended {
        end: RunEnd,
        exit: Option<ProcessExit>,
    },
    /// A turn started: the session keeps the conversation the process
    /// opened, if it opened one.
    TurnStarted {
        conversation: Option<NativeThreadId>,
    },
    /// A turn ended; a failed one may be a sign-in that ran out.
    TurnEnded { failed: bool },
    /// The first prompt, sent as the conversation opened, was refused.
    FirstRefused(CommandError),
}

/// One client process serving a session.
#[derive(Debug)]
pub(crate) struct Process {
    run: RunId,
    session: SessionId,
    harness: Harness,
    workdir: Workdir,
    settings: RunSettings,
    state: RunState,
    /// `None` for a process that never started.
    codec: Option<Driver>,
    /// The first prompt, sent once the conversation opens.
    first: Option<Prompt>,
    /// The conversation its client opened. The session resumes it only
    /// once a turn has run in it: a client keeps no conversation without
    /// one.
    opened: Option<NativeThreadId>,
    entries: Entries,
    rest: Rest,
}

/// Who a process is: its run, the session it serves, and how it works.
#[derive(Debug, Clone)]
pub(crate) struct Serving {
    pub run: RunId,
    pub session: SessionId,
    pub harness: Harness,
    pub workdir: Workdir,
    pub settings: RunSettings,
}

impl Process {
    /// A process just started with `codec`, its `opening` lines to send,
    /// holding `first` until its conversation opens.
    pub(crate) fn opening(
        serving: Serving,
        codec: Driver,
        opening: Vec<Line>,
        first: Option<Prompt>,
    ) -> (Self, Vec<ProcessEffect>) {
        let process = Self::with(serving, RunState::Starting, Some(codec), first);
        let mut effects = vec![ProcessEffect::Publish(RunUpdate::State {
            run: process.run,
            state: RunState::Starting,
        })];
        effects.extend(opening.into_iter().map(ProcessEffect::Write));
        (process, effects)
    }

    /// A process that could not be started: ended at once.
    pub(crate) fn unstarted(serving: Serving) -> (Self, Vec<ProcessEffect>) {
        let state = RunState::Ended {
            end: RunEnd::SpawnFailed,
            exit: None,
        };
        let process = Self::with(serving, state.clone(), None, None);
        let effects = vec![
            ProcessEffect::Ended {
                end: RunEnd::SpawnFailed,
                exit: None,
            },
            ProcessEffect::Publish(RunUpdate::State {
                run: process.run,
                state,
            }),
        ];
        (process, effects)
    }

    fn with(
        serving: Serving,
        state: RunState,
        codec: Option<Driver>,
        first: Option<Prompt>,
    ) -> Self {
        let Serving {
            run,
            session,
            harness,
            workdir,
            settings,
        } = serving;
        Self {
            run,
            session,
            harness,
            workdir,
            settings,
            state,
            codec,
            first,
            opened: None,
            entries: Entries::default(),
            rest: Rest::default(),
        }
    }

    pub(crate) fn run(&self) -> RunId {
        self.run
    }

    pub(crate) fn session(&self) -> SessionId {
        self.session
    }

    pub(crate) fn harness(&self) -> Harness {
        self.harness
    }

    pub(crate) fn workdir(&self) -> &Workdir {
        &self.workdir
    }

    pub(crate) fn settings(&self) -> &RunSettings {
        &self.settings
    }

    pub(crate) fn state(&self) -> &RunState {
        &self.state
    }

    /// The rest it is on, while it is idle: a process taken up idle is
    /// stopped after the idle stop, counted from then.
    pub(crate) fn resting(&self) -> Option<Rest> {
        match self.state {
            RunState::Idle { .. } => Some(self.rest),
            RunState::Starting
            | RunState::Working { .. }
            | RunState::Stopping { .. }
            | RunState::Ended { .. } => None,
        }
    }

    /// The commands its client offers a prompt; none before it says.
    pub(crate) fn commands(&self) -> &[ClientCommand] {
        self.codec.as_ref().map_or(&[], Driver::commands)
    }

    /// What its client said of images; unknown before it says.
    pub(crate) fn image_offer(&self) -> Offer {
        self.codec
            .as_ref()
            .map_or(Offer::Unknown, Driver::image_offer)
    }

    /// One frame of the client's output.
    pub(crate) fn receive(&mut self, frame: Framed) -> Vec<ProcessEffect> {
        let Some(codec) = self.codec.as_mut() else {
            return Vec::new();
        };
        let received = codec.receive(frame);
        let mut effects: Vec<ProcessEffect> = received
            .replies
            .into_iter()
            .map(ProcessEffect::Write)
            .collect();
        for event in received.events {
            match &event {
                DriveEvent::Ready { thread } => {
                    self.opened = Some(*thread);
                    effects.push(ProcessEffect::Opened(*thread));
                }
                DriveEvent::TurnStarted { .. } => effects.push(ProcessEffect::TurnStarted {
                    conversation: self.opened,
                }),
                DriveEvent::TurnEnded { end, .. } => effects.push(ProcessEffect::TurnEnded {
                    failed: matches!(end, TurnEnd::Failed { .. }),
                }),
                DriveEvent::Delta { .. }
                | DriveEvent::Item { .. }
                | DriveEvent::Approval { .. }
                | DriveEvent::ApprovalResolved { .. }
                | DriveEvent::Error { .. }
                | DriveEvent::Refused { .. }
                | DriveEvent::Unsupported { .. }
                | DriveEvent::Unknown { .. }
                | DriveEvent::Unreadable { .. } => {}
            }
            effects.extend(
                self.entries
                    .take(&event)
                    .into_iter()
                    .map(|(turn, entry)| ProcessEffect::Entry(turn, entry)),
            );
            effects.push(ProcessEffect::Publish(RunUpdate::Event {
                run: self.run,
                event: event.clone(),
            }));
            self.happened(RunEvent::Agent(event), &mut effects);
            // Open now: the first prompt goes out.
            if let RunState::Idle { .. } = self.state
                && let Some(prompt) = self.first.take()
            {
                match self.command(RunCommand::Prompt(prompt)) {
                    Ok(sent) => effects.extend(sent),
                    Err(refused) => effects.push(ProcessEffect::FirstRefused(refused)),
                }
            }
        }
        effects
    }

    /// An operator's command. A prompt for a process still opening waits for
    /// its conversation, as the first prompt. A command the run or its
    /// client refuses changes nothing.
    pub(crate) fn command(
        &mut self,
        command: RunCommand,
    ) -> Result<Vec<ProcessEffect>, CommandError> {
        if let (
            RunState::Starting,
            RunCommand::Prompt(prompt) | RunCommand::Queue(prompt) | RunCommand::Steer(prompt),
            None,
        ) = (&self.state, &command, &self.first)
        {
            self.first = Some(prompt.clone());
            return Ok(Vec::new());
        }
        let configuring = match &command {
            RunCommand::Configure(settings) => Some(settings.clone()),
            RunCommand::Prompt(_)
            | RunCommand::Queue(_)
            | RunCommand::Unqueue
            | RunCommand::Steer(_)
            | RunCommand::Answer { .. }
            | RunCommand::Interrupt
            | RunCommand::Stop => None,
        };
        let Transition { state, actions } = self
            .state
            .apply(RunEvent::Operator(command))
            .map_err(CommandError::Refused)?;
        // Encoded before anything changes, so a command the client's
        // protocol refuses leaves the process as it was.
        let mut effects = Vec::new();
        for action in actions {
            match action {
                RunAction::Send(command) => {
                    let codec = self
                        .codec
                        .as_mut()
                        .ok_or(CommandError::NoSuchRun(self.run))?;
                    effects.extend(
                        codec
                            .command(command)
                            .map_err(CommandError::Protocol)?
                            .into_iter()
                            .map(ProcessEffect::Write),
                    );
                }
                RunAction::Kill => effects.push(ProcessEffect::Kill),
            }
        }
        if let Some(settings) = configuring {
            effects.push(ProcessEffect::Configured(settings.clone()));
            self.settings = settings;
            // The state may not change, but the process did: say so.
            effects.push(ProcessEffect::Publish(RunUpdate::State {
                run: self.run,
                state: state.clone(),
            }));
        }
        self.settle(state, &mut effects);
        Ok(effects)
    }

    /// The daemon is shutting down.
    pub(crate) fn shut_down(&mut self) -> Vec<ProcessEffect> {
        let mut effects = Vec::new();
        self.happened(RunEvent::Shutdown, &mut effects);
        effects
    }

    /// The process exited.
    pub(crate) fn exited(&mut self, exit: ProcessExit) -> Vec<ProcessEffect> {
        let mut effects = Vec::new();
        self.happened(RunEvent::Exited(exit), &mut effects);
        effects
    }

    /// The idle stop armed for `rest` ran out: a process still on that rest
    /// is stopped.
    pub(crate) fn rest_over(&mut self, rest: Rest) -> Vec<ProcessEffect> {
        let mut effects = Vec::new();
        if rest == self.rest {
            self.happened(RunEvent::IdleTimeout, &mut effects);
        }
        effects
    }

    /// An event that is not the operator's. One the run refuses (it has
    /// ended) changes nothing, and a command its client will not take (a
    /// queued prompt) is not sent.
    fn happened(&mut self, event: RunEvent, effects: &mut Vec<ProcessEffect>) {
        let Ok(Transition { state, actions }) = self.state.apply(event) else {
            return;
        };
        for action in actions {
            match action {
                RunAction::Send(command) => {
                    if let Some(Ok(lines)) = self.codec.as_mut().map(|codec| codec.command(command))
                    {
                        effects.extend(lines.into_iter().map(ProcessEffect::Write));
                    }
                }
                RunAction::Kill => effects.push(ProcessEffect::Kill),
            }
        }
        self.settle(state, effects);
    }

    fn settle(&mut self, state: RunState, effects: &mut Vec<ProcessEffect>) {
        // Idle now, not before: a new rest begins.
        if let (RunState::Idle { .. }, false) =
            (&state, matches!(self.state, RunState::Idle { .. }))
        {
            self.rest = Rest(self.rest.0.wrapping_add(1));
            effects.push(ProcessEffect::Rested(self.rest));
        }
        if let (RunState::Ended { end, exit }, false) = (&state, self.state.is_ended()) {
            effects.extend(
                self.entries
                    .finish()
                    .into_iter()
                    .map(|(turn, entry)| ProcessEffect::Entry(turn, entry)),
            );
            effects.push(ProcessEffect::Ended {
                end: *end,
                exit: *exit,
            });
        }
        if state != self.state {
            self.state = state.clone();
            effects.push(ProcessEffect::Publish(RunUpdate::State {
                run: self.run,
                state,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workdir::{Bounds, Roots};
    use ren_harness::drive::Opening;
    use ren_harness::lines::{LineLimit, Lines};
    use serde_json::{Value, json};
    use std::path::PathBuf;

    const THREAD: &str = "0190f3a0-0000-7000-8000-000000000001";
    const TURN: &str = "0190f3a0-0000-7000-8000-000000000002";

    fn uuid(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(0x0190_f3a0_0000_7000_8000_0000_0000_0000 | n)
    }

    fn serving() -> Serving {
        let bounds = Bounds::new(
            Roots::new(vec![PathBuf::from("/work")]).unwrap(),
            Vec::new(),
            None,
            None,
        )
        .unwrap();
        Serving {
            run: RunId::restore(uuid(10)),
            session: SessionId::restore(uuid(11)),
            harness: Harness::Codex,
            workdir: Workdir::confirm(&bounds, PathBuf::from("/work/acme")).unwrap(),
            settings: RunSettings::default(),
        }
    }

    /// A Codex process just started, and what it first asked to do.
    fn codex(first: Option<Prompt>) -> (Process, Vec<ProcessEffect>) {
        let opening = Opening::New {
            cwd: "/work/acme".into(),
            id: NativeThreadId::new(uuid(1)).unwrap(),
            settings: RunSettings::default(),
        };
        let (codec, lines) = Driver::open(Harness::Codex, opening, "0.1.0", uuid(2));
        Process::opening(serving(), codec, lines, first)
    }

    /// The JSON-RPC messages written to the client.
    fn written(effects: &[ProcessEffect]) -> Vec<Value> {
        effects
            .iter()
            .filter_map(|effect| {
                if let ProcessEffect::Write(line) = effect {
                    serde_json::from_slice(line.as_bytes()).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    /// The client says `message`.
    fn says(process: &mut Process, message: Value) -> Vec<ProcessEffect> {
        let mut lines = Lines::new(LineLimit::DEFAULT);
        let frames = lines.push(format!("{message}\n").as_bytes());
        frames
            .into_iter()
            .flat_map(|frame| process.receive(frame))
            .collect()
    }

    /// The client answers every request in `asked`, and what came of it.
    fn answers(process: &mut Process, asked: &[ProcessEffect]) -> Vec<ProcessEffect> {
        let mut effects = Vec::new();
        for request in written(asked) {
            let result = match request["method"].as_str() {
                Some("thread/start") => json!({ "thread": { "id": THREAD } }),
                Some("turn/start") => json!({ "turn": { "id": TURN, "status": "inProgress" } }),
                Some(_) | None => json!({}),
            };
            if request.get("id").is_some() {
                effects.extend(says(
                    process,
                    json!({ "id": request["id"], "result": result }),
                ));
            }
        }
        effects
    }

    /// The client answers until nothing more is asked: its conversation
    /// opens.
    fn open(process: &mut Process, mut asked: Vec<ProcessEffect>) {
        for _ in 0..4 {
            asked = answers(process, &asked);
            if asked.is_empty() {
                break;
            }
        }
    }

    fn prompt(text: &str) -> Prompt {
        Prompt::try_from(text.to_owned()).unwrap()
    }

    #[test]
    fn a_prompt_while_opening_waits_and_goes_out_once_the_conversation_opens() {
        let (mut process, opening) = codex(None);
        assert!(matches!(
            opening.first(),
            Some(ProcessEffect::Publish(RunUpdate::State {
                state: RunState::Starting,
                ..
            }))
        ));
        // Held, not refused: nothing is sent yet.
        let held = process
            .command(RunCommand::Prompt(prompt("fix it")))
            .unwrap();
        assert!(held.is_empty());
        // A second prompt while opening is refused: one waits at most.
        assert!(matches!(
            process.command(RunCommand::Prompt(prompt("and this"))),
            Err(CommandError::Refused(crate::run::RunRefused::Starting))
        ));
        let mut asked = opening;
        let mut sent_turn = false;
        for _ in 0..4 {
            let next = answers(&mut process, &asked);
            sent_turn |= written(&next)
                .iter()
                .any(|message| message["method"] == "turn/start");
            if next.is_empty() {
                break;
            }
            asked = next;
        }
        assert!(
            sent_turn,
            "the held prompt went out as the conversation opened"
        );
        assert!(matches!(process.state(), RunState::Working { .. }));
    }

    #[test]
    fn an_idle_stop_for_an_earlier_rest_is_stale() {
        let (mut process, opening) = codex(None);
        open(&mut process, opening);
        assert!(matches!(process.state(), RunState::Idle { .. }));
        let stale = Rest::default();
        assert!(process.rest_over(stale).is_empty());
        assert!(matches!(process.state(), RunState::Idle { .. }));
        let current = process.rest;
        let stopped = process.rest_over(current);
        assert!(
            stopped
                .iter()
                .any(|effect| matches!(effect, ProcessEffect::Kill))
        );
        assert!(matches!(process.state(), RunState::Stopping { .. }));
    }

    #[test]
    fn going_idle_starts_a_new_rest_each_time() {
        let (mut process, opening) = codex(None);
        let mut rests = Vec::new();
        let mut asked = opening;
        for _ in 0..4 {
            let next = answers(&mut process, &asked);
            rests.extend(next.iter().filter_map(|effect| {
                if let ProcessEffect::Rested(rest) = effect {
                    Some(*rest)
                } else {
                    None
                }
            }));
            if next.is_empty() {
                break;
            }
            asked = next;
        }
        assert_eq!(rests, [Rest(1)]);
        // A turn, then idle again: a newer rest.
        let started = process.command(RunCommand::Prompt(prompt("go"))).unwrap();
        let _ = answers(&mut process, &started);
        let ended = says(
            &mut process,
            json!({ "method": "turn/completed", "params": { "threadId": THREAD,
                "turn": { "id": TURN, "status": "completed" } } }),
        );
        assert!(
            ended
                .iter()
                .any(|effect| matches!(effect, ProcessEffect::TurnEnded { failed: false }))
        );
        assert!(
            ended
                .iter()
                .any(|effect| matches!(effect, ProcessEffect::Rested(Rest(2))))
        );
    }

    #[test]
    fn its_end_is_recorded_once_after_its_conversation() {
        let (mut process, _) = codex(None);
        let stopping = process.shut_down();
        assert!(
            stopping
                .iter()
                .any(|effect| matches!(effect, ProcessEffect::Kill))
        );
        let ended = process.exited(ProcessExit::Signal(9));
        let ends: Vec<_> = ended
            .iter()
            .filter(|effect| matches!(effect, ProcessEffect::Ended { .. }))
            .collect();
        assert_eq!(ends.len(), 1);
        assert!(matches!(
            ended.last(),
            Some(ProcessEffect::Publish(RunUpdate::State {
                state: RunState::Ended {
                    end: RunEnd::Drained,
                    ..
                },
                ..
            }))
        ));
        // Ended is terminal: nothing more is recorded.
        assert!(process.exited(ProcessExit::Code(0)).is_empty());
        assert!(process.shut_down().is_empty());
    }

    #[test]
    fn a_process_that_never_started_has_ended_and_takes_no_command() {
        let (mut process, effects) = Process::unstarted(serving());
        assert!(matches!(
            effects.as_slice(),
            [
                ProcessEffect::Ended {
                    end: RunEnd::SpawnFailed,
                    exit: None
                },
                ProcessEffect::Publish(_)
            ]
        ));
        assert!(process.state().is_ended());
        assert!(process.command(RunCommand::Prompt(prompt("hi"))).is_err());
        assert!(process.commands().is_empty());
    }

    #[test]
    fn settings_change_once_the_run_takes_them_and_are_recorded() {
        let (mut process, opening) = codex(None);
        open(&mut process, opening);
        let settings = RunSettings {
            model: Some("gpt-fast".parse().unwrap()),
            effort: None,
            approvals: None,
        };
        let effects = process
            .command(RunCommand::Configure(settings.clone()))
            .unwrap();
        assert!(effects.iter().any(
            |effect| matches!(effect, ProcessEffect::Configured(recorded) if *recorded == settings)
        ));
        assert_eq!(process.settings(), &settings);
    }
}
