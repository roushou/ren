//! WebSocket relay scenarios on the `TestGateway`: a scripted upstream hands
//! the test the server end of an in-memory socket, and a real WebSocket client
//! connects to the gateway over loopback.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]

use axum::{
    Router,
    extract::ws::{self, WebSocketUpgrade},
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderMap, StatusCode};
use ren_daemon::config::Config;
use ren_gateway::{
    Connected, Stopped, Upstream, UpstreamBody, UpstreamHandshake, UpstreamRequest, UpstreamSocket,
    attempt::SendFailure,
};
use ren_protocol::{direction::Direction, label::Stored, route::Method};
use ren_recorder::capture::{AttemptPhase, AttemptTerminal, ChannelEnd, TerminalCause};
use ren_store::view::{CallDetail, CallView};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{io::DuplexStream, net::TcpStream, sync::mpsc, time::timeout};
use tokio_tungstenite::{
    WebSocketStream, client_async,
    tungstenite::{
        self, Message,
        client::IntoClientRequest,
        protocol::{CloseFrame, Role, frame::coding::CloseCode},
    },
};

mod support;
use support::{TestGateway, config, detail, exported, mock, recorded};

type Server = WebSocketStream<DuplexStream>;
type Client = WebSocketStream<TcpStream>;

const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const TURN: &str = "33333333-3333-4333-8333-333333333333";

/// How the scripted upstream answers the next handshake.
enum Script {
    Accept,
    Refuse,
}

/// An upstream whose handshakes follow a script. Accepted sockets are
/// in-memory pipes whose server end goes to the test.
struct ScriptedWs {
    script: Mutex<VecDeque<Script>>,
    servers: mpsc::UnboundedSender<Server>,
    seen: Mutex<Vec<HeaderMap>>,
}

impl ScriptedWs {
    fn new(
        script: impl IntoIterator<Item = Script>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Server>) {
        let (servers, receiver) = mpsc::unbounded_channel();
        let upstream = Arc::new(Self {
            script: Mutex::new(script.into_iter().collect()),
            servers,
            seen: Mutex::new(Vec::new()),
        });
        (upstream, receiver)
    }
}

impl Upstream for ScriptedWs {
    async fn send(
        &self,
        _request: UpstreamRequest,
    ) -> Result<http::Response<UpstreamBody>, SendFailure> {
        panic!("these scenarios send only WebSocket handshakes")
    }

    async fn connect(&self, handshake: UpstreamHandshake) -> Result<Connected, SendFailure> {
        self.seen.lock().unwrap().push(handshake.headers);
        let script = self.script.lock().unwrap().pop_front();
        match script.expect("unscripted handshake") {
            Script::Refuse => Ok(Connected::Refused(
                http::Response::builder()
                    .status(StatusCode::UPGRADE_REQUIRED)
                    .header("x-fixture", "refused")
                    .body(UpstreamBody::full("upgrade required"))
                    .unwrap(),
            )),
            Script::Accept => {
                // A small pipe, so an upstream that stops reading pushes back fast.
                let (gateway, server) = tokio::io::duplex(16 * 1024);
                let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
                self.servers.send(server).unwrap();
                let mut headers = HeaderMap::new();
                headers.insert("x-codex-turn-state", "fixture-state".parse().unwrap());
                headers.insert("openai-model", "fixture-gpt".parse().unwrap());
                // The client leg answers its own key: this must not reach it.
                headers.insert(
                    "sec-websocket-accept",
                    "not-the-client-key".parse().unwrap(),
                );
                headers.insert(
                    "sec-websocket-extensions",
                    "permessage-deflate".parse().unwrap(),
                );
                Ok(Connected::Upgraded {
                    status: StatusCode::SWITCHING_PROTOCOLS,
                    headers,
                    socket: UpstreamSocket::new(gateway),
                })
            }
        }
    }
}

fn base_config() -> Config {
    Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    }
}

async fn connect(
    gateway: &TestGateway,
) -> Result<(Client, tungstenite::handshake::client::Response), tungstenite::Error> {
    let mut request = format!("ws://{}/codex/responses", gateway.address())
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("session-id", SESSION.parse().unwrap());
    headers.insert("authorization", "Bearer FIXTURE".parse().unwrap());
    headers.insert(
        "openai-beta",
        "responses_websockets=2026-02-06".parse().unwrap(),
    );
    headers.insert("x-ren-pi-session-id", "local".parse().unwrap());
    let stream = TcpStream::connect(gateway.address()).await.unwrap();
    client_async(request, stream).await
}

fn create() -> String {
    format!(
        r#"{{"type":"response.create","model":"fixture-gpt","client_metadata":{{"session_id":"{SESSION}","turn_id":"{TURN}"}},"input":[]}}"#
    )
}

const CREATED: &str =
    r#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}"#;
const DELTA: &str = r#"{"type":"response.output_text.delta","delta":"hi"}"#;
const COMPLETED: &str = r#"{"type":"response.completed","response":{"id":"resp_1","status":"completed","model":"fixture-gpt","usage":{"input_tokens":3,"output_tokens":1}}}"#;

async fn next_text<S>(socket: &mut WebSocketStream<S>) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Text(text))) => return text.to_string(),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            other => panic!("expected a text message, got {other:?}"),
        }
    }
}

async fn next_close<S>(socket: &mut WebSocketStream<S>) -> Option<CloseCode>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Close(frame))) => return frame.map(|frame| frame.code),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            other => panic!("expected a close frame, got {other:?}"),
        }
    }
}

async fn send_text<S>(socket: &mut WebSocketStream<S>, text: &str)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket.send(Message::text(text)).await.unwrap();
}

/// Calls in `dir`, oldest first, with their details.
async fn calls_in(dir: &Path) -> Vec<(CallView, CallDetail)> {
    let mut calls = recorded(dir).await;
    calls.reverse();
    let mut details = Vec::new();
    for call in calls {
        let detail = detail(dir, call.id).await;
        details.push((call, detail));
    }
    details
}

fn method(call: &CallView) -> Method {
    call.method.known().unwrap()
}

fn ending(detail: &CallDetail) -> (AttemptTerminal, Option<TerminalCause>, Option<AttemptPhase>) {
    let attempt = detail.attempts.last().unwrap();
    (
        attempt.terminal.as_ref().and_then(Stored::known).unwrap(),
        attempt
            .terminal_cause
            .as_ref()
            .map(|cause| cause.known().unwrap()),
        attempt
            .terminal_phase
            .as_ref()
            .map(|phase| phase.known().unwrap()),
    )
}

fn channel_end(detail: &CallDetail) -> (Option<ChannelEnd>, Option<u16>) {
    (
        detail
            .channel
            .ending
            .as_ref()
            .map(|end| end.known().unwrap()),
        detail.channel.close_code,
    )
}

#[tokio::test]
async fn a_call_is_relayed_message_for_message_and_recorded_with_its_terminal_event() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream.clone()).await;
    let (mut client, response) = connect(&gateway).await.unwrap();
    // The upstream's handshake headers reach the client; its accept key and
    // extensions do not (the client library checked the accept key).
    assert_eq!(response.headers()["x-codex-turn-state"], "fixture-state");
    assert_eq!(response.headers()["openai-model"], "fixture-gpt");
    assert!(response.headers().get("sec-websocket-extensions").is_none());
    let mut server = servers.recv().await.unwrap();

    // Credentials and native headers pass; Ren's own hints and the client
    // leg's socket negotiation do not.
    {
        let seen = upstream.seen.lock().unwrap();
        let headers = &seen[0];
        assert_eq!(headers["authorization"], "Bearer FIXTURE");
        assert_eq!(headers["openai-beta"], "responses_websockets=2026-02-06");
        for name in [
            "x-ren-pi-session-id",
            "sec-websocket-key",
            "sec-websocket-version",
            "sec-websocket-extensions",
            "upgrade",
            "connection",
            "host",
        ] {
            assert!(headers.get(name).is_none(), "{name} was forwarded");
        }
    }

    send_text(&mut client, &create()).await;
    assert_eq!(next_text(&mut server).await, create());
    for event in [CREATED, DELTA, COMPLETED] {
        send_text(&mut server, event).await;
        assert_eq!(next_text(&mut client).await, event);
    }
    // Pings are answered by the gateway's leg, not forwarded.
    client.send(Message::Ping("fixture".into())).await.unwrap();
    client
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        })))
        .await
        .unwrap();
    assert_eq!(next_close(&mut server).await, Some(CloseCode::Normal));
    drop((client, server));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    let [(handshake, handshake_detail), (call, call_detail)] = &calls[..] else {
        panic!("expected the handshake and one call, got {}", calls.len());
    };
    assert_eq!(method(handshake), Method::Get);
    assert_eq!(handshake.status, Some(101));
    assert_eq!(ending(handshake_detail).0, AttemptTerminal::BodyComplete);
    assert_eq!(method(call), Method::Message);
    assert_eq!(handshake.channel, call.channel);
    assert_eq!(
        ending(call_detail),
        (
            AttemptTerminal::TerminalEvent,
            None,
            Some(AttemptPhase::Response)
        )
    );
    assert_eq!(call.model.as_deref(), Some("fixture-gpt"));
    assert_eq!(
        call.outcome
            .as_ref()
            .and_then(Stored::known)
            .map(|o| o.to_string()),
        Some("completed".to_owned())
    );
    // The handshake's session, with the turn its message named.
    assert!(call.turn.is_some());
    assert!(call.session.is_some());

    // Bodies are the messages back to back, framed by the message index.
    assert_eq!(
        exported(dir.path(), call.id, Direction::Request)
            .await
            .unwrap(),
        create().into_bytes()
    );
    let response = exported(dir.path(), call.id, Direction::Response)
        .await
        .unwrap();
    assert_eq!(response, [CREATED, DELTA, COMPLETED].concat().into_bytes());
    let marks: Vec<(Direction, u64, u64)> = call_detail
        .messages
        .iter()
        .map(|mark| (mark.direction.known().unwrap(), mark.offset, mark.len))
        .collect();
    let len = |text: &str| u64::try_from(text.len()).unwrap();
    assert_eq!(
        marks,
        vec![
            (Direction::Request, 0, len(&create())),
            (Direction::Response, 0, len(CREATED)),
            (Direction::Response, len(CREATED), len(DELTA)),
            (
                Direction::Response,
                len(CREATED) + len(DELTA),
                len(COMPLETED)
            ),
        ]
    );
    assert_eq!(
        channel_end(call_detail),
        (Some(ChannelEnd::ClientClosed), Some(1000))
    );
}

#[tokio::test]
async fn an_accepted_interrupt_is_the_clients_and_the_terminal_event_still_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();

    send_text(&mut client, &create()).await;
    next_text(&mut server).await;
    for event in [CREATED, DELTA] {
        send_text(&mut server, event).await;
        next_text(&mut client).await;
    }
    let interrupt =
        r#"{"type":"response.interrupt","response_id":"resp_1","mode":"discard_partial_items"}"#;
    send_text(&mut client, interrupt).await;
    assert_eq!(next_text(&mut server).await, interrupt);
    let accepted = r#"{"type":"response.interrupt.accepted","response_id":"resp_1"}"#;
    let incomplete = r#"{"type":"response.incomplete","response":{"id":"resp_1","status":"incomplete","incomplete_details":{"reason":"steered"}}}"#;
    for event in [accepted, incomplete] {
        send_text(&mut server, event).await;
        assert_eq!(next_text(&mut client).await, event);
    }
    // The channel stays usable for the next call.
    send_text(&mut client, &create()).await;
    next_text(&mut server).await;
    send_text(&mut server, COMPLETED).await;
    next_text(&mut client).await;
    client.close(None).await.unwrap();
    drop((client, server));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    let [_, (_, interrupted), (_, next)] = &calls[..] else {
        panic!("expected the handshake and two calls, got {}", calls.len());
    };
    assert_eq!(
        ending(interrupted),
        (
            AttemptTerminal::TerminalEvent,
            Some(TerminalCause::Client),
            Some(AttemptPhase::Response)
        )
    );
    // The interrupt is part of the interrupted call's request.
    assert_eq!(
        exported(dir.path(), interrupted.call.id, Direction::Request)
            .await
            .unwrap(),
        [create().as_str(), interrupt].concat().into_bytes()
    );
    assert_eq!(ending(next).1, None);
}

#[tokio::test]
async fn a_refused_handshake_reaches_the_client_and_is_recorded_so_codex_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, _servers) = ScriptedWs::new([Script::Refuse]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let Err(tungstenite::Error::Http(response)) = connect(&gateway).await else {
        panic!("the handshake should be refused");
    };
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    assert_eq!(response.headers()["x-fixture"], "refused");
    assert_eq!(response.body().as_deref(), Some(&b"upgrade required"[..]));
    gateway.stop().await;
    let calls = calls_in(dir.path()).await;
    let [(call, detail)] = &calls[..] else {
        panic!("expected one refused handshake");
    };
    assert_eq!((method(call), call.status), (Method::Get, Some(426)));
    assert_eq!(ending(detail).0, AttemptTerminal::BodyComplete);
    // No channel was upgraded, so none ended.
    assert_eq!(channel_end(detail), (None, None));
}

#[tokio::test]
async fn an_upstream_close_ends_the_open_call_and_reaches_the_client() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();
    send_text(&mut client, &create()).await;
    next_text(&mut server).await;
    send_text(&mut server, CREATED).await;
    next_text(&mut client).await;
    server
        .send(Message::Close(Some(CloseFrame {
            code: CloseCode::from(4001),
            reason: "".into(),
        })))
        .await
        .unwrap();
    assert_eq!(next_close(&mut client).await, Some(CloseCode::from(4001)));
    drop((client, server));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    let (_, call) = calls.last().unwrap();
    assert_eq!(
        ending(call),
        (
            AttemptTerminal::StreamFailed,
            Some(TerminalCause::UpstreamClosed),
            Some(AttemptPhase::Response)
        )
    );
    assert_eq!(
        channel_end(call),
        (Some(ChannelEnd::UpstreamClosed), Some(4001))
    );
}

#[tokio::test]
async fn a_stalled_upstream_pushes_back_on_the_client_instead_of_buffering() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();
    const MESSAGES: usize = 64;
    let payload = "x".repeat(256 * 1024);
    let sent = Arc::new(AtomicUsize::new(0));
    let progress = sent.clone();
    let (mut sink, _stream) = client.split();
    let sender = tokio::spawn(async move {
        for index in 0..MESSAGES {
            sink.send(Message::text(format!("{index:04}{payload}")))
                .await
                .unwrap();
            progress.fetch_add(1, Ordering::SeqCst);
        }
        sink
    });
    // 16 MiB cannot all fit between a client and an upstream that is not
    // reading, unless the relay buffers without bound.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stalled = sent.load(Ordering::SeqCst);
    assert!(
        stalled < MESSAGES,
        "the client sent everything into a stalled upstream"
    );
    // Once the upstream reads, every message arrives whole and in order.
    for index in 0..MESSAGES {
        let text = next_text(&mut server).await;
        assert_eq!(&text[..4], format!("{index:04}"));
        assert_eq!(text.len(), 4 + 256 * 1024);
    }
    let mut sink = timeout(Duration::from_secs(5), sender)
        .await
        .unwrap()
        .unwrap();
    sink.close().await.unwrap();
    drop(server);
    gateway.stop().await;
    // No call was open: the bytes were counted, not stored.
    let calls = calls_in(dir.path()).await;
    let [(_, handshake)] = &calls[..] else {
        panic!("expected only the handshake");
    };
    assert_eq!(
        handshake.channel.unattributed_messages,
        u64::try_from(MESSAGES).unwrap()
    );
}

#[tokio::test]
async fn shutdown_closes_idle_channels_and_aborts_busy_ones_with_service_restart() {
    // Idle: the drain closes it at once and the server stops drained.
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();
    let stopping = tokio::spawn(gateway.stop_expecting(Stopped::Drained));
    assert_eq!(next_close(&mut client).await, Some(CloseCode::Restart));
    assert_eq!(next_close(&mut server).await, Some(CloseCode::Away));
    drop((client, server));
    timeout(Duration::from_secs(5), stopping)
        .await
        .unwrap()
        .unwrap();
    let calls = calls_in(dir.path()).await;
    assert_eq!(
        channel_end(&calls[0].1),
        (Some(ChannelEnd::Drained), Some(1012))
    );

    // Busy past the drain deadline: the call is cancelled by shutdown.
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();
    send_text(&mut client, &create()).await;
    next_text(&mut server).await;
    let stopping = tokio::spawn(gateway.stop_expecting(Stopped::Aborted));
    assert_eq!(next_close(&mut client).await, Some(CloseCode::Restart));
    drop((client, server));
    timeout(Duration::from_secs(10), stopping)
        .await
        .unwrap()
        .unwrap();
    let calls = calls_in(dir.path()).await;
    let (_, call) = calls.last().unwrap();
    assert_eq!(
        ending(call),
        (
            AttemptTerminal::Cancelled,
            Some(TerminalCause::Shutdown),
            Some(AttemptPhase::AwaitingHead)
        )
    );
    assert_eq!(channel_end(call), (Some(ChannelEnd::Aborted), Some(1012)));
}

/// Read `socket` until it closes, as a live peer does (reading answers pings).
async fn read_until_closed<S>(mut socket: WebSocketStream<S>) -> Option<CloseCode>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match timeout(Duration::from_secs(10), socket.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Close(frame))) => return frame.map(|frame| frame.code),
            Some(Ok(_)) => {}
            other => panic!("expected messages until a close, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_client_that_stops_answering_pings_is_closed_and_frees_its_channel() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        websocket: ren_gateway::WebSocketOptions {
            max_channels: 1,
            ..Default::default()
        },
        ..base_config()
    };
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept, Script::Accept]);
    let gateway = TestGateway::with_upstream(&config, dir.path(), upstream).await;
    // A client that never reads again, so it never answers a ping.
    let (silent, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();

    // Quiet for the keepalive: both legs are pinged; the upstream answers.
    gateway.clock().advance(Duration::from_secs(30));
    match timeout(Duration::from_secs(5), server.next())
        .await
        .unwrap()
    {
        Some(Ok(Message::Ping(_))) => {}
        other => panic!("expected a ping, got {other:?}"),
    }
    let upstream_side = tokio::spawn(read_until_closed(server));
    // Silent for twice the keepalive: the client is unresponsive.
    gateway.clock().advance(Duration::from_secs(30));
    let closed = timeout(Duration::from_secs(5), upstream_side)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed, Some(CloseCode::Away));

    // Its channel is free again: the limit admits a new one.
    let (client, _) = connect(&gateway).await.unwrap();
    let server = servers.recv().await.unwrap();
    drop((silent, client, server));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    assert_eq!(
        channel_end(&calls[0].1),
        (Some(ChannelEnd::ClientUnresponsive), Some(1001))
    );
}

#[tokio::test]
async fn a_call_whose_upstream_goes_quiet_ends_as_stalled() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        stream_idle_timeout_seconds: 5,
        websocket: ren_gateway::WebSocketOptions {
            keepalive_seconds: 3600,
            ..Default::default()
        },
        ..base_config()
    };
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&config, dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();
    send_text(&mut client, &create()).await;
    assert_eq!(next_text(&mut server).await, create());
    send_text(&mut server, CREATED).await;
    assert_eq!(next_text(&mut client).await, CREATED);

    gateway.clock().advance(Duration::from_secs(5));
    let (client_closed, server_closed) =
        tokio::join!(read_until_closed(client), read_until_closed(server));
    // The client is closed with 1014, which the client library reads as a
    // protocol error; either way its socket is closed. The code is recorded.
    assert!(client_closed.is_some());
    assert_eq!(server_closed, Some(CloseCode::Away));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    let (_, call) = calls.last().unwrap();
    assert_eq!(
        ending(call),
        (
            AttemptTerminal::StreamFailed,
            Some(TerminalCause::IdleTimeout),
            Some(AttemptPhase::Response)
        )
    );
    assert_eq!(
        channel_end(call),
        (Some(ChannelEnd::UpstreamStalled), Some(1014))
    );
}

#[tokio::test]
async fn the_reqwest_upstream_upgrades_over_http1_and_relays() {
    let upstream = mock(Router::new().route(
        "/backend-api/codex/responses",
        get(|upgrade: WebSocketUpgrade| async move {
            let response: Response = upgrade.on_upgrade(|mut socket| async move {
                while let Some(Ok(message)) = socket.recv().await {
                    if let ws::Message::Text(_) = message {
                        let _ = socket.send(ws::Message::text(COMPLETED)).await;
                    }
                }
            });
            response
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    send_text(&mut client, &create()).await;
    assert_eq!(next_text(&mut client).await, COMPLETED);
    client.close(None).await.unwrap();
    drop(client);
    gateway.stop().await;
    let calls = calls_in(dir.path()).await;
    let (_, call) = calls.last().unwrap();
    assert_eq!(ending(call).0, AttemptTerminal::TerminalEvent);
}

/// A control API request with the gateway's token.
async fn control(gateway: &TestGateway, path: &str, body: &str) -> serde_json::Value {
    let mut request = support::client()
        .post(format!("{}/control/v1/{path}", gateway.base()))
        .bearer_auth(TestGateway::CONTROL_TOKEN);
    if !body.is_empty() {
        request = request
            .header("content-type", "application/json")
            .body(body.to_owned());
    }
    let response = request.send().await.unwrap();
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

#[tokio::test]
async fn a_paused_gateway_answers_a_create_with_a_ren_error_and_forwards_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();

    control(&gateway, "pause", r#"{"retry_after_seconds":5}"#).await;
    send_text(&mut client, &create()).await;
    let refusal: serde_json::Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
    assert_eq!(
        (refusal["type"].as_str(), refusal["error"]["type"].as_str()),
        (Some("error"), Some("ren_paused"))
    );
    control(&gateway, "resume", "").await;
    // Only the create sent after the resume reaches the upstream.
    send_text(&mut client, &create()).await;
    assert_eq!(next_text(&mut server).await, create());
    send_text(&mut server, COMPLETED).await;
    next_text(&mut client).await;
    client.close(None).await.unwrap();
    drop((client, server));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    let [_, (refused, refused_detail), (forwarded, _)] = &calls[..] else {
        panic!("expected the handshake and two calls, got {}", calls.len());
    };
    assert_eq!(
        refused.rejection.as_ref().and_then(Stored::known),
        Some(ren_recorder::capture::CallRejection::Paused)
    );
    assert!(refused_detail.attempts.is_empty());
    assert_eq!(refused.channel, forwarded.channel);
    assert_eq!(forwarded.operator_revision, Some(2));
}

#[tokio::test]
async fn an_operator_cancel_interrupts_upstream_and_keeps_the_answer_from_the_client() {
    let dir = tempfile::tempdir().unwrap();
    let (upstream, mut servers) = ScriptedWs::new([Script::Accept]);
    let gateway = TestGateway::with_upstream(&base_config(), dir.path(), upstream).await;
    let (mut client, _) = connect(&gateway).await.unwrap();
    let mut server = servers.recv().await.unwrap();

    send_text(&mut client, &create()).await;
    next_text(&mut server).await;
    for event in [CREATED, DELTA] {
        send_text(&mut server, event).await;
        next_text(&mut client).await;
    }
    let state = support::client()
        .get(format!("{}/control/v1/state", gateway.base()))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .send()
        .await
        .unwrap();
    let state: serde_json::Value = serde_json::from_slice(&state.bytes().await.unwrap()).unwrap();
    let call = state["running"][0]["call"].as_str().unwrap().to_owned();
    control(&gateway, &format!("calls/{call}/cancel"), "").await;
    let injected = r#"{"type":"response.interrupt","response_id":"resp_1"}"#;
    assert_eq!(next_text(&mut server).await, injected);
    let accepted = r#"{"type":"response.interrupt.accepted","response_id":"resp_1"}"#;
    let incomplete =
        r#"{"type":"response.incomplete","response":{"id":"resp_1","status":"incomplete"}}"#;
    for event in [accepted, incomplete] {
        send_text(&mut server, event).await;
    }
    // The client never asked: it sees the terminal event, not the answer.
    assert_eq!(next_text(&mut client).await, incomplete);
    client.close(None).await.unwrap();
    drop((client, server));
    gateway.stop().await;

    let calls = calls_in(dir.path()).await;
    let [_, (cancelled, detail)] = &calls[..] else {
        panic!("expected the handshake and one call, got {}", calls.len());
    };
    assert_eq!(cancelled.id.to_string(), call);
    assert_eq!(
        ending(detail),
        (
            AttemptTerminal::TerminalEvent,
            Some(TerminalCause::Operator),
            Some(AttemptPhase::Response)
        )
    );
    // Ren's interrupt is recorded in the call's request, after the create.
    assert_eq!(
        exported(dir.path(), cancelled.id, Direction::Request)
            .await
            .unwrap(),
        [create().as_str(), injected].concat().into_bytes()
    );
}
