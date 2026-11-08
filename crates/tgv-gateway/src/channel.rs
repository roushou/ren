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
//!
//! Every event comes with the monotonic time it was observed, and the relay
//! sends `Tick` periodically. From those the machine decides when a quiet side
//! is pinged, when a side that stays silent is unresponsive, and when the
//! running call's upstream has stalled (see [`Deadlines`]).

use std::time::Duration;
use tgv_protocol::websocket::{ClientMessage, CloseCode, InterruptOutcome, UpstreamMessage};
use tgv_recorder::{
    capture::{AttemptPhase, AttemptTerminal, ChannelEnd, TerminalCause, TerminalRecord},
    time::Monotonic,
};

/// How long a channel's sides and its running call may stay quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines {
    /// A side silent this long is pinged. Silent twice this long, it is
    /// unresponsive and the channel ends. Any message, ping, or pong counts.
    pub keepalive: Duration,
    /// The oldest call, forwarded upstream, with no upstream message yet.
    pub first_message: Duration,
    /// The oldest call, responding, with no upstream message since its last.
    pub stream_idle: Duration,
}

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
    /// A ping or pong arrived: the side is alive. Never returns actions.
    Control {
        side: Side,
    },
    /// The gateway admitted the call and its create is being forwarded.
    /// Never returns actions.
    Admitted(CallSlot),
    /// Time passed: check the deadlines.
    Tick,
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
    /// Send a ping to this side, which has been quiet.
    Ping(Side),
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
    deadlines: Deadlines,
    client: Liveness,
    upstream: Liveness,
}

/// When a side was last heard from, and whether it has been pinged since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Liveness {
    heard: Monotonic,
    ping: Ping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ping {
    /// Not pinged since the side was last heard.
    Due,
    Sent,
}

impl Liveness {
    fn new(at: Monotonic) -> Self {
        Self {
            heard: at,
            ping: Ping::Due,
        }
    }

    fn heard(&mut self, at: Monotonic) {
        *self = Self::new(at);
    }

    fn unresponsive(self, at: Monotonic, keepalive: Duration) -> bool {
        at.saturating_since(self.heard) >= keepalive.saturating_mul(2)
    }

    /// Whether to ping now; marks the ping sent.
    fn ping(&mut self, at: Monotonic, keepalive: Duration) -> bool {
        let due = self.ping == Ping::Due && at.saturating_since(self.heard) >= keepalive;
        if due {
            self.ping = Ping::Sent;
        }
        due
    }
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
    /// Since when the upstream has been quiet for this call: its admission,
    /// then its latest upstream message.
    quiet_since: Monotonic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// Waiting for the gateway's call admission; the create is not forwarded yet.
    Admitting,
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
    /// A channel whose handshake just completed, at `at`.
    pub fn opened(deadlines: Deadlines, at: Monotonic) -> Self {
        Self::Open(OpenChannel {
            calls: Vec::new(),
            next: CallSlot(0),
            admission: Admission::Accepting,
            deadlines,
            client: Liveness::new(at),
            upstream: Liveness::new(at),
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

    /// Apply an event observed at `at`.
    pub fn step(
        self,
        at: Monotonic,
        event: WsEvent<R>,
    ) -> Result<Transition<R>, IllegalTransition> {
        match self {
            Self::Ended(end) => Err(IllegalTransition::AfterEnd { end }),
            Self::Open(open) => Ok(open.step(at, event)),
        }
    }
}

impl<R: PartialEq> OpenChannel<R> {
    fn step(mut self, at: Monotonic, event: WsEvent<R>) -> Transition<R> {
        let mut actions = Vec::new();
        match event {
            WsEvent::Client(message) => {
                self.client.heard(at);
                self.client(at, message, &mut actions);
            }
            WsEvent::Upstream(message) => {
                self.upstream.heard(at);
                self.upstream(at, message, &mut actions);
            }
            WsEvent::Control { side } => self.side(side).heard(at),
            WsEvent::Admitted(slot) => {
                if let Some(call) = self.calls.iter_mut().find(|call| call.slot == slot)
                    && call.progress == Progress::Admitting
                {
                    call.progress = Progress::Awaiting;
                    call.quiet_since = at;
                }
            }
            WsEvent::Tick => {
                if let Some(expiry) = self.expired(at) {
                    return self.end(expiry.end(), expiry.ending(), expiry.closes(), actions);
                }
                let keepalive = self.deadlines.keepalive;
                for side in [Side::Client, Side::Upstream] {
                    if self.side(side).ping(at, keepalive) {
                        actions.push(WsAction::Ping(side));
                    }
                }
            }
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

    fn side(&mut self, side: Side) -> &mut Liveness {
        match side {
            Side::Client => &mut self.client,
            Side::Upstream => &mut self.upstream,
        }
    }

    /// The deadline that passed at `at`, if one did: a side that stayed
    /// silent through its ping, or the running call's upstream going quiet.
    fn expired(&self, at: Monotonic) -> Option<Expiry> {
        let keepalive = self.deadlines.keepalive;
        if self.client.unresponsive(at, keepalive) {
            return Some(Expiry::ClientUnresponsive);
        }
        if self.upstream.unresponsive(at, keepalive) {
            return Some(Expiry::UpstreamUnresponsive);
        }
        let call = self.calls.first()?;
        let limit = match call.progress {
            Progress::Admitting => return None,
            Progress::Awaiting => self.deadlines.first_message,
            Progress::Responding => self.deadlines.stream_idle,
        };
        (at.saturating_since(call.quiet_since) >= limit).then_some(Expiry::UpstreamStalled)
    }

    fn client(&mut self, at: Monotonic, message: ClientMessage<R>, actions: &mut Vec<WsAction>) {
        match message {
            ClientMessage::Create => match self.admission {
                Admission::Accepting => {
                    let slot = self.next;
                    self.next = CallSlot(slot.0.saturating_add(1));
                    self.calls.push(WsCall {
                        slot,
                        response: None,
                        progress: Progress::Admitting,
                        interrupt: Interrupt::None,
                        quiet_since: at,
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

    fn upstream(
        &mut self,
        at: Monotonic,
        message: UpstreamMessage<R>,
        actions: &mut Vec<WsAction>,
    ) {
        if let Some(call) = self.calls.first_mut() {
            call.heard(at);
        }
        match message {
            UpstreamMessage::Created(response) => {
                let (owner, interrupt) = match self.calls.first_mut() {
                    Some(call) => {
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

/// A deadline that passed, ending the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expiry {
    ClientUnresponsive,
    UpstreamUnresponsive,
    UpstreamStalled,
}

impl Expiry {
    fn end(self) -> ChannelEnd {
        match self {
            Self::ClientUnresponsive => ChannelEnd::ClientUnresponsive,
            Self::UpstreamUnresponsive => ChannelEnd::UpstreamUnresponsive,
            Self::UpstreamStalled => ChannelEnd::UpstreamStalled,
        }
    }

    fn ending(self) -> Ending {
        match self {
            Self::ClientUnresponsive => Ending::Client,
            Self::UpstreamUnresponsive => Ending::UpstreamUnresponsive,
            Self::UpstreamStalled => Ending::Stalled,
        }
    }

    /// The close frames to send: the silent side gets none it could not read.
    fn closes(self) -> &'static [(Side, CloseCode)] {
        match self {
            Self::ClientUnresponsive => &[(Side::Upstream, CloseCode::GOING_AWAY)],
            Self::UpstreamUnresponsive => &[(Side::Client, CloseCode::BAD_GATEWAY)],
            Self::UpstreamStalled => &[
                (Side::Client, CloseCode::BAD_GATEWAY),
                (Side::Upstream, CloseCode::GOING_AWAY),
            ],
        }
    }
}

/// Why the calls still open at a channel's end stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Client,
    UpstreamClosed,
    UpstreamFailed,
    /// The upstream did not answer TGV's ping.
    UpstreamUnresponsive,
    /// The upstream sent nothing for the running call past its deadline.
    Stalled,
    Shutdown,
    Dropped,
}

impl<R> WsCall<R> {
    /// An upstream message arrived for this call, the oldest.
    fn heard(&mut self, at: Monotonic) {
        self.progress = Progress::Responding;
        self.quiet_since = at;
    }

    fn phase(&self) -> AttemptPhase {
        match self.progress {
            Progress::Admitting | Progress::Awaiting => AttemptPhase::AwaitingHead,
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
            Ending::UpstreamUnresponsive => (
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::IdleTimeout),
            ),
            // By how far the call had got: a create never forwarded was not
            // sent; one with no upstream message may have been accepted.
            Ending::Stalled => match self.progress {
                Progress::Admitting => (AttemptTerminal::Cancelled, None),
                Progress::Awaiting => (
                    AttemptTerminal::AcceptanceUnknown,
                    Some(TerminalCause::HeaderTimeout),
                ),
                Progress::Responding => (
                    AttemptTerminal::StreamFailed,
                    Some(TerminalCause::IdleTimeout),
                ),
            },
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
        CallSlot, ChannelEnd, ClientMessage, CloseCode, Deadlines, IllegalTransition,
        InterruptOutcome, Owner, Side, UpstreamMessage, WsAction, WsChannel, WsEvent,
    };
    use std::time::Duration;
    use tgv_recorder::{
        capture::{AttemptPhase, AttemptTerminal, TerminalCause, TerminalRecord},
        time::Monotonic,
    };

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

    const DEADLINES: Deadlines = Deadlines {
        keepalive: Duration::from_secs(30),
        first_message: Duration::from_secs(120),
        stream_idle: Duration::from_secs(60),
    };

    fn second(at: u64) -> Monotonic {
        Monotonic::from_elapsed(Duration::from_secs(at))
    }

    /// Run events from a fresh channel, all at the moment it opened,
    /// returning the final state and every action.
    fn run(events: Vec<Event>) -> (Channel, Vec<WsAction>) {
        run_at(events.into_iter().map(|event| (0, event)).collect())
    }

    /// Run events, each at its second since the channel opened.
    fn run_at(events: Vec<(u64, Event)>) -> (Channel, Vec<WsAction>) {
        events.into_iter().fold(
            (Channel::opened(DEADLINES, second(0)), Vec::new()),
            |(state, mut actions), (at, event)| {
                let transition = state.step(second(at), event).unwrap();
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
            state.step(second(0), WsEvent::Upstream(UpstreamMessage::Other)),
            Err(IllegalTransition::AfterEnd {
                end: ChannelEnd::Dropped
            })
        );
    }

    #[test]
    fn a_quiet_side_is_pinged_once_and_answering_keeps_the_channel_open() {
        let (state, actions) = run_at(vec![
            (29, WsEvent::Tick),
            // Both sides quiet for the keepalive: each is pinged, once.
            (30, WsEvent::Tick),
            (31, WsEvent::Tick),
            // Both answer; the client is pinged again only after another
            // quiet keepalive.
            (35, WsEvent::Control { side: Side::Client }),
            (
                36,
                WsEvent::Control {
                    side: Side::Upstream,
                },
            ),
            (64, WsEvent::Tick),
            (65, WsEvent::Tick),
            (66, WsEvent::Tick),
            // Any message counts as hearing from a side; a side already
            // pinged is not pinged again until it is heard.
            (
                70,
                WsEvent::Control {
                    side: Side::Upstream,
                },
            ),
            (80, WsEvent::Client(ClientMessage::Other)),
            (95, WsEvent::Tick),
            (110, WsEvent::Tick),
        ]);
        assert!(matches!(state, WsChannel::Open(_)));
        assert_eq!(
            actions,
            vec![
                WsAction::Ping(Side::Client),
                WsAction::Ping(Side::Upstream),
                WsAction::Ping(Side::Client),
                WsAction::Ping(Side::Upstream),
                WsAction::Forward(Owner::Unattributed),
                WsAction::Ping(Side::Client),
                WsAction::Ping(Side::Upstream),
            ]
        );
    }

    #[test]
    fn a_side_silent_through_its_ping_is_unresponsive() {
        let cases = [
            (
                Side::Client,
                ChannelEnd::ClientUnresponsive,
                (AttemptTerminal::Cancelled, Some(TerminalCause::Client)),
                vec![(Side::Upstream, CloseCode::GOING_AWAY)],
            ),
            (
                Side::Upstream,
                ChannelEnd::UpstreamUnresponsive,
                (
                    AttemptTerminal::StreamFailed,
                    Some(TerminalCause::IdleTimeout),
                ),
                vec![(Side::Client, CloseCode::BAD_GATEWAY)],
            ),
        ];
        for (silent, end, (terminal, cause), closes) in cases {
            let alive = match silent {
                Side::Client => WsEvent::Upstream(UpstreamMessage::Other),
                Side::Upstream => WsEvent::Control { side: Side::Client },
            };
            let (state, actions) = run_at(vec![
                (0, WsEvent::Client(ClientMessage::Create)),
                (0, WsEvent::Admitted(FIRST)),
                (
                    0,
                    WsEvent::Upstream(UpstreamMessage::Created(Some("resp_1"))),
                ),
                (30, WsEvent::Tick),
                (45, alive.clone()),
                (59, WsEvent::Tick),
                (59, alive),
                (60, WsEvent::Tick),
            ]);
            assert_eq!(state, WsChannel::Ended(end));
            let mut expected = vec![WsAction::FinishCall(
                FIRST,
                record(terminal, cause, AttemptPhase::Response),
            )];
            expected.extend(
                closes
                    .into_iter()
                    .map(|(side, code)| WsAction::Close { side, code }),
            );
            expected.push(WsAction::End(end));
            assert_eq!(
                actions[actions.len() - expected.len()..],
                expected[..],
                "{end}"
            );
        }
    }

    #[test]
    fn a_running_call_whose_upstream_goes_quiet_stalls_the_channel() {
        // Both sides answer pings throughout, so only the call's deadline passes.
        let alive = |at| {
            [
                (at, WsEvent::Control { side: Side::Client }),
                (
                    at,
                    WsEvent::Control {
                        side: Side::Upstream,
                    },
                ),
            ]
        };
        let ended = |phase, terminal, cause| {
            vec![
                WsAction::FinishCall(FIRST, record(terminal, cause, phase)),
                WsAction::Close {
                    side: Side::Client,
                    code: CloseCode::BAD_GATEWAY,
                },
                WsAction::Close {
                    side: Side::Upstream,
                    code: CloseCode::GOING_AWAY,
                },
                WsAction::End(ChannelEnd::UpstreamStalled),
            ]
        };

        // Forwarded, and no upstream message within the first-message deadline.
        let mut events = vec![
            (0, WsEvent::Client(ClientMessage::Create)),
            (10, WsEvent::Admitted(FIRST)),
        ];
        for at in [40, 80, 120] {
            events.extend(alive(at));
        }
        events.push((129, WsEvent::Tick));
        let (state, _) = run_at(events.clone());
        assert!(matches!(state, WsChannel::Open(_)));
        events.push((130, WsEvent::Tick));
        let (state, actions) = run_at(events);
        assert_eq!(state, WsChannel::Ended(ChannelEnd::UpstreamStalled));
        assert_eq!(
            actions[2..],
            ended(
                AttemptPhase::AwaitingHead,
                AttemptTerminal::AcceptanceUnknown,
                Some(TerminalCause::HeaderTimeout)
            )[..]
        );

        // Responding, then quiet past the stream deadline.
        let mut events = vec![
            (0, WsEvent::Client(ClientMessage::Create)),
            (0, WsEvent::Admitted(FIRST)),
            (
                5,
                WsEvent::Upstream(UpstreamMessage::Created(Some("resp_1"))),
            ),
            (50, WsEvent::Upstream(UpstreamMessage::Other)),
        ];
        events.extend(alive(80));
        events.push((110, WsEvent::Tick));
        let (_, actions) = run_at(events);
        assert_eq!(
            actions[actions.len() - 4..],
            ended(
                AttemptPhase::Response,
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::IdleTimeout)
            )[..]
        );
    }

    #[test]
    fn a_call_waiting_for_admission_has_no_upstream_deadline() {
        let mut events = vec![(0, WsEvent::Client(ClientMessage::Create))];
        for at in [30, 60, 90, 120, 150, 180] {
            events.push((at, WsEvent::Control { side: Side::Client }));
            events.push((
                at,
                WsEvent::Control {
                    side: Side::Upstream,
                },
            ));
            events.push((at, WsEvent::Tick));
        }
        let (state, _) = run_at(events.clone());
        assert!(matches!(state, WsChannel::Open(_)));
        // Its deadline starts once it is admitted and forwarded.
        events.push((181, WsEvent::Admitted(FIRST)));
        events.push((300, WsEvent::Control { side: Side::Client }));
        events.push((
            300,
            WsEvent::Control {
                side: Side::Upstream,
            },
        ));
        events.push((300, WsEvent::Tick));
        let (state, _) = run_at(events.clone());
        assert!(matches!(state, WsChannel::Open(_)));
        events.push((301, WsEvent::Tick));
        let (state, actions) = run_at(events);
        assert_eq!(state, WsChannel::Ended(ChannelEnd::UpstreamStalled));
        assert_eq!(
            actions[actions.len() - 4],
            WsAction::FinishCall(
                FIRST,
                record(
                    AttemptTerminal::AcceptanceUnknown,
                    Some(TerminalCause::HeaderTimeout),
                    AttemptPhase::AwaitingHead
                )
            )
        );
    }
}
