//! Callers end to end: each forwarded call records who made it, identity
//! headers never reach the provider, and a refused identity is answered and
//! recorded without forwarding. Assembled with `TestGateway` against a
//! loopback mock; tokens and logins are fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]

use axum::{Router, extract::Request, response::IntoResponse};
use http::StatusCode;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tgv_daemon::config::{Config, UpstreamConfig};
use tgv_gateway::EntryIngress;
use tgv_gateway::user::{TokenDigest, UserConfig, Users};
use tgv_harness::endpoint::UserToken;
use tgv_protocol::{label::Stored, route::Provider};
use tgv_recorder::{
    caller::{CallerKind, TailscaleLogin, UserId},
    capture::CallRejection,
    id::SequentialIds,
    routing::UpstreamId,
};
use tgv_store::{
    Reader,
    query::{CallFilter, CallQuery, TimeRange},
    view::CallView,
};

mod support;

use support::{Mock, TestGateway, client, mock, recorded};

struct Tokens {
    bob: UserToken,
    unknown: UserToken,
}

fn tokens() -> Tokens {
    let ids = SequentialIds::default();
    Tokens {
        bob: Users::new_token(&ids),
        unknown: Users::new_token(&ids),
    }
}

fn config(mock: &Mock, tokens: &Tokens) -> Config {
    Config {
        users: vec![
            UserConfig {
                id: UserId::parse("alice").unwrap(),
                tailscale_logins: vec![TailscaleLogin::parse("alice@example.com").unwrap()],
                token_blake3: None,
                max_concurrent: None,
            },
            UserConfig {
                id: UserId::parse("bob").unwrap(),
                tailscale_logins: vec![],
                token_blake3: Some(TokenDigest::of(&tokens.bob)),
                max_concurrent: None,
            },
        ],
        ..support::config(mock)
    }
}

/// Answers every request, saying whether identity headers reached it.
fn upstream(reached: Arc<AtomicUsize>) -> Router {
    Router::new().fallback(move |request: Request| {
        let reached = reached.clone();
        async move {
            reached.fetch_add(1, Ordering::SeqCst);
            let leaked = request.headers().keys().any(|name| {
                name.as_str().starts_with("x-tgv-") || name.as_str().starts_with("tailscale-")
            });
            (StatusCode::OK, if leaked { "leaked" } else { "clean" }).into_response()
        }
    })
}

async fn post(gateway: &TestGateway, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut request = client()
        .post(format!("{}/anthropic/v1/messages", gateway.base()))
        .header("authorization", "Bearer CLIENT-OAUTH")
        .body(r#"{"model":"fixture","messages":[]}"#);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.unwrap()
}

fn caller(call: &CallView) -> (CallerKind, Option<&str>, Option<&str>) {
    let caller = call.caller.as_ref().unwrap();
    (
        caller.kind.known().unwrap(),
        caller.user.as_deref(),
        caller.login.as_deref(),
    )
}

#[tokio::test]
async fn every_call_records_its_caller_and_identity_never_reaches_the_provider() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let gateway = TestGateway::start(&config(&mock, &tokens), dir.path()).await;
    let requests: [&[(&str, &str)]; 4] = [
        &[("tailscale-user-login", "alice@example.com")],
        &[(UserToken::HEADER, tokens.bob.expose())],
        &[("tailscale-user-login", "carol@example.com")],
        &[],
    ];
    for headers in requests {
        let response = post(&gateway, headers).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "clean");
    }
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 4);

    let mut calls = recorded(dir.path()).await;
    calls.reverse();
    let callers: Vec<_> = calls.iter().map(caller).collect();
    assert_eq!(
        callers,
        [
            (
                CallerKind::UserTailscale,
                Some("alice"),
                Some("alice@example.com")
            ),
            (CallerKind::UserToken, Some("bob"), None),
            (CallerKind::Unlisted, None, Some("carol@example.com")),
            (CallerKind::Unidentified, None, None),
        ]
    );

    // History filters by user, and usage counts each user's calls.
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let query = CallQuery {
        filter: CallFilter {
            user: Some(UserId::parse("bob").unwrap()),
            ..CallFilter::default()
        },
        ..CallQuery::default()
    };
    let bobs = reader.calls(&query).await.unwrap().calls;
    assert_eq!(bobs.len(), 1);
    assert_eq!(bobs[0].id, calls[1].id);
    let mut usage: Vec<(Option<String>, u64)> = reader
        .usage(TimeRange::default())
        .await
        .unwrap()
        .users
        .into_iter()
        .map(|user| (user.user, user.usage.calls))
        .collect();
    usage.sort();
    assert_eq!(
        usage,
        [
            (None, 2),
            (Some("alice".to_owned()), 1),
            (Some("bob".to_owned()), 1)
        ]
    );
    reader.close().await;
}

#[tokio::test]
async fn a_refused_identity_is_answered_and_recorded_without_forwarding() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let gateway = TestGateway::start(&config(&mock, &tokens), dir.path()).await;
    let refused: [&[(&str, &str)]; 2] = [
        &[(UserToken::HEADER, tokens.unknown.expose())],
        &[
            ("tailscale-user-login", "alice@example.com"),
            (UserToken::HEADER, tokens.bob.expose()),
        ],
    ];
    for headers in refused {
        let response = post(&gateway, headers).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["error"]["type"], "tgv_identity_refused");
        let message = body["error"]["message"].as_str().unwrap();
        assert!(!message.contains("tgvu_"), "{message}");
    }
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 0);

    let calls = recorded(dir.path()).await;
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(
            call.rejection.as_ref().and_then(Stored::known),
            Some(CallRejection::IdentityRefused)
        );
        assert_eq!(caller(call), (CallerKind::Unidentified, None, None));
    }
}

#[tokio::test]
async fn an_entry_for_some_users_refuses_everyone_else_before_forwarding() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let config = Config {
        upstreams: vec![UpstreamConfig {
            id: UpstreamId::parse("team-key").unwrap(),
            provider: Provider::Anthropic,
            wire: None,
            base_url: format!("http://{}", mock.address),
            operator_key_env: TestGateway::OPERATOR_KEY_ENV.into(),
            ingress: EntryIngress::Direct,
            users: Some(vec![UserId::parse("bob").unwrap()]),
        }],
        ..config(&mock, &tokens)
    };
    let gateway = TestGateway::start(&config, dir.path()).await;
    let send = |headers: &'static [(&'static str, &'static str)], token: Option<&str>| {
        let mut request = client()
            .post(format!("{}/upstreams/team-key/v1/messages", gateway.base()))
            .body(r#"{"model":"fixture","messages":[]}"#);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(token) = token {
            request = request.header(UserToken::HEADER, token);
        }
        request.send()
    };
    let bob = send(&[], Some(tokens.bob.expose())).await.unwrap();
    assert_eq!(bob.status(), StatusCode::OK);
    for refused in [
        send(&[("tailscale-user-login", "alice@example.com")], None),
        send(&[], None),
    ] {
        let response = refused.await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["error"]["type"], "tgv_user_not_allowed");
    }
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 1);

    let calls = recorded(dir.path()).await;
    let rejections: Vec<_> = calls
        .iter()
        .map(|call| call.rejection.as_ref().and_then(Stored::known))
        .collect();
    assert_eq!(
        rejections,
        [
            Some(CallRejection::UserNotAllowed),
            Some(CallRejection::UserNotAllowed),
            None
        ]
    );
}

fn control(gateway: &TestGateway, path: &str) -> reqwest::RequestBuilder {
    client()
        .post(format!("{}/control/v1/{path}", gateway.base()))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
}

#[tokio::test]
async fn an_operator_blocks_a_user_at_once_and_unblocks_them() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let gateway = TestGateway::start(&config(&mock, &tokens), dir.path()).await;
    let bob: &[(&str, &str)] = &[(UserToken::HEADER, tokens.bob.expose())];

    let blocked = control(&gateway, "users/bob/block").send().await.unwrap();
    assert_eq!(blocked.status(), StatusCode::OK);
    let refused = post(&gateway, bob).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(refused.headers()["x-should-retry"], "false");
    // Other callers are unaffected.
    assert_eq!(post(&gateway, &[]).await.status(), StatusCode::OK);
    let health: serde_json::Value = serde_json::from_str(
        &client()
            .get(format!("{}/health", gateway.base()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(health["operator"]["blocked_users"], 1);
    // Only configured users can be blocked.
    assert_eq!(
        control(&gateway, "users/carol/block")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let unblocked = control(&gateway, "users/bob/unblock").send().await.unwrap();
    assert_eq!(unblocked.status(), StatusCode::OK);
    assert_eq!(post(&gateway, bob).await.status(), StatusCode::OK);
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 2);

    let calls = recorded(dir.path()).await;
    let rejections: Vec<_> = calls
        .iter()
        .map(|call| call.rejection.as_ref().and_then(Stored::known))
        .collect();
    assert_eq!(rejections, [None, None, Some(CallRejection::UserBlocked)]);
}

#[tokio::test]
async fn a_user_at_its_limit_gets_429_until_a_call_ends() {
    let reached = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(tokio::sync::Notify::new());
    let held = gate.clone();
    let counter = reached.clone();
    let app = Router::new().fallback(move || {
        let gate = held.clone();
        let reached = counter.clone();
        async move {
            if reached.fetch_add(1, Ordering::SeqCst) == 0 {
                gate.notified().await;
            }
            "done"
        }
    });
    let mock = mock(app).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let mut config = config(&mock, &tokens);
    config.users[1].max_concurrent = std::num::NonZeroU32::new(1);
    let gateway = TestGateway::start(&config, dir.path()).await;
    let bob: &[(&str, &str)] = &[(UserToken::HEADER, tokens.bob.expose())];

    let first = {
        let request = post(&gateway, bob);
        async move { request.await.text().await.unwrap() }
    };
    let second = async {
        while reached.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let busy = post(&gateway, bob).await;
        assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(busy.headers()["retry-after"], "1");
        // Another caller is not limited by bob's calls.
        assert_eq!(post(&gateway, &[]).await.status(), StatusCode::OK);
        gate.notify_one();
    };
    let (first, ()) = tokio::join!(first, second);
    assert_eq!(first, "done");
    assert_eq!(post(&gateway, bob).await.status(), StatusCode::OK);
    gateway.stop().await;

    let calls = recorded(dir.path()).await;
    let busy = calls
        .iter()
        .filter(|call| {
            call.rejection.as_ref().and_then(Stored::known) == Some(CallRejection::UserBusy)
        })
        .count();
    assert_eq!((calls.len(), busy), (4, 1));
}
