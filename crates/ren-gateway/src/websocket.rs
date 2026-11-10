//! The WebSocket relay: one upgraded client socket pinned to one upstream
//! socket, driven by the pure [`WsChannel`] machine.
//!
//! Two pumps run concurrently in the channel's task, one per direction. Each
//! reads whole messages from its side, classifies them, feeds the machine, and
//! runs the actions it returns. A pump forwards a message before it reads the
//! next one, so a slow reader on one side slows the writer on the other: no
//! unbounded buffering. Messages pass unchanged and in order; pings and pongs
//! are answered by each side's socket and not forwarded. Extensions are not
//! negotiated on either leg, so no compression state crosses the relay.
//!
//! Close frames the machine asks for are sent after both pumps stop, so a
//! blocked write never holds a close back, and a pump never waits on a socket
//! the other pump is writing.
//!
//! A watcher ticks the machine every second; the machine decides from the
//! clock when to ping a quiet side and when a channel has gone dead (see
//! [`Deadlines`]). Pings are Ren's own on each leg, never forwarded.

use super::{ingress::Shutdown, upstream::UpstreamSocket};
use crate::{
    channel::{CallSlot, Deadlines, Owner, Side, WsAction, WsChannel, WsEvent},
    control::{Cancel, Controls, Registered},
    policy::{Admission, CallFacts, Policy, Rejection},
};
use axum::extract::ws::{self, WebSocket};
use bytes::Bytes;
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use ren_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
    native::MessageHints,
    route::Transport,
    websocket::{ClientEvent, CloseCode, Payload, ResponseId, UpstreamMessage},
};
use ren_recorder::{
    AttemptStart, CallStart, CallTap, ChannelTap, Recorder,
    batch::Recorded,
    capture::{AttemptTerminal, ChannelEnd},
    id::ChannelId,
    routing::UpstreamId,
    time::Clock,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{self, protocol::Role},
};
use tokio_util::{sync::CancellationToken, task::task_tracker::TaskTrackerToken};

/// Whether Codex Responses WebSocket handshakes are relayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketMode {
    Relay,
    /// Answer `426 Upgrade Required`; Codex uses HTTP/SSE for the session.
    Refuse,
}

/// Declarative WebSocket settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebSocketOptions {
    pub mode: WebSocketMode,
    /// Upgraded sockets open at once. Handshakes beyond it get `426`. Calls on
    /// a socket count against the gateway's call limit, so an idle socket
    /// holds no call admission.
    pub max_channels: usize,
    /// A side of a channel silent this long is pinged; silent twice this
    /// long, the channel is closed as unresponsive.
    pub keepalive_seconds: u64,
}

impl Default for WebSocketOptions {
    fn default() -> Self {
        Self {
            mode: WebSocketMode::Relay,
            max_channels: 16,
            keepalive_seconds: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebSocketOptionsError {
    #[error("websocket.max_channels must be between 1 and 1024")]
    MaxChannels,
    #[error("websocket.keepalive_seconds must be between 1 and 3600")]
    Keepalive,
}

impl WebSocketOptions {
    pub fn validate(&self) -> Result<(), WebSocketOptionsError> {
        if !(1..=1024).contains(&self.max_channels) {
            return Err(WebSocketOptionsError::MaxChannels);
        }
        if !(1..=3600).contains(&self.keepalive_seconds) {
            return Err(WebSocketOptionsError::Keepalive);
        }
        Ok(())
    }

    pub fn keepalive(&self) -> Duration {
        Duration::from_secs(self.keepalive_seconds)
    }
}

type ClientSink = SplitSink<WebSocket, ws::Message>;
type UpstreamStream = WebSocketStream<UpstreamSocket>;
type UpstreamSink = SplitSink<UpstreamStream, tungstenite::Message>;

/// What a channel needs from the gateway, taken when its handshake succeeded.
pub(super) struct ChannelSetup {
    pub recorder: Recorder,
    pub tap: ChannelTap,
    /// The gateway's call admission, shared with HTTP.
    pub admission: Arc<Semaphore>,
    /// The fields every call on this channel shares.
    pub call: CallStart,
    pub shutdown: Shutdown,
    /// The channel limit, held for the channel's lifetime.
    pub channel_permit: OwnedSemaphorePermit,
    /// The caller's own limit, if it has one, held likewise.
    pub user_permit: Option<OwnedSemaphorePermit>,
    /// Keeps the ingress's drain waiting for this channel.
    pub tracked: TaskTrackerToken,
    /// The route's entry: a channel is pinned to it.
    pub upstream: UpstreamId,
    /// Each `response.create` is admitted by policy, like an HTTP call.
    pub policy: Policy,
    pub controls: Arc<Controls>,
    pub clock: Arc<dyn Clock>,
    pub deadlines: Deadlines,
}

/// One whole message: its framing and bytes.
#[derive(Debug, Clone)]
struct Frame {
    payload: Payload,
    bytes: Bytes,
}

/// What a socket produced.
enum Incoming {
    Frame(Frame),
    Closed(Option<CloseCode>),
    Failed,
    /// A ping or pong, answered by the socket itself.
    Control,
}

impl Incoming {
    fn from_client(next: Option<Result<ws::Message, axum::Error>>) -> Self {
        match next {
            None | Some(Err(_)) => Self::Failed,
            Some(Ok(message)) => match message {
                ws::Message::Text(text) => Self::Frame(Frame {
                    payload: Payload::Text,
                    bytes: Bytes::from(text),
                }),
                ws::Message::Binary(bytes) => Self::Frame(Frame {
                    payload: Payload::Binary,
                    bytes,
                }),
                ws::Message::Ping(_) | ws::Message::Pong(_) => Self::Control,
                ws::Message::Close(frame) => {
                    Self::Closed(frame.map(|frame| CloseCode::new(frame.code)))
                }
            },
        }
    }

    fn from_upstream(next: Option<Result<tungstenite::Message, tungstenite::Error>>) -> Self {
        match next {
            None | Some(Err(_)) => Self::Failed,
            Some(Ok(message)) => match message {
                tungstenite::Message::Text(text) => Self::Frame(Frame {
                    payload: Payload::Text,
                    bytes: Bytes::from(text),
                }),
                tungstenite::Message::Binary(bytes) => Self::Frame(Frame {
                    payload: Payload::Binary,
                    bytes,
                }),
                tungstenite::Message::Ping(_)
                | tungstenite::Message::Pong(_)
                | tungstenite::Message::Frame(_) => Self::Control,
                tungstenite::Message::Close(frame) => {
                    Self::Closed(frame.map(|frame| CloseCode::new(u16::from(frame.code))))
                }
            },
        }
    }
}

/// A frame that cannot be re-sent: text that is not UTF-8. The socket layer
/// already validated it, so this means a relay bug, handled as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unsendable;

impl Frame {
    fn to_upstream(&self) -> Result<tungstenite::Message, Unsendable> {
        match self.payload {
            Payload::Text => tungstenite::Utf8Bytes::try_from(self.bytes.clone())
                .map(tungstenite::Message::Text)
                .map_err(|_| Unsendable),
            Payload::Binary => Ok(tungstenite::Message::Binary(self.bytes.clone())),
        }
    }

    fn to_client(&self) -> Result<ws::Message, Unsendable> {
        match self.payload {
            Payload::Text => ws::Utf8Bytes::try_from(self.bytes.clone())
                .map(ws::Message::Text)
                .map_err(|_| Unsendable),
            Payload::Binary => Ok(ws::Message::Binary(self.bytes.clone())),
        }
    }
}

/// The message an event came with, if any.
struct Observed {
    side: Side,
    frame: Frame,
    hints: MessageHints,
    /// The plan of an admitted `response.create`.
    plan: Option<Planned>,
}

/// What an admitted call records about its plan.
#[derive(Clone)]
struct Planned {
    operator_revision: ren_recorder::routing::OperatorRevision,
    plan: Option<Recorded>,
}

/// A call open on the channel.
struct OpenCall {
    tap: Arc<CallTap>,
    /// Taken once the gateway admits the call.
    admission: Option<OwnedSemaphorePermit>,
    /// Keeps the call cancellable by the operator.
    _registered: Registered,
}

/// Shared by the pumps and the shutdown watcher of one channel.
struct Relay {
    machine: Mutex<WsChannel<ResponseId>>,
    calls: Mutex<HashMap<CallSlot, OpenCall>>,
    client: tokio::sync::Mutex<ClientSink>,
    upstream: tokio::sync::Mutex<UpstreamSink>,
    /// Close frames to send once the pumps stop, per side.
    closes: Mutex<Closes>,
    /// Cancelled when the channel ends.
    ended: CancellationToken,
    channel: ChannelId,
    tap: ChannelTap,
    recorder: Recorder,
    admission: Arc<Semaphore>,
    call: CallStart,
    shutdown: Shutdown,
    upstream_id: UpstreamId,
    policy: Policy,
    controls: Arc<Controls>,
    clock: Arc<dyn Clock>,
    /// Operator cancels, by call slot.
    interrupts: mpsc::UnboundedSender<CallSlot>,
}

#[derive(Debug, Default)]
struct Closes {
    client: Option<CloseCode>,
    upstream: Option<CloseCode>,
}

impl Relay {
    /// How long the close handshake may take after the channel ends.
    const CLOSE_GRACE: Duration = Duration::from_secs(2);
    /// How often the machine checks its deadlines, and the longest a ping
    /// may wait for its socket.
    const TICK: Duration = Duration::from_secs(1);

    /// Relay until the channel ends, then send the close frames it decided.
    async fn run(setup: ChannelSetup, client: WebSocket, upstream: UpstreamSocket) {
        let ChannelSetup {
            recorder,
            tap,
            admission,
            call,
            shutdown,
            channel_permit,
            user_permit,
            tracked,
            upstream: upstream_id,
            policy,
            controls,
            clock,
            deadlines,
        } = setup;
        let (interrupts, interrupted) = mpsc::unbounded_channel();
        let upstream = WebSocketStream::from_raw_socket(upstream, Role::Client, None).await;
        let (client_sink, client_stream) = client.split();
        let (upstream_sink, upstream_stream) = upstream.split();
        let relay = Arc::new(Self {
            machine: Mutex::new(WsChannel::opened(deadlines, clock.now().monotonic)),
            calls: Mutex::new(HashMap::new()),
            client: tokio::sync::Mutex::new(client_sink),
            upstream: tokio::sync::Mutex::new(upstream_sink),
            closes: Mutex::new(Closes::default()),
            ended: CancellationToken::new(),
            channel: tap.channel(),
            tap,
            recorder,
            admission,
            call,
            shutdown,
            upstream_id,
            policy,
            controls,
            clock,
            interrupts,
        });
        tokio::join!(
            relay.pump(Side::Client, PumpStream::Client(client_stream)),
            relay.pump(Side::Upstream, PumpStream::Upstream(upstream_stream)),
            relay.watch_shutdown(),
            relay.watch_operator(interrupted),
            relay.watch_deadlines(),
        );
        relay.close_sockets().await;
        drop((channel_permit, user_permit, tracked));
    }

    async fn pump(&self, side: Side, mut stream: PumpStream) {
        loop {
            let incoming = tokio::select! {
                biased;
                () = self.ended.cancelled() => return,
                incoming = stream.next() => incoming,
            };
            match incoming {
                Incoming::Control => self.step(WsEvent::Control { side }, None).await,
                Incoming::Frame(frame) => {
                    let mut plan = None;
                    let (event, hints) = match side {
                        Side::Client => {
                            let event = ClientEvent::classify(frame.payload, &frame.bytes);
                            let hints = match &event {
                                ClientEvent::Create(hints) => *hints,
                                ClientEvent::Interrupt(_) | ClientEvent::Other => {
                                    MessageHints::default()
                                }
                            };
                            if let ClientEvent::Create(_) = &event {
                                match self.admit(hints, &frame) {
                                    Ok(planned) => plan = Some(planned),
                                    // Refused: answered by Ren, not forwarded.
                                    Err(refusal) => {
                                        if !self.send_client(&refusal).await {
                                            self.step(WsEvent::Failed { side: Side::Client }, None)
                                                .await;
                                        }
                                        continue;
                                    }
                                }
                            }
                            (WsEvent::Client(event.lifecycle()), hints)
                        }
                        Side::Upstream => (
                            WsEvent::Upstream(UpstreamMessage::classify(
                                frame.payload,
                                &frame.bytes,
                            )),
                            MessageHints::default(),
                        ),
                    };
                    self.step(
                        event,
                        Some(Observed {
                            side,
                            frame,
                            hints,
                            plan,
                        }),
                    )
                    .await;
                }
                Incoming::Closed(code) => self.step(WsEvent::Closed { side, code }, None).await,
                Incoming::Failed => self.step(WsEvent::Failed { side }, None).await,
            }
        }
    }

    /// Plan a `response.create` against the operator state and policy. A
    /// refusal is recorded as a call and answered with a Ren `error` event.
    fn admit(&self, hints: MessageHints, frame: &Frame) -> Result<Planned, Frame> {
        let attribution = self.call.attribution.for_message(hints);
        let operator = self.controls.snapshot();
        let plan = self.policy.plan(
            &CallFacts {
                transport: Transport::WebSocket,
                provider: self.call.api.provider(),
                upstream: &self.upstream_id,
                endpoint: self.call.endpoint,
                model: None,
                session: attribution.hint.as_ref().map(|hint| hint.session),
                user: self.call.caller.user(),
                now: self.clock.now().wall,
            },
            &operator,
        );
        let planned = Planned {
            operator_revision: plan.operator_revision,
            plan: Recorded::of(&plan.record()).ok(),
        };
        let rejection = match &plan.admission {
            Admission::Admit => return Ok(planned),
            Admission::Reject(rejection) => rejection,
        };
        self.recorder.reject(
            Some(self.channel),
            CallStart {
                attribution,
                operator_revision: Some(planned.operator_revision),
                plan: planned.plan,
                attempts: 0,
                ..self.call.clone()
            },
            Some(&frame.bytes),
            rejection.label(),
        );
        Err(Self::refusal(rejection))
    }

    /// Send a Ren message to the client. Returns whether it was sent.
    async fn send_client(&self, frame: &Frame) -> bool {
        let Ok(message) = frame.to_client() else {
            return false;
        };
        let mut sink = self.client.lock().await;
        tokio::select! {
            biased;
            () = self.ended.cancelled() => false,
            sent = sink.send(message) => sent.is_ok(),
        }
    }

    /// Send `response.interrupt` upstream for the operator, recording it in
    /// the call's request as Ren's message. Returns whether it was sent.
    async fn interrupt(&self, slot: CallSlot) -> bool {
        let response = self
            .machine
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .response(slot)
            .map(|response| response.as_str().to_owned());
        let Some(response) = response else {
            return true;
        };
        let message = serde_json::json!({ "type": "response.interrupt", "response_id": response });
        let frame = Frame {
            payload: Payload::Text,
            bytes: Bytes::from(message.to_string()),
        };
        if let Some(open) = self
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&slot)
        {
            open.tap
                .message(Direction::Request, frame.payload, &frame.bytes);
        }
        let Ok(message) = frame.to_upstream() else {
            return false;
        };
        let mut sink = self.upstream.lock().await;
        tokio::select! {
            biased;
            () = self.ended.cancelled() => true,
            sent = sink.send(message) => sent.is_ok(),
        }
    }

    /// The `error` event a refused `response.create` is answered with.
    fn refusal(rejection: &Rejection) -> Frame {
        let (code, message) = match rejection {
            Rejection::Paused { .. } => ("ren_paused", "Ren is paused by its operator".to_owned()),
            Rejection::SessionBlocked => (
                "ren_session_blocked",
                "this session is blocked by the Ren operator".to_owned(),
            ),
            Rejection::UserBlocked => (
                "ren_user_blocked",
                "this user is blocked by the Ren operator".to_owned(),
            ),
            Rejection::Blocked { rule, reason } => (
                "ren_blocked",
                format!("blocked by Ren policy rule {}: {reason}", rule.as_str()),
            ),
            Rejection::UserNotAllowed { upstream } => (
                "ren_user_not_allowed",
                format!("upstream {upstream} is not open to this caller"),
            ),
        };
        let event = serde_json::json!({
            "type": "error",
            "error": { "type": code, "code": code, "message": message },
        });
        Frame {
            payload: Payload::Text,
            bytes: Bytes::from(event.to_string()),
        }
    }

    /// Operator cancels for this channel's calls.
    async fn watch_operator(&self, mut interrupted: mpsc::UnboundedReceiver<CallSlot>) {
        loop {
            let slot = tokio::select! {
                biased;
                () = self.ended.cancelled() => return,
                slot = interrupted.recv() => match slot {
                    Some(slot) => slot,
                    None => return,
                },
            };
            self.step(WsEvent::OperatorInterrupt(slot), None).await;
        }
    }

    /// Tick the machine, which pings quiet sides and ends a dead channel.
    async fn watch_deadlines(&self) {
        let mut ticks = tokio::time::interval(Self::TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = self.ended.cancelled() => return,
                _ = ticks.tick() => self.step(WsEvent::Tick, None).await,
            }
        }
    }

    /// Ping a quiet side. A socket busy past a tick is left alone: the
    /// deadline still passes if the side stays silent. Returns the side whose
    /// write failed, if it did.
    async fn ping(&self, side: Side) -> Option<Side> {
        let send = async {
            match side {
                Side::Client => {
                    let mut sink = self.client.lock().await;
                    sink.send(ws::Message::Ping(Bytes::new())).await.is_ok()
                }
                Side::Upstream => {
                    let mut sink = self.upstream.lock().await;
                    sink.send(tungstenite::Message::Ping(Bytes::new()))
                        .await
                        .is_ok()
                }
            }
        };
        let sent = tokio::select! {
            biased;
            () = self.ended.cancelled() => return None,
            sent = tokio::time::timeout(Self::TICK, send) => sent,
        };
        match sent {
            Ok(false) => Some(side),
            Ok(true) | Err(_) => None,
        }
    }

    async fn watch_shutdown(&self) {
        tokio::select! {
            biased;
            () = self.ended.cancelled() => return,
            () = self.shutdown.draining() => self.step(WsEvent::Drain, None).await,
        }
        let abort = self.shutdown.abort_token();
        tokio::select! {
            biased;
            () = self.ended.cancelled() => {}
            () = abort.cancelled() => self.step(WsEvent::Abort, None).await,
        }
    }

    /// Feed an event and run its actions. A failed write becomes the next
    /// event, so it is classified like any other.
    async fn step(&self, event: WsEvent<ResponseId>, observed: Option<Observed>) {
        let mut pending = VecDeque::from([(event, observed)]);
        while let Some((event, observed)) = pending.pop_front() {
            for action in self.transition(event) {
                if let Some(failed) = self.act(action, observed.as_ref()).await {
                    pending.push_back((WsEvent::Failed { side: failed }, None));
                }
            }
        }
    }

    fn transition(&self, event: WsEvent<ResponseId>) -> Vec<WsAction> {
        let at = self.clock.now().monotonic;
        let mut machine = self.machine.lock().unwrap_or_else(PoisonError::into_inner);
        let state = std::mem::replace(&mut *machine, WsChannel::Ended(ChannelEnd::Dropped));
        match state.step(at, event) {
            Ok(transition) => {
                *machine = transition.state;
                transition.actions
            }
            // A message read just before the other side ended the channel.
            Err(crate::channel::IllegalTransition::AfterEnd { end }) => {
                *machine = WsChannel::Ended(end);
                Vec::new()
            }
        }
    }

    /// Run one action. Returns the side whose write failed, if one did.
    async fn act(&self, action: WsAction, observed: Option<&Observed>) -> Option<Side> {
        match action {
            WsAction::OpenCall(slot) => {
                self.open_call(
                    slot,
                    observed.map_or_else(MessageHints::default, |o| o.hints),
                    observed.and_then(|o| o.plan.clone()),
                )
                .await;
                None
            }
            WsAction::Interrupt(slot) => (!self.interrupt(slot).await).then_some(Side::Upstream),
            WsAction::Ping(side) => self.ping(side).await,
            WsAction::Forward(owner) => {
                let observed = observed?;
                self.record(owner, observed);
                self.forward(observed).await
            }
            WsAction::Withhold(owner) => {
                if let Some(observed) = observed {
                    self.record(owner, observed);
                }
                None
            }
            WsAction::FinishCall(slot, record) => {
                let open = self
                    .calls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&slot);
                if let Some(open) = open {
                    // Client messages arrive whole, so the request body ends
                    // with the call; the response ended only at its terminal event.
                    open.tap.end(Direction::Request);
                    match record.terminal {
                        AttemptTerminal::TerminalEvent => open.tap.end(Direction::Response),
                        AttemptTerminal::BodyComplete
                        | AttemptTerminal::NotSent
                        | AttemptTerminal::Refused
                        | AttemptTerminal::AcceptanceUnknown
                        | AttemptTerminal::StreamFailed
                        | AttemptTerminal::Cancelled
                        | AttemptTerminal::Interrupted => {}
                    }
                    open.tap.finish(record);
                }
                None
            }
            WsAction::Close { side, code } => {
                let mut closes = self.closes.lock().unwrap_or_else(PoisonError::into_inner);
                match side {
                    Side::Client => closes.client = Some(code),
                    Side::Upstream => closes.upstream = Some(code),
                }
                None
            }
            WsAction::End(end) => {
                let code = {
                    let closes = self.closes.lock().unwrap_or_else(PoisonError::into_inner);
                    closes.client.or(closes.upstream)
                };
                self.tap.close(end, code.map(CloseCode::get));
                self.ended.cancel();
                None
            }
        }
    }

    /// Start recording the call, then wait for the gateway to admit it. The
    /// client's next message waits too: that is the backpressure.
    async fn open_call(&self, slot: CallSlot, hints: MessageHints, planned: Option<Planned>) {
        let attribution = self.call.attribution.for_message(hints);
        let session = attribution.hint.as_ref().map(|hint| hint.session);
        let tap = self.recorder.start_in(
            self.channel,
            CallStart {
                attribution,
                operator_revision: planned.as_ref().map(|planned| planned.operator_revision),
                plan: planned.and_then(|planned| planned.plan),
                attempts: 1,
                ..self.call.clone()
            },
            AttemptStart::unchanged(self.upstream_id.clone(), self.call.api.provider()),
        );
        let registered = self.controls.register(
            tap.call(),
            Transport::WebSocket,
            session,
            self.call.caller.user().cloned(),
            Cancel::WebSocket {
                slot,
                relay: self.interrupts.clone(),
            },
        );
        tap.response_head(
            http::StatusCode::SWITCHING_PROTOCOLS.as_u16(),
            ResponseFormat::WebSocketMessages,
            ResponseEncoding::Identity,
        );
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                slot,
                OpenCall {
                    tap,
                    admission: None,
                    _registered: registered,
                },
            );
        let permit = tokio::select! {
            biased;
            () = self.ended.cancelled() => return,
            permit = self.admission.clone().acquire_owned() => permit,
        };
        if let Ok(permit) = permit
            && let Some(open) = self
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_mut(&slot)
        {
            open.admission = Some(permit);
        }
        // The create is forwarded next: the upstream's deadline starts now.
        let none: Vec<WsAction> = self.transition(WsEvent::Admitted(slot));
        debug_assert!(none.is_empty());
    }

    fn record(&self, owner: Owner, observed: &Observed) {
        let direction = match observed.side {
            Side::Client => Direction::Request,
            Side::Upstream => Direction::Response,
        };
        match owner {
            Owner::Call(slot) => {
                let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(open) = calls.get(&slot) {
                    open.tap
                        .message(direction, observed.frame.payload, &observed.frame.bytes);
                }
            }
            Owner::Unattributed => self.tap.unattributed(&observed.frame.bytes),
        }
    }

    /// Send the message to the other side. Returns the side whose write failed.
    async fn forward(&self, observed: &Observed) -> Option<Side> {
        match observed.side {
            Side::Client => {
                let sent = match observed.frame.to_upstream() {
                    Ok(message) => {
                        let mut sink = self.upstream.lock().await;
                        tokio::select! {
                            biased;
                            () = self.ended.cancelled() => return None,
                            sent = sink.send(message) => sent.is_ok(),
                        }
                    }
                    Err(Unsendable) => false,
                };
                (!sent).then_some(Side::Upstream)
            }
            Side::Upstream => {
                let sent = match observed.frame.to_client() {
                    Ok(message) => {
                        let mut sink = self.client.lock().await;
                        tokio::select! {
                            biased;
                            () = self.ended.cancelled() => return None,
                            sent = sink.send(message) => sent.is_ok(),
                        }
                    }
                    Err(Unsendable) => false,
                };
                (!sent).then_some(Side::Client)
            }
        }
    }

    /// Send the close frames the machine decided, and flush any close reply
    /// a socket owes its peer, within a grace period.
    async fn close_sockets(&self) {
        let closes =
            std::mem::take(&mut *self.closes.lock().unwrap_or_else(PoisonError::into_inner));
        let client = async {
            let mut sink = self.client.lock().await;
            if let Some(code) = closes.client {
                let _sent = sink
                    .send(ws::Message::Close(Some(ws::CloseFrame {
                        code: code.get(),
                        reason: ws::Utf8Bytes::from_static(""),
                    })))
                    .await;
            }
            let _closed = sink.close().await;
        };
        let upstream = async {
            let mut sink = self.upstream.lock().await;
            if let Some(code) = closes.upstream {
                let _sent = sink
                    .send(tungstenite::Message::Close(Some(
                        tungstenite::protocol::CloseFrame {
                            code: code.get().into(),
                            reason: tungstenite::Utf8Bytes::from_static(""),
                        },
                    )))
                    .await;
            }
            let _closed = sink.close().await;
        };
        // A peer that stopped reading only delays its own close.
        let _graced = tokio::time::timeout(Self::CLOSE_GRACE, async {
            tokio::join!(client, upstream);
        })
        .await;
    }
}

impl Drop for Relay {
    /// A relay dropped without an end (its task was cancelled): calls still
    /// open are interrupted, and the channel is recorded as dropped.
    fn drop(&mut self) {
        let actions = self.transition(WsEvent::Dropped);
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        for action in actions {
            match action {
                WsAction::FinishCall(slot, record) => {
                    if let Some(open) = calls.remove(&slot) {
                        open.tap.finish(record);
                    }
                }
                WsAction::End(end) => self.tap.close(end, None),
                WsAction::OpenCall(_)
                | WsAction::Interrupt(_)
                | WsAction::Ping(_)
                | WsAction::Forward(_)
                | WsAction::Withhold(_)
                | WsAction::Close { .. } => {}
            }
        }
    }
}

/// The read half of either socket.
enum PumpStream {
    Client(SplitStream<WebSocket>),
    Upstream(SplitStream<UpstreamStream>),
}

impl PumpStream {
    async fn next(&mut self) -> Incoming {
        match self {
            Self::Client(stream) => Incoming::from_client(stream.next().await),
            Self::Upstream(stream) => Incoming::from_upstream(stream.next().await),
        }
    }
}

impl ChannelSetup {
    /// The relay to start once the client's upgrade completes. The callback
    /// owns the setup and the upstream socket, so a client that never finishes
    /// its upgrade drops them, which records the channel as dropped.
    pub(super) fn relay(
        self,
        upstream: UpstreamSocket,
    ) -> impl FnOnce(WebSocket) -> futures_util::future::BoxFuture<'static, ()> + Send + 'static
    {
        move |client| Box::pin(Relay::run(self, client, upstream))
    }
}

#[cfg(test)]
mod tests {
    use super::{WebSocketMode, WebSocketOptions, WebSocketOptionsError};
    use std::time::Duration;

    #[test]
    fn options_parse_and_validate() {
        let options: WebSocketOptions =
            toml::from_str("mode = \"refuse\"\nmax_channels = 2\nkeepalive_seconds = 45").unwrap();
        assert_eq!(options.mode, WebSocketMode::Refuse);
        assert_eq!(options.keepalive(), Duration::from_secs(45));
        assert!(options.validate().is_ok());
        assert!(toml::from_str::<WebSocketOptions>("enabled = true").is_err());
        let none = WebSocketOptions {
            max_channels: 0,
            ..WebSocketOptions::default()
        };
        assert_eq!(none.validate(), Err(WebSocketOptionsError::MaxChannels));
        let never = WebSocketOptions {
            keepalive_seconds: 0,
            ..WebSocketOptions::default()
        };
        assert_eq!(never.validate(), Err(WebSocketOptionsError::Keepalive));
    }
}
