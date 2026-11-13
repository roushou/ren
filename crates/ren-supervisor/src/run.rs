//! One run's lifecycle as a pure state machine: what the agent reported, what
//! the operator asked, and how the process ended, in; the next state and what
//! to do about it, out. No I/O.

use ren_harness::drive::{
    ApprovalAsk, ApprovalId, Decision, DriveCommand, DriveEvent, Prompt, RequestKind,
};
use ren_harness::settings::RunSettings;
use ren_protocol::native::{NativeThreadId, NativeTurnId};
pub use ren_recorder::capture::ProcessExit;
use ren_recorder::capture::RunEnd;

/// Where a run is in its life.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum RunState {
    /// The process started; the conversation is not open yet.
    Starting,
    /// The conversation is open and waits for a prompt.
    Idle { thread: NativeThreadId },
    /// A turn was asked for or is running. Its id is known once the agent
    /// reports it; approvals wait for the operator.
    Working {
        thread: NativeThreadId,
        turn: Option<NativeTurnId>,
        approvals: Approvals,
        /// The operator's next prompt, sent as the turn ends.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queued: Option<Prompt>,
    },
    /// Ren asked the process to end and waits for it to exit.
    Stopping { cause: StopCause },
    /// The process is gone. Terminal.
    Ended {
        end: RunEnd,
        exit: Option<ProcessExit>,
    },
}

/// Approvals waiting for the operator, in the order the agent asked, each
/// with what it asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Approvals(Vec<Waiting>);

/// One approval waiting for an answer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Waiting {
    pub approval: ApprovalId,
    pub ask: ApprovalAsk,
}

impl Approvals {
    /// Add one; an approval asked again keeps its place and takes the newer
    /// ask.
    pub fn insert(&mut self, approval: ApprovalId, ask: ApprovalAsk) {
        match self
            .0
            .iter_mut()
            .find(|waiting| waiting.approval == approval)
        {
            Some(waiting) => waiting.ask = ask,
            None => self.0.push(Waiting { approval, ask }),
        }
    }

    /// Whether `approval` was waiting; it no longer is.
    pub fn remove(&mut self, approval: &ApprovalId) -> bool {
        let before = self.0.len();
        self.0.retain(|waiting| waiting.approval != *approval);
        self.0.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The oldest waiting approval.
    pub fn first(&self) -> Option<&Waiting> {
        self.0.first()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Waiting> {
        self.0.iter()
    }
}

/// Why Ren is stopping a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCause {
    Operator,
    Shutdown,
    /// The client refused to open the conversation.
    OpenRefused,
    /// The client sat idle between turns.
    Idle,
}

impl StopCause {
    fn end(self) -> RunEnd {
        match self {
            Self::Operator => RunEnd::Stopped,
            Self::Shutdown => RunEnd::Drained,
            Self::OpenRefused => RunEnd::OpenRefused,
            Self::Idle => RunEnd::Idled,
        }
    }
}

/// What the operator asks of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCommand {
    /// Start a turn with this prompt; refused while one runs.
    Prompt(Prompt),
    /// Send this as the running turn ends (now, when none runs). It
    /// replaces any prompt queued before.
    Queue(Prompt),
    /// Take back the queued prompt.
    Unqueue,
    /// Add this to the running turn (start a turn with it when none runs).
    Steer(Prompt),
    /// Run on these settings from the next turn on.
    Configure(RunSettings),
    Answer {
        approval: ApprovalId,
        decision: Decision,
    },
    Interrupt,
    Stop,
}

/// How a prompt goes to a run.
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub enum Delivery {
    /// Start a turn; refused while one runs.
    Start,
    /// Send it as the running turn ends, or now when none runs.
    Queue,
    /// Add it to the running turn, or start one when none runs.
    Steer,
}

impl Delivery {
    /// The command that delivers `prompt` this way.
    pub fn command(self, prompt: Prompt) -> RunCommand {
        match self {
            Self::Start => RunCommand::Prompt(prompt),
            Self::Queue => RunCommand::Queue(prompt),
            Self::Steer => RunCommand::Steer(prompt),
        }
    }
}

/// Something that happened to a run.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
    /// The agent reported this.
    Agent(DriveEvent),
    /// The operator asked for this.
    Operator(RunCommand),
    /// The process exited.
    Exited(ProcessExit),
    /// The daemon is shutting down.
    Shutdown,
    /// The run sat idle for as long as its idle stop allows.
    IdleTimeout,
}

/// What the shell must do after a transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    /// Send this to the agent.
    Send(DriveCommand),
    /// End the process.
    Kill,
}

/// The state after an event, and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Transition {
    pub state: RunState,
    pub actions: Vec<RunAction>,
}

/// Why a run refused an event.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunRefused {
    #[error("the run is still starting")]
    Starting,
    #[error("a turn is already running")]
    Busy,
    #[error("no turn is running")]
    NotWorking,
    #[error("the turn has not started yet")]
    TurnNotStarted,
    #[error("no approval {0} is waiting")]
    NoSuchApproval(ApprovalId),
    #[error("the run is stopping")]
    Stopping,
    #[error("the run has ended")]
    Ended,
    #[error("no prompt is queued")]
    NothingQueued,
}

impl RunState {
    /// The state after `event`, or why it does not apply.
    pub fn apply(&self, event: RunEvent) -> Result<Transition, RunRefused> {
        match event {
            RunEvent::Agent(event) => self.observe(event),
            RunEvent::Operator(command) => self.command(command),
            RunEvent::Exited(exit) => self.exited(exit),
            RunEvent::Shutdown => Ok(match self {
                Self::Starting | Self::Idle { .. } | Self::Working { .. } => {
                    Self::stop(StopCause::Shutdown)
                }
                Self::Stopping { .. } | Self::Ended { .. } => self.stay(),
            }),
            // Only an idle run is stopped: a turn running, or one starting,
            // is not idle, whatever the timer says.
            RunEvent::IdleTimeout => match self {
                Self::Idle { .. } => Ok(Self::stop(StopCause::Idle)),
                Self::Starting | Self::Working { .. } => Err(RunRefused::Busy),
                Self::Stopping { .. } => Err(RunRefused::Stopping),
                Self::Ended { .. } => Err(RunRefused::Ended),
            },
        }
    }

    /// Whether the run has ended.
    pub fn is_ended(&self) -> bool {
        match self {
            Self::Ended { .. } => true,
            Self::Starting | Self::Idle { .. } | Self::Working { .. } | Self::Stopping { .. } => {
                false
            }
        }
    }

    fn stay(&self) -> Transition {
        Transition {
            state: self.clone(),
            actions: Vec::new(),
        }
    }

    fn to(state: Self) -> Transition {
        Transition {
            state,
            actions: Vec::new(),
        }
    }

    fn stop(cause: StopCause) -> Transition {
        Transition {
            state: Self::Stopping { cause },
            actions: vec![RunAction::Kill],
        }
    }

    fn send(state: Self, command: DriveCommand) -> Transition {
        Transition {
            state,
            actions: vec![RunAction::Send(command)],
        }
    }

    /// The agent's reports are facts: they never fail, though some leave the
    /// state as it was.
    fn observe(&self, event: DriveEvent) -> Result<Transition, RunRefused> {
        if let Self::Ended { .. } = self {
            return Err(RunRefused::Ended);
        }
        Ok(match (self, event) {
            (Self::Starting, DriveEvent::Ready { thread }) => Self::to(Self::Idle { thread }),
            (Self::Starting, DriveEvent::Refused { request, .. }) if Self::opens(request) => {
                Self::stop(StopCause::OpenRefused)
            }
            (Self::Idle { thread }, DriveEvent::TurnStarted { turn }) => Self::to(Self::Working {
                thread: *thread,
                turn: Some(turn),
                approvals: Approvals::default(),
                queued: None,
            }),
            (
                Self::Working {
                    thread,
                    approvals,
                    queued,
                    ..
                },
                DriveEvent::TurnStarted { turn },
            ) => Self::to(Self::Working {
                thread: *thread,
                turn: Some(turn),
                approvals: approvals.clone(),
                queued: queued.clone(),
            }),
            (Self::Idle { thread }, DriveEvent::Approval { approval, ask }) => {
                let mut approvals = Approvals::default();
                approvals.insert(approval, ask);
                Self::to(Self::Working {
                    thread: *thread,
                    turn: None,
                    approvals,
                    queued: None,
                })
            }
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                    queued,
                },
                DriveEvent::Approval { approval, ask },
            ) => {
                let mut approvals = approvals.clone();
                approvals.insert(approval, ask);
                Self::to(Self::Working {
                    thread: *thread,
                    turn: *turn,
                    approvals,
                    queued: queued.clone(),
                })
            }
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                    queued,
                },
                DriveEvent::ApprovalResolved { approval },
            ) => {
                let mut approvals = approvals.clone();
                approvals.remove(&approval);
                Self::to(Self::Working {
                    thread: *thread,
                    turn: *turn,
                    approvals,
                    queued: queued.clone(),
                })
            }
            // The turn ended: the queued prompt starts the next one.
            (
                Self::Working {
                    thread,
                    queued: Some(queued),
                    ..
                },
                DriveEvent::TurnEnded { .. },
            ) => Self::send(
                Self::Working {
                    thread: *thread,
                    turn: None,
                    approvals: Approvals::default(),
                    queued: None,
                },
                DriveCommand::Prompt(queued.clone()),
            ),
            (Self::Working { thread, .. }, DriveEvent::TurnEnded { .. }) => {
                Self::to(Self::Idle { thread: *thread })
            }
            (
                Self::Working {
                    thread, turn: None, ..
                },
                DriveEvent::Refused {
                    request: RequestKind::TurnStart,
                    ..
                },
            ) => Self::to(Self::Idle { thread: *thread }),
            (
                Self::Starting
                | Self::Idle { .. }
                | Self::Working { .. }
                | Self::Stopping { .. }
                | Self::Ended { .. },
                DriveEvent::Ready { .. }
                | DriveEvent::TurnStarted { .. }
                | DriveEvent::Delta { .. }
                | DriveEvent::Item { .. }
                | DriveEvent::Approval { .. }
                | DriveEvent::ApprovalResolved { .. }
                | DriveEvent::TurnEnded { .. }
                | DriveEvent::Error { .. }
                | DriveEvent::Refused { .. }
                | DriveEvent::Unsupported { .. }
                | DriveEvent::Unknown { .. }
                | DriveEvent::Unreadable { .. },
            ) => self.stay(),
        })
    }

    fn opens(request: RequestKind) -> bool {
        match request {
            RequestKind::Initialize | RequestKind::ThreadStart | RequestKind::ThreadResume => true,
            RequestKind::TurnStart
            | RequestKind::TurnInterrupt
            | RequestKind::TurnSteer
            | RequestKind::Configure
            | RequestKind::ListCommands => false,
        }
    }

    fn command(&self, command: RunCommand) -> Result<Transition, RunRefused> {
        match self {
            Self::Stopping { .. } => return Err(RunRefused::Stopping),
            Self::Ended { .. } => return Err(RunRefused::Ended),
            Self::Starting | Self::Idle { .. } | Self::Working { .. } => {}
        }
        match (self, command) {
            // Answered above; listed so the match stays exhaustive.
            (
                Self::Stopping { .. },
                RunCommand::Prompt(_)
                | RunCommand::Queue(_)
                | RunCommand::Unqueue
                | RunCommand::Steer(_)
                | RunCommand::Configure(_)
                | RunCommand::Answer { .. }
                | RunCommand::Interrupt
                | RunCommand::Stop,
            ) => Err(RunRefused::Stopping),
            (
                Self::Ended { .. },
                RunCommand::Prompt(_)
                | RunCommand::Queue(_)
                | RunCommand::Unqueue
                | RunCommand::Steer(_)
                | RunCommand::Configure(_)
                | RunCommand::Answer { .. }
                | RunCommand::Interrupt
                | RunCommand::Stop,
            ) => Err(RunRefused::Ended),
            (Self::Starting | Self::Idle { .. } | Self::Working { .. }, RunCommand::Stop) => {
                Ok(Self::stop(StopCause::Operator))
            }
            (
                Self::Starting,
                RunCommand::Prompt(_)
                | RunCommand::Queue(_)
                | RunCommand::Unqueue
                | RunCommand::Steer(_)
                | RunCommand::Configure(_)
                | RunCommand::Interrupt
                | RunCommand::Answer { .. },
            ) => Err(RunRefused::Starting),
            // With no turn running, every way to send a prompt starts one.
            (
                Self::Idle { thread },
                RunCommand::Prompt(prompt) | RunCommand::Queue(prompt) | RunCommand::Steer(prompt),
            ) => Ok(Self::send(
                Self::Working {
                    thread: *thread,
                    turn: None,
                    approvals: Approvals::default(),
                    queued: None,
                },
                DriveCommand::Prompt(prompt),
            )),
            (Self::Working { .. }, RunCommand::Prompt(_)) => Err(RunRefused::Busy),
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                    ..
                },
                RunCommand::Queue(prompt),
            ) => Ok(Self::to(Self::Working {
                thread: *thread,
                turn: *turn,
                approvals: approvals.clone(),
                queued: Some(prompt),
            })),
            (Self::Idle { .. }, RunCommand::Unqueue) => Err(RunRefused::NothingQueued),
            (Self::Working { queued: None, .. }, RunCommand::Unqueue) => {
                Err(RunRefused::NothingQueued)
            }
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                    queued: Some(_),
                },
                RunCommand::Unqueue,
            ) => Ok(Self::to(Self::Working {
                thread: *thread,
                turn: *turn,
                approvals: approvals.clone(),
                queued: None,
            })),
            (Self::Working { turn: None, .. }, RunCommand::Steer(_)) => {
                Err(RunRefused::TurnNotStarted)
            }
            (Self::Working { turn: Some(_), .. }, RunCommand::Steer(prompt)) => {
                Ok(Self::send(self.clone(), DriveCommand::Steer(prompt)))
            }
            (Self::Idle { .. } | Self::Working { .. }, RunCommand::Configure(settings)) => {
                Ok(Self::send(self.clone(), DriveCommand::Configure(settings)))
            }
            (Self::Idle { .. }, RunCommand::Interrupt) => Err(RunRefused::NotWorking),
            (Self::Working { turn: None, .. }, RunCommand::Interrupt) => {
                Err(RunRefused::TurnNotStarted)
            }
            (Self::Working { turn: Some(_), .. }, RunCommand::Interrupt) => {
                Ok(Self::send(self.clone(), DriveCommand::Interrupt))
            }
            (Self::Idle { .. }, RunCommand::Answer { approval, .. }) => {
                Err(RunRefused::NoSuchApproval(approval))
            }
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                    queued,
                },
                RunCommand::Answer { approval, decision },
            ) => {
                let mut waiting = approvals.clone();
                if !waiting.remove(&approval) {
                    return Err(RunRefused::NoSuchApproval(approval));
                }
                Ok(Self::send(
                    Self::Working {
                        thread: *thread,
                        turn: *turn,
                        approvals: waiting,
                        queued: queued.clone(),
                    },
                    DriveCommand::Answer { approval, decision },
                ))
            }
        }
    }

    fn exited(&self, exit: ProcessExit) -> Result<Transition, RunRefused> {
        let end = match self {
            Self::Ended { .. } => return Err(RunRefused::Ended),
            Self::Stopping { cause } => cause.end(),
            // Exiting mid-turn leaves the turn's outcome unknown; the run's
            // end says only that the process left on its own.
            Self::Starting | Self::Idle { .. } | Self::Working { .. } => RunEnd::Exited,
        };
        Ok(Self::to(Self::Ended {
            end,
            exit: Some(exit),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Approvals, ProcessExit, RunAction, RunCommand, RunEvent, RunRefused, RunState, StopCause,
    };
    use ren_harness::drive::{
        ApprovalAsk, ApprovalId, Decision, DriveCommand, DriveEvent, Prompt, RequestKind, TurnEnd,
    };
    use ren_protocol::native::{NativeThreadId, NativeTurnId};
    use ren_recorder::capture::RunEnd;
    use uuid::Uuid;

    fn thread() -> NativeThreadId {
        NativeThreadId::new(Uuid::from_u128(1)).unwrap()
    }

    fn turn() -> NativeTurnId {
        NativeTurnId::new(Uuid::from_u128(2)).unwrap()
    }

    fn approval(id: &str) -> ApprovalId {
        id.parse().unwrap()
    }

    fn prompt() -> RunCommand {
        RunCommand::Prompt(Prompt::parse("fix it".into()).unwrap())
    }

    fn agent(state: &RunState, event: DriveEvent) -> RunState {
        state.apply(RunEvent::Agent(event)).unwrap().state
    }

    fn operator(state: &RunState, command: RunCommand) -> Result<super::Transition, RunRefused> {
        state.apply(RunEvent::Operator(command))
    }

    fn waiting_for(ids: &[&str]) -> Approvals {
        let mut approvals = Approvals::default();
        for id in ids {
            approvals.insert(approval(id), ApprovalAsk::FileChange { reason: None });
        }
        approvals
    }

    fn ask(id: &str) -> DriveEvent {
        DriveEvent::Approval {
            approval: approval(id),
            ask: ApprovalAsk::FileChange { reason: None },
        }
    }

    #[test]
    fn a_turn_runs_from_prompt_to_end_with_an_approval() {
        let idle = agent(&RunState::Starting, DriveEvent::Ready { thread: thread() });
        assert_eq!(idle, RunState::Idle { thread: thread() });

        let asked = operator(&idle, prompt()).unwrap();
        assert!(matches!(
            asked.actions.as_slice(),
            [RunAction::Send(DriveCommand::Prompt(_))]
        ));
        let working = agent(&asked.state, DriveEvent::TurnStarted { turn: turn() });
        let waiting = agent(&working, ask("a1"));
        assert_eq!(
            waiting,
            RunState::Working {
                thread: thread(),
                turn: Some(turn()),
                approvals: waiting_for(&["a1"]),
                queued: None,
            }
        );

        let answered = operator(
            &waiting,
            RunCommand::Answer {
                approval: approval("a1"),
                decision: Decision::Accept,
            },
        )
        .unwrap();
        assert_eq!(answered.state, working);
        assert_eq!(
            answered.actions,
            vec![RunAction::Send(DriveCommand::Answer {
                approval: approval("a1"),
                decision: Decision::Accept
            })]
        );

        let interrupted = operator(&working, RunCommand::Interrupt).unwrap();
        assert_eq!(interrupted.state, working);
        assert_eq!(
            interrupted.actions,
            vec![RunAction::Send(DriveCommand::Interrupt)]
        );
        let ended = agent(
            &working,
            DriveEvent::TurnEnded {
                turn: turn(),
                end: TurnEnd::Interrupted,
            },
        );
        assert_eq!(ended, idle);
    }

    #[test]
    fn refuses_what_the_state_does_not_allow() {
        let idle = RunState::Idle { thread: thread() };
        let requested = operator(&idle, prompt()).unwrap().state;
        let cases = [
            (RunState::Starting, prompt(), RunRefused::Starting),
            (requested.clone(), prompt(), RunRefused::Busy),
            (idle.clone(), RunCommand::Interrupt, RunRefused::NotWorking),
            (
                requested.clone(),
                RunCommand::Interrupt,
                RunRefused::TurnNotStarted,
            ),
            (
                requested,
                RunCommand::Answer {
                    approval: approval("x"),
                    decision: Decision::Decline,
                },
                RunRefused::NoSuchApproval(approval("x")),
            ),
            (
                RunState::Stopping {
                    cause: StopCause::Operator,
                },
                RunCommand::Stop,
                RunRefused::Stopping,
            ),
            (
                RunState::Ended {
                    end: RunEnd::Exited,
                    exit: None,
                },
                prompt(),
                RunRefused::Ended,
            ),
        ];
        for (state, command, refused) in cases {
            assert_eq!(operator(&state, command), Err(refused));
        }
    }

    #[test]
    fn a_refused_turn_returns_to_idle_and_a_refused_opening_stops() {
        let idle = RunState::Idle { thread: thread() };
        let requested = operator(&idle, prompt()).unwrap().state;
        let refused = |request| DriveEvent::Refused {
            request,
            message: "no".into(),
        };
        assert_eq!(agent(&requested, refused(RequestKind::TurnStart)), idle);
        let stopping = RunState::Starting
            .apply(RunEvent::Agent(refused(RequestKind::ThreadStart)))
            .unwrap();
        assert_eq!(
            stopping.state,
            RunState::Stopping {
                cause: StopCause::OpenRefused
            }
        );
        assert_eq!(stopping.actions, vec![RunAction::Kill]);
    }

    #[test]
    fn an_exit_ends_the_run_with_its_cause() {
        let exit = ProcessExit::Signal(9);
        let cases = [
            (RunState::Starting, RunEnd::Exited),
            (RunState::Idle { thread: thread() }, RunEnd::Exited),
            (
                RunState::Working {
                    thread: thread(),
                    turn: Some(turn()),
                    approvals: Approvals::default(),
                    queued: None,
                },
                RunEnd::Exited,
            ),
            (
                RunState::Stopping {
                    cause: StopCause::Operator,
                },
                RunEnd::Stopped,
            ),
            (
                RunState::Stopping {
                    cause: StopCause::Shutdown,
                },
                RunEnd::Drained,
            ),
            (
                RunState::Stopping {
                    cause: StopCause::OpenRefused,
                },
                RunEnd::OpenRefused,
            ),
        ];
        for (state, end) in cases {
            let ended = state.apply(RunEvent::Exited(exit)).unwrap();
            assert_eq!(
                ended.state,
                RunState::Ended {
                    end,
                    exit: Some(exit)
                }
            );
            assert!(ended.state.is_ended());
            assert_eq!(
                ended.state.apply(RunEvent::Exited(exit)),
                Err(RunRefused::Ended),
                "ended is terminal"
            );
        }
    }

    #[test]
    fn stop_and_shutdown_kill_the_process() {
        let idle = RunState::Idle { thread: thread() };
        let stopped = operator(&idle, RunCommand::Stop).unwrap();
        assert_eq!(stopped.actions, vec![RunAction::Kill]);
        let drained = idle.apply(RunEvent::Shutdown).unwrap();
        assert_eq!(
            drained.state,
            RunState::Stopping {
                cause: StopCause::Shutdown
            }
        );
        assert_eq!(drained.actions, vec![RunAction::Kill]);
        let again = drained.state.apply(RunEvent::Shutdown).unwrap();
        assert!(again.actions.is_empty(), "a stopping run is killed once");
    }

    #[test]
    fn only_an_exit_after_shutdown_is_drained() {
        // The daemon drains its runs itself: the shutdown asks for the
        // kill, and the exit that follows is the drain's.
        let idle = RunState::Idle { thread: thread() };
        let stopping = idle.apply(RunEvent::Shutdown).unwrap().state;
        let drained = stopping
            .apply(RunEvent::Exited(ProcessExit::Signal(9)))
            .unwrap();
        assert_eq!(
            drained.state,
            RunState::Ended {
                end: RunEnd::Drained,
                exit: Some(ProcessExit::Signal(9))
            }
        );
        // An exit seen before the shutdown reached the run was not Ren's
        // doing, whatever signalled it: it stays an exit, with its signal.
        let exited = idle
            .apply(RunEvent::Exited(ProcessExit::Signal(15)))
            .unwrap();
        assert_eq!(
            exited.state,
            RunState::Ended {
                end: RunEnd::Exited,
                exit: Some(ProcessExit::Signal(15))
            }
        );
        assert_eq!(
            exited.state.apply(RunEvent::Shutdown).unwrap().state,
            exited.state,
            "a shutdown does not rewrite an end already recorded"
        );
    }

    #[test]
    fn states_read_back_from_their_json() {
        let states = [
            RunState::Starting,
            RunState::Idle { thread: thread() },
            RunState::Working {
                thread: thread(),
                turn: Some(turn()),
                approvals: waiting_for(&["ap-1"]),
                queued: None,
            },
            RunState::Stopping {
                cause: StopCause::Shutdown,
            },
            RunState::Ended {
                end: RunEnd::Exited,
                exit: Some(ProcessExit::Code(1)),
            },
        ];
        for state in states {
            let json = serde_json::to_value(&state).unwrap();
            assert_eq!(serde_json::from_value::<RunState>(json).unwrap(), state);
        }
    }

    #[test]
    fn the_agent_cannot_report_after_the_end() {
        let ended = RunState::Ended {
            end: RunEnd::Exited,
            exit: None,
        };
        assert_eq!(
            ended.apply(RunEvent::Agent(DriveEvent::Ready { thread: thread() })),
            Err(RunRefused::Ended)
        );
    }

    fn text(text: &str) -> Prompt {
        Prompt::parse(text.into()).unwrap()
    }

    fn running() -> RunState {
        RunState::Working {
            thread: thread(),
            turn: Some(turn()),
            approvals: Approvals::default(),
            queued: None,
        }
    }

    /// A prompt queued while a turn runs is kept, shown, replaceable, and
    /// sent as the turn ends; taken back, nothing is sent.
    #[test]
    fn a_queued_prompt_goes_out_as_the_turn_ends() {
        let queued = operator(&running(), RunCommand::Queue(text("then the docs"))).unwrap();
        assert!(
            queued.actions.is_empty(),
            "nothing sent while the turn runs"
        );
        let replaced = operator(&queued.state, RunCommand::Queue(text("then the tests"))).unwrap();
        let RunState::Working {
            queued: Some(kept), ..
        } = &replaced.state
        else {
            panic!("queued: {:?}", replaced.state);
        };
        assert_eq!(kept.as_str(), "then the tests");
        let ended = replaced
            .state
            .apply(RunEvent::Agent(DriveEvent::TurnEnded {
                turn: turn(),
                end: TurnEnd::Completed,
            }))
            .unwrap();
        assert_eq!(
            ended.actions,
            vec![RunAction::Send(DriveCommand::Prompt(text(
                "then the tests"
            )))]
        );
        assert!(matches!(
            ended.state,
            RunState::Working {
                turn: None,
                queued: None,
                ..
            }
        ));
        // Taken back before the turn ends, the turn ends idle.
        let taken = operator(&queued.state, RunCommand::Unqueue).unwrap();
        assert_eq!(
            agent(
                &taken.state,
                DriveEvent::TurnEnded {
                    turn: turn(),
                    end: TurnEnd::Completed
                }
            ),
            RunState::Idle { thread: thread() }
        );
        assert_eq!(
            operator(&running(), RunCommand::Unqueue),
            Err(RunRefused::NothingQueued)
        );
    }

    /// Steering goes to the running turn; with none running, every way to
    /// send a prompt starts a turn.
    #[test]
    fn steering_goes_to_the_running_turn_and_idle_runs_start_one() {
        let steered = operator(&running(), RunCommand::Steer(text("use the old API"))).unwrap();
        assert_eq!(
            steered.actions,
            vec![RunAction::Send(DriveCommand::Steer(text(
                "use the old API"
            )))]
        );
        assert_eq!(steered.state, running());
        let not_started = RunState::Working {
            thread: thread(),
            turn: None,
            approvals: Approvals::default(),
            queued: None,
        };
        assert_eq!(
            operator(&not_started, RunCommand::Steer(text("x"))),
            Err(RunRefused::TurnNotStarted)
        );
        let idle = RunState::Idle { thread: thread() };
        for command in [RunCommand::Queue(text("go")), RunCommand::Steer(text("go"))] {
            let started = operator(&idle, command).unwrap();
            assert_eq!(
                started.actions,
                vec![RunAction::Send(DriveCommand::Prompt(text("go")))]
            );
        }
    }

    /// Settings go to the client whether or not a turn runs; a run still
    /// starting takes none.
    #[test]
    fn settings_go_to_the_client_once_the_conversation_is_open() {
        use ren_harness::settings::{Effort, RunSettings};
        let settings = RunSettings {
            model: None,
            effort: Some(Effort::High),
            approvals: None,
        };
        let configured = operator(&running(), RunCommand::Configure(settings.clone())).unwrap();
        assert_eq!(
            configured.actions,
            vec![RunAction::Send(DriveCommand::Configure(settings.clone()))]
        );
        assert_eq!(configured.state, running());
        assert_eq!(
            operator(&RunState::Starting, RunCommand::Configure(settings)),
            Err(RunRefused::Starting)
        );
    }
}
