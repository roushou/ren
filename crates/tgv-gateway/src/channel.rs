//! The lifecycle of one WebSocket channel and the calls it carries, as a pure
//! state machine.
//!
//! A channel is one upgraded client socket pinned to one upstream socket. Each
//! `response.create` the client sends opens a call; the call ends with its
//! terminal event, or when either socket closes or fails, or at shutdown. The
//! relay classifies every message (see `tgv_protocol::websocket`) and feeds it here;
//! the machine decides which call owns the message, whether it is forwarded,
//! which calls end and how, and which close frames to send. It performs no I/O.
//!
//! Responses are matched by their provider-native identifier `R`, so an
//! interrupt and its acknowledgement are attributed to the call they name and
//! never to whichever call happens to be open.

use tgv_protocol::websocket::{ClientMessage, CloseCode, InterruptOutcome, UpstreamMessage};
use tgv_recorder::capture::{
    AttemptPhase, AttemptTerminal, ChannelEnd, TerminalCause, TerminalRecord,
};

/// A call's position among the calls a channel has opened, counting from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallSlot(u64);

impl CallSlot {
    pub fn ordinal(self) -> u64 {
        self.0
    }
}

/// Which socket an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Client,
    Upstream,
}

/// What the relay observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent<R> {
    Client(ClientMessage<R>),
    Upstream(UpstreamMessage<R>),
    /// A close frame arrived, with its code if it carried one.
    Closed {
        side: Side,
        code: Option<CloseCode>,
    },
    /// The socket failed or ended without a close frame.
    Failed {
        side: Side,
    },
    /// Shutdown started: close once no call is open.
    Drain,
    /// Shutdown's deadline passed: close now.
    Abort,
    /// The operator asked to stop this call.
    OperatorInterrupt(CallSlot),
    /// The relay was dropped without an end.
    Dropped,
}

/// Which body a message is recorded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Call(CallSlot),
    /// No call was open, or none matched the response it named.
    Unattributed,
}

/// What the shell must do, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsAction {
    /// Admit and start recording a new call. Its first message follows.
    OpenCall(CallSlot),
    /// Forward the message just observed, recorded under `Owner`.
    Forward(Owner),
    /// Do not forward the message just observed. It is still recorded.
    Withhold(Owner),
    /// Record the call's terminal and release its admission.
    FinishCall(CallSlot, TerminalRecord),
    /// Send `response.interrupt` upstream for this call's response, on the
    /// operator's behalf. Its answer is withheld from the client.
    Interrupt(CallSlot),
    /// Send a close frame to this side, with this code.
    Close { side: Side, code: CloseCode },
    /// The channel ended; record it. Nothing follows.
    End(ChannelEnd),
}

/// Where a channel is. `Ended` is the only terminal state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum WsChannel<R> {
    Open(OpenChannel<R>),
    Ended(ChannelEnd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChannel<R> {
    /// Open calls, oldest first. Upstream events belong to the oldest: one
    /// response runs at a time per connection.
    calls: Vec<WsCall<R>>,
    next: CallSlot,
    admission: Admission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Accepting,
    /// Shutdown started: no new calls; close when the last one ends.
    Draining,
}

/// One call on a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WsCall<R> {
    slot: CallSlot,
    response: Option<R>,
    progress: Progress,
    interrupt: Interrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// The create was forwarded; no upstream event for it yet.
    Awaiting,
    Responding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    None,
    Requested,
    Answered(InterruptOutcome),
    /// The operator asked before the response had an id: send once it does.
    OperatorPending,
    /// TGV sent an interrupt for the operator; the answer, once it arrives.
    Operator(Option<InterruptOutcome>),
}

/// The state after an event, and the actions it requires.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Transition<R> {
    pub state: WsChannel<R>,
    pub actions: Vec<WsAction>,
}

/// Events after the end are expected races (a message read just before the
/// other side's close); the shell decides what to do with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IllegalTransition {
    #[error("the channel already ended ({end})")]
    AfterEnd { end: ChannelEnd },
}

impl<R: PartialEq> WsChannel<R> {
    /// A channel whose handshake just completed.
    pub fn opened() -> Self {
        Self::Open(OpenChannel {
            calls: Vec::new(),
            next: CallSlot(0),
            admission: Admission::Accepting,
        })
    }

    /// The response id of an open call, once the upstream named it.
    pub fn response(&self, slot: CallSlot) -> Option<&R> {
        match self {
            Self::Ended(_) => None,
            Self::Open(open) => open
                .calls
                .iter()
                .find(|call| call.slot == slot)
                .and_then(|call| call.response.as_ref()),
        }
    }

    pub fn step(self, event: WsEvent<R>) -> Result<Transition<R>, IllegalTransition> {
        match self {
            Self::Ended(end) => Err(IllegalTransition::AfterEnd { end }),
            Self::Open(open) => Ok(open.step(event)),
        }
    }
}

impl<R: PartialEq> OpenChannel<R> {
    fn step(mut self, event: WsEvent<R>) -> Transition<R> {
        let mut actions = Vec::new();
        match event {
            WsEvent::Client(message) => self.client(message, &mut actions),
            WsEvent::Upstream(message) => self.upstream(message, &mut actions),
            WsEvent::Closed { side, code } => {
                let (end, cause) = match side {
                    Side::Client => (ChannelEnd::ClientClosed, Ending::Client),
                    Side::Upstream => (ChannelEnd::UpstreamClosed, Ending::UpstreamClosed),
                };
                // A close frame without a code is forwarded as a normal close:
                // "no status" (1005) cannot be sent.
                let code = code.unwrap_or(CloseCode::NORMAL);
                return self.end(end, cause, &[(side.other(), code)], actions);
            }
            WsEvent::Failed { side } => {
                let (end, cause, close) = match side {
                    Side::Client => (
                        ChannelEnd::ClientFailed,
                        Ending::Client,
                        (Side::Upstream, CloseCode::GOING_AWAY),
                    ),
                    Side::Upstream => (
                        ChannelEnd::UpstreamFailed,
                        Ending::UpstreamFailed,
                        (Side::Client, CloseCode::BAD_GATEWAY),
                    ),
                };
                return self.end(end, cause, &[close], actions);
            }
            WsEvent::Drain => self.admission = Admission::Draining,
            WsEvent::OperatorInterrupt(slot) => {
                if let Some(call) = self.calls.iter_mut().find(|call| call.slot == slot) {
                    match (call.interrupt, call.response.is_some()) {
                        (Interrupt::Operator(_) | Interrupt::OperatorPending, _) => {}
                        (Interrupt::None | Interrupt::Requested | Interrupt::Answered(_), true) => {
                            call.interrupt = Interrupt::Operator(None);
                            actions.push(WsAction::Interrupt(slot));
                        }
                        (
                            Interrupt::None | Interrupt::Requested | Interrupt::Answered(_),
                            false,
                        ) => {
                            call.interrupt = Interrupt::OperatorPending;
                        }
                    }
                }
            }
            WsEvent::Abort => {
                return self.end(
                    ChannelEnd::Aborted,
                    Ending::Shutdown,
                    &Self::SHUTDOWN_CLOSE,
                    actions,
                );
            }
            WsEvent::Dropped => {
                return self.end(ChannelEnd::Dropped, Ending::Dropped, &[], actions);
            }
        }
        match self.admission {
            Admission::Draining if self.calls.is_empty() => self.end(
                ChannelEnd::Drained,
                Ending::Shutdown,
                &Self::SHUTDOWN_CLOSE,
                actions,
            ),
            Admission::Accepting | Admission::Draining => Transition {
                state: WsChannel::Open(self),
                actions,
            },
        }
    }

    /// The client may reconnect; the upstream is told TGV is going away.
    const SHUTDOWN_CLOSE: [(Side, CloseCode); 2] = [
        (Side::Client, CloseCode::SERVICE_RESTART),
        (Side::Upstream, CloseCode::GOING_AWAY),
    ];

    fn client(&mut self, message: ClientMessage<R>, actions: &mut Vec<WsAction>) {
        match message {
            ClientMessage::Create => match self.admission {
                Admission::Accepting => {
                    let slot = self.next;
                    self.next = CallSlot(slot.0.saturating_add(1));
                    self.calls.push(WsCall {
                        slot,
                        response: None,
                        progress: Progress::Awaiting,
                        interrupt: Interrupt::None,
                    });
                    actions.push(WsAction::OpenCall(slot));
                    actions.push(WsAction::Forward(Owner::Call(slot)));
                }
                // A call started now could outlive the drain; the client
                // retries it after the 1012 close.
                Admission::Draining => actions.push(WsAction::Withhold(Owner::Unattributed)),
            },
            ClientMessage::Interrupt(response) => {
                let owner = match self.named(response.as_ref()) {
                    Some(call) => {
                        call.interrupt = Interrupt::Requested;
                        Owner::Call(call.slot)
                    }
                    None => Owner::Unattributed,
                };
                actions.push(WsAction::Forward(owner));
            }
            ClientMessage::Other => actions.push(WsAction::Forward(self.oldest())),
        }
    }

    fn upstream(&mut self, message: UpstreamMessage<R>, actions: &mut Vec<WsAction>) {
        match message {
            UpstreamMessage::Created(response) => {
                let (owner, interrupt) = match self.calls.first_mut() {
                    Some(call) => {
                        call.progress = Progress::Responding;
                        if call.response.is_none() {
                            call.response = response;
                        }
                        let interrupt =
                            call.interrupt == Interrupt::OperatorPending && call.response.is_some();
                        if interrupt {
                            call.interrupt = Interrupt::Operator(None);
                        }
                        (Owner::Call(call.slot), interrupt.then_some(call.slot))
                    }
                    None => (Owner::Unattributed, None),
                };
                actions.push(WsAction::Forward(owner));
                actions.extend(interrupt.map(WsAction::Interrupt));
            }
            UpstreamMessage::Other => {
                if let Some(call) = self.calls.first_mut() {
                    call.progress = Progress::Responding;
                }
                actions.push(WsAction::Forward(self.oldest()));
            }
            UpstreamMessage::InterruptAnswered { response, outcome } => {
                actions.push(match self.named(response.as_ref()) {
                    // The client never sent this interrupt: keep the answer.
                    Some(call) if matches!(call.interrupt, Interrupt::Operator(_)) => {
                        call.interrupt = Interrupt::Operator(Some(outcome));
                        WsAction::Withhold(Owner::Call(call.slot))
                    }
                    Some(call) => {
                        call.interrupt = Interrupt::Answered(outcome);
                        WsAction::Forward(Owner::Call(call.slot))
                    }
                    None => WsAction::Forward(Owner::Unattributed),
                });
            }
            UpstreamMessage::Terminal => {
                if self.calls.is_empty() {
                    actions.push(WsAction::Forward(Owner::Unattributed));
                } else {
                    let call = self.calls.remove(0);
                    actions.push(WsAction::Forward(Owner::Call(call.slot)));
                    actions.push(WsAction::FinishCall(call.slot, call.terminal_event()));
                }
            }
        }
    }

    /// The open call for `response`. A message that names no response, or one
    /// no open call has, belongs to no call.
    fn named(&mut self, response: Option<&R>) -> Option<&mut WsCall<R>> {
        let response = response?;
        self.calls
            .iter_mut()
            .find(|call| call.response.as_ref() == Some(response))
    }

    fn oldest(&self) -> Owner {
        self.calls
            .first()
            .map_or(Owner::Unattributed, |call| Owner::Call(call.slot))
    }

    fn end(
        self,
        end: ChannelEnd,
        cause: Ending,
        closes: &[(Side, CloseCode)],
        mut actions: Vec<WsAction>,
    ) -> Transition<R> {
        for call in self.calls {
            actions.push(WsAction::FinishCall(call.slot, call.ended(cause)));
        }
        for &(side, code) in closes {
            actions.push(WsAction::Close { side, code });
        }
        actions.push(WsAction::End(end));
        Transition {
            state: WsChannel::Ended(end),
            actions,
        }
    }
}

/// Why the calls still open at a channel's end stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Client,
    UpstreamClosed,
    UpstreamFailed,
    Shutdown,
    Dropped,
}

impl<R> WsCall<R> {
    fn phase(&self) -> AttemptPhase {
        match self.progress {
            Progress::Awaiting => AttemptPhase::AwaitingHead,
            Progress::Responding => AttemptPhase::Response,
        }
    }

    /// The terminal event arrived. An interrupt the upstream accepted is the
    /// client's doing; a rejected or unanswered one is not evidence of anything.
    fn terminal_event(&self) -> TerminalRecord {
        let cause = match self.interrupt {
            Interrupt::Answered(InterruptOutcome::Accepted) => Some(TerminalCause::Client),
            Interrupt::Operator(Some(InterruptOutcome::Accepted)) => Some(TerminalCause::Operator),
            Interrupt::None
            | Interrupt::Requested
            | Interrupt::Answered(InterruptOutcome::Rejected)
            | Interrupt::OperatorPending
            | Interrupt::Operator(None | Some(InterruptOutcome::Rejected)) => None,
        };
        TerminalRecord {
            terminal: AttemptTerminal::TerminalEvent,
            cause,
            phase: Some(AttemptPhase::Response),
        }
    }

    fn ended(&self, ending: Ending) -> TerminalRecord {
        let (terminal, cause) = match ending {
            Ending::Client => (AttemptTerminal::Cancelled, Some(TerminalCause::Client)),
            Ending::UpstreamClosed => (
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::UpstreamClosed),
            ),
            Ending::UpstreamFailed => (
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::BodyError),
            ),
            Ending::Shutdown => (AttemptTerminal::Cancelled, Some(TerminalCause::Shutdown)),
            Ending::Dropped => (AttemptTerminal::Interrupted, None),
        };
        TerminalRecord {
            terminal,
            cause,
            phase: Some(self.phase()),
        }
    }
}

impl Side {
    fn other(self) -> Self {
        match self {
            Self::Client => Self::Upstream,
            Self::Upstream => Self::Client,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CallSlot, ChannelEnd, ClientMessage, CloseCode, IllegalTransition, InterruptOutcome, Owner,
        Side, UpstreamMessage, WsAction, WsChannel, WsEvent,
    };
    use tgv_recorder::capture::{AttemptPhase, AttemptTerminal, TerminalCause, TerminalRecord};

    #[test]
    fn an_operator_interrupt_is_sent_once_and_its_answer_withheld() {
        let slot = CallSlot(0);
        let (state, actions) = run(vec![
            Event::Client(ClientMessage::Create),
            // Before the response has an id: sent once it does.
            Event::OperatorInterrupt(slot),
            Event::Upstream(UpstreamMessage::Created(Some("resp_1"))),
            Event::OperatorInterrupt(slot),
            Event::Upstream(UpstreamMessage::InterruptAnswered {
                response: Some("resp_1"),
                outcome: InterruptOutcome::Accepted,
            }),
            Event::Upstream(UpstreamMessage::Terminal),
        ]);
        assert!(matches!(state, WsChannel::Open(_)));
        assert_eq!(
            actions,
            vec![
                WsAction::OpenCall(slot),
                WsAction::Forward(Owner::Call(slot)),
                WsAction::Forward(Owner::Call(slot)),
                WsAction::Interrupt(slot),
                WsAction::Withhold(Owner::Call(slot)),
                WsAction::Forward(Owner::Call(slot)),
                WsAction::FinishCall(
                    slot,
                    record(
                        AttemptTerminal::TerminalEvent,
                        Some(TerminalCause::Operator),
                        AttemptPhase::Response,
                    ),
                ),
            ]
        );
        // An interrupt for a call that is not open does nothing.
        let (_, actions) = run(vec![Event::OperatorInterrupt(CallSlot(9))]);
        assert!(actions.is_empty());
    }

    type Channel = WsChannel<&'static str>;
    type Event = WsEvent<&'static str>;

    /// Run events from a fresh channel, returning the final state and every action.
    fn run(events: Vec<Event>) -> (Channel, Vec<WsAction>) {
        events.into_iter().fold(
            (Channel::opened(), Vec::new()),
            |(state, mut actions), event| {
                let transition = state.step(event).unwrap();
                actions.extend(transition.actions);
                (transition.state, actions)
            },
        )
    }

    fn record(
        terminal: AttemptTerminal,
        cause: Option<TerminalCause>,
        phase: AttemptPhase,
    ) -> TerminalRecord {
        TerminalRecord {
            terminal,
            cause,
            phase: Some(phase),
        }
    }

    const FIRST: CallSlot = CallSlot(0);
    const SECOND: CallSlot = CallSlot(1);

    #[test]
    fn a_call_runs_from_create_to_its_terminal_event() {
        let (state, actions) = run(vec![
            WsEvent::Client(ClientMessage::Create),
            WsEvent::Upstream(UpstreamMessage::Created(Some("resp_1"))),
            WsEvent::Upstream(UpstreamMessage::Other),
            WsEvent::Upstream(UpstreamMessage::Terminal),
            // Between calls, events belong to nothing.
            WsEvent::Upstream(UpstreamMessage::Other),
            WsEvent::Client(ClientMessage::Create),
        ]);
        assert!(matches!(state, WsChannel::Open(_)));
        assert_eq!(
            actions,
            vec![
                WsAction::OpenCall(FIRST),
                WsAction::Forward(Owner::Call(FIRST)),
                WsAction::Forward(Owner::Call(FIRST)),
                WsAction::Forward(Owner::Call(FIRST)),
                WsAction::Forward(Owner::Call(FIRST)),
                WsAction::FinishCall(
                    FIRST,
                    record(AttemptTerminal::TerminalEvent, None, AttemptPhase::Response)
                ),
                WsAction::Forward(Owner::Unattributed),
                WsAction::OpenCall(SECOND),
                WsAction::Forward(Owner::Call(SECOND)),
            ]
        );
    }

    #[test]
    fn an_accepted_interrupt_is_the_clients_cause_and_the_terminal_still_passes() {
        for (outcome, cause) in [
            (InterruptOutcome::Accepted, Some(TerminalCause::Client)),
            (InterruptOutcome::Rejected, None),
        ] {
            let (_, actions) = run(vec![
                WsEvent::Client(ClientMessage::Create),
                WsEvent::Upstream(UpstreamMessage::Created(Some("resp_1"))),
                WsEvent::Client(ClientMessage::Interrupt(Some("resp_1"))),
                WsEvent::Upstream(UpstreamMessage::InterruptAnswered {
                    response: Some("resp_1"),
                    outcome,
                }),
                WsEvent::Upstream(UpstreamMessage::Terminal),
            ]);
            assert_eq!(
                actions[3..],
                [
                    WsAction::Forward(Owner::Call(FIRST)),
                    WsAction::Forward(Owner::Call(FIRST)),
                    WsAction::Forward(Owner::Call(FIRST)),
                    WsAction::FinishCall(
                        FIRST,
                        record(
                            AttemptTerminal::TerminalEvent,
                            cause,
                            AttemptPhase::Response
                        )
                    ),
                ]
            );
        }
    }

    #[test]
    fn an_interrupt_for_another_response_belongs_to_no_call() {
        let (_, actions) = run(vec![
            WsEvent::Client(ClientMessage::Create),
            WsEvent::Upstream(UpstreamMessage::Created(Some("resp_2"))),
            // A late interrupt for an earlier response, and its answer.
            WsEvent::Client(ClientMessage::Interrupt(Some("resp_1"))),
            WsEvent::Upstream(UpstreamMessage::InterruptAnswered {
                response: Some("resp_1"),
                outcome: InterruptOutcome::Accepted,
            }),
            WsEvent::Client(ClientMessage::Interrupt(None)),
            WsEvent::Upstream(UpstreamMessage::Terminal),
        ]);
        assert_eq!(
            actions[3..],
            [
                WsAction::Forward(Owner::Unattributed),
                WsAction::Forward(Owner::Unattributed),
                WsAction::Forward(Owner::Unattributed),
                WsAction::Forward(Owner::Call(FIRST)),
                WsAction::FinishCall(
                    FIRST,
                    record(AttemptTerminal::TerminalEvent, None, AttemptPhase::Response)
                ),
            ]
        );
    }

    #[test]
    fn closes_and_failures_end_open_calls_and_propagate() {
        let cases = [
            (
                WsEvent::Closed {
                    side: Side::Client,
                    code: Some(CloseCode::new(4000)),
                },
                ChannelEnd::ClientClosed,
                (AttemptTerminal::Cancelled, Some(TerminalCause::Client)),
                vec![(Side::Upstream, CloseCode::new(4000))],
            ),
            (
                WsEvent::Closed {
                    side: Side::Upstream,
                    code: None,
                },
                ChannelEnd::UpstreamClosed,
                (
                    AttemptTerminal::StreamFailed,
                    Some(TerminalCause::UpstreamClosed),
                ),
                vec![(Side::Client, CloseCode::NORMAL)],
            ),
            (
                WsEvent::Failed { side: Side::Client },
                ChannelEnd::ClientFailed,
                (AttemptTerminal::Cancelled, Some(TerminalCause::Client)),
                vec![(Side::Upstream, CloseCode::GOING_AWAY)],
            ),
            (
                WsEvent::Failed {
                    side: Side::Upstream,
                },
                ChannelEnd::UpstreamFailed,
                (
                    AttemptTerminal::StreamFailed,
                    Some(TerminalCause::BodyError),
                ),
                vec![(Side::Client, CloseCode::BAD_GATEWAY)],
            ),
            (
                WsEvent::Abort,
                ChannelEnd::Aborted,
                (AttemptTerminal::Cancelled, Some(TerminalCause::Shutdown)),
                vec![
                    (Side::Client, CloseCode::SERVICE_RESTART),
                    (Side::Upstream, CloseCode::GOING_AWAY),
                ],
            ),
            (
                WsEvent::Dropped,
                ChannelEnd::Dropped,
                (AttemptTerminal::Interrupted, None),
                vec![],
            ),
        ];
        for (event, end, (terminal, cause), closes) in cases {
            let (state, actions) = run(vec![
                WsEvent::Client(ClientMessage::Create),
                WsEvent::Upstream(UpstreamMessage::Created(Some("resp_1"))),
                WsEvent::Client(ClientMessage::Create),
                event,
            ]);
            assert_eq!(state, WsChannel::Ended(end));
            let mut expected = vec![
                WsAction::FinishCall(FIRST, record(terminal, cause, AttemptPhase::Response)),
                // The second call never saw an upstream event.
                WsAction::FinishCall(SECOND, record(terminal, cause, AttemptPhase::AwaitingHead)),
            ];
            expected.extend(
                closes
                    .into_iter()
                    .map(|(side, code)| WsAction::Close { side, code }),
            );
            expected.push(WsAction::End(end));
            assert_eq!(actions[5..], expected[..], "{end}");
        }
    }

    #[test]
    fn draining_closes_when_idle_and_withholds_new_calls() {
        // Idle: close at once.
        let (state, actions) = run(vec![WsEvent::Drain]);
        assert_eq!(state, WsChannel::Ended(ChannelEnd::Drained));
        assert_eq!(
            actions,
            vec![
                WsAction::Close {
                    side: Side::Client,
                    code: CloseCode::SERVICE_RESTART
                },
                WsAction::Close {
                    side: Side::Upstream,
                    code: CloseCode::GOING_AWAY
                },
                WsAction::End(ChannelEnd::Drained),
            ]
        );

        // Busy: the running call finishes; a new create is withheld; then close.
        let (state, actions) = run(vec![
            WsEvent::Client(ClientMessage::Create),
            WsEvent::Drain,
            WsEvent::Client(ClientMessage::Create),
            WsEvent::Upstream(UpstreamMessage::Terminal),
        ]);
        assert_eq!(state, WsChannel::Ended(ChannelEnd::Drained));
        assert_eq!(
            actions[2..],
            [
                WsAction::Withhold(Owner::Unattributed),
                WsAction::Forward(Owner::Call(FIRST)),
                WsAction::FinishCall(
                    FIRST,
                    record(AttemptTerminal::TerminalEvent, None, AttemptPhase::Response)
                ),
                WsAction::Close {
                    side: Side::Client,
                    code: CloseCode::SERVICE_RESTART
                },
                WsAction::Close {
                    side: Side::Upstream,
                    code: CloseCode::GOING_AWAY
                },
                WsAction::End(ChannelEnd::Drained),
            ]
        );
    }

    #[test]
    fn events_after_the_end_are_illegal() {
        let (state, _) = run(vec![WsEvent::Dropped]);
        assert_eq!(
            state.step(WsEvent::Upstream(UpstreamMessage::Other)),
            Err(IllegalTransition::AfterEnd {
                end: ChannelEnd::Dropped
            })
        );
    }

    #[test]
    fn channel_end_labels_are_unique() {}
}
