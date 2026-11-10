//! One run's lifecycle as a pure state machine: what the agent reported, what
//! the operator asked, and how the process ended, in; the next state and what
//! to do about it, out. No I/O.

use ren_harness::drive::{ApprovalId, Decision, DriveCommand, DriveEvent, Prompt, RequestKind};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
pub use ren_recorder::capture::ProcessExit;
use ren_recorder::capture::RunEnd;
use std::collections::BTreeSet;

/// Where a run is in its life.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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
        approvals: BTreeSet<ApprovalId>,
    },
    /// Ren asked the process to end and waits for it to exit.
    Stopping { cause: StopCause },
    /// The process is gone. Terminal.
    Ended {
        end: RunEnd,
        exit: Option<ProcessExit>,
    },
}

/// Why Ren is stopping a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCause {
    Operator,
    Shutdown,
    /// The client refused to open the conversation.
    OpenRefused,
}

impl StopCause {
    fn end(self) -> RunEnd {
        match self {
            Self::Operator => RunEnd::Stopped,
            Self::Shutdown => RunEnd::Drained,
            Self::OpenRefused => RunEnd::OpenRefused,
        }
    }
}

/// What the operator asks of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCommand {
    Prompt(Prompt),
    Answer {
        approval: ApprovalId,
        decision: Decision,
    },
    Interrupt,
    Stop,
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
                approvals: BTreeSet::new(),
            }),
            (
                Self::Working {
                    thread, approvals, ..
                },
                DriveEvent::TurnStarted { turn },
            ) => Self::to(Self::Working {
                thread: *thread,
                turn: Some(turn),
                approvals: approvals.clone(),
            }),
            (Self::Idle { thread }, DriveEvent::Approval { approval, .. }) => {
                Self::to(Self::Working {
                    thread: *thread,
                    turn: None,
                    approvals: BTreeSet::from([approval]),
                })
            }
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                },
                DriveEvent::Approval { approval, .. },
            ) => {
                let mut approvals = approvals.clone();
                approvals.insert(approval);
                Self::to(Self::Working {
                    thread: *thread,
                    turn: *turn,
                    approvals,
                })
            }
            (
                Self::Working {
                    thread,
                    turn,
                    approvals,
                },
                DriveEvent::ApprovalResolved { approval },
            ) => {
                let mut approvals = approvals.clone();
                approvals.remove(&approval);
                Self::to(Self::Working {
                    thread: *thread,
                    turn: *turn,
                    approvals,
                })
            }
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
            RequestKind::TurnStart | RequestKind::TurnInterrupt => false,
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
                | RunCommand::Answer { .. }
                | RunCommand::Interrupt
                | RunCommand::Stop,
            ) => Err(RunRefused::Stopping),
            (
                Self::Ended { .. },
                RunCommand::Prompt(_)
                | RunCommand::Answer { .. }
                | RunCommand::Interrupt
                | RunCommand::Stop,
            ) => Err(RunRefused::Ended),
            (Self::Starting | Self::Idle { .. } | Self::Working { .. }, RunCommand::Stop) => {
                Ok(Self::stop(StopCause::Operator))
            }
            (
                Self::Starting,
                RunCommand::Prompt(_) | RunCommand::Interrupt | RunCommand::Answer { .. },
            ) => Err(RunRefused::Starting),
            (Self::Idle { thread }, RunCommand::Prompt(prompt)) => Ok(Self::send(
                Self::Working {
                    thread: *thread,
                    turn: None,
                    approvals: BTreeSet::new(),
                },
                DriveCommand::Prompt(prompt),
            )),
            (Self::Working { .. }, RunCommand::Prompt(_)) => Err(RunRefused::Busy),
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
    use super::{ProcessExit, RunAction, RunCommand, RunEvent, RunRefused, RunState, StopCause};
    use ren_harness::drive::{
        ApprovalAsk, ApprovalId, Decision, DriveCommand, DriveEvent, Prompt, RequestKind, TurnEnd,
    };
    use ren_protocol::native::{NativeThreadId, NativeTurnId};
    use ren_recorder::capture::RunEnd;
    use std::collections::BTreeSet;
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
                approvals: BTreeSet::from([approval("a1")]),
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
            (
                RunState::Working {
                    thread: thread(),
                    turn: Some(turn()),
                    approvals: BTreeSet::new(),
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
}
