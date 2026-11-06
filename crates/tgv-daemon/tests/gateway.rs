#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    response::Response,
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::StatusCode;
use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tgv_daemon::config::Config;
use tgv_gateway::{
    Connected, Stopped, Upstream, UpstreamBody, UpstreamHandshake, UpstreamRequest, WebSocketMode,
    attempt::SendFailure,
};
use tgv_protocol::{
    direction::Direction,
    label::Stored,
    route::{Endpoint, Provider},
};
use tgv_recorder::{
    capture::{AttemptPhase, AttemptTerminal, BodyEnding, TerminalCause},
    routing::UpstreamId,
};
use tgv_store::{Reader, query::PageLimit};
use tokio::{
    net::TcpListener,
    sync::{Notify, mpsc, oneshot},
    time::timeout,
};

mod support;

use support::{TestGateway, client, config, detail, ending, exported, mock, recorded, terminal};

#[tokio::test]
async fn native_axum_routes_forward_only_registered_methods_and_paths() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let upstream = mock(Router::new().fallback(move |request: Request| {
        count.fetch_add(1, Ordering::SeqCst);
        async move { format!("{} {}", request.method(), request.uri()) }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let base = format!("http://{}", gateway.address());
    let client = client();

    for (method, path, target) in [
        (http::Method::POST, "/anthropic/v1/messages", "/v1/messages"),
        (
            http::Method::POST,
            "/anthropic/v1/messages/count_tokens",
            "/v1/messages/count_tokens",
        ),
        (http::Method::GET, "/anthropic/v1/models", "/v1/models"),
        (
            http::Method::POST,
            "/codex/responses",
            "/backend-api/codex/responses",
        ),
        (
            http::Method::POST,
            "/codex/responses/compact",
            "/backend-api/codex/responses/compact",
        ),
        (
            http::Method::GET,
            "/codex/models",
            "/backend-api/codex/models",
        ),
        (http::Method::POST, "/openai/v1/responses", "/v1/responses"),
        (
            http::Method::POST,
            "/openai/v1/responses/compact",
            "/v1/responses/compact",
        ),
        (http::Method::GET, "/openai/v1/models", "/v1/models"),
    ] {
        let response = client
            .request(method.clone(), format!("{base}{path}?value=a%2Fb"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.unwrap(),
            format!("{method} {target}?value=a%2Fb")
        );

        for unsupported in [
            http::Method::GET,
            http::Method::POST,
            http::Method::HEAD,
            http::Method::DELETE,
            http::Method::OPTIONS,
        ] {
            if unsupported == method {
                continue;
            }
            let response = client
                .request(unsupported, format!("{base}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
            assert_eq!(response.headers()["allow"], method.as_str());
        }
        for suffix in ["/", "/arbitrary"] {
            let response = client
                .request(method.clone(), format!("{base}{path}{suffix}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }
    for path in [
        "/openai/v1/chat/completions",
        "/unknown",
        "/codex/responses%2Fcompact",
    ] {
        let response = client
            .post(format!("{base}{path}"))
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 9);
    gateway.stop().await;
    assert_eq!(recorded(dir.path()).await.len(), 9);
}

#[tokio::test]
async fn resolved_destinations_use_distinct_transports_and_preserve_recorded_labels() {
    let anthropic = mock(Router::new().fallback(|| async { "anthropic fixture" })).await;
    let codex = mock(Router::new().fallback(|| async { "codex fixture" })).await;
    let openai = mock(Router::new().fallback(|| async { "openai fixture" })).await;
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        anthropic_base_url: format!("http://{}", anthropic.address),
        codex_base_url: format!("http://{}", codex.address),
        openai_base_url: format!("http://{}", openai.address),
        ..Config::default()
    };
    let revision = config.revision().unwrap();
    let gateway = TestGateway::start(&config, dir.path()).await;
    for (path, expected) in [
        ("/anthropic/v1/models", "anthropic fixture"),
        ("/codex/models", "codex fixture"),
        ("/openai/v1/models", "openai fixture"),
    ] {
        let response = client()
            .get(format!("http://{}{path}", gateway.address()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), expected);
    }
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 3);
    for provider in [Provider::Anthropic, Provider::Codex, Provider::OpenAi] {
        let record = records
            .iter()
            .find(|r| r.provider == Stored::Known(provider))
            .unwrap();
        assert_eq!(record.endpoint, Stored::Known(Endpoint::Models));
        assert_eq!(record.config_revision, revision);
        assert!(record.capture_complete);
    }
}

#[tokio::test]
async fn native_streams_headers_and_query_survive_and_history_reopens() {
    for (path, expected_path) in [
        ("/anthropic/v1/messages", "/v1/messages"),
        ("/codex/responses", "/backend-api/codex/responses"),
        ("/openai/v1/responses", "/v1/responses"),
    ] {
        let (received, mut requests) = mpsc::channel(1);
        let release = Arc::new(Notify::new());
        let gate = release.clone();
        let upstream = mock(Router::new().fallback(move |request: Request| {
            let received = received.clone();
            let gate = gate.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = to_bytes(body, 1024 * 1024).await.unwrap();
                received.send((parts, body)).await.unwrap();
                let first = futures_util::stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                    b"event: unknown\ndata: ",
                ))]);
                let tail = futures_util::stream::once(async move {
                    gate.notified().await;
                    Ok::<_, Infallible>(Bytes::from_static(b"{\"text\":\"\xf0\x9f"))
                })
                .chain(futures_util::stream::iter([
                    Ok(Bytes::from_static(b"\x9a\x84\"}\n")),
                    Ok(Bytes::from_static(b"\ndata: [DONE]\n\n")),
                ]));
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .header("request-id", "req_fixture")
                    .header("anthropic-ratelimit-requests-remaining", "7")
                    .body(Body::from_stream(first.chain(tail)))
                    .unwrap()
            }
        }))
        .await;
        let dir = tempfile::tempdir().unwrap();
        let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
        let original =
            b"{ \"model\":\"fixture\", \"unknown_future_field\": [1,2], \"stream\":true }";
        let response = timeout(
            Duration::from_secs(3),
            client()
                .post(format!(
                    "http://{}{path}?beta=true&access_token=QUERY_SECRET",
                    gateway.address()
                ))
                .header("authorization", "Bearer AUTH_SECRET")
                .header("x-api-key", "KEY_SECRET")
                .header("x-custom-secret", "CUSTOM_SECRET")
                .header("tailscale-user-login", "fixture@example.test")
                .header("tailscale-user-name", "Fixture User")
                .header("tailscale-user-profile-pic", "https://example.test/profile")
                .header("tailscale-app-capabilities", "{}")
                .header("tailscale-headers-info", "fixture")
                .header("tailscale-future-identity", "fixture")
                .header("x-forwarded-port", "8443")
                .header("forwarded", "for=100.64.0.1")
                .header("x-forwarded-for", "100.64.0.1")
                .header("x-forwarded-host", "gateway.test")
                .header("x-forwarded-proto", "https")
                .header("anthropic-beta", "oauth-2025-04-20,future-beta")
                .header("chatgpt-account-id", "fixture-account")
                .body(original.to_vec())
                .send(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["request-id"], "req_fixture");
        assert_eq!(
            response.headers()["anthropic-ratelimit-requests-remaining"],
            "7"
        );
        let mut stream = response.bytes_stream();
        let first = timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("gateway buffered SSE")
            .unwrap()
            .unwrap();
        assert_eq!(&first[..], b"event: unknown\ndata: ");
        let (parts, body) = requests.recv().await.unwrap();
        assert_eq!(parts.uri.path(), expected_path);
        assert_eq!(
            parts.uri.query(),
            Some("beta=true&access_token=QUERY_SECRET")
        );
        assert_eq!(&body[..], original);
        assert_eq!(parts.headers["authorization"], "Bearer AUTH_SECRET");
        for name in [
            "tailscale-user-login",
            "tailscale-user-name",
            "tailscale-user-profile-pic",
            "tailscale-app-capabilities",
            "tailscale-headers-info",
            "tailscale-future-identity",
            "x-forwarded-port",
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
        ] {
            assert!(!parts.headers.contains_key(name));
        }
        assert_eq!(parts.headers["x-custom-secret"], "CUSTOM_SECRET");
        assert_eq!(
            parts.headers["anthropic-beta"],
            "oauth-2025-04-20,future-beta"
        );
        assert_eq!(parts.headers["chatgpt-account-id"], "fixture-account");
        let mut output = first.to_vec();
        release.notify_one();
        while let Some(chunk) = timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
        {
            output.extend_from_slice(&chunk.unwrap());
        }
        drop(stream);
        gateway.stop().await;
        let records = recorded(dir.path()).await;
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert!(record.capture_complete);
        assert_eq!(terminal(record), AttemptTerminal::BodyComplete);
        assert_eq!(
            exported(dir.path(), record.id, Direction::Request)
                .await
                .unwrap(),
            original
        );
        assert_eq!(
            exported(dir.path(), record.id, Direction::Response)
                .await
                .unwrap(),
            output
        );
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            for secret in ["AUTH_SECRET", "KEY_SECRET", "CUSTOM_SECRET", "QUERY_SECRET"] {
                assert!(!text.contains(secret), "persisted credential: {secret}");
            }
        }
    }
}

#[tokio::test]
async fn upstream_errors_and_redirects_are_forwarded_without_retries() {
    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::TEMPORARY_REDIRECT,
    ] {
        let hits = Arc::new(AtomicUsize::new(0));
        let calls = hits.clone();
        let upstream = mock(Router::new().fallback(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                Response::builder()
                    .status(status)
                    .header("retry-after", "1")
                    .header("location", "/should-not-be-followed")
                    .body(Body::from("provider response"))
                    .unwrap()
            }
        }))
        .await;
        let dir = tempfile::tempdir().unwrap();
        let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
        let response = client()
            .post(format!(
                "http://{}/anthropic/v1/messages",
                gateway.address()
            ))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["retry-after"], "1");
        assert_eq!(response.bytes().await.unwrap(), "provider response");
        gateway.stop().await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let records = recorded(dir.path()).await;
        assert!(records[0].capture_complete);
        assert_eq!(records[0].status, Some(status.as_u16()));
    }
}

struct PendingBody {
    first: Option<Bytes>,
    dropped: Option<oneshot::Sender<()>>,
}
impl Stream for PendingBody {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.first.take() {
            Some(bytes) => Poll::Ready(Some(Ok(bytes))),
            None => Poll::Pending,
        }
    }
}
impl Drop for PendingBody {
    fn drop(&mut self) {
        if let Some(sender) = self.dropped.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn downstream_disconnect_cancels_upstream_and_records_interruption() {
    let (dropped, observed) = oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(dropped)));
    let upstream = mock(Router::new().fallback(move || {
        let dropped = sender.lock().unwrap().take();
        async move {
            Body::from_stream(PendingBody {
                first: Some(Bytes::from_static(b"data: first\n\n")),
                dropped,
            })
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let response = client()
        .post(format!("http://{}/codex/responses", gateway.address()))
        .body("{}")
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    timeout(Duration::from_secs(3), observed)
        .await
        .expect("upstream body was not dropped")
        .unwrap();
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(terminal(&records[0]), AttemptTerminal::Interrupted);
    assert!(!records[0].capture_complete);
}

#[tokio::test]
async fn admission_is_held_until_the_stream_ends_and_shutdown_aborts_it_at_the_deadline() {
    let upstream = mock(Router::new().fallback(|| async {
        Body::from_stream(PendingBody {
            first: Some(Bytes::from_static(b"data: first\n\n")),
            dropped: None,
        })
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&upstream);
    config.max_concurrent_requests = 1;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let url = format!("http://{}/codex/responses", gateway.address());
    let response = client().post(&url).body("{}").send().await.unwrap();
    let mut stream = response.bytes_stream();
    assert!(stream.next().await.unwrap().is_ok());
    let rejected = client().post(&url).body("{}").send().await.unwrap();
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    gateway.shutdown();
    // Draining leaves the stream open until the deadline, then aborts it.
    assert!(
        timeout(TestGateway::DRAIN_DEADLINE / 2, stream.next())
            .await
            .is_err()
    );
    let next = timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap();
    assert!(next.is_none_or(|value| value.is_err()));
    drop(stream);
    gateway.stop_expecting(Stopped::Aborted).await;
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 1);
    assert_eq!(terminal(&records[0]), AttemptTerminal::Cancelled);
    let attempt = &detail(dir.path(), records[0].id).await.attempts[0];
    assert_eq!(
        (
            attempt.terminal_cause.clone(),
            attempt.terminal_phase.clone()
        ),
        (
            Some(Stored::Known(TerminalCause::Shutdown)),
            Some(Stored::Known(AttemptPhase::Response))
        )
    );
}

#[tokio::test]
async fn truncated_upstream_body_is_not_reported_complete() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
            .await
            .unwrap();
        socket.shutdown().await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        codex_base_url: format!("http://{address}"),
        ..Config::default()
    };
    let gateway = TestGateway::start(&config, dir.path()).await;
    let result = client()
        .get(format!("http://{}/codex/models", gateway.address()))
        .send()
        .await;
    if let Ok(response) = result {
        assert!(response.bytes().await.is_err());
    }
    upstream.await.unwrap();
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(
        ending(dir.path(), records[0].id).await,
        (
            AttemptTerminal::StreamFailed,
            Some(TerminalCause::BodyError),
            Some(AttemptPhase::Response)
        )
    );
    assert_eq!(
        records[0].response.as_ref().unwrap().ending,
        Stored::Known(BodyEnding::Unfinished)
    );
    assert!(!records[0].capture_complete);
}

#[tokio::test]
async fn empty_responses_and_model_catalogs_finish_without_a_body_poll() {
    let upstream = mock(Router::new().fallback(|| async { StatusCode::NO_CONTENT })).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let response = client()
        .get(format!("http://{}/codex/models", gateway.address()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(response.bytes().await.unwrap().is_empty());
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert!(records[0].capture_complete);
    assert_eq!(terminal(&records[0]), AttemptTerminal::BodyComplete);
}

#[tokio::test]
async fn recording_limit_does_not_truncate_forwarding() {
    let upstream = mock(Router::new().fallback(|request: Request| async {
        let body = to_bytes(request.into_body(), 1024).await.unwrap();
        Response::new(Body::from(body))
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&upstream);
    config.capture.body_limit_bytes = 3;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let response = client()
        .post(format!("http://{}/codex/responses", gateway.address()))
        .body("abcdef")
        .send()
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap(), "abcdef");
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    let record = &records[0];
    assert!(!record.capture_complete);
    for body in [&record.request, &record.response] {
        let body = body.as_ref().unwrap();
        assert_eq!((body.observed_bytes, body.stored_bytes), (Some(6), 3));
    }
    assert!(
        exported(dir.path(), record.id, Direction::Response)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn response_header_deadline_is_bounded_and_does_not_leak_request_urls() {
    let upstream =
        mock(Router::new().fallback(|| async { std::future::pending::<StatusCode>().await })).await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&upstream);
    config.response_header_timeout_seconds = 1;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let response = timeout(
        Duration::from_secs(3),
        client()
            .post(format!(
                "http://{}/codex/responses?token=SECRET",
                gateway.address()
            ))
            .body("{}")
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = response.text().await.unwrap();
    assert!(!body.contains("SECRET"));
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    // The request was sent in full; the upstream may have accepted it.
    assert_eq!(terminal(&records[0]), AttemptTerminal::AcceptanceUnknown);
    let attempt = &detail(dir.path(), records[0].id).await.attempts[0];
    assert_eq!(
        (
            attempt.terminal_cause.clone(),
            attempt.terminal_phase.clone()
        ),
        (
            Some(Stored::Known(TerminalCause::HeaderTimeout)),
            Some(Stored::Known(AttemptPhase::AwaitingHead))
        )
    );
    assert!(!records[0].capture_complete);
}

#[tokio::test]
async fn model_completion_survives_downstream_close_before_http_eof() {
    use tgv_protocol::inspect::{Coverage, FormatSource, Limitation, ModelOutcome};
    const EVENT: &[u8] = b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"model\":\"fixture\",\"usage\":{\"input_tokens\":11,\"output_tokens\":7},\"output\":[{\"type\":\"function_call\",\"call_id\":\"call_fixture\",\"name\":\"read_file\",\"arguments\":\"PRIVATE_ARGUMENTS\"}]}}\n\n";
    let (dropped, observed) = oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(dropped)));
    let upstream = mock(Router::new().fallback(move || {
        let dropped = sender.lock().unwrap().take();
        async move {
            Response::builder()
                .body(Body::from_stream(PendingBody {
                    first: Some(Bytes::from_static(EVENT)),
                    dropped,
                }))
                .unwrap()
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let mut stream = client()
        .post(format!("http://{}/codex/responses", gateway.address()))
        .body("{}")
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let mut received = Vec::new();
    while received.len() < EVENT.len() {
        received.extend_from_slice(
            &timeout(Duration::from_secs(3), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(received, EVENT);
    drop(stream);
    timeout(Duration::from_secs(3), observed)
        .await
        .unwrap()
        .unwrap();
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(terminal(&records[0]), AttemptTerminal::Interrupted);
    assert!(!records[0].capture_complete);
    let detail = detail(dir.path(), records[0].id).await;
    let inspection = detail.attempts[0].inspection.as_ref().unwrap();
    assert_eq!(inspection.format, Stored::Known(FormatSource::Inferred));
    assert_eq!(
        inspection.outcome,
        Some(Stored::Known(ModelOutcome::Completed))
    );
    assert_eq!(inspection.coverage, Stored::Known(Coverage::Partial));
    assert_eq!(
        inspection.limitations,
        [Stored::Known(Limitation::BodyUnfinished)]
    );
    assert_eq!(inspection.usage.as_ref().unwrap().output_tokens, Some(7));
    assert_eq!(inspection.tool_calls.len(), 1);
    assert_eq!(inspection.tool_calls[0].name.as_deref(), Some("read_file"));
    assert!(
        !serde_json::to_string(inspection)
            .unwrap()
            .contains("PRIVATE_ARGUMENTS")
    );
}

#[tokio::test]
async fn tool_follow_up_is_grouped_and_visible_through_read_only_api() {
    const SESSION: &str = "11111111-1111-4111-8111-111111111111";
    const TURN: &str = "22222222-2222-4222-8222-222222222222";
    const FIRST: &str = r#"{"model":"fixture","messages":[{"role":"user","content":"read fixture"}],"stream":true}"#;
    const SECOND: &str = r#"{"model":"fixture","messages":[{"role":"assistant","content":[{"type":"tool_use","id":"tool_fixture","name":"read","input":{"path":"fixture"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool_fixture","content":"PRIVATE_RESULT"}]}],"stream":true}"#;
    let hits = Arc::new(AtomicUsize::new(0));
    let calls = hits.clone();
    let upstream = mock(Router::new().fallback(move |request: Request| {
        let index = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            assert_eq!(request.headers()["x-claude-code-session-id"], SESSION);
            assert_eq!(request.headers()["x-claude-code-prompt-id"], TURN);
            assert_eq!(request.headers()["authorization"], "Bearer PRIVATE_CREDENTIAL");
            let body = to_bytes(request.into_body(), 4096).await.unwrap();
            assert_eq!(body.as_ref(), if index == 0 { FIRST.as_bytes() } else { SECOND.as_bytes() });
            let content = if index == 0 {
                "data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool_fixture\",\"name\":\"read\"}}\n\n"
            } else { "" };
            Response::builder().header("content-type", "text/event-stream")
                .body(Body::from(format!("{content}data: {{\"type\":\"message_stop\"}}\n\n"))).unwrap()
        }
    })).await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&upstream);
    config.capture.body_limit_bytes = 0;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let base = format!("http://{}", gateway.address());
    for body in [FIRST, SECOND] {
        let response = client()
            .post(format!("{base}/anthropic/v1/messages"))
            .header("x-claude-code-session-id", SESSION)
            .header("x-claude-code-prompt-id", TURN)
            .header("authorization", "Bearer PRIVATE_CREDENTIAL")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .text()
                .await
                .unwrap()
                .ends_with("data: {\"type\":\"message_stop\"}\n\n")
        );
    }
    let sessions: serde_json::Value = timeout(Duration::from_secs(3), async {
        loop {
            let response = client()
                .get(format!("{base}/api/sessions"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.headers()["cache-control"], "no-store");
            let value: serde_json::Value =
                serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            assert_eq!(value["api_version"], 1);
            if value["sessions"][0]["call_count"] == 2 {
                break value["sessions"].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["turn_count"], 1);
    let id = sessions[0]["id"].as_str().unwrap();
    let url = format!("{base}/api/sessions/{id}/calls?limit=1");
    let first: serde_json::Value = serde_json::from_slice(
        &client()
            .get(&url)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    let second: serde_json::Value = serde_json::from_slice(
        &client()
            .get(format!("{url}&after={}", first["next"].as_str().unwrap()))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["calls"][0]["turn"], second["calls"][0]["turn"]);
    assert!(first["calls"][0]["turn"].is_string());
    assert!(second["next"].is_null());
    assert_ne!(first["calls"][0]["id"], second["calls"][0]["id"]);
    for path in [
        "/api/calls?limit=0",
        "/api/sessions?limit=1001",
        "/api/sessions?extra=1",
    ] {
        assert_eq!(
            client()
                .get(format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        client()
            .post(format!("{base}/api/sessions"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        client()
            .get(format!("{base}/api/calls/{SESSION}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let json = client()
        .get(format!("{base}/api/calls"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!json.contains("PRIVATE_CREDENTIAL") && !json.contains("PRIVATE_RESULT"));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    gateway.stop().await;
}

#[tokio::test]
async fn pi_attribution_is_local_and_scoped_by_provider_and_client_source() {
    const SESSION: &str = "11111111-1111-4111-8111-111111111111";
    const TURN: &str = "22222222-2222-4222-8222-222222222222";
    let upstream = mock(Router::new().fallback(|request: Request| async move {
        assert!(!request.headers().contains_key("x-tgv-pi-session-id"));
        assert!(!request.headers().contains_key("x-tgv-pi-turn-id"));
        assert!(!request.headers().contains_key("x-tgv-pi-future-hint"));
        assert_eq!(request.headers()["authorization"], "Bearer FIXTURE");
        assert_eq!(request.headers()["originator"], "pi-fixture");
        assert_eq!(
            to_bytes(request.into_body(), 1024).await.unwrap(),
            "native bytes"
        );
        "response bytes"
    }))
    .await;
    let data = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), data.path()).await;
    for (route, pi) in [
        ("anthropic/v1/messages", true),
        ("codex/responses", true),
        ("openai/v1/responses", true),
        ("anthropic/v1/messages", false),
    ] {
        let session_header = if pi {
            "x-tgv-pi-session-id"
        } else {
            "x-claude-code-session-id"
        };
        let turn_header = if pi {
            "x-tgv-pi-turn-id"
        } else {
            "x-claude-code-prompt-id"
        };
        let response = client()
            .post(format!("http://{}/{route}", gateway.address()))
            .header(session_header, SESSION)
            .header(turn_header, TURN)
            .header("x-tgv-pi-future-hint", "local")
            .header("authorization", "Bearer FIXTURE")
            .header("originator", "pi-fixture")
            .body("native bytes")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "response bytes");
    }
    gateway.stop().await;
    let reader = Reader::open(data.path(), 1).await.unwrap();
    let sessions = reader.sessions(PageLimit::new(10).unwrap()).await.unwrap();
    assert_eq!(sessions.len(), 4);
    assert_eq!(
        sessions
            .iter()
            .filter(|s| s.source.label() == "pi_extension_headers")
            .count(),
        3
    );
    assert!(
        sessions
            .iter()
            .all(|s| s.turn_count == 1 && s.call_count == 1)
    );
}

#[tokio::test]
async fn body_previews_are_read_only_private_and_distinguish_capture_limits() {
    for limit in [0, 5, 1024] {
        let upstream = mock(Router::new().fallback(|| async {
            Response::builder()
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"content":[{"type":"text","text":"<script>fixture</script>"}]}"#,
                ))
                .unwrap()
        }))
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut conf = config(&upstream);
        conf.capture.body_limit_bytes = limit;
        let gateway = TestGateway::start(&conf, dir.path()).await;
        let response = client()
            .post(format!(
                "http://{}/anthropic/v1/messages",
                gateway.address()
            ))
            .header("authorization", "Bearer NEVER_IN_BODY")
            .body(r#"{"messages":[{"role":"user","content":"hello"}]}"#)
            .send()
            .await
            .unwrap();
        response.bytes().await.unwrap();
        gateway.stop().await;
        let id = recorded(dir.path()).await[0].id;
        let gateway = TestGateway::start(&conf, dir.path()).await;
        let base = format!("http://{}", gateway.address());
        for direction in ["request", "response"] {
            let url = format!("{base}/api/calls/{id}/body/{direction}");
            let response = client().get(&url).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert!(
                !response
                    .headers()
                    .contains_key("access-control-allow-origin")
            );
            let raw = response.text().await.unwrap();
            assert!(!raw.contains("NEVER_IN_BODY"));
            let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                value["extent"],
                if limit == 1024 { "complete" } else { "gap" }
            );
            assert_eq!(value["ending"], "ended");
            if limit == 1024 && direction == "request" {
                let request = r#"{"messages":[{"role":"user","content":"hello"}]}"#;
                assert_eq!(
                    value,
                    serde_json::json!({
                        "api_version": 1,
                        "observed_bytes": request.len(), "stored_bytes": request.len(),
                        "ending": "ended", "captured_bytes": request.len(),
                        "extent": "complete",
                        "raw": { "text": request, "format": "utf8", "truncated": false },
                        "text": request, "text_end": "intact",
                        "decoding": { "applied": "none", "integrity": "not_decoded", "truncated": false },
                    })
                );
            }
            assert_eq!(
                value["captured_bytes"],
                limit.min(if direction == "request" { 48 } else { 63 })
            );
            assert_eq!(
                client().post(&url).send().await.unwrap().status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
            // A full export only of a body stored whole, byte for byte.
            let export = client().get(format!("{url}/export")).send().await.unwrap();
            assert_eq!(export.headers()["cache-control"], "no-store");
            if limit == 1024 {
                assert_eq!(export.status(), StatusCode::OK);
                assert_eq!(export.headers()["content-type"], "application/octet-stream");
                let expected: &[u8] = if direction == "request" {
                    br#"{"messages":[{"role":"user","content":"hello"}]}"#
                } else {
                    br#"{"content":[{"type":"text","text":"<script>fixture</script>"}]}"#
                };
                assert_eq!(export.bytes().await.unwrap(), expected);
            } else {
                assert_eq!(export.status(), StatusCode::NOT_FOUND);
                let value: serde_json::Value =
                    serde_json::from_slice(&export.bytes().await.unwrap()).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({ "api_version": 1, "error": "not_found" })
                );
            }
        }
        assert_eq!(
            client()
                .get(format!("{base}/api/calls/{id}/body/invalid"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            client()
                .get(format!(
                    "{base}/api/calls/33333333-3333-4333-8333-333333333333/body/request"
                ))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        gateway.stop().await;
    }
}

#[tokio::test]
async fn live_previews_advance_without_claiming_completion_and_traffic_filters_validate() {
    let gate = Arc::new(Notify::new());
    let release = gate.clone();
    let upstream = mock(Router::new().fallback(move || {
        let gate = gate.clone();
        async move {
            let first = futures_util::stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"hello\"}\n\n"))]);
            let tail = futures_util::stream::once(async move {
                gate.notified().await;
                Ok::<_, Infallible>(Bytes::from_static(b"data: {\"type\":\"response.completed\",\"response\":{\"model\":\"live-fixture\",\"status\":\"completed\"}}\n\n"))
            });
            Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(first.chain(tail))).unwrap()
        }
    })).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let base = format!("http://{}", gateway.address());
    let client = client();
    async fn wait_json(
        client: &reqwest::Client,
        url: &str,
        ready: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        timeout(Duration::from_secs(3), async {
            loop {
                let response = client.get(url).send().await.unwrap();
                if response.status().is_success() {
                    let value: serde_json::Value =
                        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
                    if ready(&value) {
                        break value;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    let mut stream = client
        .post(format!("{base}/openai/v1/responses"))
        .body("{}")
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let first = stream.next().await.unwrap().unwrap();
    let page = wait_json(
        &client,
        &format!("{base}/api/calls?provider=openai&status=active"),
        |v| v["calls"].as_array().is_some_and(|a| a.len() == 1),
    )
    .await;
    let id = page["calls"][0]["id"].as_str().unwrap();
    let url = format!("{base}/api/calls/{id}/body/response");
    let before = wait_json(&client, &url, |v| v["captured_bytes"] == first.len()).await;
    assert_eq!(before["ending"], "open");
    assert_eq!(before["extent"], "partial");
    assert!(before["observed_bytes"].is_null());
    assert_eq!(before["raw"]["text"], std::str::from_utf8(&first).unwrap());
    release.notify_one();
    let mut total = first.len();
    while let Some(chunk) = stream.next().await {
        total += chunk.unwrap().len();
    }
    let after = wait_json(&client, &url, |v| v["extent"] == "complete").await;
    assert_eq!(after["ending"], "ended");
    assert_eq!(after["captured_bytes"], total);
    assert_eq!(after["observed_bytes"], total);
    let page = wait_json(
        &client,
        &format!("{base}/api/calls?model=live-fixture&status=finished"),
        |v| v["calls"].as_array().is_some_and(|a| a.len() == 1),
    )
    .await;
    assert_eq!(page["calls"][0]["id"], id);
    for query in [
        "provider=invalid",
        "status=invalid",
        "limit=0",
        "from_ms=2&until_ms=1",
        "after=bad",
        "typo=1",
    ] {
        assert_eq!(
            client
                .get(format!("{base}/api/calls?{query}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let response = client
        .get(format!("{base}/api/calls?status=errors"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    let errors: serde_json::Value =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert!(errors["calls"].as_array().unwrap().is_empty());
    assert_eq!(
        client
            .post(format!("{base}/api/calls"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    gateway.stop().await;
}

#[tokio::test]
async fn refused_websocket_handshakes_ask_codex_to_fall_back_to_http() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let upstream = mock(Router::new().fallback(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        async { "unexpected" }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    // The public OpenAI route always refuses; Codex refuses in this mode.
    let mut config = config(&upstream);
    config.websocket.mode = WebSocketMode::Refuse;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let base = format!("http://{}", gateway.address());
    let client = client();
    for path in ["/codex/responses", "/openai/v1/responses"] {
        for method in [reqwest::Method::GET, reqwest::Method::POST] {
            let response = client
                .request(method, format!("{base}{path}"))
                .header("connection", "upgrade")
                .header("upgrade", "websocket")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
        }
        let plain = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(plain.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(plain.headers()["allow"], "POST");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    gateway.stop().await;
    assert!(recorded(dir.path()).await.is_empty());
}

#[tokio::test]
async fn the_live_feed_streams_committed_observations_and_ends_on_shutdown() {
    let upstream = mock(Router::new().fallback(|| async {
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from("data: {\"type\":\"message_stop\"}\n\n"))
            .unwrap()
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let base = format!("http://{}", gateway.address());
    let live = client()
        .get(format!("{base}/api/live"))
        .send()
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    assert_eq!(live.headers()["content-type"], "text/event-stream");
    let mut events = live.bytes_stream();
    let response = client()
        .post(format!("{base}/anthropic/v1/messages"))
        .body("{}")
        .send()
        .await
        .unwrap();
    response.bytes().await.unwrap();
    let mut text = String::new();
    timeout(Duration::from_secs(3), async {
        while !text.contains("call_finished") {
            let chunk = events.next().await.unwrap().unwrap();
            text.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    })
    .await
    .unwrap();
    let kinds: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str::<serde_json::Value>(data).unwrap()["kind"].clone())
        .collect();
    assert_eq!(
        kinds,
        [
            "call_opened",
            "attempt_started",
            "response_head",
            "attempt_finished",
            "call_finished"
        ]
    );
    let first_seq: u64 = text
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .unwrap()
        .parse()
        .unwrap();

    // Catching up from a position replays committed observations after it.
    let mut replay = client()
        .get(format!("{base}/api/live?after={first_seq}"))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let mut replayed = String::new();
    timeout(Duration::from_secs(3), async {
        while !replayed.contains("call_finished") {
            replayed.push_str(std::str::from_utf8(&replay.next().await.unwrap().unwrap()).unwrap());
        }
    })
    .await
    .unwrap();
    assert!(!replayed.contains("call_opened"));
    assert!(replayed.contains("attempt_started"));

    // Open feeds do not hold up shutdown.
    gateway.stop().await;
    assert!(
        timeout(Duration::from_secs(3), async {
            while let Some(Ok(_)) = events.next().await {}
        })
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn a_client_that_leaves_mid_upload_is_recorded_without_reaching_the_upstream() {
    use tokio::io::AsyncWriteExt;
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = reached.clone();
    let upstream = mock(Router::new().fallback(move || {
        seen.store(true, std::sync::atomic::Ordering::SeqCst);
        async { "unreachable" }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let mut socket = tokio::net::TcpStream::connect(gateway.address())
        .await
        .unwrap();
    socket
        .write_all(
            b"POST /anthropic/v1/messages HTTP/1.1\r\nhost: tgv\r\ncontent-length: 100\r\n\r\n{\"partial\":",
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(socket);
    tokio::time::sleep(Duration::from_millis(200)).await;
    gateway.stop().await;
    // Requests are buffered whole before dispatch: nothing went upstream,
    // and the call is recorded as refused, with no attempt.
    assert!(!reached.load(std::sync::atomic::Ordering::SeqCst));
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].rejection.as_ref().and_then(|r| r.known()),
        Some(tgv_recorder::capture::CallRejection::UploadFailed)
    );
    assert!(detail(dir.path(), records[0].id).await.attempts.is_empty());
}

#[tokio::test]
async fn a_handler_dropped_mid_upload_still_records_the_call() {
    use tower::ServiceExt;
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = reached.clone();
    let upstream = mock(Router::new().fallback(move || {
        seen.store(true, std::sync::atomic::Ordering::SeqCst);
        async { "unreachable" }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    // Part of the body arrives, then nothing: the read neither ends nor fails.
    let body = futures_util::stream::once(async {
        Ok::<_, Infallible>(Bytes::from_static(b"{\"partial\":"))
    })
    .chain(futures_util::stream::pending());
    let request = Request::post("/anthropic/v1/messages")
        .header("content-length", "100")
        .body(Body::from_stream(body))
        .unwrap();
    // The connection closes under the handler, which is dropped mid-read.
    let dropped = tokio::time::timeout(
        Duration::from_millis(200),
        gateway.router().oneshot(request),
    )
    .await;
    assert!(dropped.is_err(), "the handler must still be reading");
    gateway.stop().await;
    assert!(!reached.load(std::sync::atomic::Ordering::SeqCst));
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 1, "a dropped upload must leave a record");
    assert_eq!(
        records[0].rejection.as_ref().and_then(|r| r.known()),
        Some(tgv_recorder::capture::CallRejection::Abandoned)
    );
    assert!(detail(dir.path(), records[0].id).await.attempts.is_empty());
}

#[tokio::test]
async fn a_shutdown_during_upload_is_recorded_as_a_shutdown() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let upstream = mock(Router::new().fallback(|| async { "unreachable" })).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let mut socket = tokio::net::TcpStream::connect(gateway.address())
        .await
        .unwrap();
    socket
        .write_all(
            b"POST /anthropic/v1/messages HTTP/1.1\r\nhost: tgv\r\ncontent-length: 100\r\n\r\n{\"partial\":",
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The upload stays open past the drain deadline, so shutdown aborts it.
    let reply = tokio::spawn(async move {
        let mut reply = Vec::new();
        let _ = socket.read_to_end(&mut reply).await;
        String::from_utf8_lossy(&reply).into_owned()
    });
    gateway.stop_expecting(Stopped::Aborted).await;
    let reply = timeout(Duration::from_secs(5), reply)
        .await
        .unwrap()
        .unwrap();
    assert!(reply.starts_with("HTTP/1.1 503"), "{reply}");
    assert!(reply.contains("shutting_down"), "{reply}");
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].rejection.as_ref().and_then(|r| r.known()),
        Some(tgv_recorder::capture::CallRejection::Shutdown)
    );
    assert!(detail(dir.path(), records[0].id).await.attempts.is_empty());
}

#[tokio::test]
async fn only_a_refused_connection_is_recorded_as_not_sent() {
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        codex_base_url: format!("http://{address}"),
        ..Config::default()
    };
    let gateway = TestGateway::start(&config, dir.path()).await;
    let response = client()
        .get(format!("{}/codex/models", gateway.base()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.text().await.unwrap(),
        "upstream connection failed\n"
    );
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(
        ending(dir.path(), records[0].id).await,
        (
            AttemptTerminal::NotSent,
            Some(TerminalCause::ConnectFailed),
            Some(AttemptPhase::AwaitingHead)
        )
    );
}

#[tokio::test]
async fn a_failed_send_on_a_reused_connection_is_acceptance_unknown() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // One connection: answer the first request and keep it alive, then read the
    // second request in full and close without a response.
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        for answer in [true, false] {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            if answer {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                    .await
                    .unwrap();
            }
        }
        drop(socket);
        listener
    });
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        codex_base_url: format!("http://{address}"),
        ..Config::default()
    };
    config.response_header_timeout_seconds = 3;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let url = format!("{}/codex/models", gateway.base());
    let first = client().get(&url).send().await.unwrap();
    assert_eq!(first.text().await.unwrap(), "ok");
    let second = client().get(&url).send().await.unwrap();
    assert_eq!(second.status(), StatusCode::BAD_GATEWAY);
    let _listener = upstream.await.unwrap();
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    // Newest first: the second request reached the upstream before it failed.
    assert_eq!(
        ending(dir.path(), records[0].id).await,
        (
            AttemptTerminal::AcceptanceUnknown,
            Some(TerminalCause::SendFailed),
            Some(AttemptPhase::AwaitingHead)
        )
    );
    assert_eq!(terminal(&records[1]), AttemptTerminal::BodyComplete);
}

#[tokio::test]
async fn a_quiet_stream_times_out_as_idle_not_as_a_broken_stream() {
    let upstream = mock(Router::new().fallback(|| async {
        Body::from_stream(PendingBody {
            first: Some(Bytes::from_static(b"data: first\n\n")),
            dropped: None,
        })
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&upstream);
    config.stream_idle_timeout_seconds = 1;
    let gateway = TestGateway::start(&config, dir.path()).await;
    let response = client()
        .post(format!("{}/codex/responses", gateway.base()))
        .body("{}")
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    assert!(stream.next().await.unwrap().is_ok());
    let next = timeout(Duration::from_secs(3), stream.next())
        .await
        .unwrap();
    assert!(next.is_none_or(|value| value.is_err()));
    drop(stream);
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(
        ending(dir.path(), records[0].id).await,
        (
            AttemptTerminal::StreamFailed,
            Some(TerminalCause::IdleTimeout),
            Some(AttemptPhase::Response)
        )
    );
}

/// What reached a scripted upstream.
struct Received {
    upstream: UpstreamId,
    path: &'static str,
    query: Option<String>,
    headers: http::HeaderMap,
}

/// An upstream without sockets: it records what reached it and answers from a
/// script.
struct ScriptedUpstream {
    seen: std::sync::Mutex<Vec<Received>>,
    answer: fn() -> Result<http::Response<UpstreamBody>, SendFailure>,
}

impl Upstream for ScriptedUpstream {
    async fn send(
        &self,
        request: UpstreamRequest,
    ) -> Result<http::Response<UpstreamBody>, SendFailure> {
        self.seen.lock().unwrap().push(Received {
            upstream: request.upstream,
            path: request.path,
            query: request.query,
            headers: request.headers,
        });
        (self.answer)()
    }

    async fn connect(&self, _handshake: UpstreamHandshake) -> Result<Connected, SendFailure> {
        panic!("these scenarios send no WebSocket handshake")
    }
}

#[tokio::test]
async fn the_upstream_seam_receives_policy_filtered_requests_and_typed_failures() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    };
    let upstream = Arc::new(ScriptedUpstream {
        seen: std::sync::Mutex::new(Vec::new()),
        answer: || {
            Ok(http::Response::builder()
                .header("connection", "x-hop")
                .header("x-hop", "1")
                .header("keep-alive", "timeout=5")
                .header("retry-after", "7")
                .body(UpstreamBody::full("scripted"))
                .unwrap())
        },
    });
    let gateway = TestGateway::with_upstream(&config, dir.path(), upstream.clone()).await;
    let response = client()
        .post(format!("{}/codex/responses?beta=true", gateway.base()))
        .header("authorization", "Bearer FIXTURE")
        .header("x-tgv-pi-session-id", "local")
        .header("tailscale-user-login", "someone@example.com")
        .header("x-forwarded-for", "100.64.0.1")
        .header("connection", "x-private")
        .header("x-private", "hop")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["retry-after"], "7");
    for removed in ["x-hop", "keep-alive"] {
        assert!(!response.headers().contains_key(removed), "{removed}");
    }
    assert_eq!(response.text().await.unwrap(), "scripted");
    {
        let seen = upstream.seen.lock().unwrap();
        let Received {
            upstream,
            path,
            query,
            headers,
        } = &seen[0];
        assert_eq!(
            (upstream.as_str(), *path, query.as_deref()),
            ("codex", "/responses", Some("beta=true"))
        );
        assert_eq!(headers["authorization"], "Bearer FIXTURE");
        for removed in [
            "x-tgv-pi-session-id",
            "tailscale-user-login",
            "x-forwarded-for",
            "x-private",
            "connection",
            "host",
        ] {
            assert!(
                !headers.contains_key(removed),
                "{removed} reached the upstream"
            );
        }
    }
    gateway.stop().await;

    let failing = Arc::new(ScriptedUpstream {
        seen: std::sync::Mutex::new(Vec::new()),
        answer: || Err(SendFailure::NotSent),
    });
    let gateway = TestGateway::with_upstream(&config, dir.path(), failing).await;
    let response = client()
        .get(format!("{}/openai/v1/models", gateway.base()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    gateway.stop().await;
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 2);
    assert_eq!(terminal(&records[0]), AttemptTerminal::NotSent);
    assert_eq!(terminal(&records[1]), AttemptTerminal::BodyComplete);
}

#[tokio::test]
async fn a_declared_length_delivered_whole_is_a_complete_body_even_before_the_stream_ends() {
    // An HTTP/2 upstream can send a body's last DATA frame and its
    // END_STREAM separately. The server stops polling once it has written
    // the declared length to an HTTP/1.1 client, so the end may never be
    // observed.
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    };
    let upstream = Arc::new(ScriptedUpstream {
        seen: std::sync::Mutex::new(Vec::new()),
        answer: || {
            let body = futures_util::stream::iter([Ok::<_, tgv_gateway::attempt::StreamFailure>(
                http_body::Frame::data(Bytes::from_static(b"{\"input_tokens\":18208}")),
            )])
            .chain(futures_util::stream::pending());
            Ok(http::Response::builder()
                .header("content-type", "application/json")
                .header("content-length", "22")
                .body(UpstreamBody::new(http_body_util::StreamBody::new(body)))
                .unwrap())
        },
    });
    let gateway = TestGateway::with_upstream(&config, dir.path(), upstream).await;
    let response = client()
        .post(format!(
            "{}/anthropic/v1/messages/count_tokens",
            gateway.base()
        ))
        .body("{}")
        .send()
        .await
        .unwrap();
    let body = timeout(Duration::from_secs(5), response.bytes())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body, "{\"input_tokens\":18208}");
    gateway.stop().await;
    let id = recorded(dir.path()).await[0].id;
    assert_eq!(
        ending(dir.path(), id).await,
        (
            AttemptTerminal::BodyComplete,
            Some(TerminalCause::DeclaredLength),
            Some(AttemptPhase::Response)
        )
    );
    let call = &recorded(dir.path()).await[0];
    assert!(call.capture_complete);
}
