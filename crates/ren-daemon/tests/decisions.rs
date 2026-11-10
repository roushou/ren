//! Policy decisions end to end: buffering limits, policy blocks, transforms,
//! failover to an operator-key upstream, and operator controls, each recorded.
//! Assembled with `TestGateway` against a loopback mock; credentials are
//! fixtures.
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
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use http::StatusCode;
use ren_daemon::config::{Config, UpstreamConfig};
use ren_gateway::EntryIngress;
use ren_protocol::{
    direction::Direction,
    label::Stored,
    route::{Provider, Wire},
};
use ren_recorder::{
    capture::{AttemptPhase, AttemptTerminal, CallRejection, TerminalCause},
    routing::UpstreamId,
};
use ren_store::{Reader, query::PageLimit};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Notify, time::timeout};

mod support;

use support::{Mock, TestGateway, client, detail, ending, exported, mock, recorded};

const SESSION: &str = "11111111-2222-4333-8444-555555555555";

/// Anthropic's default entry under `/primary`, an operator-key entry under
/// `/fallback`, and `policy` as TOML.
fn config(mock: &Mock, policy: &str) -> Config {
    Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        anthropic_base_url: format!("http://{}/primary", mock.address),
        codex_base_url: format!("http://{}/backend-api/codex", mock.address),
        openai_base_url: format!("http://{}", mock.address),
        upstreams: vec![UpstreamConfig {
            id: UpstreamId::parse("anthropic-key").unwrap(),
            provider: Provider::Anthropic,
            wire: None,
            base_url: format!("http://{}/fallback", mock.address),
            operator_key_env: TestGateway::OPERATOR_KEY_ENV.into(),
            ingress: EntryIngress::FallbackOnly,
            users: None,
        }],
        policy: toml::from_str(policy).unwrap(),
        ..Config::default()
    }
}

const POLICY: &str = r#"
[[block]]
id = "no-opus"
match = { provider = "anthropic", model = "fixture-opus" }
reason = "not on this gateway"

[[transform]]
id = "drop-temperature"
match = { endpoint = "messages" }
steps = [{ op = "json_remove", path = "/temperature" }]
reason = "fixture transform"

[[fallback]]
id = "plan-limit"
provider = "anthropic"
to = "anthropic-key"
on_status = [429]
"#;

/// `/primary` answers with `primary`'s status; `/fallback` echoes the body
/// and reports the credentials it saw.
fn upstream(primary: StatusCode, reached: Arc<AtomicUsize>) -> Router {
    Router::new().fallback(move |request: Request| {
        let reached = reached.clone();
        async move {
            reached.fetch_add(1, Ordering::SeqCst);
            let path = request.uri().path().to_owned();
            let headers = request.headers().clone();
            let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
            if path.starts_with("/primary") {
                return (
                    primary,
                    r#"{"type":"error","error":{"type":"rate_limit_error"}}"#,
                )
                    .into_response();
            }
            let seen = |name: &str| {
                headers
                    .get(name)
                    .map_or("-".to_owned(), |value| value.to_str().unwrap().to_owned())
            };
            Response::builder()
                .header("x-seen-key", seen("x-api-key"))
                .header("x-seen-auth", seen("authorization"))
                .header("x-seen-beta", seen("anthropic-beta"))
                .body(Body::from(body))
                .unwrap()
        }
    })
}

async fn post(gateway: &TestGateway, body: &'static str) -> reqwest::Response {
    client()
        .post(format!("{}/anthropic/v1/messages", gateway.base()))
        .header("authorization", "Bearer CLIENT-OAUTH")
        .header("anthropic-beta", "oauth-2025-04-20,fixture-beta")
        .header("x-claude-code-session-id", SESSION)
        .body(body)
        .send()
        .await
        .unwrap()
}

fn rejection(call: &ren_store::view::CallView) -> Option<CallRejection> {
    call.rejection.as_ref().and_then(Stored::known)
}

async fn operator_actions(dir: &Path) -> usize {
    let reader = Reader::open(dir, 1).await.unwrap();
    let actions = reader.operator_actions(PageLimit::DEFAULT).await.unwrap();
    reader.close().await;
    actions.len()
}

#[tokio::test]
async fn a_plan_limit_falls_over_to_the_operator_key_with_every_decision_recorded() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(StatusCode::TOO_MANY_REQUESTS, reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&mock, POLICY), dir.path()).await;
    let original = r#"{"model":"fixture-sonnet","temperature":1,"messages":[]}"#;
    let response = post(&gateway, original).await;
    assert_eq!(response.status(), StatusCode::OK);
    // The operator key replaced the client's credentials and the OAuth beta
    // only; other betas pass.
    assert_eq!(response.headers()["x-seen-key"], TestGateway::OPERATOR_KEY);
    assert_eq!(response.headers()["x-seen-auth"], "-");
    assert_eq!(response.headers()["x-seen-beta"], "fixture-beta");
    // The transform removed one key; the rest keeps its order.
    let changed = r#"{"model":"fixture-sonnet","messages":[]}"#;
    assert_eq!(response.text().await.unwrap(), changed);
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 2);

    let calls = recorded(dir.path()).await;
    let call = &calls[0];
    assert_eq!(call.status, Some(200));
    assert_eq!(call.operator_revision, Some(0));
    let plan = call.plan.as_ref().unwrap();
    assert_eq!(plan["fallback"], "plan-limit");
    assert_eq!(plan["candidates"][1], "anthropic-key");
    assert_eq!(plan["transforms"][0], "drop-temperature");
    let detail = detail(dir.path(), call.id).await;
    let [first, second] = detail.attempts.as_slice() else {
        panic!("two attempts expected, got {}", detail.attempts.len());
    };
    assert_eq!(
        (first.upstream.as_str(), first.status),
        ("anthropic", Some(429))
    );
    assert_eq!(
        (second.upstream.as_str(), second.status),
        ("anthropic-key", Some(200))
    );
    // The refused attempt's response was read and recorded before failover.
    assert!(first.response.as_ref().unwrap().complete());
    let changes = second.changes.as_ref().unwrap();
    assert_eq!(changes["auth"], "operator_key");
    assert_eq!(changes["body"], "changed");
    assert_eq!(changes["steps"][0]["outcome"], "applied");
    assert_eq!(changes["steps"][0]["reason"], "fixture transform");
    // Original and resulting bodies are both kept; no credential is.
    let changed_len = u64::try_from(changed.len()).unwrap();
    for attempt in [first, second] {
        let request = attempt.request.as_ref().unwrap();
        assert_eq!(request.stored_bytes, changed_len);
    }
    assert_eq!(
        exported(dir.path(), call.id, Direction::Request)
            .await
            .unwrap(),
        original.as_bytes()
    );
    let everything = serde_json::to_string(
        &detail
            .attempts
            .iter()
            .map(|a| &a.changes)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!everything.contains("FIXTURE") && !everything.contains("CLIENT-OAUTH"));
}

#[tokio::test]
async fn a_direct_entry_has_routes_of_its_own_and_a_fallback_only_entry_has_none() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(StatusCode::TOO_MANY_REQUESTS, reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let send = |gateway: &TestGateway, id: &str| {
        client()
            .post(format!("{}/upstreams/{id}/v1/messages", gateway.base()))
            .header("authorization", "Bearer CLIENT-OAUTH")
            .header("anthropic-beta", "oauth-2025-04-20,fixture-beta")
            .header("x-claude-code-session-id", SESSION)
            .body(r#"{"model":"fixture-sonnet"}"#)
            .send()
    };

    // Fallback only, as when `ingress` is omitted: no route.
    let gateway = TestGateway::start(&config(&mock, POLICY), dir.path()).await;
    let response = send(&gateway, "anthropic-key").await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 0);

    let mut direct = config(&mock, POLICY);
    direct.upstreams[0].ingress = EntryIngress::Direct;
    let gateway = TestGateway::start(&direct, dir.path()).await;
    let response = send(&gateway, "anthropic-key").await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-seen-key"], TestGateway::OPERATOR_KEY);
    assert_eq!(response.headers()["x-seen-auth"], "-");
    assert_eq!(response.headers()["x-seen-beta"], "fixture-beta");
    gateway.stop().await;
    // Sent straight to the entry's base URL, never to the native upstream.
    assert_eq!(reached.load(Ordering::SeqCst), 1);

    let calls = recorded(dir.path()).await;
    let call = &calls[0];
    assert_eq!(call.provider, Stored::Known(Provider::Anthropic));
    assert_eq!(call.wire, Some(Stored::Known(Wire::AnthropicMessages)));
    assert!(call.session.is_some(), "Claude Code's headers are read");
    let plan = call.plan.as_ref().unwrap();
    assert_eq!(plan["candidates"], serde_json::json!(["anthropic-key"]));
    assert_eq!(plan["fallback"], serde_json::Value::Null);
    let detail = detail(dir.path(), call.id).await;
    let [attempt] = detail.attempts.as_slice() else {
        panic!("one attempt expected, got {}", detail.attempts.len());
    };
    assert_eq!(attempt.upstream, "anthropic-key");
    assert_eq!(attempt.provider, Some(Stored::Known(Provider::Anthropic)));
    assert_eq!(attempt.changes.as_ref().unwrap()["auth"], "operator_key");
}

#[tokio::test]
async fn a_chat_completions_entry_streams_and_records_model_usage_and_tool_calls() {
    use ren_protocol::inspect::{Coverage, ModelOutcome, TerminalEvent};
    use ren_store::activity::ActivityBudget;
    // Two tool calls whose arguments arrive in fragments, then the usage
    // chunk the request asked for, then `[DONE]`.
    const STREAM: &str = concat!(
        "data: {\"object\":\"chat.completion.chunk\",\"model\":\"fixture-chat\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Checking.\"},\"finish_reason\":null}]}\n\n",
        "data: {\"object\":\"chat.completion.chunk\",\"model\":\"fixture-chat\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_read\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"object\":\"chat.completion.chunk\",\"model\":\"fixture-chat\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_list\",\"type\":\"function\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"object\":\"chat.completion.chunk\",\"model\":\"fixture-chat\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\":\\\"a.rs\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"object\":\"chat.completion.chunk\",\"model\":\"fixture-chat\",\"choices\":[],\"usage\":{\"prompt_tokens\":42,\"completion_tokens\":17,\"total_tokens\":59,\"prompt_tokens_details\":{\"cached_tokens\":32}}}\n\n",
        "data: [DONE]\n\n",
    );
    let mock = mock(Router::new().fallback(|request: Request| async move {
        assert_eq!(request.uri().path(), "/chat/v1/chat/completions");
        let auth = request.headers()["authorization"]
            .to_str()
            .unwrap()
            .to_owned();
        Response::builder()
            .header("content-type", "text/event-stream")
            .header("x-seen-auth", auth)
            .body(Body::from(STREAM))
            .unwrap()
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&mock, "");
    config.upstreams[0] = UpstreamConfig {
        id: UpstreamId::parse("deepseek-chat").unwrap(),
        provider: Provider::DeepSeek,
        wire: Some(Wire::OpenAiChatCompletions),
        base_url: format!("http://{}/chat", mock.address),
        operator_key_env: TestGateway::OPERATOR_KEY_ENV.into(),
        ingress: EntryIngress::Direct,
        users: None,
    };
    let gateway = TestGateway::start(&config, dir.path()).await;
    let request = r#"{"model":"fixture-chat","stream":true,"stream_options":{"include_usage":true},"messages":[{"role":"user","content":"Read a.rs"}]}"#;
    let response = client()
        .post(format!(
            "{}/upstreams/deepseek-chat/v1/chat/completions",
            gateway.base()
        ))
        .header("authorization", "Bearer CLIENT-KEY")
        .body(request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["x-seen-auth"],
        format!("Bearer {}", TestGateway::OPERATOR_KEY)
    );
    // Forwarded byte for byte.
    assert_eq!(response.bytes().await.unwrap(), STREAM.as_bytes());
    gateway.stop().await;

    let calls = recorded(dir.path()).await;
    let call = &calls[0];
    assert_eq!(call.provider, Stored::Known(Provider::DeepSeek));
    assert_eq!(call.wire, Some(Stored::Known(Wire::OpenAiChatCompletions)));
    let detail = detail(dir.path(), call.id).await;
    let [attempt] = detail.attempts.as_slice() else {
        panic!("one attempt expected, got {}", detail.attempts.len());
    };
    let inspection = attempt.inspection.as_ref().unwrap();
    assert_eq!(inspection.coverage, Stored::Known(Coverage::Complete));
    assert_eq!(
        inspection.terminal_event,
        Stored::Known(TerminalEvent::Seen)
    );
    assert_eq!(
        inspection.outcome,
        Some(Stored::Known(ModelOutcome::Completed))
    );
    assert_eq!(inspection.model.as_deref(), Some("fixture-chat"));
    let usage = inspection.usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens
        ),
        (Some(42), Some(17), Some(32))
    );
    let names: Vec<_> = inspection
        .tool_calls
        .iter()
        .map(|call| (call.call_id.as_deref(), call.name.as_deref()))
        .collect();
    assert_eq!(
        names,
        [
            (Some("call_read"), Some("read_file")),
            (Some("call_list"), Some("list_dir"))
        ]
    );

    // The digest assembled each call's fragmented arguments.
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let activity = reader
        .call_activity(call.id, ActivityBudget::DEFAULT)
        .await
        .unwrap();
    reader.close().await;
    let activity = serde_json::to_value(&activity).unwrap();
    assert_eq!(activity["model"], "fixture-chat");
    assert_eq!(activity["tool_calls"][0]["name"], "read_file");
    assert!(
        activity["tool_calls"][0]["input"]
            .to_string()
            .contains("a.rs"),
        "{activity}"
    );
    assert_eq!(activity["tool_calls"][1]["name"], "list_dir");
    assert_eq!(activity["limitations"], serde_json::json!([]));
}

#[tokio::test]
async fn statuses_the_policy_does_not_list_pass_through_without_failover() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(StatusCode::SERVICE_UNAVAILABLE, reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&mock, POLICY), dir.path()).await;
    let response = post(&gateway, r#"{"model":"fixture-sonnet"}"#).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 1);
    let calls = recorded(dir.path()).await;
    assert_eq!(detail(dir.path(), calls[0].id).await.attempts.len(), 1);
}

#[tokio::test]
async fn blocked_and_oversized_requests_are_refused_and_recorded_without_dispatch() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(StatusCode::OK, reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&mock, POLICY);
    config.buffer.request_limits = ren_gateway::RequestLimits::all(1024);
    let gateway = TestGateway::start(&config, dir.path()).await;

    let blocked = post(&gateway, r#"{"model":"fixture-opus"}"#).await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    assert_eq!(blocked.headers()["x-should-retry"], "false");
    let body: serde_json::Value = read(blocked).await;
    assert_eq!(body["error"]["type"], "ren_blocked");

    let oversized = client()
        .post(format!("{}/anthropic/v1/messages", gateway.base()))
        .body(vec![b' '; 2048])
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    gateway.stop().await;
    assert_eq!(
        reached.load(Ordering::SeqCst),
        0,
        "nothing reached the upstream"
    );

    let calls = recorded(dir.path()).await;
    let rejections: Vec<_> = calls.iter().map(rejection).collect();
    assert_eq!(
        rejections,
        [Some(CallRejection::TooLarge), Some(CallRejection::Blocked)]
    );
    for call in &calls {
        assert!(detail(dir.path(), call.id).await.attempts.is_empty());
    }
    // The blocked request was read whole and kept; the plan names the rule.
    let blocked = &calls[1];
    assert_eq!(blocked.plan.as_ref().unwrap()["rule"], "no-opus");
    assert!(blocked.request.as_ref().unwrap().complete());
}

async fn read(response: reqwest::Response) -> serde_json::Value {
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

/// A control API request with the gateway's token.
fn control(gateway: &TestGateway, path: &str) -> reqwest::RequestBuilder {
    client()
        .post(format!("{}/control/v1/{path}", gateway.base()))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
}

#[tokio::test]
async fn the_control_api_refuses_browsers_strangers_and_missing_tokens() {
    let mock = mock(upstream(StatusCode::OK, Arc::default())).await;
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&mock, "");
    config.control.tailscale_logins = vec!["operator@example.com".into()];
    let gateway = TestGateway::start(&config, dir.path()).await;
    let state = format!("{}/control/v1/state", gateway.base());
    let status =
        |request: reqwest::RequestBuilder| async move { request.send().await.unwrap().status() };
    assert_eq!(status(client().get(&state)).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        status(client().get(&state).bearer_auth("wrong")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(
            client()
                .get(&state)
                .bearer_auth(TestGateway::CONTROL_TOKEN)
                .header("origin", "https://evil.example")
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(
            client()
                .get(&state)
                .header("tailscale-user-login", "someone@example.com")
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(
            client()
                .get(&state)
                .header("tailscale-user-login", "operator@example.com")
        )
        .await,
        StatusCode::OK
    );
    let response = client()
        .get(&state)
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    gateway.stop().await;
    assert_eq!(operator_actions(dir.path()).await, 0);
}

#[tokio::test]
async fn pause_and_session_blocks_refuse_calls_and_are_recorded() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(StatusCode::OK, reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&mock, ""), dir.path()).await;

    let paused = control(&gateway, "pause")
        .header("content-type", "application/json")
        .body(serde_json::json!({ "retry_after_seconds": 30 }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(paused.status(), StatusCode::OK);
    let response = post(&gateway, "{}").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["retry-after"], "30");
    // The read API reports the pause without control credentials.
    let health = |gateway: &TestGateway| {
        let url = format!("{}/health", gateway.base());
        async move { read(client().get(url).send().await.unwrap()).await }
    };
    let reported: serde_json::Value = health(&gateway).await;
    assert_eq!(reported["instance"]["environment"], "production");
    assert_eq!(
        reported["operator"],
        serde_json::json!({ "pause": { "state": "paused", "retry_after": 30 }, "blocked_sessions": 0, "blocked_users": 0 })
    );
    assert_eq!(
        control(&gateway, "resume").send().await.unwrap().status(),
        StatusCode::OK
    );

    let blocked = control(&gateway, &format!("sessions/{SESSION}/block"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "duration_seconds": 600 }).to_string())
        .send()
        .await
        .unwrap();
    let state: serde_json::Value = read(blocked).await;
    assert_eq!(state["state"]["revision"], 3);
    let response = post(&gateway, "{}").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()["x-should-retry"], "false");
    let reported: serde_json::Value = health(&gateway).await;
    assert_eq!(reported["operator"]["pause"]["state"], "running");
    assert_eq!(reported["operator"]["blocked_sessions"], 1);
    control(&gateway, &format!("sessions/{SESSION}/unblock"))
        .send()
        .await
        .unwrap();
    assert_eq!(post(&gateway, "{}").await.status(), StatusCode::OK);
    // A malformed command changes nothing.
    let invalid = control(&gateway, "pause")
        .header("content-type", "application/json")
        .body(serde_json::json!({ "retry_after_seconds": 0 }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 1);

    let calls = recorded(dir.path()).await;
    let summary: Vec<_> = calls
        .iter()
        .map(|call| (rejection(call), call.operator_revision))
        .collect();
    assert_eq!(
        summary,
        [
            (None, Some(4)),
            (Some(CallRejection::SessionBlocked), Some(3)),
            (Some(CallRejection::Paused), Some(1)),
        ]
    );
    assert_eq!(operator_actions(dir.path()).await, 4);
}

/// The id of the one running call, once the gateway registered it.
async fn running(gateway: &TestGateway) -> String {
    timeout(Duration::from_secs(5), async {
        loop {
            let state = client()
                .get(format!("{}/control/v1/state", gateway.base()))
                .bearer_auth(TestGateway::CONTROL_TOKEN)
                .send()
                .await
                .unwrap();
            let state = read(state).await;
            if let Some(call) = state["running"][0]["call"].as_str() {
                return call.to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn an_operator_cancel_before_the_head_is_a_ren_error_and_can_block_the_session() {
    let release = Arc::new(Notify::new());
    let held = release.clone();
    let mock = mock(Router::new().fallback(move || {
        let held = held.clone();
        async move {
            held.notified().await;
            "late"
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = Arc::new(TestGateway::start(&config(&mock, ""), dir.path()).await);
    let sending = tokio::spawn({
        let gateway = gateway.clone();
        async move { post(&gateway, "{}").await }
    });
    let call = running(&gateway).await;
    let cancelled = control(&gateway, &format!("calls/{call}/cancel"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "block_session_seconds": 60 }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let response = sending.await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers()["x-should-retry"], "false");
    let body: serde_json::Value = read(response).await;
    assert_eq!(body["error"]["type"], "ren_operator_cancelled");
    // The client's retry is refused by the session block.
    assert_eq!(post(&gateway, "{}").await.status(), StatusCode::FORBIDDEN);
    // A finished call is no longer cancellable.
    let again = control(&gateway, &format!("calls/{call}/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    release.notify_waiters();
    Arc::into_inner(gateway).unwrap().stop().await;
    let calls = recorded(dir.path()).await;
    let cancelled = calls.iter().find(|c| c.id.to_string() == call).unwrap();
    assert_eq!(
        ending(dir.path(), cancelled.id).await,
        (
            AttemptTerminal::Cancelled,
            Some(TerminalCause::Operator),
            Some(AttemptPhase::AwaitingHead)
        )
    );
    // Cancel, then block: two recorded actions.
    assert_eq!(operator_actions(dir.path()).await, 2);
}

#[tokio::test]
async fn an_operator_cancel_while_streaming_interrupts_the_stream() {
    let mock = mock(Router::new().fallback(|| async {
        let stream = futures_util::stream::once(async {
            Ok::<_, std::io::Error>(Bytes::from_static(b"event: one\n\n"))
        })
        .chain(futures_util::stream::pending());
        Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(stream))
            .unwrap()
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&mock, ""), dir.path()).await;
    let mut response = post(&gateway, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &response.chunk().await.unwrap().unwrap()[..],
        b"event: one\n\n"
    );
    let call = running(&gateway).await;
    control(&gateway, &format!("calls/{call}/cancel"))
        .send()
        .await
        .unwrap();
    assert!(response.chunk().await.is_err(), "the stream is interrupted");
    gateway.stop().await;
    let calls = recorded(dir.path()).await;
    assert_eq!(
        ending(dir.path(), calls[0].id).await,
        (
            AttemptTerminal::Cancelled,
            Some(TerminalCause::Operator),
            Some(AttemptPhase::Response)
        )
    );
}

use futures_util::StreamExt as _;

#[tokio::test]
async fn a_backup_through_the_control_api_is_a_readable_copy() {
    let mock = mock(upstream(StatusCode::OK, Arc::default())).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&mock, ""), dir.path()).await;
    assert_eq!(post(&gateway, "{}").await.status(), StatusCode::OK);
    let url = format!("{}/control/v1/backup", gateway.base());
    // The same access rules as every control action.
    for request in [
        client().post(&url),
        client()
            .post(&url)
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .header("origin", "https://evil.example"),
    ] {
        assert!(request.send().await.unwrap().status().is_client_error());
    }
    assert!(!dir.path().join("backup").exists());
    // The recorder commits the call shortly after the response ends.
    let report = timeout(Duration::from_secs(5), async {
        loop {
            let response = control(&gateway, "backup").send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let report = read(response).await;
            if report["backup"]["chunks"].as_u64() > Some(0) {
                return report;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(report["api_version"], 1);
    gateway.stop().await;
    let backup = std::path::PathBuf::from(report["backup"]["directory"].as_str().unwrap());
    assert_eq!(backup, dir.path().join("backup"));
    let copy = ren_store::Reader::open(&backup, 1).await.unwrap();
    let calls = copy
        .calls(&ren_store::query::CallQuery::default())
        .await
        .unwrap();
    assert_eq!(calls.calls.len(), 1);
}

/// The default Anthropic upstream refuses every stream (HTTP/2
/// `REFUSED_STREAM`); every other upstream answers `200`.
struct RefusingPrimary {
    sent: std::sync::Mutex<Vec<UpstreamId>>,
}

impl ren_gateway::Upstream for RefusingPrimary {
    async fn send(
        &self,
        request: ren_gateway::UpstreamRequest,
    ) -> Result<http::Response<ren_gateway::UpstreamBody>, ren_gateway::attempt::SendFailure> {
        self.sent.lock().unwrap().push(request.upstream.clone());
        if request.upstream == UpstreamId::parse("anthropic").unwrap() {
            return Err(ren_gateway::attempt::SendFailure::Refused);
        }
        Ok(http::Response::new(ren_gateway::UpstreamBody::full("{}")))
    }

    async fn connect(
        &self,
        _handshake: ren_gateway::UpstreamHandshake,
    ) -> Result<ren_gateway::Connected, ren_gateway::attempt::SendFailure> {
        panic!("this scenario sends no WebSocket handshake")
    }
}

#[tokio::test]
async fn a_refused_stream_falls_over_like_a_refused_connection() {
    let mock = mock(upstream(StatusCode::OK, Arc::default())).await;
    let dir = tempfile::tempdir().unwrap();
    let scripted = Arc::new(RefusingPrimary {
        sent: std::sync::Mutex::default(),
    });
    // The upstream never answers 529, so only the refused stream can fall over.
    let policy = r#"
[[fallback]]
id = "refused"
provider = "anthropic"
to = "anthropic-key"
on_status = [529]
"#;
    let gateway =
        TestGateway::with_upstream(&config(&mock, policy), dir.path(), scripted.clone()).await;
    let response = post(&gateway, r#"{"model":"fixture-sonnet"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    gateway.stop().await;
    let sent: Vec<String> = scripted
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(sent, ["anthropic", "anthropic-key"]);

    let call = &recorded(dir.path()).await[0];
    let detail = detail(dir.path(), call.id).await;
    let [first, second] = detail.attempts.as_slice() else {
        panic!("two attempts expected, got {}", detail.attempts.len());
    };
    assert_eq!(
        first.terminal.as_ref().and_then(Stored::known),
        Some(AttemptTerminal::Refused)
    );
    assert_eq!(
        first.terminal_cause.as_ref().and_then(Stored::known),
        Some(TerminalCause::StreamRefused)
    );
    assert_eq!(second.status, Some(200));
}
